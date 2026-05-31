use anyhow::{Result, bail};
use clap::ValueEnum;

use crate::color::{ColorMode, Rgb};
use crate::frame::Frame;

pub const RAMP_LONG: &str =
    " .'`^\",:;Il!i><~+_-?][}{1)(|/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$";
pub const RAMP_SHORT: &str = " .:-=+*#%@";
pub const TOP_BAR_LINES: u16 = 1;
pub const BOTTOM_BAR_LINES: u16 = 1;
pub const CHAR_ASPECT_FALLBACK: f32 = 0.45;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RenderMode {
    Ascii,
    Braille,
}

impl RenderMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ascii => "ASCII",
            Self::Braille => "Braille",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Ascii => Self::Braille,
            Self::Braille => Self::Ascii,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenderConfig {
    pub cols: usize,
    pub rows: usize,
    pub ramp: String,
    pub color_mode: ColorMode,
    pub contrast: f32,
    pub brightness: i16,
    pub invert: bool,
    pub mode: RenderMode,
}

#[derive(Clone, Debug)]
pub struct RenderedFrame {
    pub width: usize,
    pub height: usize,
    pub lines: Vec<String>,
    pub colors: Option<Vec<Vec<Rgb>>>,
    terminal_text: Option<String>,
}

impl PartialEq for RenderedFrame {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.lines == other.lines
            && self.colors == other.colors
    }
}

impl Eq for RenderedFrame {}

impl RenderedFrame {
    pub fn new(
        width: usize,
        height: usize,
        lines: Vec<String>,
        colors: Option<Vec<Vec<Rgb>>>,
    ) -> Result<Self> {
        if width == 0 || height == 0 {
            bail!("rendered frame dimensions must be non-zero");
        }
        if lines.len() != height {
            bail!("rendered frame has {} rows, expected {height}", lines.len());
        }
        if lines.iter().any(|line| line.chars().count() != width) {
            bail!("rendered frame line width does not match header width");
        }
        if let Some(colors) = &colors
            && (colors.len() != height || colors.iter().any(|row| row.len() != width))
        {
            bail!("rendered frame color grid does not match dimensions");
        }
        Ok(Self {
            width,
            height,
            lines,
            colors,
            terminal_text: None,
        })
    }

    pub fn with_terminal_text(mut self, terminal_text: String) -> Self {
        self.terminal_text = Some(terminal_text);
        self
    }

    pub fn plain_lines(&self) -> Vec<String> {
        self.lines.clone()
    }

    pub fn terminal_text(&self) -> String {
        if let Some(text) = &self.terminal_text {
            return text.clone();
        }
        let Some(colors) = &self.colors else {
            return self.lines.join("\n");
        };
        let mut out = String::new();
        for (y, line) in self.lines.iter().enumerate() {
            for (x, ch) in line.chars().enumerate() {
                let rgb = colors[y][x];
                out.push_str(&ColorMode::TrueColor.ansi_prefix(rgb));
                out.push(ch);
            }
            if y + 1 < self.lines.len() {
                out.push('\n');
            }
        }
        out.push_str("\x1b[0m");
        out
    }
}

pub fn compute_render_size(
    term_cols: usize,
    term_rows: usize,
    cam_w: usize,
    cam_h: usize,
    char_aspect: f32,
) -> (usize, usize) {
    let cam_aspect = cam_w as f32 / cam_h as f32;
    let mut render_cols = term_cols.max(1);
    let mut render_rows = term_rows.max(1);
    let effective_aspect = render_cols as f32 * char_aspect / render_rows as f32;
    if effective_aspect > cam_aspect {
        render_cols = (render_rows as f32 * cam_aspect / char_aspect).floor() as usize;
    } else {
        render_rows = (render_cols as f32 * char_aspect / cam_aspect).floor() as usize;
    }
    (render_cols.max(1), render_rows.max(1))
}

pub fn build_lut(ramp: &str) -> Vec<char> {
    let chars: Vec<char> = ramp.chars().collect();
    let last = chars.len().saturating_sub(1);
    (0..=255)
        .map(|value| {
            let index = (value * last) / 255;
            chars[index]
        })
        .collect()
}

pub fn render_frame(frame: &Frame, config: &RenderConfig) -> RenderedFrame {
    match config.mode {
        RenderMode::Ascii => render_ascii(frame, config),
        RenderMode::Braille => render_braille(frame, config),
    }
}

