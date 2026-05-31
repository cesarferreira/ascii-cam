use std::io::{Write, stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, size,
};

use crate::capture::{
    CameraDevice, FfmpegCapture, Platform, Resolution, discover_cameras, ensure_supported_platform,
    resolve_capture_dimensions,
};
use crate::color::ColorMode;
use crate::recording::{RecordingDecoder, RecordingEncoder, RecordingOptions};
use crate::render::{
    BOTTOM_BAR_LINES, CHAR_ASPECT_FALLBACK, RAMP_LONG, RAMP_SHORT, RenderConfig, RenderedFrame,
    TOP_BAR_LINES, compute_render_size, render_frame,
};
use crate::screenshot::write_html;
use crate::ui::{Shortcut, center_ansi_line, center_block, pad_ansi_line, shortcut_bar};

#[derive(Clone, Debug, Args)]
#[command(
    about = "Broadcast ASCII frames over HTTP",
    long_about = "Stream the camera as ANSI text. Defaults listen on all interfaces so other \
                  machines on your LAN or Tailscale can watch.\n\n\
                  Examples:\n  \
                  ascii-cam serve --token mytoken\n  \
                  ascii-cam serve --local\n  \
                  make serve ARGS=\"--token mytoken\""
)]
pub struct ServeArgs {
    #[arg(long, default_value_t = 8080, help = "HTTP port")]
    pub port: u16,
    #[arg(
        long,
        default_value = "0.0.0.0",
        help = "Address to bind (default: all interfaces; use with --local for loopback only)"
    )]
    pub bind: String,
    #[arg(
        long,
        help = "Listen on 127.0.0.1 only — not reachable from other machines",
        conflicts_with = "bind"
    )]
    pub local: bool,
    #[arg(
        long,
        help = "Shared secret; viewers must pass ?token=VALUE on /stream and /"
    )]
    pub token: Option<String>,
    #[arg(long, default_value_t = 120, help = "ASCII frame width in characters")]
    pub cols: usize,
    #[arg(long, default_value_t = 40, help = "ASCII frame height in characters")]
    pub rows: usize,
    #[arg(
        long,
        help = "Send a full terminal clear (\\x1b[2J) once when each viewer connects"
    )]
    pub clear_each_frame: bool,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Command {
    /// Broadcast ASCII frames over HTTP (curl or browser)
    Serve(ServeArgs),
}

#[derive(Clone, Parser, Debug)]
#[command(version, about = "Real-time ASCII camera for the terminal")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[arg(long, value_enum, default_value_t = Resolution::Medium)]
    pub resolution: Resolution,
    #[arg(long)]
    pub camera: Option<u32>,
    #[arg(long, help = "Open an interactive camera picker before starting")]
    pub pick_camera: bool,
    #[arg(long, value_enum, default_value_t = Platform::Auto)]
    pub platform: Platform,
    #[arg(long, default_value_t = 30)]
    pub fps: u8,
    #[arg(long, default_value_t = 1.0)]
    pub contrast: f32,
    #[arg(long, default_value_t = 0)]
    pub brightness: i16,
    #[arg(long, default_value = "long")]
    pub ramp: RampChoice,
    #[arg(long)]
    pub invert: bool,
    #[arg(long, default_value_t = 0)]
    pub rotate: u8,
    #[arg(long, value_enum, default_value_t = ColorMode::TrueColor)]
    pub color: ColorMode,
    #[arg(long)]
    pub no_color: bool,
    #[arg(long)]
    pub record: Option<PathBuf>,
    #[arg(long)]
    pub play: Option<PathBuf>,
    #[arg(long, default_value_t = CHAR_ASPECT_FALLBACK)]
    pub char_aspect: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum RampChoice {
    Long,
    Short,
}

impl std::str::FromStr for RampChoice {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "long" => Ok(Self::Long),
            "short" => Ok(Self::Short),
            other => Err(format!("expected long or short, got {other}")),
        }
    }
}

impl std::fmt::Display for RampChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Long => write!(f, "long"),
            Self::Short => write!(f, "short"),
        }
    }
}

