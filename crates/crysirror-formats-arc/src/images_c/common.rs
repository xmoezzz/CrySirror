// SPDX-License-Identifier: GPL-3.0-or-later
// Safe CPU translation of arc_unpacker (C) 2016 rr-, pinned in arc-images-c.json.
use anyhow::{ensure, Context, Result};
use crysirror_image::{rgba_len, ImageSurface, MAX_DECODED_BYTES};
use std::io::Read;
pub(super) struct Cursor<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}
impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn at(data: &'a [u8], pos: usize) -> Result<Self> {
        ensure!(pos <= data.len(), "image data offset outside source");
        Ok(Self { data, pos })
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).context("image range overflow")?;
        ensure!(end <= self.data.len(), "truncated image stream");
        let b = &self.data[self.pos..end];
        self.pos = end;
        Ok(b)
    }
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n)?;
        Ok(())
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    pub fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }
}
pub(super) fn bounded(n: usize) -> Result<usize> {
    ensure!(n <= MAX_DECODED_BYTES, "image intermediate exceeds 64 MiB");
    Ok(n)
}
pub(super) fn pixels(w: u32, h: u32) -> Result<usize> {
    Ok(rgba_len(w, h)? / 4)
}
#[derive(Clone, Copy)]
pub(super) enum Format {
    Gray,
    Bgr,
    Bgra,
    Bgrx,
    Bgr555,
    Bgr565,
}
pub(super) fn raw(w: u32, h: u32, b: &[u8], fmt: Format) -> Result<ImageSurface> {
    let n = pixels(w, h)?;
    let channels = match fmt {
        Format::Gray => 1,
        Format::Bgr => 3,
        Format::Bgra | Format::Bgrx => 4,
        _ => 2,
    };
    ensure!(b.len() >= n * channels, "insufficient image pixel data");
    let mut rgba = Vec::with_capacity(n * 4);
    for c in b[..n * channels].chunks_exact(channels) {
        let p = match fmt {
            Format::Gray => [c[0], c[0], c[0], 255],
            Format::Bgr => [c[2], c[1], c[0], 255],
            Format::Bgra => [c[2], c[1], c[0], c[3]],
            Format::Bgrx => [c[2], c[1], c[0], 255],
            Format::Bgr555 => {
                let v = u16::from_le_bytes([c[0], c[1]]);
                [
                    ((v >> 10) & 31) as u8 * 8,
                    ((v >> 5) & 31) as u8 * 8,
                    (v & 31) as u8 * 8,
                    255,
                ]
            }
            Format::Bgr565 => {
                let v = u16::from_le_bytes([c[0], c[1]]);
                [
                    ((v >> 11) & 31) as u8 * 8,
                    ((v >> 5) & 63) as u8 * 4,
                    (v & 31) as u8 * 8,
                    255,
                ]
            }
        };
        rgba.extend(p);
    }
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
pub(super) fn palette(w: u32, h: u32, b: &[u8], pal: &[[u8; 4]]) -> Result<ImageSurface> {
    let n = pixels(w, h)?;
    ensure!(b.len() >= n, "truncated palette indices");
    let rgba = b[..n]
        .iter()
        .flat_map(|&i| pal.get(i as usize).copied().unwrap_or([i, i, i, 0]))
        .collect();
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba,
    })
}
pub(super) fn read_palette(
    c: &mut Cursor<'_>,
    n: usize,
    channels: usize,
    opaque: bool,
) -> Result<Vec<[u8; 4]>> {
    ensure!(
        n <= 256 && (channels == 3 || channels == 4),
        "invalid image palette"
    );
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let p = c.take(channels)?;
        out.push([
            p[2],
            p[1],
            p[0],
            if channels == 4 && !opaque { p[3] } else { 255 },
        ]);
    }
    Ok(out)
}
pub(super) fn flip(image: &mut ImageSurface) {
    let stride = image.width as usize * 4;
    for y in 0..image.height as usize / 2 {
        let a = y * stride;
        let b = (image.height as usize - 1 - y) * stride;
        for x in 0..stride {
            image.rgba.swap(a + x, b + x);
        }
    }
}
pub(super) fn blit(
    dst: &mut ImageSurface,
    src: &ImageSurface,
    x: u32,
    y: u32,
    nontransparent: bool,
) {
    for sy in 0..src.height.min(dst.height.saturating_sub(y)) {
        for sx in 0..src.width.min(dst.width.saturating_sub(x)) {
            let a = ((sy * src.width + sx) * 4) as usize;
            let b = (((sy + y) * dst.width + sx + x) * 4) as usize;
            if !nontransparent || src.rgba[a + 3] != 0 {
                dst.rgba[b..b + 4].copy_from_slice(&src.rgba[a..a + 4]);
            }
        }
    }
}
pub(super) fn blank(w: u32, h: u32) -> Result<ImageSurface> {
    Ok(ImageSurface {
        width: w,
        height: h,
        rgba: vec![0; rgba_len(w, h)?],
    })
}
pub(super) fn zlib(input: &[u8], expected: usize) -> Result<Vec<u8>> {
    bounded(expected)?;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(input)
        .take((expected + 1) as u64)
        .read_to_end(&mut out)?;
    ensure!(out.len() == expected, "zlib image length mismatch");
    Ok(out)
}
pub(super) fn lzss(input: &[u8], n: usize, leaf: bool) -> Result<Vec<u8>> {
    bounded(n)?;
    let mut c = Cursor::new(input);
    let mut out = Vec::with_capacity(n);
    let mut dict = [0u8; 4096];
    let mut write = 0xfee;
    while out.len() < n {
        let flag = c.u8()?;
        for bit in 0..8 {
            if out.len() == n {
                break;
            }
            let literal = if leaf {
                (!flag >> (7 - bit)) & 1
            } else {
                (flag >> bit) & 1
            };
            if literal != 0 {
                let b = if leaf { !c.u8()? } else { c.u8()? };
                out.push(b);
                dict[write] = b;
                write = (write + 1) & 4095;
            } else {
                let lo = if leaf { !c.u8()? } else { c.u8()? };
                let hi = if leaf { !c.u8()? } else { c.u8()? };
                let (mut read, count) = if leaf {
                    let v = u16::from_le_bytes([lo, hi]);
                    ((v >> 4) as usize, (v & 15) as usize + 3)
                } else {
                    (
                        lo as usize | ((hi as usize & 0xf0) << 4),
                        (hi & 15) as usize + 3,
                    )
                };
                for _ in 0..count.min(n - out.len()) {
                    let b = dict[read];
                    read = (read + 1) & 4095;
                    out.push(b);
                    dict[write] = b;
                    write = (write + 1) & 4095;
                }
            }
        }
    }
    Ok(out)
}
pub(super) struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Bits<'a> {
    pub fn bytes_used(&self) -> usize {
        self.pos.div_ceil(8)
    }
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn get(&mut self, n: usize) -> Result<usize> {
        ensure!(
            n <= 32
                && self
                    .pos
                    .checked_add(n)
                    .is_some_and(|end| end <= self.data.len() * 8),
            "truncated image bitstream"
        );
        let mut v = 0;
        for _ in 0..n {
            v = v << 1 | ((self.data[self.pos / 8] >> (7 - self.pos % 8)) & 1) as usize;
            self.pos += 1;
        }
        Ok(v)
    }
}
