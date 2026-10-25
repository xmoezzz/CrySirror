// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker's WPX decoder/retrieval/transcription helpers,
// Copyright (C) 2016 rr-. The bit buffer and direct byte cursor intentionally
// share a stream: literal bytes and distances can bypass buffered flag bits.
use super::common::*;
use anyhow::{ensure, Context, Result};
use crysirror_image::ImageSurface;
use std::collections::BTreeMap;
struct Reader<'a> {
    cursor: Cursor<'a>,
    buffer: u64,
    left: usize,
}
impl<'a> Reader<'a> {
    fn new(b: &'a [u8], pos: usize) -> Result<Self> {
        Ok(Self {
            cursor: Cursor::at(b, pos)?,
            buffer: 0,
            left: 0,
        })
    }
    fn bits(&mut self, n: usize) -> Result<usize> {
        ensure!(n <= 31, "WPX bit width overflow");
        while self.left < n {
            self.buffer = (self.buffer << 8) | self.cursor.u8()? as u64;
            self.left += 8;
        }
        self.left -= n;
        Ok(((self.buffer >> self.left) & ((1u64 << n) - 1)) as usize)
    }
    fn count(&mut self) -> Result<usize> {
        let mut n = 0;
        while self.bits(1)? == 0 {
            n += 1;
            ensure!(n < 31, "WPX count overflow");
        }
        Ok((1usize << n | self.bits(n)?) - 1)
    }
}
fn table(r: &mut Reader<'_>) -> Result<Vec<(u8, u8)>> {
    let mut sizes = [0usize; 256];
    for i in (0..256).step_by(2) {
        sizes[i + 1] = r.bits(4)?;
        sizes[i] = r.bits(4)?;
    }
    let mut table = vec![(0, 0); 32768];
    for (i, &size) in sizes.iter().enumerate() {
        if size > 0 {
            let index = (r.bits(size)? << (15 - size)) & 0x7fff;
            table[index] = (size as u8, i as u8);
        }
    }
    Ok(table)
}
fn symbol(r: &mut Reader<'_>, table: &[(u8, u8)]) -> Result<u8> {
    let mut index = 0;
    let mut mask = 0x4000;
    for size in 1..=15 {
        if r.bits(1)? != 0 {
            index |= mask;
        }
        if table[index].0 == size {
            return Ok(table[index].1);
        }
        mask >>= 1;
    }
    anyhow::bail!("invalid WPX retrieval code")
}
#[derive(Clone, Copy)]
struct Section {
    format: u8,
    pos: usize,
    size: usize,
    comp: usize,
}
fn section(b: &[u8], s: Section, channels: usize, w: u32) -> Result<Vec<u8>> {
    bounded(s.size)?;
    if s.format & 128 != 0 || s.comp == 0 {
        return Ok(Cursor::at(b, s.pos)?.take(s.size)?.to_vec());
    }
    ensure!(
        s.format & 1 == 0 || s.format & 8 == 0,
        "unknown WPX transcription type"
    );
    let bytes = Cursor::at(b, s.pos)?.take(s.comp)?;
    ensure!(
        s.size >= channels && bytes.len() >= 4,
        "truncated WPX initial channels"
    );
    let mut out = bytes[..channels].to_vec();
    let mut r = Reader::new(bytes, channels.next_multiple_of(4))?;
    let retriever = if s.format & 4 != 0 {
        1
    } else if s.format & 2 != 0 {
        2
    } else {
        3
    };
    let table = if retriever != 3 {
        table(&mut r)?
    } else {
        Vec::new()
    };
    let mut dict = if retriever == 1 {
        (0..256)
            .flat_map(|i| (0..256).map(move |j| 255u8.wrapping_sub(i as u8).wrapping_sub(j as u8)))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let stride = (w as usize * channels + 3) & !3;
    let offsets = [
        channels,
        channels * 2,
        channels * 3,
        if5(channels, stride, channels * 4, stride - channels),
        if5(channels, stride, channels * 5, stride),
        if5(channels, stride, channels * 6, stride + channels),
        if5(channels, stride, channels * 7, stride + channels * 2),
        if5(channels, stride, channels * 8, stride * 2),
    ];
    while out.len() < s.size {
        if r.bits(1)? != 0 {
            let value = match retriever {
                1 => {
                    let base = out[out.len() - channels] as usize * 256;
                    let position = symbol(&mut r, &table)? as usize;
                    let value = dict[base + position];
                    dict.copy_within(base..base + position, base + 1);
                    dict[base] = value;
                    value
                }
                2 => symbol(&mut r, &table)?,
                _ => r.cursor.u8()?,
            };
            out.push(value);
        } else {
            let (distance, base) = if s.format & 1 != 0 {
                ensure!(s.format & 8 == 0, "unknown WPX transcription type");
                if r.bits(1)? != 0 {
                    if r.bits(1)? != 0 {
                        (r.cursor.u8()? as usize + 1, 2)
                    } else {
                        (r.cursor.u16()? as usize + 1, 3)
                    }
                } else {
                    (offsets[r.bits(3)?], if channels == 1 { 2 } else { 1 })
                }
            } else {
                (offsets[r.bits(3)?], if channels == 1 { 2 } else { 1 })
            };
            let count = base + r.count()?;
            ensure!(
                distance > 0 && distance <= out.len(),
                "WPX backreference outside decoded prefix"
            );
            for _ in 0..count.min(s.size - out.len()) {
                out.push(out[out.len() - distance]);
            }
        }
    }
    Ok(out)
}
fn if5(channels: usize, stride: usize, a: usize, b: usize) -> usize {
    if channels * 5 >= stride {
        a
    } else {
        b
    }
}
fn image(
    b: &[u8],
    sections: &BTreeMap<u8, Section>,
    id: u8,
    w: u32,
    h: u32,
    channels: usize,
) -> Result<ImageSurface> {
    let s = *sections.get(&id).context("missing WPX image section")?;
    let data = section(b, s, channels, w)?;
    let stride = (w as usize * channels + 3) & !3;
    ensure!(
        data.len() >= stride * h as usize,
        "truncated WPX padded scanlines"
    );
    let mut packed = Vec::with_capacity(w as usize * h as usize * channels);
    for y in 0..h as usize {
        packed.extend_from_slice(&data[y * stride..y * stride + w as usize * channels]);
    }
    raw(
        w,
        h,
        &packed,
        match channels {
            1 => Format::Gray,
            3 => Format::Bgr,
            _ => Format::Bgra,
        },
    )
}
pub(super) fn decode(b: &[u8]) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 12)?;
    ensure!(c.u8()? == 1, "invalid WPX header");
    c.skip(1)?;
    let count = c.u8()?;
    c.skip(1)?;
    let mut sections = BTreeMap::new();
    for _ in 0..count {
        let id = c.u8()?;
        let format = c.u8()?;
        c.skip(2)?;
        let s = Section {
            format,
            pos: c.u32()? as usize,
            size: bounded(c.u32()? as usize)?,
            comp: c.u32()? as usize,
        };
        ensure!(sections.insert(id, s).is_none(), "duplicate WPX section");
    }
    let meta = *sections.get(&16).context("missing WPX metadata")?;
    ensure!(
        meta.format & 128 != 0 || meta.comp == 0,
        "WPX metadata is compressed"
    );
    let bytes = section(b, meta, 1, 1)?;
    let mut m = Cursor::at(&bytes, 4)?;
    let w = m.u16()? as u32;
    let h = m.u16()? as u32;
    pixels(w, h)?;
    m.skip(4)?;
    let depth = m.u8()?;
    ensure!(
        depth == 8 || depth == 24 || depth == 32,
        "unknown WBM bit depth"
    );
    let mut out = image(b, &sections, 17, w, h, (depth / 8) as usize)?;
    if sections.contains_key(&19) {
        let mask = image(b, &sections, 19, w, h, 1)?;
        for (p, m) in out.rgba.chunks_exact_mut(4).zip(mask.rgba.chunks_exact(4)) {
            p[3] = m[0];
        }
    }
    Ok(out)
}
