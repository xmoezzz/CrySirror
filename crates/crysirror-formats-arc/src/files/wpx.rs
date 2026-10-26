// SPDX-License-Identifier: GPL-3.0-or-later
// WPX byte sections translated from arc_unpacker, Copyright (C) 2016 rr-.
use super::compression::{copy, Bits};
use super::{at, u32at};
use anyhow::{ensure, Context, Result};
/// Shared WPX section codec used by both WBI files and WWA audio.
pub(crate) fn section(d: &[u8], id: u8, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        d.starts_with(b"WPX\x1a") && at(d, 12, 1)? == [1],
        "WPX header"
    );
    let count = at(d, 14, 1)?[0] as usize;
    let mut found = None;
    for i in 0..count {
        let p = 16 + i * 16;
        if at(d, p, 1)?[0] == id {
            found = Some((
                at(d, p + 1, 1)?[0],
                u32at(d, p + 4)? as usize,
                u32at(d, p + 8)? as usize,
                u32at(d, p + 12)? as usize,
            ));
        }
    }
    let (fmt, offset, n, packed) = found.context("missing WPX section")?;
    ensure!(n <= limit, "WPX section output limit");
    if fmt & 128 != 0 || packed == 0 {
        return Ok(at(d, offset, n)?.to_vec());
    }
    ensure!(
        fmt & 8 == 0,
        "unsupported WPX plain transcription compression"
    );
    ensure!(n > 0, "empty compressed WPX section");
    let data = at(d, offset, packed)?;
    let mut o = vec![at(data, 0, 1)?[0]];
    let mut bits = Bits::new(at(data, 4, data.len().saturating_sub(4))?);
    let mut table = vec![[0u8; 2]; 32768];
    if fmt & 6 != 0 {
        let mut sizes = [0u8; 256];
        for i in (0..256).step_by(2) {
            sizes[i + 1] = bits.get(4)? as u8;
            sizes[i] = bits.get(4)? as u8;
        }
        for (i, &s) in sizes.iter().enumerate() {
            if s > 0 {
                let index = (bits.get(s as usize)? << (15 - s)) as usize & 32767;
                table[index] = [s, i as u8];
            }
        }
    }
    let mut dict = if fmt & 4 != 0 {
        let mut v = vec![0u8; 65536];
        for i in 0..256 {
            for j in 0..256 {
                v[i * 256 + j] = 255u8.wrapping_sub(i as u8).wrapping_sub(j as u8);
            }
        }
        v
    } else {
        Vec::new()
    };
    while o.len() < n {
        if bits.get(1)? != 0 {
            let byte = if fmt & 6 == 0 {
                bits.c.u8()?
            } else {
                let mut index = 0;
                let mut found = None;
                for width in 1..=15 {
                    index |= (bits.get(1)? as usize) << (15 - width);
                    if table[index][0] as usize == width {
                        found = Some(table[index][1]);
                        break;
                    }
                }
                let value = found.context("WPX retrieval Huffman code")?;
                if fmt & 4 != 0 {
                    let base = o[o.len() - 1] as usize * 256;
                    let p = value as usize;
                    let b = dict[base + p];
                    dict.copy_within(base..base + p, base + 1);
                    dict[base] = b;
                    b
                } else {
                    value
                }
            };
            o.push(byte);
        } else {
            let (dist, base) = if bits.get(1)? != 0 {
                (bits.c.u8()? as usize + 1, 2)
            } else {
                (bits.c.le16()? as usize + 1, 3)
            };
            let mut zeros = 0;
            while bits.get(1)? == 0 {
                zeros += 1;
                ensure!(zeros <= 30, "WPX repetition width");
            }
            let mut count = 1usize;
            for _ in 0..zeros {
                count = (count << 1) + bits.get(1)? as usize;
            }
            copy(&mut o, dist, base + count - 1, n)?;
        }
    }
    Ok(o)
}
