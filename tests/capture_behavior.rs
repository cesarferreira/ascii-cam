use ascii_cam::capture::{Platform, build_ffmpeg_args, parse_avfoundation_video_devices};

#[test]
fn macos_ffmpeg_args_let_camera_negotiate_input_pixel_format() {
    let args = build_ffmpeg_args(Platform::Macos, 0, 30, 640, 480);

    assert!(args.windows(2).any(|pair| pair == ["-f", "avfoundation"]));
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
