// SPDX-License-Identifier: GPL-3.0-or-later
// Entis lossless image/transform port from arc_unpacker, Copyright (C) 2016 rr-.
use super::{
    common::*,
    eri_entropy::{Entropy, Huff},
};
use anyhow::{ensure, Context, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface};
fn sections(bytes: &[u8]) -> Result<Vec<(String, &[u8])>> {
    let mut c = C::new(bytes);
    let mut out = Vec::new();
    while c.left() != 0 {
        ensure!(out.len() < 65536, "ERI section count limit");
        let raw = c.take(8)?;
        let name = std::str::from_utf8(raw)?.trim_end_matches(' ').to_string();
        let n = u64::from_le_bytes(c.take(8)?.try_into().unwrap());
        let n = usize::try_from(n).context("ERI section length overflow")?;
        out.push((name, c.take(n)?));
    }
    Ok(out)
}
fn one<'a>(s: &[(String, &'a [u8])], name: &str) -> Result<&'a [u8]> {
    let mut found = s.iter().filter(|(n, _)| n == name);
    let v = found
        .next()
        .with_context(|| format!("missing ERI {name} section"))?
        .1;
    ensure!(found.next().is_none(), "duplicate ERI {name} section");
    Ok(v)
}
struct Header {
    w: u32,
    h: u32,
    flip: bool,
    architecture: i32,
    format: u32,
    alpha: bool,
    depth: u32,
    degree: usize,
}
pub fn decode(bytes: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    ensure!(
        bytes.starts_with(b"Entis\x1a\0\0\0\x01\0\x03\0\0\0\0Entis Rasterized Image"),
        "ERI magic"
    );
    let sections = sections(C::at(bytes, 0x40)?.take(bytes.len() - 0x40)?)?;
    let hs = self::sections(one(&sections, "Header")?)?;
    let mut c = C::new(one(&hs, "ImageInf")?);
    let version = c.u32()?;
    ensure!(
        matches!(version, 0x20100 | 0x20200),
        "unsupported ERI version"
    );
    ensure!(c.u32()? == 0x03020000, "unsupported ERI transformation");
    let architecture = c.u32()? as i32;
    let format = c.u32()?;
    let (sw, sh) = (c.u32()? as i32, c.u32()? as i32);
    let w = sw.checked_abs().context("ERI width overflow")? as u32;
    let h = sh.checked_abs().context("ERI height overflow")? as u32;
    let _ = image(w, h)?;
    let depth = c.u32()?;
    c.skip(4 + 4 + 8 + 8)?;
    let degree = c.u32()? as usize;
    let h = Header {
        w,
        h,
        flip: sh > 0,
        architecture,
        format: format & 0xffffff,
        alpha: format >> 24 & 4 != 0,
        depth,
        degree,
    };
    let ss = self::sections(one(&sections, "Stream")?)?;
    let frames: Vec<_> = ss.iter().filter(|(n, _)| n == "ImageFrm").collect();
    ensure!(!frames.is_empty(), "ERI no image frames");
    let mut out = image(
        h.w,
        h.h.checked_mul(u32::try_from(frames.len())?)
            .context("ERI frame canvas overflow")?,
    )?;
    for (i, (_, data)) in frames.into_iter().enumerate() {
        let raw = pixels(&h, data)?;
        let ch = raw.len() / (h.w as usize * h.h as usize);
        let part = if ch == 1 {
            gray(h.w, h.h, &raw, h.flip)?
        } else {
            bgr(h.w, h.h, &raw, ch, h.flip)?
        };
        let at = i * part.rgba.len();
        out.rgba[at..at + part.rgba.len()].copy_from_slice(&part.rgba);
    }
    if let Some((_, data)) = hs.iter().find(|(n, _)| n == "descript") {
        let description = if data.starts_with(&[255, 254]) {
            let u: Vec<u16> = data[2..]
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect();
            String::from_utf16(&u)?
        } else {
            match std::str::from_utf8(data) {
                Ok(s) => s.to_string(),
                Err(_) => {
                    let (s, _, errors) = encoding_rs::SHIFT_JIS.decode(data);
                    ensure!(!errors, "ERI description encoding");
                    s.into_owned()
                }
            }
        };
        let desc = description.replace('\r', "");
        let lines: Vec<&str> = desc.split('\n').filter(|s| !s.is_empty()).collect();
        for pair in lines.chunks_exact(2) {
            if pair[0] == "#reference-file" {
                if let Some(bytes) = ctx.try_read_relative(pair[1])? {
                    let mut base = ctx.decode_embedded(pair[1], &bytes)?;
                    for y in 0..base.height.min(out.height) as usize {
                        for x in 0..base.width.min(out.width) as usize {
                            let i = (y * base.width as usize + x) * 4;
                            let j = (y * out.width as usize + x) * 4;
                            for channel in 0..3 {
                                base.rgba[i + channel] =
                                    base.rgba[i + channel].wrapping_add(out.rgba[j + channel]);
                            }
                        }
                    }
                    out = base;
                }
            }
        }
    }
    Ok(out)
}
fn pixels(h: &Header, data: &[u8]) -> Result<Vec<u8>> {
    let mut d = Entropy::new(data, h.architecture)?;
    let version = d.bits.get(8)?;
    let table = d.bits.get(8)?;
    let encoding = d.bits.get(8)?;
    let bits = d.bits.get(8)?;
    ensure!(
        table == 0 && encoding & 254 == 0,
        "unsupported ERI stream metadata"
    );
    ensure!(
        match version {
            1 => bits == 0,
            8 => bits == 8,
            16 => bits == 8 && encoding == 0,
            _ => false,
        },
        "unsupported ERI stream version/depth"
    );
    ensure!(h.degree > 0 && h.degree <= 10, "ERI blocking degree limit");
    let channels = match h.format {
        1 => {
            if h.depth <= 8 {
                1
            } else if h.alpha {
                4
            } else {
                3
            }
        }
        2 => 1,
        _ => anyhow::bail!("unsupported ERI pixel format"),
    };
    let bs = 1usize << h.degree;
    let area = bs * bs;
    let samples = area * channels;
    let stride = bs * channels;
    let (xb, yb) = ((h.w as usize).div_ceil(bs), (h.h as usize).div_ceil(bs));
    let total = xb
        .checked_mul(yb)
        .and_then(|n| n.checked_mul(samples))
        .context("ERI block size overflow")?;
    ensure!(
        total <= crysirror_image::MAX_DECODED_BYTES,
        "ERI padded output limit"
    );
    let mut tree = Huff::new();
    let mut codes = Vec::new();
    if encoding & 1 != 0 && channels >= 3 {
        for _ in 0..xb * yb {
            codes.push(match h.architecture {
                -1 => 0xc0 | d.bits.get(4)? as u8,
                -4 => tree.read(&mut d.bits, false)? as u8,
                _ => anyhow::bail!("unsupported ERI prefetched op architecture"),
            });
        }
    }
    ensure!(d.bits.get(1)? == 0, "ERI expected separator zero bit");
    if h.architecture != -1 || encoding & 1 != 0 {
        d.reset()?;
    }
    let mut out = vec![0u8; total];
    let mut prev_col = vec![0u8; yb * stride];
    let mut prev_row = vec![0u8; xb * stride];
    for yb_i in 0..yb {
        for xb_i in 0..xb {
            let code = if channels < 3 {
                if encoding & 1 == 0 && h.architecture == -1 {
                    d.reset()?;
                }
                if h.format == 2 {
                    0xc0
                } else {
                    0
                }
            } else if encoding & 1 != 0 {
                codes[yb_i * xb + xb_i]
            } else {
                match h.architecture {
                    -1 => {
                        let code = 0xc0 | d.bits.get(4)? as u8;
                        d.reset()?;
                        code
                    }
                    -4 => tree.read(&mut d.bits, false)? as u8,
                    -16 => d.operation()?,
                    _ => unreachable!(),
                }
            };
            let arranged = d.decode(samples)?;
            let mut block = vec![0u8; samples];
            let mut at = 0;
            let transposed = code & 0x10 != 0;
            let interleaved = code & 0x20 != 0;
            if interleaved {
                for y in 0..bs {
                    for x in 0..bs {
                        for c in 0..channels {
                            let p = if transposed { x * bs + y } else { y * bs + x };
                            block[c * area + p] = arranged[at];
                            at += 1;
                        }
                    }
                }
            } else {
                for c in 0..channels {
                    for y in 0..bs {
                        for x in 0..bs {
                            let p = if transposed { x * bs + y } else { y * bs + x };
                            block[c * area + p] = arranged[at];
                            at += 1;
                        }
                    }
                }
            }
            if code != 0 {
                let color = code & 15;
                let (source, targets) = match color {
                    5 => (0, [1, usize::MAX]),
                    6 => (0, [2, usize::MAX]),
                    7 => (0, [1, 2]),
                    9 => (1, [0, usize::MAX]),
                    10 => (1, [2, usize::MAX]),
                    11 => (1, [0, 2]),
                    13 => (2, [0, usize::MAX]),
                    14 => (2, [1, usize::MAX]),
                    15 => (2, [0, 1]),
                    _ => (0, [usize::MAX; 2]),
                };
                for target in targets {
                    if target != usize::MAX {
                        ensure!(
                            source < channels && target < channels,
                            "ERI color transform on grayscale"
                        );
                        for i in 0..area {
                            block[target * area + i] =
                                block[target * area + i].wrapping_add(block[source * area + i]);
                        }
                    }
                }
                for row in 0..stride {
                    let col = yb_i * stride + row;
                    let mut last = prev_col[col];
                    for x in 0..bs {
                        let i = row * bs + x;
                        if code & 0x40 != 0 {
                            last = last.wrapping_add(block[i]);
                            block[i] = last;
                        } else {
                            last = block[i];
                        }
                    }
                    prev_col[col] = last;
                }
                for channel in 0..channels {
                    for yy in 0..bs {
                        for xx in 0..bs {
                            let i = channel * area + yy * bs + xx;
                            let prev = if yy == 0 {
                                prev_row[xb_i * stride + channel * bs + xx]
                            } else {
                                block[i - bs]
                            };
                            block[i] = block[i].wrapping_add(prev);
                        }
                    }
                    for xx in 0..bs {
                        prev_row[xb_i * stride + channel * bs + xx] =
                            block[channel * area + (bs - 1) * bs + xx];
                    }
                }
            }
            for channel in 0..channels {
                for yy in 0..bs {
                    for xx in 0..bs {
                        let pos = (yb_i * bs + yy) * xb * bs + xb_i * bs + xx;
                        out[pos * channels + channel] = block[channel * area + yy * bs + xx];
                    }
                }
            }
        }
    }
    let mut cropped = Vec::with_capacity(h.w as usize * h.h as usize * channels);
    for y in 0..h.h as usize {
        cropped.extend_from_slice(&out[y * xb * bs * channels..][..h.w as usize * channels]);
    }
    Ok(cropped)
}
