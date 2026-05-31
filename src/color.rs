use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn luminance(self) -> u8 {
        (0.299_f32.mul_add(self.r as f32, 0.587 * self.g as f32) + 0.114 * self.b as f32)
            .clamp(0.0, 255.0) as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ColorMode {
    Off,
    TrueColor,
    Ansi256,
    Ansi16,
    Gray,
    Green,
    GreenGradient,
    Red,
    RedGradient,
}

impl ColorMode {
    pub const CYCLE: [Self; 9] = [
        Self::TrueColor,
        Self::Ansi256,
        Self::Ansi16,
        Self::Gray,
        Self::Green,
        Self::GreenGradient,
        Self::Red,
        Self::RedGradient,
        Self::Off,
    ];

    pub fn next(self) -> Self {
        let index = Self::CYCLE
            .iter()
            .position(|mode| *mode == self)
            .unwrap_or(0);
        Self::CYCLE[(index + 1) % Self::CYCLE.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::TrueColor => "24bit",
            Self::Ansi256 => "256c",
            Self::Ansi16 => "16c",
            Self::Gray => "Gray",
            Self::Green => "Green",
            Self::GreenGradient => "GreenGrad",
            Self::Red => "Red",
            Self::RedGradient => "RedGrad",
        }
    }

    pub fn effective_rgb(self, rgb: Rgb) -> Rgb {
        match self {
            Self::Off | Self::TrueColor => rgb,
            Self::Ansi256 => {
                let ri = ((rgb.r as f32 * 5.0 / 255.0).round() as u8).min(5);
                let gi = ((rgb.g as f32 * 5.0 / 255.0).round() as u8).min(5);
                let bi = ((rgb.b as f32 * 5.0 / 255.0).round() as u8).min(5);
                Rgb::new(ri * 51, gi * 51, bi * 51)
            }
            Self::Ansi16 => {
                let r = if rgb.r > 85 { 255 } else { 0 };
                let g = if rgb.g > 85 { 255 } else { 0 };
                let b = if rgb.b > 85 { 255 } else { 0 };
                Rgb::new(r, g, b)
            }
            Self::Gray => {
                let gray = rgb.luminance();
                let step = (gray as f32 * 23.0 / 255.0).round();
                let value = (step * 255.0 / 23.0).round() as u8;
                Rgb::new(value, value, value)
            }
            Self::Green => Rgb::new(0, 255, 0),
            Self::GreenGradient => Rgb::new(0, rgb.luminance().max(20), 0),
            Self::Red => Rgb::new(255, 0, 0),
            Self::RedGradient => Rgb::new(rgb.luminance().max(20), 0, 0),
        }
    }

    pub fn ansi_prefix(self, rgb: Rgb) -> String {
        match self {
            Self::Off => String::new(),
            Self::TrueColor => format!("\x1b[38;2;{};{};{}m", rgb.r, rgb.g, rgb.b),
            Self::Ansi256 => {
                let ri = ((rgb.r as f32 * 5.0 / 255.0).round() as u8).min(5);
                let gi = ((rgb.g as f32 * 5.0 / 255.0).round() as u8).min(5);
                let bi = ((rgb.b as f32 * 5.0 / 255.0).round() as u8).min(5);
                let idx = 16 + 36 * ri + 6 * gi + bi;
                format!("\x1b[38;5;{idx}m")
            }
            Self::Ansi16 => {
                let brightness = (rgb.r as u16 + rgb.g as u16 + rgb.b as u16) / 3;
                let mut code =
                    30 + ((rgb.b > 85) as u8) * 4 + ((rgb.g > 85) as u8) * 2 + ((rgb.r > 85) as u8);
                if brightness > 127 && code != 30 {
                    code += 60;
                }
                format!("\x1b[{code}m")
            }
            Self::Gray => {
                let gray = rgb.luminance();
                let idx = 232 + (gray as f32 * 23.0 / 255.0).round() as u8;
                format!("\x1b[38;5;{idx}m")
            }
            Self::Green => "\x1b[38;2;0;255;0m".to_string(),
            Self::GreenGradient => {
                let rgb = self.effective_rgb(rgb);
                format!("\x1b[38;2;0;{};0m", rgb.g)
            }
            Self::Red => "\x1b[38;2;255;0;0m".to_string(),
            Self::RedGradient => {
                let rgb = self.effective_rgb(rgb);
                format!("\x1b[38;2;{};0;0m", rgb.r)
            }
        }
    }
}