pub fn run(cli: Cli) -> Result<()> {
    if let Some(Command::Serve(args)) = cli.command.clone() {
        return crate::serve::run(&cli, args);
    }
    if let Some(path) = cli.play {
        return play_recording(path);
    }
    let mut app = LiveApp::new(cli)?;
    if app.cli.pick_camera {
        app.camera_index = pick_camera_interactive(app.platform, app.camera_index)?;
    }
    save_last_camera(app.camera_index);
    app.run()
}

#[derive(Clone, Copy, Debug)]
enum Preset {
    Raw,
    Max,
    Gray,
    Ascii,
    Green,
    GreenGradient,
    Red,
    RedGradient,
}

impl Preset {
    const CYCLE: [Self; 8] = [
        Self::Raw,
        Self::Max,
        Self::Gray,
        Self::Ascii,
        Self::Green,
        Self::GreenGradient,
        Self::Red,
        Self::RedGradient,
    ];

    fn next(self) -> Self {
        let index = Self::CYCLE
            .iter()
            .position(|preset| *preset as u8 == self as u8)
            .unwrap_or(0);
        Self::CYCLE[(index + 1) % Self::CYCLE.len()]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Raw => "Raw",
            Self::Max => "Max",
            Self::Gray => "Gray",
            Self::Ascii => "Ascii",
            Self::Green => "Green",
            Self::GreenGradient => "GreenGrad",
            Self::Red => "Red",
            Self::RedGradient => "RedGrad",
        }
    }
}

struct LiveApp {
    cli: Cli,
    platform: Platform,
    camera_index: u32,
    color_mode: ColorMode,
    preset: Preset,
    contrast: f32,
    brightness: i16,
    invert: bool,
    rotation: u8,
    recording_options: RecordingOptions,
    encoder: Option<RecordingEncoder>,
    recording_dimensions: Option<(usize, usize)>,
    record_path: Option<PathBuf>,
    show_help: bool,
    show_settings: bool,
    open_picker: bool,
    last_rendered: Option<RenderedFrame>,
}

impl LiveApp {
    fn new(cli: Cli) -> Result<Self> {
        let platform = ensure_supported_platform(cli.platform)?;
        let color_mode = if cli.no_color {
            ColorMode::Off
        } else {
            cli.color
        };
        let camera_index = cli
            .camera
            .unwrap_or_else(|| load_last_camera().unwrap_or(0));
        Ok(Self {
            contrast: cli.contrast,
            brightness: cli.brightness,
            invert: cli.invert,
            rotation: cli.rotate % 4,
            record_path: cli.record.clone(),
            cli,
            platform,
            camera_index,
            color_mode,
            preset: Preset::Raw,
            recording_options: RecordingOptions::default(),
            encoder: None,
            recording_dimensions: None,
            show_help: false,
            show_settings: false,
            open_picker: false,
            last_rendered: None,
        })
    }

