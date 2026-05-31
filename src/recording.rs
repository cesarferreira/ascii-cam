use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use bitflags::bitflags;
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::color::Rgb;
use crate::render::RenderedFrame;

const MAGIC: &[u8; 4] = b"ACAM";
const VERSION: u8 = 1;
const FRAME_KEY: u8 = 0;
const FRAME_DELTA: u8 = 1;
const FRAME_SKIP: u8 = 2;

bitflags! {
    #[derive(Clone, Copy, Debug)]
    struct HeaderFlags: u8 {
        const COMPRESSED = 0b0000_0001;
        const SKIP_IDENTICAL = 0b0000_0010;
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RecordingOptions {
    pub compress: bool,
    pub skip_identical: bool,
    pub keyframe_interval: u32,
}

impl Default for RecordingOptions {
    fn default() -> Self {
        Self {
            compress: true,
            skip_identical: true,
            keyframe_interval: 30,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedFrame {
    pub timestamp_ms: u64,
    pub frame: RenderedFrame,
}

pub struct RecordingEncoder {
    file: File,
    width: usize,
    height: usize,
    options: RecordingOptions,
    previous: Option<RenderedFrame>,
    frame_count: u32,
}

impl RecordingEncoder {
    pub fn create(
        path: impl AsRef<Path>,
        width: usize,
        height: usize,
        fps: u8,
        options: RecordingOptions,
    ) -> Result<Self> {
        if width > u16::MAX as usize || height > u16::MAX as usize {
            bail!("recording dimensions exceed u16 format limit");
        }
        let mut file = File::create(path.as_ref())
            .with_context(|| format!("create recording {}", path.as_ref().display()))?;
        let mut flags = HeaderFlags::empty();
        flags.set(HeaderFlags::COMPRESSED, options.compress);
        flags.set(HeaderFlags::SKIP_IDENTICAL, options.skip_identical);

        file.write_all(MAGIC)?;
        file.write_all(&[VERSION])?;
        file.write_all(&(width as u16).to_le_bytes())?;
        file.write_all(&(height as u16).to_le_bytes())?;
        file.write_all(&[fps])?;
        file.write_all(&[flags.bits()])?;

        Ok(Self {
            file,
            width,
            height,
            options,
            previous: None,
            frame_count: 0,
        })
    }

    pub fn write_frame_at(&mut self, timestamp_ms: u64, frame: &RenderedFrame) -> Result<()> {
        self.validate_frame(frame)?;
        let is_keyframe = self.previous.is_none()
            || self
                .frame_count
                .is_multiple_of(self.options.keyframe_interval.max(1));
        let (frame_type, payload) = if is_keyframe {
            (FRAME_KEY, encode_keyframe(frame))
        } else {
            match self.encode_delta(frame)? {
                Some(delta) => (FRAME_DELTA, delta),
                None => {
                    self.write_frame_header(FRAME_SKIP, timestamp_ms, 0)?;
                    self.previous = Some(frame.clone());
                    self.frame_count += 1;
                    return Ok(());
                }
            }
        };
        let payload = if self.options.compress {
            compress(&payload)?
        } else {
            payload
        };
        self.write_frame_header(frame_type, timestamp_ms, payload.len() as u32)?;
        self.file.write_all(&payload)?;
        self.previous = Some(frame.clone());
        self.frame_count += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }

    fn validate_frame(&self, frame: &RenderedFrame) -> Result<()> {
        if frame.width != self.width || frame.height != self.height {
            bail!(
                "recording frame is {}x{}, expected {}x{}",
                frame.width,
                frame.height,
                self.width,
                self.height
            );
        }
        Ok(())
    }

    fn write_frame_header(&mut self, frame_type: u8, timestamp_ms: u64, len: u32) -> Result<()> {
        self.file.write_all(&[frame_type])?;
        self.file.write_all(&timestamp_ms.to_le_bytes())?;
        self.file.write_all(&len.to_le_bytes())?;
        Ok(())
    }

    fn encode_delta(&self, frame: &RenderedFrame) -> Result<Option<Vec<u8>>> {
        let previous = self.previous.as_ref().context("missing previous frame")?;
        let colors_enabled = frame.colors.is_some();
        let mut changes = Vec::new();
        for y in 0..self.height {
            let current_line = frame.lines[y].as_bytes();
            let previous_line = previous.lines[y].as_bytes();
            for x in 0..self.width {
                let current_color = frame
                    .colors
                    .as_ref()
                    .and_then(|colors| colors.get(y).and_then(|row| row.get(x)))
                    .copied();
                let previous_color = previous
                    .colors
                    .as_ref()
                    .and_then(|colors| colors.get(y).and_then(|row| row.get(x)))
                    .copied();
                let changed =
                    current_line[x] != previous_line[x] || current_color != previous_color;
                if changed {
                    changes.push((y * self.width + x, current_line[x], current_color));
                }
            }
        }
        if changes.is_empty() && self.options.skip_identical {
            return Ok(None);
        }

        let mut out = Vec::new();
        out.extend_from_slice(&(changes.len() as u32).to_le_bytes());
        out.push(colors_enabled as u8);
        for (index, ch, color) in changes {
            out.extend_from_slice(&(index as u32).to_le_bytes());
            out.push(ch);
            if colors_enabled {
                let rgb = color.unwrap_or(Rgb::new(255, 255, 255));
                out.extend_from_slice(&[rgb.r, rgb.g, rgb.b]);
            }
        }
        Ok(Some(out))
    }
}

pub struct RecordingDecoder {
    file: File,
    width: usize,
    height: usize,
    compressed: bool,
    current: Option<RenderedFrame>,
}

impl RecordingDecoder {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = File::open(path.as_ref())
            .with_context(|| format!("open recording {}", path.as_ref().display()))?;
        let mut magic = [0; 4];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            bail!("not an .ascicam recording");
        }
        let version = read_u8(&mut file)?;
        if version != VERSION {
            bail!("unsupported .ascicam version {version}");
        }
        let width = read_u16(&mut file)? as usize;
        let height = read_u16(&mut file)? as usize;
        let _fps = read_u8(&mut file)?;
        let flags = HeaderFlags::from_bits_truncate(read_u8(&mut file)?);
        Ok(Self {
            file,
            width,
            height,
            compressed: flags.contains(HeaderFlags::COMPRESSED),
            current: None,
        })
    }

    pub fn read_frame(&mut self) -> Result<Option<DecodedFrame>> {
        let mut frame_type = [0; 1];
        let count = self.file.read(&mut frame_type)?;
        if count == 0 {
            return Ok(None);
        }
        let timestamp_ms = read_u64(&mut self.file)?;
        let len = read_u32(&mut self.file)? as usize;
        if frame_type[0] == FRAME_SKIP {
            let frame = self
                .current
                .clone()
                .context("skip frame encountered before first frame")?;
            return Ok(Some(DecodedFrame {
                timestamp_ms,
                frame,
            }));
        }

        let mut payload = vec![0; len];
        self.file.read_exact(&mut payload)?;
        if self.compressed {
            payload = decompress(&payload)?;
        }
        let frame = match frame_type[0] {
            FRAME_KEY => decode_keyframe(self.width, self.height, &payload)?,
            FRAME_DELTA => {
                let previous = self
                    .current
                    .as_ref()
                    .context("delta frame encountered before keyframe")?;
                decode_delta(previous, &payload)?
            }
            other => bail!("unknown frame type {other}"),
        };
        self.current = Some(frame.clone());
        Ok(Some(DecodedFrame {
            timestamp_ms,
            frame,
        }))
    }
}

fn encode_keyframe(frame: &RenderedFrame) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(frame.colors.is_some() as u8);
    for line in &frame.lines {
        out.extend_from_slice(line.as_bytes());
    }
    if let Some(colors) = &frame.colors {
        for row in colors {
            for rgb in row {
                out.extend_from_slice(&[rgb.r, rgb.g, rgb.b]);
            }
        }
    }
    out
}

fn decode_keyframe(width: usize, height: usize, payload: &[u8]) -> Result<RenderedFrame> {
    let color_enabled = payload.first().copied().context("empty keyframe")? != 0;
    let text_len = width * height;
    if payload.len() < 1 + text_len {
        bail!("truncated keyframe text");
    }
    let text = &payload[1..1 + text_len];
    let mut lines = Vec::with_capacity(height);
    for row in text.chunks_exact(width) {
        lines.push(String::from_utf8(row.to_vec()).context("recording text is not utf-8")?);
    }
    let colors = if color_enabled {
        let color_bytes = &payload[1 + text_len..];
        if color_bytes.len() < text_len * 3 {
            bail!("truncated keyframe colors");
        }
        let mut rows = Vec::with_capacity(height);
        for row in color_bytes.chunks_exact(width * 3).take(height) {
            let pixels = row
                .chunks_exact(3)
                .map(|chunk| Rgb::new(chunk[0], chunk[1], chunk[2]))
                .collect();
            rows.push(pixels);
        }
        Some(rows)
    } else {
        None
    };
    RenderedFrame::new(width, height, lines, colors)
}

fn decode_delta(previous: &RenderedFrame, payload: &[u8]) -> Result<RenderedFrame> {
    if payload.len() < 5 {
        bail!("truncated delta");
    }
    let change_count = u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize;
    let color_enabled = payload[4] != 0;
    let entry_len = if color_enabled { 8 } else { 5 };
    if payload.len() < 5 + change_count * entry_len {
        bail!("truncated delta entries");
    }

    let mut lines: Vec<Vec<u8>> = previous
        .lines
        .iter()
        .map(|line| line.as_bytes().to_vec())
        .collect();
    let mut colors = previous.colors.clone();
    let mut offset = 5;
    for _ in 0..change_count {
        let index = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let ch = payload[offset];
        offset += 1;
        let y = index / previous.width;
        let x = index % previous.width;
        if y >= previous.height {
            bail!("delta index outside frame");
        }
        lines[y][x] = ch;
        if color_enabled {
            let rgb = Rgb::new(payload[offset], payload[offset + 1], payload[offset + 2]);
            offset += 3;
            if let Some(colors) = &mut colors {
                colors[y][x] = rgb;
            }
        }
    }
    let lines = lines
        .into_iter()
        .map(|line| String::from_utf8(line).context("delta text is not utf-8"))
        .collect::<Result<Vec<_>>>()?;
    RenderedFrame::new(previous.width, previous.height, lines, colors)
}

fn compress(payload: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(payload)?;
    Ok(encoder.finish()?)
}

fn decompress(payload: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(payload);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

fn read_u8(file: &mut File) -> Result<u8> {
    let mut bytes = [0; 1];
    file.read_exact(&mut bytes)?;
    Ok(bytes[0])
}

fn read_u16(file: &mut File) -> Result<u16> {
    let mut bytes = [0; 2];
    file.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(file: &mut File) -> Result<u32> {
    let mut bytes = [0; 4];
    file.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(file: &mut File) -> Result<u64> {
    let mut bytes = [0; 8];
    file.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}
