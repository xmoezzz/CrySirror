// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Result};
use crysirror_image::ImageSurface;
fn cg(
    out: &mut [u8],
    offset: usize,
    shift: usize,
    input_shift: usize,
    c: &mut Cursor<'_>,
) -> Result<()> {
    ensure!(
        shift >= input_shift && offset + input_shift <= shift && out.len() % shift == 0,
        "invalid CG channel layout"
    );
    let size = c.u32()? as usize;
    let comp = bounded(c.u32()? as usize)?;
    let count = c.u16()? as usize;
    c.skip(2)?;
    ensure!(
        count > 0 && size == out.len() / shift * input_shift,
        "CG decoded size/table mismatch"
    );
    let table = c.take(count * input_shift)?;
    let mut bits = Bits::new(c.take(comp)?);
    let (escape, prefix) = if count < 4096 { (6, 3) } else { (14, 4) };
    let mut written = 0;
    let pixels = out.len() / shift;
    while written < pixels {
        let mut sequence = 1;
        let mut width = bits.get(prefix)?;
        if width == 0 {
            sequence = bits.get(4)? + 2;
            width = bits.get(prefix)?;
        }
        ensure!(width > 0, "invalid CG table prefix");
        let index = if width == 1 {
            bits.get(1)?
        } else {
            width -= 1;
            if width >= escape {
                while bits.get(1)? != 0 {
                    width += 1;
                    ensure!(width < 31, "CG table index overflow");
                }
            }
            (1usize << width) | bits.get(width)?
        };
        ensure!(index < count, "CG table index outside dictionary");
        for _ in 0..sequence.min(pixels - written) {
            let target = written * shift + offset;
            out[target..target + input_shift]
                .copy_from_slice(&table[index * input_shift..(index + 1) * input_shift]);
            written += 1;
        }
    }
    Ok(())
}
pub(super) fn decode(b: &[u8], wcg: bool) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 2)?;
    let version = c.u16()?;
    let depth = c.u16()?;
    let bits = c.u16()?;
    let w = c.u32()?;
    let h = c.u32()?;
    let n = pixels(w, h)?;
    if wcg {
        ensure!(
            version & 15 == 1 && version & 0x1c0 == 64 && depth == 32,
            "unsupported WCG version/depth"
        );
        let mut data = vec![0; n * 4];
        cg(&mut data, 2, 4, 2, &mut c)?;
        cg(&mut data, 0, 4, 2, &mut c)?;
        for p in data.chunks_exact_mut(4) {
            p[3] ^= 255;
        }
        raw(w, h, &data, Format::Bgra)
    } else {
        ensure!(
            version & 0x10 != 0 && version & 15 != 0 && depth == 16 && bits == 8,
            "unsupported LIM version/depth"
        );
        let mut data = vec![0; n * 2];
        cg(&mut data, 0, 2, 2, &mut c)?;
        let mut out = raw(w, h, &data, Format::Bgr565)?;
        if !c.rest().is_empty() {
            let mut mask = vec![0; n];
            cg(&mut mask, 0, 1, 1, &mut c)?;
            for (p, m) in out.rgba.chunks_exact_mut(4).zip(mask) {
                p[3] = !m;
            }
        }
        Ok(out)
    }
}
