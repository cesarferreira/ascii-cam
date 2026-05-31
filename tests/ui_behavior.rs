use ascii_cam::ui::{
    Shortcut, center_ansi_line, center_block, pad_ansi_line, shortcut_bar, top_align_block,
    trim_blank_text_rows, visible_width,
};

#[test]
fn shortcut_bar_renders_colored_keycaps_with_plain_labels() {
    let bar = shortcut_bar(&[
        Shortcut::new("v", "variant"),
        Shortcut::new("p", "pkg filter"),
    ]);

    assert!(bar.contains("\u{1b}[48;2;191;197;255m"));
    assert!(bar.contains("\u{1b}[38;2;35;31;48m v \u{1b}[0m"));
    assert!(bar.contains(" variant"));
    assert!(bar.contains(" pkg filter"));
    assert_eq!(visible_width(&bar), 27);
}

#[test]
fn ansi_padding_counts_visible_cells_instead_of_escape_bytes() {
    let bar = shortcut_bar(&[Shortcut::new("q", "quit")]);
    let padded = pad_ansi_line(&bar, 20);

    assert_eq!(visible_width(&padded), 20);
    assert!(padded.ends_with("\u{1b}[K"));
}

#[test]
fn center_ansi_line_centers_visible_text_and_pads_to_width() {
    let centered = center_ansi_line("abc", 7);

    assert_eq!(centered, "  abc  \u{1b}[K");
    assert_eq!(visible_width(&centered), 7);
}

#[test]
fn center_block_centers_content_horizontally_and_vertically() {
    let centered = center_block("abc\ndef", 7, 4);
    let lines: Vec<&str> = centered.lines().collect();

    assert_eq!(lines.len(), 4);
    assert_eq!(visible_width(lines[0]), 7);
    assert_eq!(lines[0], "       \u{1b}[K");
    assert_eq!(lines[1], "  abc  \u{1b}[K");
    assert_eq!(lines[2], "  def  \u{1b}[K");
    assert_eq!(lines[3], "       \u{1b}[K");
}

#[test]
fn center_block_preserves_ansi_sequences_while_centering_visible_width() {
    let colored = "\u{1b}[38;2;255;0;0m@\u{1b}[0m";
    let centered = center_block(colored, 5, 1);

    assert_eq!(visible_width(&centered), 5);
    assert!(centered.starts_with("  \u{1b}[38;2;255;0;0m@"));
    assert!(centered.ends_with("  \u{1b}[K"));
}

#[test]
fn top_align_block_centers_lines_without_top_padding() {
    let aligned = top_align_block("abc\ndef", 7, 4);
    let lines: Vec<&str> = aligned.lines().collect();

    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "  abc  \u{1b}[K");
    assert_eq!(lines[1], "  def  \u{1b}[K");
    assert_eq!(lines[2], "       \u{1b}[K");
    assert_eq!(lines[3], "       \u{1b}[K");
}

#[test]
fn trim_blank_text_rows_removes_empty_top_and_bottom_rows() {
    let (trimmed, rows) = trim_blank_text_rows(&[
        "    ".to_string(),
        "  ab".to_string(),
        " cd ".to_string(),
        "    ".to_string(),
    ]);

    assert_eq!(rows, 2);
    assert_eq!(trimmed, vec!["  ab".to_string(), " cd ".to_string()]);
}

#[test]
fn trim_blank_text_rows_keeps_one_row_when_everything_is_blank() {
    let (trimmed, rows) = trim_blank_text_rows(&["   ".to_string(), "   ".to_string()]);

    assert_eq!(rows, 1);
    assert_eq!(trimmed, vec!["   ".to_string()]);
}
