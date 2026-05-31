use std::io::{Write, stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::Parser;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, size,
};

use crate::capture::{
    CameraDevice, FfmpegCapture, Platform, Resolution, discover_cameras, ensure_supported_platform,
};
use crate::color::ColorMode;
use crate::recording::{RecordingDecoder, RecordingEncoder, RecordingOptions};
use crate::render::{
    BOTTOM_BAR_LINES, CHAR_ASPECT_FALLBACK, RAMP_LONG, RAMP_SHORT, RenderConfig, RenderedFrame,
    TOP_BAR_LINES, compute_render_size, render_frame,
};
use crate::screenshot::write_html;
use crate::ui::{Shortcut, center_ansi_line, center_block, pad_ansi_line, shortcut_bar};

#[derive(Parser, Debug)]
#[command(version, about = "Real-time ASCII camera for the terminal")]
pub struct Cli {
    #[arg(long, value_enum, default_value_t = Resolution::Medium)]
    pub resolution: Resolution,
    #[arg(long, default_value_t = 0)]
    pub camera: u32,
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

#[derive(Clone, Debug)]
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
    if let Some(path) = cli.play {
        return play_recording(path);
    }
    let mut app = LiveApp::new(cli)?;
    if app.cli.pick_camera {
        app.cli.camera = pick_camera_interactive(app.platform)?;
    }
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
        Ok(Self {
            contrast: cli.contrast,
            brightness: cli.brightness,
            invert: cli.invert,
            rotation: cli.rotate % 4,
            record_path: cli.record.clone(),
            cli,
            platform,
            color_mode,
            preset: Preset::Raw,
            recording_options: RecordingOptions::default(),
            encoder: None,
            recording_dimensions: None,
            show_help: false,
            show_settings: false,
            last_rendered: None,
        })
    }

    fn run(mut self) -> Result<()> {
        let (cam_w, cam_h) = self.cli.resolution.dimensions();
        let mut capture =
            FfmpegCapture::spawn(self.platform, self.cli.camera, self.cli.fps, cam_w, cam_h)?;
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
                Ok(Event::Key(key)) => {
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

fn pick_camera_interactive(platform: Platform) -> Result<u32> {
    let devices = discover_cameras(platform)?;
    if devices.is_empty() {
        bail!("no cameras discovered; pass --camera N to choose a camera index manually");
    }

    let _terminal = TerminalGuard::enter()?;
    let mut out = stdout();
    let mut selected = 0_usize;
    loop {
        let (cols, _) = size()?;
        execute!(
            out,
            MoveTo(0, 0),
            Clear(ClearType::All),
            Print(camera_picker_view(&devices, selected, cols as usize))
        )?;
        out.flush()?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Up => selected = selected.saturating_sub(1),
                KeyCode::Down => selected = (selected + 1).min(devices.len() - 1),
                KeyCode::Enter => return Ok(devices[selected].index),
                KeyCode::Esc | KeyCode::Char('q') => bail!("camera picker cancelled"),
                _ => {}
            }
        }
    }
}

fn camera_picker_view(devices: &[CameraDevice], selected: usize, width: usize) -> String {
    let mut lines = vec![
        pad_ansi_line("ASCII-CAM CAMERA", width),
        pad_ansi_line("", width),
        pad_ansi_line("Use Up/Down, Enter to select, q to cancel", width),
        pad_ansi_line("", width),
    ];
    for (index, device) in devices.iter().enumerate() {
        let marker = if index == selected { ">" } else { " " };
        let line = format!("{marker} [{}] {}", device.index, device.name);
        lines.push(pad_ansi_line(&line, width));
    }
    lines.join("\n")
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
        execute!(
            out,
            MoveTo(0, 0),
            Clear(ClearType::All),
            Print(decoded.frame.terminal_text())
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
    use super::{RecordingFrameAction, recording_frame_action};

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
}
