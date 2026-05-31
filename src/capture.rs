use std::io::Read;
use std::process::{Child, ChildStdout, Command, Stdio};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;

use crate::frame::Frame;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Platform {
    Auto,
    Macos,
    Linux,
}

impl Platform {
    pub fn detect(self) -> Self {
        match self {
            Self::Auto if cfg!(target_os = "macos") => Self::Macos,
            Self::Auto => Self::Linux,
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Resolution {
    Low,
    Medium,
    High,
}

impl Resolution {
    pub fn dimensions(self) -> (usize, usize) {
        match self {
            Self::Low => (320, 240),
            Self::Medium => (640, 480),
            Self::High => (1280, 720),
        }
    }
}

pub struct FfmpegCapture {
    child: Child,
    stdout: ChildStdout,
    width: usize,
    height: usize,
    buffer: Vec<u8>,
}

impl FfmpegCapture {
    pub fn spawn(
        platform: Platform,
        camera: u32,
        fps: u8,
        width: usize,
        height: usize,
    ) -> Result<Self> {
        let platform = platform.detect();
        let mut args: Vec<String> = vec!["-hide_banner", "-loglevel", "error"]
            .into_iter()
            .map(str::to_string)
            .collect();
        match platform {
            Platform::Macos => {
                args.extend(
                    [
                        "-f",
                        "avfoundation",
                        "-framerate",
                        &fps.to_string(),
                        "-video_size",
                        &format!("{width}x{height}"),
                        "-pixel_format",
                        "yuyv422",
                        "-i",
                        &camera.to_string(),
                    ]
                    .into_iter()
                    .map(str::to_string),
                );
            }
            Platform::Linux | Platform::Auto => {
                args.extend(
                    [
                        "-f",
                        "v4l2",
                        "-framerate",
                        &fps.to_string(),
                        "-video_size",
                        &format!("{width}x{height}"),
                        "-i",
                        &format!("/dev/video{camera}"),
                    ]
                    .into_iter()
                    .map(str::to_string),
                );
            }
        }
        args.extend(
            [
                "-vf", "hflip", "-f", "rawvideo", "-pix_fmt", "rgb24", "-an", "pipe:1",
            ]
            .into_iter()
            .map(str::to_string),
        );

        let mut child = Command::new("ffmpeg")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(
                || "failed to start ffmpeg; install ffmpeg and check camera permissions",
            )?;
        let stdout = child.stdout.take().context("ffmpeg stdout was not piped")?;
        Ok(Self {
            child,
            stdout,
            width,
            height,
            buffer: vec![0; width * height * 3],
        })
    }

    pub fn read_frame(&mut self) -> Result<Frame> {
        self.stdout
            .read_exact(&mut self.buffer)
            .with_context(|| "failed to read a complete RGB frame from ffmpeg")?;
        Frame::from_rgb24(self.width, self.height, &self.buffer)
    }
}

impl Drop for FfmpegCapture {
    fn drop(&mut self) {
        if let Ok(Some(_)) = self.child.try_wait() {
            return;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn ensure_supported_platform(platform: Platform) -> Result<Platform> {
    let detected = platform.detect();
    if detected == Platform::Linux || detected == Platform::Macos {
        Ok(detected)
    } else {
        bail!("unsupported platform")
    }
}
