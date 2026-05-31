use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::color::Rgb;
use crate::render::RenderedFrame;

pub fn write_html(path: impl AsRef<Path>, frame: &RenderedFrame) -> Result<()> {
    let mut file = File::create(path.as_ref())
        .with_context(|| format!("create screenshot {}", path.as_ref().display()))?;
    writeln!(
        file,
        "<!doctype html><meta charset=\"utf-8\"><title>ascii-cam screenshot</title><style>body{{margin:0;background:#050505;color:#eee}}pre{{font:12px/1 monospace;white-space:pre;margin:16px}}</style><pre>"
    )?;
    for (y, line) in frame.lines.iter().enumerate() {
        for (x, ch) in line.chars().enumerate() {
            let rgb = frame
                .colors
                .as_ref()
                .map(|colors| colors[y][x])
                .unwrap_or(Rgb::new(230, 230, 230));
            write!(
                file,
                "<span style=\"color:rgb({},{},{})\">{}</span>",
                rgb.r,
                rgb.g,
                rgb.b,
                escape_html_char(ch)
            )?;
        }
        if y + 1 < frame.lines.len() {
            writeln!(file)?;
        }
    }
    writeln!(file, "</pre>")?;
    Ok(())
}

fn escape_html_char(ch: char) -> String {
    match ch {
        '&' => "&amp;".to_string(),
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '"' => "&quot;".to_string(),
        '\'' => "&#39;".to_string(),
        _ => ch.to_string(),
    }
}
