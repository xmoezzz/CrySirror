// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from arc_unpacker, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{bail, ensure, Result};
use crysirror_image::{ImageDecodeContext, ImageSurface};
fn sgd(b: &[u8], n: usize) -> Result<Vec<u8>> {
    let mut c = Cursor::new(b);
    let alpha_size = c.u32()? as usize;
    let mut a = Cursor::new(c.take(alpha_size)?);
    let mut out = vec![0; n * 4];
    let mut pos = 0;
    while !a.rest().is_empty() {
        let flag = a.u16()?;
        if flag & 0x8000 != 0 {
            let value = !a.u8()?;
            let count = (flag & 0x7fff) as usize + 1;
            ensure!(count <= n - pos, "SGD alpha exceeds canvas");
            for _ in 0..count {
                out[pos * 4 + 3] = value;
                pos += 1;
            }
        } else {
            let count = flag as usize;
            ensure!(count <= n - pos, "SGD alpha literal exceeds canvas");
            for _ in 0..count {
                out[pos * 4 + 3] = !a.u8()?;
                pos += 1;
            }
        }
    }
    let color_size = c.u32()? as usize;
    let mut colors = Cursor::new(c.take(color_size)?);
    let mut pos = 0;
    while !colors.rest().is_empty() {
        let flag = colors.u8()?;
        match flag & 0xc0 {
            0x80 => {
                ensure!(pos > 0, "SGD delta before first pixel");
                let mut value = [
                    out[(pos - 1) * 4],
                    out[(pos - 1) * 4 + 1],
                    out[(pos - 1) * 4 + 2],
                ];
                let count = (flag & 63) as usize;
                ensure!(count <= n - pos, "SGD delta exceeds canvas");
                for _ in 0..count {
                    let delta = colors.u16()?;
                    for j in 0..3 {
                        let v = (delta >> (j * 5)) & 31;
                        let change = if delta & 0x8000 != 0 {
                            v as i16
                        } else {
                            (v & 15) as i16 * if v & 16 != 0 { -1 } else { 1 }
                        };
                        value[j] = value[j].wrapping_add(change as u8);
                    }
                    out[pos * 4..pos * 4 + 3].copy_from_slice(&value);
                    pos += 1;
                }
            }
            0x40 => {
                let value = colors.take(3)?;
                let count = (flag & 63) as usize + 1;
                ensure!(count <= n - pos, "SGD repeat exceeds canvas");
                for _ in 0..count {
                    out[pos * 4..pos * 4 + 3].copy_from_slice(value);
                    pos += 1;
                }
            }
            0 => {
                let count = flag as usize;
                ensure!(count <= n - pos, "SGD literal exceeds canvas");
                for _ in 0..count {
                    out[pos * 4..pos * 4 + 3].copy_from_slice(colors.take(3)?);
                    pos += 1;
                }
            }
            _ => bail!("invalid SGD color command"),
        }
    }
    Ok(out)
}
pub(super) fn decode(b: &[u8], ctx: &ImageDecodeContext<'_>) -> Result<ImageSurface> {
    let mut c = Cursor::at(b, 12)?;
    let w = c.u16()? as u32;
    let h = c.u16()? as u32;
    let n = pixels(w, h)?;
    let size = bounded(c.u32()? as usize)?;
    let total = c.u32()? as usize;
    let kind = c.u32()?;
    c.skip(64)?;
    let comp = c.u32()? as usize;
    ensure!(
        total
            == comp
                .checked_add(4)
                .ok_or_else(|| anyhow::anyhow!("MGD size overflow"))?,
        "MGD compressed size mismatch"
    );
    let bytes = c.take(comp)?;
    let image = match kind {
        0 => raw(w, h, bytes, Format::Bgra)?,
        1 => {
            ensure!(size == n * 4, "SGD decoded size mismatch");
            raw(w, h, &sgd(bytes, n)?, Format::Bgra)?
        }
        2 => ctx.decode_embedded("image.png", bytes)?,
        _ => bail!("unknown MGD compression"),
    };
    while !c.rest().is_empty() {
        c.skip(4)?;
        let size = c.u32()? as usize;
        let count = c.u16()? as usize;
        ensure!(c.u16()? == 4, "unknown MGD region format");
        ensure!(size == c.rest().len(), "MGD region size mismatch");
        c.skip(count * 8)?;
        if c.rest().len() < 4 {
            break;
        }
        c.skip(4)?;
    }
    Ok(image)
}