    fn run(mut self) -> Result<()> {
        let (cam_w, cam_h) = self.cli.resolution.dimensions();
        let (cam_w, cam_h) =
            resolve_capture_dimensions(self.platform, self.camera_index, cam_w, cam_h);
        let mut capture =
            FfmpegCapture::spawn(self.platform, self.camera_index, self.cli.fps, cam_w, cam_h)?;
        let _terminal = TerminalGuard::enter()?;
        let key_events = spawn_key_reader();
        let mut out = stdout();
        let mut frame_counter = 0_u32;
        let mut fps_window = Instant::now();
        let mut fps_actual = 0.0_f32;
        let started = Instant::now();

        loop {
            while let Ok(key) = key_events.try_recv() {
                if self.handle_key(key)? {
                    return Ok(());
                }
                if self.open_picker {
                    break;
                }
            }

            if std::mem::take(&mut self.open_picker) {
                if let Some(new_index) =
                    pick_camera_in_session(self.platform, &key_events, self.camera_index, &mut out)?
                    && new_index != self.camera_index
                {
                    let (w, h) = self.cli.resolution.dimensions();
                    let (w, h) = resolve_capture_dimensions(self.platform, new_index, w, h);
                    let new_capture =
                        FfmpegCapture::spawn(self.platform, new_index, self.cli.fps, w, h)?;
                    capture = new_capture;
                    self.camera_index = new_index;
                    save_last_camera(new_index);
                    self.last_rendered = None;
                }
                continue;
            }

            let frame = capture.read_frame()?.rotate(self.rotation);
            let (term_cols, term_rows) = size()?;
            let image_rows = term_rows
                .saturating_sub(TOP_BAR_LINES + BOTTOM_BAR_LINES)
                .max(1);
            let (cols, rows) = compute_render_size(
                term_cols as usize,
                image_rows as usize,
                frame.width,
                frame.height,
                self.cli.char_aspect,
            );
            let config = RenderConfig {
                cols,
                rows,
                ramp: match self.cli.ramp {
                    RampChoice::Long => RAMP_LONG.to_string(),
                    RampChoice::Short => RAMP_SHORT.to_string(),
                },
                color_mode: self.color_mode,
                contrast: self.contrast,
                brightness: self.brightness,
                invert: self.invert,
            };
            let rendered = render_frame(&frame, &config);

            if self.encoder.is_some() {
                match recording_frame_action(
                    self.recording_dimensions,
                    (rendered.width, rendered.height),
                ) {
                    RecordingFrameAction::Write => {
                        if let Some(encoder) = &mut self.encoder {
                            encoder
                                .write_frame_at(started.elapsed().as_millis() as u64, &rendered)?;
                        }
                    }
                    RecordingFrameAction::Stop => self.stop_recording()?,
                }
            }

            frame_counter += 1;
            let elapsed = fps_window.elapsed();
            if elapsed >= Duration::from_secs(1) {
                fps_actual = frame_counter as f32 / elapsed.as_secs_f32();
                frame_counter = 0;
                fps_window = Instant::now();
            }

            self.draw(&mut out, &rendered, term_cols, image_rows, fps_actual)?;
            self.last_rendered = Some(rendered);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.show_help {
            match key.code {
                KeyCode::Char('h') | KeyCode::Esc | KeyCode::Char('q') => self.show_help = false,
                _ => {}
            }
            return Ok(false);
        }
        if self.show_settings {
            self.handle_settings_key(key);
            return Ok(false);
        }
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('h') => self.show_help = true,
            KeyCode::Char('s') => self.show_settings = true,
            KeyCode::Char('1') => self.invert = !self.invert,
            KeyCode::Char('2') => self.rotation = (self.rotation + 1) % 4,
            KeyCode::Char('3') => self.toggle_recording()?,
            KeyCode::Char('4') => self.capture_screenshots()?,
            KeyCode::Char('5') => self.apply_preset(self.preset.next()),
            KeyCode::Char('c') => self.open_picker = true,
            KeyCode::Up => self.contrast = (self.contrast + 0.1).min(3.0),
            KeyCode::Down => self.contrast = (self.contrast - 0.1).max(0.1),
            KeyCode::Right => self.brightness = (self.brightness + 5).min(100),
            KeyCode::Left => self.brightness = (self.brightness - 5).max(-100),
            _ => {}
        }
        Ok(false)
    }

