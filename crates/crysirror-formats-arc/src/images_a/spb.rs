// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from arc_unpacker NScripter SPB decoder, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::Result;
use crysirror_image::ImageSurface;
pub fn decode(bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    let w = u16::from_be_bytes(c.take(2)?.try_into().unwrap()) as u32;
    let h = u16::from_be_bytes(c.take(2)?.try_into().unwrap()) as u32;
    let mut out = image(w, h)?;
    let mut b = Bits::new(c.take(c.left())?, true);
    let n = w as usize * h as usize;
    for ch in [2, 1, 0] {
        let mut val = b.get(8)? as u8;
        let mut plane = vec![val];
        while plane.len() < n {
            let t = b.get(3)?;
            if t == 0 {
                plane.extend([val; 4]);
            } else {
                let mask = if t == 7 { b.get(1)? + 1 } else { t + 2 } as usize;
                for _ in 0..4 {
                    if mask == 8 {
                        val = b.get(8)? as u8;
                    } else {
                        let v = b.get(mask)? as u8;
                        val = if v & 1 != 0 {
                            val.wrapping_add((v >> 1) + 1)
                        } else {
                            val.wrapping_sub(v >> 1)
                        };
                    }
                    plane.push(val);
                }
            }
        }
        for y in 0..h as usize {
            for x in 0..w as usize {
                let xx = if y & 1 != 0 { w as usize - 1 - x } else { x };
                out.rgba[(y * w as usize + xx) * 4 + ch] = plane[y * w as usize + x];
            }
        }
    }
    for p in out.rgba.chunks_exact_mut(4) {
        p[3] = 255;
    }
    Ok(out)
}
