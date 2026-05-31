use ascii_cam::ui::{Shortcut, pad_ansi_line, shortcut_bar, visible_width};

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
