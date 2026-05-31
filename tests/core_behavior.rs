use ascii_cam::color::{ColorMode, Rgb};
use ascii_cam::frame::Frame;
use ascii_cam::recording::{RecordingDecoder, RecordingEncoder, RecordingOptions};
use ascii_cam::render::{
    RenderConfig, RenderedFrame, build_lut, compute_render_size, render_frame,
};

#[test]
fn compute_render_size_preserves_camera_aspect_with_character_correction() {
    assert_eq!(compute_render_size(100, 40, 640, 480, 0.5), (100, 37));
    assert_eq!(compute_render_size(80, 40, 1280, 720, 0.5), (80, 22));
}

#[test]
fn lut_maps_dark_to_first_character_and_bright_to_last_character() {
    let lut = build_lut(" .:-=+*#%@");

    assert_eq!(lut[0], ' ');
    assert_eq!(lut[255], '@');
    assert_eq!(lut[128], '=');
}

#[test]
fn color_modes_produce_expected_display_values_and_ansi_sequences() {
    let rgb = Rgb::new(250, 120, 10);

    assert_eq!(ColorMode::TrueColor.effective_rgb(rgb), rgb);
    assert_eq!(ColorMode::Ansi256.effective_rgb(rgb), Rgb::new(255, 102, 0));
    assert_eq!(ColorMode::Green.effective_rgb(rgb), Rgb::new(0, 255, 0));
    assert_eq!(
        ColorMode::RedGradient.effective_rgb(rgb),
        Rgb::new(146, 0, 0)
    );
    assert_eq!(ColorMode::Off.ansi_prefix(rgb), "");
    assert_eq!(
        ColorMode::TrueColor.ansi_prefix(rgb),
        "\u{1b}[38;2;250;120;10m"
    );
}

#[test]
fn frame_rotation_repositions_pixels_clockwise_and_counter_clockwise() {
    let frame = Frame::new(
        2,
        3,
        vec![
            Rgb::new(1, 0, 0),
            Rgb::new(2, 0, 0),
            Rgb::new(3, 0, 0),
            Rgb::new(4, 0, 0),
            Rgb::new(5, 0, 0),
            Rgb::new(6, 0, 0),
        ],
    )
    .unwrap();

    let ccw = frame.rotate(1);
    assert_eq!(ccw.width, 3);
    assert_eq!(ccw.height, 2);
    assert_eq!(reds(&ccw), vec![2, 4, 6, 1, 3, 5]);

    let cw = frame.rotate(3);
    assert_eq!(cw.width, 3);
    assert_eq!(cw.height, 2);
    assert_eq!(reds(&cw), vec![5, 3, 1, 6, 4, 2]);
}

#[test]
fn render_frame_applies_brightness_contrast_invert_and_color_mode() {
    let frame = Frame::new(
        2,
        2,
        vec![
            Rgb::new(0, 0, 0),
            Rgb::new(80, 80, 80),
            Rgb::new(180, 10, 10),
            Rgb::new(255, 255, 255),
        ],
    )
    .unwrap();
    let config = RenderConfig {
        cols: 2,
        rows: 2,
        ramp: " .:-=+*#%@".to_string(),
        color_mode: ColorMode::Gray,
        contrast: 1.0,
        brightness: 0,
        invert: true,
    };

    let rendered = render_frame(&frame, &config);

    assert_eq!(rendered.plain_lines(), vec!["@*", "# "]);
    assert!(rendered.terminal_text().contains("\u{1b}[38;5;"));
    assert_eq!(rendered.width, 2);
    assert_eq!(rendered.height, 2);
}

#[test]
fn recording_round_trips_keyframes_deltas_and_skip_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.ascicam");
    let options = RecordingOptions::default();
    let first = RenderedFrame::new(
        2,
        2,
        vec![" .".to_string(), ":@".to_string()],
        Some(vec![
            vec![Rgb::new(0, 0, 0), Rgb::new(10, 20, 30)],
            vec![Rgb::new(30, 20, 10), Rgb::new(255, 255, 255)],
        ]),
    )
    .unwrap();
    let mut second = first.clone();
    second.lines[1].replace_range(1..2, "#");

    {
        let mut encoder = RecordingEncoder::create(&path, 2, 2, 30, options).unwrap();
        encoder.write_frame_at(0, &first).unwrap();
        encoder.write_frame_at(33, &second).unwrap();
        encoder.write_frame_at(66, &second).unwrap();
        encoder.finish().unwrap();
    }

    let mut decoder = RecordingDecoder::open(&path).unwrap();
    let decoded_first = decoder.read_frame().unwrap().unwrap();
    let decoded_second = decoder.read_frame().unwrap().unwrap();
    let decoded_third = decoder.read_frame().unwrap().unwrap();

    assert_eq!(decoded_first.timestamp_ms, 0);
    assert_eq!(decoded_first.frame, first);
    assert_eq!(decoded_second.timestamp_ms, 33);
    assert_eq!(decoded_second.frame, second);
    assert_eq!(decoded_third.timestamp_ms, 66);
    assert_eq!(decoded_third.frame, second);
    assert!(decoder.read_frame().unwrap().is_none());
}

fn reds(frame: &Frame) -> Vec<u8> {
    frame.pixels.iter().map(|rgb| rgb.r).collect()
}