    fn handle_settings_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('s') => self.show_settings = false,
            KeyCode::Char('1') => self.color_mode = self.color_mode.next(),
            KeyCode::Char('2') => {
                self.recording_options.compress = !self.recording_options.compress
            }
            KeyCode::Char('3') => {
                self.recording_options.skip_identical = !self.recording_options.skip_identical
            }
            KeyCode::Char('4') => self.apply_preset(self.preset.next()),
            _ => {}
        }
    }

    fn apply_preset(&mut self, preset: Preset) {
        self.preset = preset;
        match preset {
            Preset::Raw => {
                self.color_mode = ColorMode::TrueColor;
                self.recording_options.compress = false;
                self.recording_options.skip_identical = false;
            }
            Preset::Max => {
                self.color_mode = ColorMode::Ansi256;
                self.recording_options.compress = true;
                self.recording_options.skip_identical = true;
            }
            Preset::Gray => self.color_mode = ColorMode::Gray,
            Preset::Ascii => self.color_mode = ColorMode::Off,
            Preset::Green => self.color_mode = ColorMode::Green,
            Preset::GreenGradient => self.color_mode = ColorMode::GreenGradient,
            Preset::Red => self.color_mode = ColorMode::Red,
            Preset::RedGradient => self.color_mode = ColorMode::RedGradient,
        }
    }

    fn toggle_recording(&mut self) -> Result<()> {
        if self.encoder.is_some() {
            return self.stop_recording();
        }
        let Some(frame) = &self.last_rendered else {
            return Ok(());
        };
        let path = self
            .record_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("recording-{}.ascicam", timestamp())));
        self.encoder = Some(RecordingEncoder::create(
            &path,
            frame.width,
            frame.height,
            self.cli.fps,
            self.recording_options,
        )?);
        self.recording_dimensions = Some((frame.width, frame.height));
        self.record_path = Some(path);
        Ok(())
    }

    fn stop_recording(&mut self) -> Result<()> {
        if let Some(encoder) = self.encoder.take() {
            encoder.finish()?;
        }
        self.recording_dimensions = None;
        Ok(())
    }

    fn capture_screenshots(&mut self) -> Result<()> {
        let Some(frame) = &self.last_rendered else {
            return Ok(());
        };
        let stamp = timestamp();
        write_html(format!("screenshot-{stamp}.html"), frame)?;
        let mut encoder = RecordingEncoder::create(
            format!("snapshot-{stamp}.ascicam"),
            frame.width,
            frame.height,
            1,
            self.recording_options,
        )?;
        encoder.write_frame_at(0, frame)?;
        encoder.finish()?;
        Ok(())
    }

    fn draw(
        &self,
        out: &mut std::io::Stdout,
        rendered: &RenderedFrame,
        term_cols: u16,
        image_rows: u16,
        fps_actual: f32,
    ) -> Result<()> {
        if self.show_help {
            execute!(
                out,
                MoveTo(0, 0),
                Clear(ClearType::All),
                Print(self.help_overlay(term_cols))
            )?;
        } else if self.show_settings {
            execute!(
                out,
                MoveTo(0, 0),
                Clear(ClearType::All),
                Print(self.settings_overlay(term_cols))
            )?;
        } else {
            execute!(
                out,
                MoveTo(0, 0),
                Print(self.status_bar(term_cols, fps_actual))
            )?;
            execute!(
                out,
                MoveTo(0, TOP_BAR_LINES),
                Print(center_block(
                    &rendered.terminal_text(),
                    term_cols as usize,
                    image_rows as usize
                )),
                Clear(ClearType::FromCursorDown)
            )?;
            execute!(
                out,
                MoveTo(0, TOP_BAR_LINES + image_rows),
                Print(self.shortcut_bar(term_cols))
            )?;
        }
        out.flush()?;
        Ok(())
    }

    fn status_bar(&self, term_cols: u16, fps_actual: f32) -> String {
        let rec = if self.encoder.is_some() { " | REC" } else { "" };
        let status = format!(
            "\x1b[1;38;2;156;224;236mASCII-CAM\x1b[0m  \x1b[38;2;191;197;255m{:>5.1} fps\x1b[0m  \x1b[38;2;255;237;181m{}\x1b[0m  \x1b[38;2;244;177;105m{}\x1b[0m  \x1b[38;2;176;232;190mcontrast {:>3.1}\x1b[0m  \x1b[38;2;202;170;246mbrightness {:+4}\x1b[0m  \x1b[38;2;255;184;208mrotation {}\x1b[0m  \x1b[38;2;191;222;255minvert {}\x1b[0m{}",
            fps_actual,
            self.color_mode.label(),
            self.preset.label(),
            self.contrast,
            self.brightness,
            self.rotation * 90,
            if self.invert { "on" } else { "off" },
            rec
        );
        center_ansi_line(&status, term_cols as usize)
    }

    fn shortcut_bar(&self, term_cols: u16) -> String {
        let shortcuts = shortcut_bar(&[
            Shortcut::new("1", "invert"),
            Shortcut::new("2", "rotate"),
            Shortcut::new("3", "record"),
            Shortcut::new("4", "capture"),
            Shortcut::new("5", "preset"),
            Shortcut::new("c", "camera"),
            Shortcut::new("s", "settings"),
            Shortcut::new("h", "help"),
            Shortcut::new("q", "quit"),
        ]);
        center_ansi_line(&shortcuts, term_cols as usize)
    }

    fn help_overlay(&self, term_cols: u16) -> String {
        [
            "ASCII-CAM HELP",
            "",
            "1 toggle invert",
            "2 rotate 0/90/180/270",
            "3 start or stop .ascicam recording",
            "4 save HTML screenshot and .ascicam snapshot",
            "5 cycle preset",
            "c switch camera",
            "arrow keys adjust contrast and brightness",
            "s settings",
            "h close help",
            "q quit",
        ]
        .into_iter()
        .map(|line| pad_ansi_line(line, term_cols as usize))
        .collect::<Vec<_>>()
        .join("\n")
    }

    fn settings_overlay(&self, term_cols: u16) -> String {
        [
            "ASCII-CAM SETTINGS",
            "",
            &format!("1 color mode: {}", self.color_mode.label()),
            &format!("2 compression: {}", on_off(self.recording_options.compress)),
            &format!(
                "3 skip identical frames: {}",
                on_off(self.recording_options.skip_identical)
            ),
            &format!("4 preset: {}", self.preset.label()),
            "",
            "s or Esc close",
        ]
        .into_iter()
        .map(|line| pad_ansi_line(line, term_cols as usize))
        .collect::<Vec<_>>()
        .join("\n")
    }
}

