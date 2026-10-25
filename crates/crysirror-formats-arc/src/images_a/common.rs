// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{ensure, Context, Result};
use crysirror_image::{ImageSurface, MAX_DECODED_BYTES};
use std::io::Read;
pub struct C<'a> {
    pub b: &'a [u8],
    pub p: usize,
}
impl<'a> C<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    pub fn at(b: &'a [u8], p: usize) -> Result<Self> {
        ensure!(p <= b.len(), "image offset outside input");
        Ok(Self { b, p })
    }
    pub fn left(&self) -> usize {
        self.b.len() - self.p
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let e = self.p.checked_add(n).context("image cursor overflow")?;
        ensure!(e <= self.b.len(), "truncated image");
        let s = &self.b[self.p..e];
        self.p = e;
        Ok(s)
    }
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n)?;
        Ok(())
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}
pub fn image(w: u32, h: u32) -> Result<ImageSurface> {
    ImageSurface::filled(w, h, [0; 4])
}
pub fn inflate(b: &[u8], limit: usize) -> Result<Vec<u8>> {
    ensure!(limit <= MAX_DECODED_BYTES, "image inflate limit");
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(b)
        .take(limit as u64 + 1)
        .read_to_end(&mut out)?;
    ensure!(out.len() <= limit, "image decompression exceeds limit");
    Ok(out)
}
pub fn gray(w: u32, h: u32, data: &[u8], flip: bool) -> Result<ImageSurface> {
    let mut out = image(w, h)?;
    ensure!(
        data.len() >= w as usize * h as usize,
        "short grayscale plane"
    );
    for y in 0..h as usize {
        for x in 0..w as usize {
            let v = data[y * w as usize + x];
            let yy = if flip { h as usize - 1 - y } else { y };
            out.rgba[(yy * w as usize + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    Ok(out)
}
pub fn bgr(w: u32, h: u32, data: &[u8], channels: usize, flip: bool) -> Result<ImageSurface> {
    let mut out = image(w, h)?;
    let n = w as usize * h as usize;
    ensure!(data.len() >= n * channels, "short BGR image");
    for y in 0..h as usize {
        for x in 0..w as usize {
            let p = &data[(y * w as usize + x) * channels..];
            let yy = if flip { h as usize - 1 - y } else { y };
            out.rgba[(yy * w as usize + x) * 4..][..4].copy_from_slice(&[
                p[2],
                p[1],
                p[0],
                if channels == 4 { p[3] } else { 255 },
            ]);
        }
    }
    Ok(out)
}
pub fn palette(w: u32, h: u32, indices: &[u8], pal: &[[u8; 4]]) -> Result<ImageSurface> {
    let mut out = image(w, h)?;
    ensure!(
        indices.len() >= w as usize * h as usize,
        "short palette indices"
    );
    for (p, i) in out.rgba.chunks_exact_mut(4).zip(indices) {
        if let Some(c) = pal.get(*i as usize) {
            p.copy_from_slice(c);
        } else {
            p.copy_from_slice(&[*i, *i, *i, 0]);
        }
    }
    Ok(out)
}
pub struct Bits<'a> {
    pub b: &'a [u8],
    pub p: usize,
    pub msb: bool,
}
impl<'a> Bits<'a> {
    pub fn new(b: &'a [u8], msb: bool) -> Self {
        Self { b, p: 0, msb }
    }
    pub fn get(&mut self, n: usize) -> Result<u32> {
        ensure!(
            n <= 32 && self.p.checked_add(n).is_some_and(|e| e <= self.b.len() * 8),
            "truncated image bitstream"
        );
        let mut v = 0;
        for i in 0..n {
            let x = (self.b[self.p / 8] >> if self.msb { 7 - self.p % 8 } else { self.p % 8 }) & 1;
            if self.msb {
                v = (v << 1) | u32::from(x);
            } else {
                v |= u32::from(x) << i;
            }
            self.p += 1;
        }
        Ok(v)
    }
}
pub fn back(out: &mut Vec<u8>, distance: usize, count: usize, limit: usize) -> Result<()> {
    ensure!(
        distance > 0 && distance <= out.len(),
        "invalid image back-reference"
    );
    ensure!(count <= limit - out.len(), "image run exceeds output");
    for _ in 0..count {
        let b = out[out.len() - distance];
        out.push(b);
    }
    Ok(())
}

pub fn size(w: u32, h: u32, channels: usize) -> Result<usize> {
    Ok(crysirror_image::rgba_len(w, h)? / 4 * channels)
}