fn render_ascii(frame: &Frame, config: &RenderConfig) -> RenderedFrame {
    let ramp: Vec<char> = config.ramp.chars().collect();
    let max_index = ramp.len().saturating_sub(1);
    let rows = config.rows.max(1);
    let cols = config.cols.max(1);
    let row_indices: Vec<usize> = (0..rows)
        .map(|y| y * (frame.height - 1) / rows.saturating_sub(1).max(1))
        .collect();
    let col_indices: Vec<usize> = (0..cols)
        .map(|x| x * (frame.width - 1) / cols.saturating_sub(1).max(1))
        .collect();

    let mut plain_lines = Vec::with_capacity(rows);
    let mut color_rows = if config.color_mode == ColorMode::Off {
        None
    } else {
        Some(Vec::with_capacity(rows))
    };
    let mut terminal = String::new();

    for (line_index, source_y) in row_indices.iter().enumerate() {
        let mut line = String::with_capacity(cols);
        let mut color_row = Vec::with_capacity(cols);
        for source_x in &col_indices {
            let source = frame.get(*source_x, *source_y);
            let adjusted = adjust_rgb(source, config.contrast, config.brightness);
            let gray = adjusted_luma(adjusted, config);
            let char_index = (gray as f32 * max_index as f32 / 255.0).round() as usize;
            let ch = ramp[char_index.min(max_index)];
            line.push(ch);

            if config.color_mode != ColorMode::Off {
                let display_rgb = config.color_mode.effective_rgb(adjusted);
                terminal.push_str(&config.color_mode.ansi_prefix(adjusted));
                terminal.push(ch);
                color_row.push(display_rgb);
            } else {
                terminal.push(ch);
            }
        }
        if line_index + 1 < rows {
            terminal.push('\n');
        }
        plain_lines.push(line);
        if let Some(rows) = &mut color_rows {
            rows.push(color_row);
        }
    }
    if config.color_mode != ColorMode::Off {
        terminal.push_str("\x1b[0m");
    }

    RenderedFrame::new(cols, rows, plain_lines, color_rows)
        .expect("renderer produced internally consistent frame")
        .with_terminal_text(terminal)
}

// Bit values per (sub_y, sub_x) in the 2×4 braille dot grid, matching the
// Unicode Block "Braille Patterns" (U+2800..U+28FF) encoding.
const BRAILLE_DOT_BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

const BRAILLE_THRESHOLD: u8 = 128;

fn render_braille(frame: &Frame, config: &RenderConfig) -> RenderedFrame {
    let rows = config.rows.max(1);
    let cols = config.cols.max(1);
    let sub_rows = rows * 4;
    let sub_cols = cols * 2;
    let row_indices: Vec<usize> = (0..sub_rows)
        .map(|y| y * (frame.height - 1) / sub_rows.saturating_sub(1).max(1))
        .collect();
    let col_indices: Vec<usize> = (0..sub_cols)
        .map(|x| x * (frame.width - 1) / sub_cols.saturating_sub(1).max(1))
        .collect();

    let mut plain_lines = Vec::with_capacity(rows);
    let mut color_rows = if config.color_mode == ColorMode::Off {
        None
    } else {
        Some(Vec::with_capacity(rows))
    };
    let mut terminal = String::new();

    for row in 0..rows {
        let mut line = String::with_capacity(cols);
        let mut color_row = Vec::with_capacity(cols);
        for col in 0..cols {
            let mut mask: u8 = 0;
            let mut r_sum: u32 = 0;
            let mut g_sum: u32 = 0;
            let mut b_sum: u32 = 0;
            for dy in 0..4 {
                for dx in 0..2 {
                    let sx = col_indices[col * 2 + dx];
                    let sy = row_indices[row * 4 + dy];
                    let adjusted =
                        adjust_rgb(frame.get(sx, sy), config.contrast, config.brightness);
                    if adjusted_luma(adjusted, config) >= BRAILLE_THRESHOLD {
                        mask |= BRAILLE_DOT_BITS[dy][dx];
                    }
                    r_sum += adjusted.r as u32;
                    g_sum += adjusted.g as u32;
                    b_sum += adjusted.b as u32;
                }
            }
            let ch = char::from_u32(0x2800 | mask as u32).expect("braille codepoint");
            line.push(ch);

            if config.color_mode != ColorMode::Off {
                let mean = Rgb::new((r_sum / 8) as u8, (g_sum / 8) as u8, (b_sum / 8) as u8);
                let display_rgb = config.color_mode.effective_rgb(mean);
                terminal.push_str(&config.color_mode.ansi_prefix(mean));
                terminal.push(ch);
                color_row.push(display_rgb);
            } else {
                terminal.push(ch);
            }
        }
        if row + 1 < rows {
            terminal.push('\n');
        }
        plain_lines.push(line);
        if let Some(rows_acc) = &mut color_rows {
            rows_acc.push(color_row);
        }
    }
    if config.color_mode != ColorMode::Off {
        terminal.push_str("\x1b[0m");
    }

    RenderedFrame::new(cols, rows, plain_lines, color_rows)
        .expect("renderer produced internally consistent frame")
        .with_terminal_text(terminal)
}

fn adjusted_luma(rgb: Rgb, config: &RenderConfig) -> u8 {
    let gray = rgb.luminance();
    if config.invert { 255 - gray } else { gray }
}

fn adjust_rgb(rgb: Rgb, contrast: f32, brightness: i16) -> Rgb {
    fn one(value: u8, contrast: f32, brightness: i16) -> u8 {
        let adjusted = 128.0 + contrast * (value as f32 - 128.0) + brightness as f32;
        adjusted.clamp(0.0, 255.0) as u8
    }
    Rgb::new(
        one(rgb.r, contrast, brightness),
        one(rgb.g, contrast, brightness),
        one(rgb.b, contrast, brightness),
    )
}