fn spawn_key_reader() -> Receiver<KeyEvent> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        loop {
            match event::read() {
                Ok(Event::Key(key)) if should_forward_key_event(key) => {
                    if tx.send(key).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    rx
}

fn should_forward_key_event(key: KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn pick_camera_interactive(platform: Platform, current: u32) -> Result<u32> {
    let devices = discover_cameras(platform)?;
    if devices.is_empty() {
        bail!("no cameras discovered; pass --camera N to choose a camera index manually");
    }
    let _terminal = TerminalGuard::enter()?;
    let mut out = stdout();
    match camera_picker_loop(&devices, current, &mut out, || {
        loop {
            match event::read() {
                Ok(Event::Key(key)) if should_forward_key_event(key) => return Ok(key),
                Ok(_) => continue,
                Err(err) => return Err(anyhow::Error::from(err)),
            }
        }
    })? {
        PickerOutcome::Selected(index) => Ok(index),
        PickerOutcome::Cancelled => bail!("camera picker cancelled"),
    }
}

fn pick_camera_in_session(
    platform: Platform,
    key_events: &Receiver<KeyEvent>,
    current: u32,
    out: &mut std::io::Stdout,
) -> Result<Option<u32>> {
    let devices = discover_cameras(platform)?;
    if devices.is_empty() {
        return Ok(None);
    }
    let outcome = camera_picker_loop(&devices, current, out, || {
        key_events
            .recv()
            .map_err(|err| anyhow::anyhow!("key reader stopped: {err}"))
    })?;
    Ok(match outcome {
        PickerOutcome::Selected(index) => Some(index),
        PickerOutcome::Cancelled => None,
    })
}

enum PickerOutcome {
    Selected(u32),
    Cancelled,
}

fn camera_picker_loop<F>(
    devices: &[CameraDevice],
    current: u32,
    out: &mut std::io::Stdout,
    mut next_key: F,
) -> Result<PickerOutcome>
where
    F: FnMut() -> Result<KeyEvent>,
{
    let mut selected = devices
        .iter()
        .position(|device| device.index == current)
        .unwrap_or(0);
    loop {
        let (cols, rows) = size()?;
        render_camera_picker(devices, selected, out, cols as usize, rows as usize)?;
        let key = next_key()?;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(devices.len() - 1),
            KeyCode::Home => selected = 0,
            KeyCode::End => selected = devices.len() - 1,
            KeyCode::Enter | KeyCode::Char(' ') => {
                return Ok(PickerOutcome::Selected(devices[selected].index));
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('c') => {
                return Ok(PickerOutcome::Cancelled);
            }
            _ => {}
        }
    }
}

fn render_camera_picker(
    devices: &[CameraDevice],
    selected: usize,
    out: &mut std::io::Stdout,
    term_cols: usize,
    term_rows: usize,
) -> Result<()> {
    let lines = camera_picker_lines(devices, selected);
    let top = term_rows.saturating_sub(lines.len()) / 2;
    execute!(out, MoveTo(0, 0), Clear(ClearType::All))?;
    for (i, line) in lines.iter().enumerate() {
        let row = top + i;
        if row >= term_rows {
            break;
        }
        execute!(
            out,
            MoveTo(0, row as u16),
            Print(center_ansi_line(line, term_cols)),
        )?;
    }
    out.flush()?;
    Ok(())
}

pub(crate) fn camera_picker_lines(devices: &[CameraDevice], selected: usize) -> Vec<String> {
    use crate::ui::visible_width;
    const KEYCAP_PALETTE: [(u8, u8, u8); 8] = [
        (191, 197, 255),
        (255, 237, 181),
        (156, 224, 236),
        (202, 170, 246),
        (244, 177, 105),
        (176, 232, 190),
        (255, 184, 208),
        (191, 222, 255),
    ];
    const KEY_TEXT: (u8, u8, u8) = (35, 31, 48);
    const SELECTED_BG: (u8, u8, u8) = (60, 55, 85);
    const TITLE_FG: (u8, u8, u8) = (156, 224, 236);
    const SUBTITLE_FG: (u8, u8, u8) = (200, 200, 220);

    let row_width = devices
        .iter()
        .map(|device| {
            let chip = 4_usize;
            let marker = 2_usize;
            let leading = 1_usize;
            let gap = 2_usize;
            let trailing = 1_usize;
            leading + marker + chip + gap + device.name.chars().count() + trailing
        })
        .max()
        .unwrap_or(28)
        .max(28);

    let mut lines: Vec<String> = Vec::with_capacity(devices.len() + 6);
    lines.push(format!(
        "\x1b[1;38;2;{};{};{}mASCII-CAM\x1b[0m",
        TITLE_FG.0, TITLE_FG.1, TITLE_FG.2,
    ));
    lines.push(String::new());
    lines.push(format!(
        "\x1b[38;2;{};{};{}mSelect a camera\x1b[0m",
        SUBTITLE_FG.0, SUBTITLE_FG.1, SUBTITLE_FG.2,
    ));
    lines.push(String::new());

    for (i, device) in devices.iter().enumerate() {
        let (cr, cg, cb) = KEYCAP_PALETTE[i % KEYCAP_PALETTE.len()];
        let chip = format!(
            "\x1b[48;2;{cr};{cg};{cb}m\x1b[38;2;{};{};{}m {:>2} \x1b[0m",
            KEY_TEXT.0, KEY_TEXT.1, KEY_TEXT.2, device.index,
        );
        let marker = if i == selected { "▶ " } else { "  " };
        let row = format!(" {marker}{chip}  {} ", device.name);
        let visible = visible_width(&row);
        let pad = row_width.saturating_sub(visible);
        let padded = format!("{row}{}", " ".repeat(pad));
        if i == selected {
            lines.push(format!(
                "\x1b[48;2;{};{};{}m{padded}\x1b[0m",
                SELECTED_BG.0, SELECTED_BG.1, SELECTED_BG.2,
            ));
        } else {
            lines.push(padded);
        }
    }

    lines.push(String::new());
    lines.push(shortcut_bar(&[
        Shortcut::new("\u{2191}\u{2193}", "move"),
        Shortcut::new("\u{23ce}", "select"),
        Shortcut::new("esc", "cancel"),
    ]));

    lines
}

fn play_recording(path: PathBuf) -> Result<()> {
    let mut decoder = RecordingDecoder::open(&path)?;
    let _terminal = TerminalGuard::enter()?;
    let mut out = stdout();
    let mut last_timestamp = 0;
    while let Some(decoded) = decoder.read_frame()? {
        let delay = decoded.timestamp_ms.saturating_sub(last_timestamp);
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay.min(1000)));
        }
        let (term_cols, term_rows) = size()?;
        execute!(
            out,
            MoveTo(0, 0),
            Print(playback_frame_view(
                &decoded.frame,
                term_cols as usize,
                term_rows as usize
            ))
        )?;
        out.flush()?;
        last_timestamp = decoded.timestamp_ms;
        if event::poll(Duration::from_millis(0))?
            && let Event::Key(key) = event::read()?
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            break;
        }
    }
    loop {
        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            break;
        }
    }
    Ok(())
}

