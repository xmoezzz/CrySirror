// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
struct Bit<'a> {
    b: &'a [u8],
    p: usize,
}
impl Bit<'_> {
    fn read(&mut self, n: usize) -> Result<u32> {
        let mut v = 0;
        for _ in 0..n {
            let b = *self
                .b
                .get(self.p / 8)
                .context("truncated ActiveSoft bitstream")?;
            v = (v << 1) | u32::from((b >> (self.p % 8)) & 1);
            self.p += 1;
        }
        Ok(v)
    }
    fn gamma(&mut self) -> Result<usize> {
        let mut n = 0;
        while self.read(1)? != 0 {
            n += 1;
            ensure!(n < 31, "ActiveSoft gamma overflow");
        }
        let mut v = 1;
        for _ in 0..n {
            v = (v << 1) | self.read(1)? as usize;
        }
        Ok(v)
    }
}
pub fn ed8(b: &[u8]) -> Result<ImageSurface> {
    let w = u32::from(u16le(b, 14)?);
    let h = u32::from(u16le(b, 16)?);
    let n = rgba_len(w, h)? / 4;
    let count = u32le(b, 18)? as usize;
    ensure!(count <= 256, "ED8 palette too large");
    let data_size = u32le(b, 22)? as usize;
    let mut r = Reader::new(b, 26);
    let mut palette = Vec::with_capacity(count);
    for _ in 0..count {
        let p = r.take(3)?;
        palette.push([p[2], p[1], p[0], 255]);
    }
    let mut bit = Bit {
        b: r.take(data_size)?,
        p: 0,
    };
    let shifts = [
        (-1, 0),
        (0, -1),
        (-2, 0),
        (-1, -1),
        (1, -1),
        (0, -2),
        (-2, -1),
        (2, -1),
        (-2, -2),
        (-1, -2),
        (1, -2),
        (2, -2),
        (0, -3),
        (-1, -3),
    ];
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        out.push(bit.read(8)? as u8);
        if out.len() == n {
            break;
        }
        if bit.read(1)? != 0 {
            continue;
        }
        let mut last = None;
        while out.len() < n {
            let mut idx = 0;
            if bit.read(1)? != 0 {
                if bit.read(1)? != 0 {
                    idx = bit.read(1)? as usize + 1;
                }
                idx = (idx << 1) + bit.read(1)? as usize + 1;
            }
            idx = (idx << 1) + bit.read(1)? as usize;
            if last == Some(idx) {
                break;
            }
            last = Some(idx);
            let count = bit.gamma()? + usize::from(idx >= 2);
            let (dx, dy) = *shifts.get(idx).context("invalid ED8 neighbor index")?;
            let delta = i64::from(dx) + i64::from(dy) * i64::from(w);
            ensure!(delta < 0, "ED8 forward neighbor");
            copy_back(&mut out, usize::try_from(-delta)?, count, n)?;
        }
    }
    indexed(w, h, &out, &palette)
}
pub fn edt(b: &[u8]) -> Result<ImageSurface> {
    let w = u32::from(u16le(b, 14)?);
    let h = u32::from(u16le(b, 16)?);
    let n = rgba_len(w, h)? / 4 * 3;
    let meta_size = u32le(b, 22)? as usize;
    let data_size = u32le(b, 26)? as usize;
    let raw_size = u32le(b, 30)? as usize;
    let mut r = Reader::new(b, 34);
    let meta = r.take(meta_size)?;
    let transparent = if meta.starts_with(b".EDT_DIFF\0") && meta.len() > 14 {
        Some(slice(meta, 10, 3)?)
    } else {
        None
    };
    let mut bit = Bit {
        b: r.take(data_size)?,
        p: 0,
    };
    let mut raw = Reader::new(r.take(raw_size)?, 0);
    let mut neighbors = Vec::new();
    for y in -4..0 {
        for x in -3..=3 {
            neighbors.push(-((x + i64::from(w) * y) * 3));
        }
    }
    for x in -4..0 {
        neighbors.push(-x * 3);
    }
    let mut out = raw.take(3)?.to_vec();
    while out.len() < n {
        if bit.read(1)? == 0 {
            out.extend(raw.take(3)?);
            continue;
        }
        if bit.read(1)? != 0 {
            let mut distance = 3;
            if bit.read(1)? != 0 {
                let shift = bit.read(2)? * 8;
                distance = neighbors[((0x11191718u32 >> shift) & 255) as usize];
            }
            let distance = usize::try_from(distance).context("invalid EDT neighbor")?;
            ensure!(
                distance > 0 && distance <= out.len(),
                "invalid EDT back-reference"
            );
            for _ in 0..3 {
                let mut v = out[out.len() - distance].clamp(2, 253);
                if bit.read(1)? != 0 {
                    let d = bit.read(1)? as u8 + 1;
                    v = if bit.read(1)? != 0 {
                        v.wrapping_add(d)
                    } else {
                        v.wrapping_sub(d)
                    };
                }
                out.push(v);
            }
        } else {
            let idx = bit.read(5)? as usize;
            let count = bit.gamma()?.checked_mul(3).context("EDT run overflow")?;
            let distance = usize::try_from(neighbors[idx]).context("invalid EDT neighbor")?;
            let count = count.min(n - out.len());
            copy_back(&mut out, distance, count, n)?;
        }
    }
    let mut image = raw_surface(w, h, &out)?;
    if let Some(c) = transparent {
        for p in image.rgba.chunks_exact_mut(4) {
            if p[..3] == [c[2], c[1], c[0]] {
                p[3] = 0;
            }
        }
    }
    Ok(image)
}
fn raw_surface(w: u32, h: u32, b: &[u8]) -> Result<ImageSurface> {
    super::super::util::raw(w, h, b, 3)
}
