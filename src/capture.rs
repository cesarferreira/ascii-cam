use std::fs;
use std::io::Read;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;

use crate::frame::Frame;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CameraDevice {
    pub index: u32,
    pub name: String,
}

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
    stderr: Arc<Mutex<Vec<u8>>>,
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
        let args = build_ffmpeg_args(platform, camera, fps, width, height);

        let mut child = Command::new("ffmpeg")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(
                || "failed to start ffmpeg; install ffmpeg and check camera permissions",
            )?;
        let stdout = child.stdout.take().context("ffmpeg stdout was not piped")?;
        let stderr_pipe = child.stderr.take().context("ffmpeg stderr was not piped")?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        spawn_stderr_reader(stderr_pipe, Arc::clone(&stderr));
        Ok(Self {
            child,
            stdout,
            width,
            height,
            buffer: vec![0; width * height * 3],
            stderr,
        })
    }

    pub fn read_frame(&mut self) -> Result<Frame> {
        if let Err(error) = self.stdout.read_exact(&mut self.buffer) {
            thread::sleep(Duration::from_millis(20));
            let status = self.child.try_wait().ok().flatten();
            let stderr = self.stderr_text();
            let mut message = String::from("ffmpeg closed before producing a complete RGB frame");
            if let Some(status) = status {
                message.push_str(&format!(" (status: {status})"));
            }
            if !stderr.trim().is_empty() {
                message.push_str("\nffmpeg stderr:\n");
                message.push_str(stderr.trim());
            }
            return Err(error).with_context(|| message);
        }
        Frame::from_rgb24(self.width, self.height, &self.buffer)
    }

    fn stderr_text(&self) -> String {
        let bytes = self
            .stderr
            .lock()
            .map(|stderr| stderr.clone())
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes).to_string()
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

pub fn discover_cameras(platform: Platform) -> Result<Vec<CameraDevice>> {
    match platform.detect() {
        Platform::Macos => discover_macos_cameras(),
        Platform::Linux | Platform::Auto => discover_linux_cameras(),
    }
}

pub fn build_ffmpeg_args(
    platform: Platform,
    camera: u32,
    fps: u8,
    width: usize,
    height: usize,
) -> Vec<String> {
    let platform = platform.detect();
    let mut args: Vec<String> = vec!["-hide_banner", "-nostdin", "-loglevel", "error"]
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
                    "nv12",
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
    args
}

fn spawn_stderr_reader(mut stderr_pipe: impl Read + Send + 'static, stderr: Arc<Mutex<Vec<u8>>>) {
    thread::spawn(move || {
        let mut buffer = [0; 4096];
        loop {
            match stderr_pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if let Ok(mut stderr) = stderr.lock() {
                        stderr.extend_from_slice(&buffer[..count]);
                        if stderr.len() > 16 * 1024 {
                            let excess = stderr.len() - 16 * 1024;
                            stderr.drain(..excess);
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
}

fn discover_macos_cameras() -> Result<Vec<CameraDevice>> {
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-f",
            "avfoundation",
            "-list_devices",
            "true",
            "-i",
            "",
        ])
        .output()
        .with_context(|| "failed to run ffmpeg to list macOS cameras")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(parse_avfoundation_video_devices(&stderr))
}

fn discover_linux_cameras() -> Result<Vec<CameraDevice>> {
    let mut devices = Vec::new();
    for entry in fs::read_dir("/dev").with_context(|| "failed to read /dev for video devices")? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(number) = name.strip_prefix("video") else {
            continue;
        };
        let Ok(index) = number.parse::<u32>() else {
            continue;
        };
        let label = fs::read_to_string(format!("/sys/class/video4linux/{name}/name"))
            .map(|value| value.trim().to_string())
            .unwrap_or_else(|_| format!("/dev/{name}"));
        devices.push(CameraDevice { index, name: label });
    }
    devices.sort_by_key(|device| device.index);
    Ok(devices)
}

pub fn parse_avfoundation_video_devices(stderr: &str) -> Vec<CameraDevice> {
    let mut in_video_section = false;
    let mut devices = Vec::new();
    for line in stderr.lines() {
        if line.contains("AVFoundation video devices:") {
            in_video_section = true;
            continue;
        }
        if line.contains("AVFoundation audio devices:") {
            break;
        }
        if !in_video_section {
            continue;
        }
        let Some(bracket_start) = line.rfind('[') else {
            continue;
        };
        let Some(relative_end) = line[bracket_start + 1..].find(']') else {
            continue;
        };
        let bracket_end = bracket_start + 1 + relative_end;
        let Ok(index) = line[bracket_start + 1..bracket_end].parse::<u32>() else {
            continue;
        };
        let name = line[bracket_end + 1..].trim();
        if !name.is_empty() {
            devices.push(CameraDevice {
                index,
                name: name.to_string(),
            });
        }
    }
    devices
}
