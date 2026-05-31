#[derive(Clone, Copy, Debug)]
pub struct Shortcut<'a> {
    pub key: &'a str,
    pub label: &'a str,
}

impl<'a> Shortcut<'a> {
    pub const fn new(key: &'a str, label: &'a str) -> Self {
        Self { key, label }
    }
}

const KEYCAPS: [(u8, u8, u8); 8] = [
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

pub fn shortcut_bar(shortcuts: &[Shortcut<'_>]) -> String {
    shortcuts
        .iter()
        .enumerate()
        .map(|(index, shortcut)| {
            let (r, g, b) = KEYCAPS[index % KEYCAPS.len()];
            format!(
                "\x1b[48;2;{r};{g};{b}m\x1b[38;2;{};{};{}m {} \x1b[0m {}",
                KEY_TEXT.0, KEY_TEXT.1, KEY_TEXT.2, shortcut.key, shortcut.label
            )
        })
        .collect::<Vec<_>>()
        .join("  ")
}

pub fn pad_ansi_line(line: &str, width: usize) -> String {
    let mut out = truncate_ansi(line, width);
    let visible = visible_width(&out);
    if visible < width {
        out.push_str(&" ".repeat(width - visible));
    }
    out.push_str("\x1b[K");
    out
}

pub fn center_block(text: &str, width: usize, height: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let content_height = lines.len().min(height);
    let top_padding = height.saturating_sub(content_height) / 2;
    let bottom_padding = height.saturating_sub(content_height) - top_padding;
    let mut out = Vec::with_capacity(height);

    for _ in 0..top_padding {
        out.push(pad_ansi_line("", width));
    }
    for line in lines.iter().take(content_height) {
        out.push(center_ansi_line(line, width));
    }
    for _ in 0..bottom_padding {
        out.push(pad_ansi_line("", width));
    }

    out.join("\n")
}

pub fn top_align_block(text: &str, width: usize, height: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let content_height = lines.len().min(height);
    let mut out = Vec::with_capacity(height);

    for line in lines.iter().take(content_height) {
        out.push(center_ansi_line(line, width));
    }
    for _ in content_height..height {
        out.push(pad_ansi_line("", width));
    }

    out.join("\n")
}

pub fn center_ansi_line(line: &str, width: usize) -> String {
    let visible = visible_width(line);
    if visible >= width {
        return pad_ansi_line(line, width);
    }
    let left = (width - visible) / 2;
    let right = width - visible - left;
    format!("{}{}{}\x1b[K", " ".repeat(left), line, " ".repeat(right))
}

pub fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for esc in chars.by_ref() {
                if esc.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            width += 1;
        }
    }
    width
}

fn truncate_ansi(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut visible = 0;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            out.push(ch);
            out.push(chars.next().expect("peeked CSI marker"));
            for esc in chars.by_ref() {
                out.push(esc);
                if esc.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if visible < width {
            out.push(ch);
            visible += 1;
        } else {
            break;
        }
    }
    if visible >= width {
        out.push_str("\x1b[0m");
    }
    out
}
