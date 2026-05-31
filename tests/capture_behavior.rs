use ascii_cam::capture::{
    Platform, build_ffmpeg_args, choose_supported_video_size, parse_avfoundation_video_devices,
    parse_supported_video_sizes,
};

#[test]
fn macos_ffmpeg_args_use_supported_avfoundation_input_format() {
    let args = build_ffmpeg_args(Platform::Macos, 0, 30, 640, 480);

    assert!(args.windows(2).any(|pair| pair == ["-f", "avfoundation"]));
    assert!(
        args.windows(2)
            .any(|pair| pair == ["-pixel_format", "nv12"])
    );
    assert!(args.windows(2).any(|pair| pair == ["-pix_fmt", "rgb24"]));
    assert!(
        !args
            .windows(2)
            .any(|pair| pair == ["-pixel_format", "yuyv422"])
    );
}

#[test]
fn linux_ffmpeg_args_use_v4l2_device_and_rgb24_output() {
    let args = build_ffmpeg_args(Platform::Linux, 2, 30, 320, 240);

    assert!(args.windows(2).any(|pair| pair == ["-f", "v4l2"]));
    assert!(args.windows(2).any(|pair| pair == ["-i", "/dev/video2"]));
    assert!(args.windows(2).any(|pair| pair == ["-pix_fmt", "rgb24"]));
}

#[test]
fn parses_only_avfoundation_video_devices() {
    let stderr = r#"
[AVFoundation indev @ 0x123] AVFoundation video devices:
[AVFoundation indev @ 0x123] [0] FaceTime HD Camera
[AVFoundation indev @ 0x123] [1] OBS Virtual Camera
[AVFoundation indev @ 0x123] AVFoundation audio devices:
[AVFoundation indev @ 0x123] [0] MacBook Pro Microphone
"#;

    let devices = parse_avfoundation_video_devices(stderr);

    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].index, 0);
    assert_eq!(devices[0].name, "FaceTime HD Camera");
    assert_eq!(devices[1].index, 1);
    assert_eq!(devices[1].name, "OBS Virtual Camera");
}

#[test]
fn parses_supported_video_sizes_from_ffmpeg_stderr() {
    let stderr = r#"
[in#0 @ 0x831030000] Selected video size (640x480) is not supported by the device.
[in#0 @ 0x831030000] Supported modes:
[in#0 @ 0x831030000]   1280x720@[30.000030 30.000030]fps
[in#0 @ 0x831030000]   1280x720@[25.000000 25.000000]fps
[in#0 @ 0x831030000]   1920x1080@[30.000030 30.000030]fps
"#;

    let sizes = parse_supported_video_sizes(stderr);

    assert_eq!(sizes, vec![(1280, 720), (1920, 1080)]);
}

#[test]
fn chooses_smallest_supported_video_size_that_satisfies_request() {
    let supported = [(1280, 720), (1920, 1080), (3840, 2160)];

    assert_eq!(
        choose_supported_video_size((640, 480), &supported),
        Some((1280, 720))
    );
    assert_eq!(
        choose_supported_video_size((1920, 1080), &supported),
        Some((1920, 1080))
    );
}

#[test]
fn chooses_supported_video_size_with_matching_aspect_when_available() {
    let supported = [(1280, 720), (1280, 960), (1920, 1080)];

    assert_eq!(
        choose_supported_video_size((640, 480), &supported),
        Some((1280, 960))
    );
}
