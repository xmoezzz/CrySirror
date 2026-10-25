// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from arc_unpacker Kaguya image decoders, Copyright (C) 2016 rr-.
use super::common::*;
use anyhow::{ensure, Result};
use crysirror_image::ImageSurface;
pub fn decode(id: &str, bytes: &[u8]) -> Result<ImageSurface> {
    let mut c = C::new(bytes);
    match id {
        "kaguya/ap" => {
            ensure!(c.take(2)? == b"AP", "AP magic");
            let (w, h) = (c.u32()?, c.u32()?);
            c.skip(2)?;
            bgr(w, h, c.take(size(w, h, 4)?)?, 4, true)
        }
        "kaguya/ao" => {
            ensure!(c.take(2)? == b"AO", "AO magic");
            let (w, h) = (c.u32()?, c.u32()?);
            c.skip(2)?;
            let (x, y) = (c.u32()?, c.u32()?);
            let part = bgr(w, h, c.take(size(w, h, 4)?)?, 4, true)?;
            let mut out = image(
                x.checked_add(w)
                    .ok_or_else(|| anyhow::anyhow!("AO width overflow"))?,
                y.checked_add(h)
                    .ok_or_else(|| anyhow::anyhow!("AO height overflow"))?,
            )?;
            for yy in 0..h as usize {
                for xx in 0..w as usize {
                    let p = &part.rgba[(yy * w as usize + xx) * 4..][..4];
                    if p[3] != 0 {
                        let i = ((yy + y as usize) * out.width as usize + xx + x as usize) * 4;
                        out.rgba[i..i + 4].copy_from_slice(p);
                    }
                }
            }
            Ok(out)
        }
        "kaguya/ap0" => {
            ensure!(c.take(4)? == b"AP-0", "AP0 magic");
            let (w, h) = (c.u32()?, c.u32()?);
            gray(w, h, c.take(size(w, h, 1)?)?, true)
        }
        "kaguya/ap2" | "kaguya/ap3" => {
            ensure!(
                c.take(4)? == if id.ends_with('2') { b"AP-2" } else { b"AP-3" },
                "AP variant magic"
            );
            c.skip(8)?;
            let (w, h, d) = (c.u32()?, c.u32()?, c.u32()?);
            let ch = if id.ends_with('2') {
                ensure!(matches!(d, 24 | 32), "AP2 depth");
                4
            } else {
                ensure!(d == 24, "AP3 depth");
                3
            };
            bgr(w, h, c.take(size(w, h, ch)?)?, ch, true)
        }
        "kaguya/raw-mask" => {
            let (w, h) = (c.u32()?, c.u32()?);
            gray(w, h, c.take(size(w, h, 1)?)?, false)
        }
        "kaguya/aps3" => {
            ensure!(c.take(5)? == b"\x04APS3", "APS3 magic");
            let n = c.u32()?;
            ensure!(n <= 65536, "APS3 entry limit");
            for _ in 0..n {
                c.skip(4)?;
                let n = c.u8()? as usize;
                c.skip(n + 28)?;
            }
            let n = c.u32()? as usize;
            ensure!(n == c.left(), "APS3 remaining size");
            let compressed = c.u16()? == 1;
            let data = if compressed {
                let (packed, size) = (c.u32()? as usize, c.u32()? as usize);
                ensure!(
                    size <= crysirror_image::MAX_DECODED_BYTES,
                    "APS3 output limit"
                );
                let mut bits = Bits::new(c.take(packed)?, true);
                let mut dict = [0u8; 4096];
                let mut pos = 1;
                let mut out = Vec::with_capacity(size);
                while out.len() < size {
                    if bits.get(1)? != 0 {
                        let b = bits.get(8)? as u8;
                        out.push(b);
                        dict[pos] = b;
                        pos = (pos + 1) & 4095;
                    } else {
                        let mut at = bits.get(12)? as usize;
                        let n = bits.get(4)? as usize + 2;
                        for _ in 0..n.min(size - out.len()) {
                            let b = dict[at];
                            at = (at + 1) & 4095;
                            out.push(b);
                            dict[pos] = b;
                            pos = (pos + 1) & 4095;
                        }
                    }
                }
                out
            } else {
                let n = c.u32()? as usize;
                c.take(n)?.to_vec()
            };
            let id = if data.starts_with(b"AO") {
                "kaguya/ao"
            } else {
                ensure!(
                    data.starts_with(b"AP") && !data.starts_with(b"AP-"),
                    "APS3 unsupported embedded image"
                );
                "kaguya/ap"
            };
            decode(id, &data)
        }
        _ => anyhow::bail!("unknown Kaguya codec"),
    }
}
