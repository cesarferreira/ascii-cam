use anyhow::{Result, bail};

use crate::color::Rgb;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Rgb>,
}

impl Frame {
    pub fn new(width: usize, height: usize, pixels: Vec<Rgb>) -> Result<Self> {
        if width == 0 || height == 0 {
            bail!("frame dimensions must be non-zero");
        }
        if pixels.len() != width * height {
            bail!(
                "frame has {} pixels, expected {}",
                pixels.len(),
                width * height
            );
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn from_rgb24(width: usize, height: usize, bytes: &[u8]) -> Result<Self> {
        if bytes.len() != width * height * 3 {
            bail!(
                "rgb24 frame has {} bytes, expected {}",
                bytes.len(),
                width * height * 3
            );
        }
        let pixels = bytes
            .chunks_exact(3)
            .map(|chunk| Rgb::new(chunk[0], chunk[1], chunk[2]))
            .collect();
        Self::new(width, height, pixels)
    }

    pub fn get(&self, x: usize, y: usize) -> Rgb {
        self.pixels[y * self.width + x]
    }

    pub fn rotate(&self, turns_ccw: u8) -> Self {
        match turns_ccw % 4 {
            0 => self.clone(),
            1 => {
                let new_width = self.height;
                let new_height = self.width;
                let mut pixels = vec![Rgb::new(0, 0, 0); self.pixels.len()];
                for y in 0..self.height {
                    for x in 0..self.width {
                        let dst_x = y;
                        let dst_y = self.width - 1 - x;
                        pixels[dst_y * new_width + dst_x] = self.get(x, y);
                    }
                }
                Self {
                    width: new_width,
                    height: new_height,
                    pixels,
                }
            }
            2 => {
                let mut pixels = self.pixels.clone();
                pixels.reverse();
                Self {
                    width: self.width,
                    height: self.height,
                    pixels,
                }
            }
            _ => {
                let new_width = self.height;
                let new_height = self.width;
                let mut pixels = vec![Rgb::new(0, 0, 0); self.pixels.len()];
                for y in 0..self.height {
                    for x in 0..self.width {
                        let dst_x = self.height - 1 - y;
                        let dst_y = x;
                        pixels[dst_y * new_width + dst_x] = self.get(x, y);
                    }
                }
                Self {
                    width: new_width,
                    height: new_height,
                    pixels,
                }
            }
        }
    }
}