fn playback_frame_view(frame: &RenderedFrame, term_cols: usize, term_rows: usize) -> String {
    let width = term_cols.max(1);
    let height = term_rows.max(1);
    let mut lines = frame
        .terminal_text()
        .lines()
        .take(height)
        .map(|line| pad_ansi_line(line, width))
        .collect::<Vec<_>>();

    while lines.len() < height {
        lines.push(pad_ansi_line("", width));
    }

    lines.join("\n")
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode().context("enable terminal raw mode")?;
        execute!(stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn last_camera_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("ascii-cam").join("last_camera"))
}

fn load_last_camera() -> Option<u32> {
    let path = last_camera_path()?;
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn save_last_camera(index: u32) {
    let Some(path) = last_camera_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, format!("{index}\n"));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordingFrameAction {
    Write,
    Stop,
}

fn recording_frame_action(
    recording_dimensions: Option<(usize, usize)>,
    frame_dimensions: (usize, usize),
) -> RecordingFrameAction {
    match recording_dimensions {
        Some(dimensions) if dimensions != frame_dimensions => RecordingFrameAction::Stop,
        _ => RecordingFrameAction::Write,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use crate::capture::CameraDevice;
    use crate::render::RenderedFrame;
    use crate::ui::visible_width;

    use super::{
        RecordingFrameAction, camera_picker_lines, playback_frame_view, recording_frame_action,
        should_forward_key_event,
    };

    #[test]
    fn recording_stops_instead_of_exiting_when_frame_dimensions_change() {
        assert_eq!(
            recording_frame_action(Some((80, 24)), (24, 80)),
            RecordingFrameAction::Stop
        );
    }

    #[test]
    fn recording_continues_when_frame_dimensions_match() {
        assert_eq!(
            recording_frame_action(Some((80, 24)), (80, 24)),
            RecordingFrameAction::Write
        );
    }

    #[test]
    fn key_reader_ignores_release_events() {
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('2'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );

        assert!(!should_forward_key_event(release));
    }

    #[test]
    fn playback_view_clips_recordings_to_current_terminal_size() {
        let frame = RenderedFrame::new(5, 1, vec!["abcde".to_string()], None).unwrap();
        let view = playback_frame_view(&frame, 3, 1);

        assert_eq!(view, "abc\x1b[0m\x1b[K");
        assert_eq!(view.lines().count(), 1);
        assert_eq!(visible_width(&view), 3);
    }

    #[test]
    fn playback_view_does_not_center_or_add_top_spacing() {
        let frame = RenderedFrame::new(2, 1, vec!["ab".to_string()], None).unwrap();
        let view = playback_frame_view(&frame, 6, 3);
        let lines: Vec<&str> = view.lines().collect();

        assert_eq!(lines[0], "ab    \x1b[K");
        assert_eq!(lines[1], "      \x1b[K");
        assert_eq!(lines[2], "      \x1b[K");
    }

    #[test]
    fn camera_picker_lists_all_devices_with_their_index_chips_and_names() {
        let devices = vec![
            CameraDevice {
                index: 0,
                name: "FaceTime HD Camera".to_string(),
            },
            CameraDevice {
                index: 2,
                name: "iPhone Camera".to_string(),
            },
        ];

        let joined = camera_picker_lines(&devices, 0).join("\n");

        assert!(joined.contains("ASCII-CAM"));
        assert!(joined.contains("Select a camera"));
        assert!(joined.contains("FaceTime HD Camera"));
        assert!(joined.contains("iPhone Camera"));
        // Index chips use the keycap palette (first chip uses palette[0] bg)
        assert!(joined.contains("\u{1b}[48;2;191;197;255m"));
        assert!(joined.contains(" 0 "));
        assert!(joined.contains(" 2 "));
    }

    #[test]
    fn camera_picker_highlights_only_the_selected_row() {
        let devices = vec![
            CameraDevice {
                index: 0,
                name: "Cam A".to_string(),
            },
            CameraDevice {
                index: 1,
                name: "Cam B".to_string(),
            },
            CameraDevice {
                index: 2,
                name: "Cam C".to_string(),
            },
        ];

        let lines = camera_picker_lines(&devices, 1);
        let selection_bg = "\u{1b}[48;2;60;55;85m";
        let highlighted: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains(selection_bg))
            .collect();

        assert_eq!(highlighted.len(), 1);
        assert!(highlighted[0].contains("Cam B"));
        assert!(highlighted[0].contains("\u{25b6}"));
        assert!(!highlighted[0].contains("Cam A"));
    }
}
