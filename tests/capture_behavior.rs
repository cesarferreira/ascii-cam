use ascii_cam::capture::{Platform, build_ffmpeg_args};

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
