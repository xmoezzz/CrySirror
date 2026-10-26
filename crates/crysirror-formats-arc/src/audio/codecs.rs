// SPDX-License-Identifier: GPL-3.0-or-later
// Translated from arc_unpacker, Copyright (C) 2016 rr-.
use super::{at, u16at, u32at, Audio};
use crate::common::Cursor;
use anyhow::{ensure, Context, Result};
const STEP: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4028, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const WADY: [i16; 64] = [
    0, 2, 4, 6, 8, 10, 12, 15, 18, 21, 24, 28, 32, 36, 40, 44, 49, 54, 59, 64, 70, 76, 82, 88, 95,
    102, 109, 116, 124, 132, 140, 148, 160, 170, 180, 190, 200, 210, 220, 230, 240, 255, 270, 285,
    300, 320, 340, 360, 380, 400, 425, 450, 475, 500, 525, 550, 580, 610, 650, 700, 750, 800, 900,
    1000,
];
fn bytes(samples: impl IntoIterator<Item = i16>) -> Vec<u8> {
    samples.into_iter().flat_map(i16::to_le_bytes).collect()
}
pub fn v(d: &[u8], limit: usize) -> Result<Audio> {
    let rate = u32at(d, 0)?;
    let channels = u16at(d, 4)?;
    ensure!(matches!(channels, 1 | 2), "V channel count");
    let data = at(d, 196, d.len().saturating_sub(196))?;
    ensure!(data.len() <= limit / 4, "V decoded size limit");
    let mut prev = [0i32; 2];
    let mut acc = [0usize; 2];
    let mut out = Vec::with_capacity(data.len() * 4);
    for b in data {
        for (j, v) in [b >> 4, b & 15].into_iter().enumerate() {
            let c = if channels == 1 { 0 } else { j };
            let mul = STEP[acc[c]];
            let low = v & 7;
            acc[c] =
                (acc[c] as i32 + [-1, -1, -1, -1, 2, 4, 6, 8][low as usize]).clamp(0, 88) as usize;
            let delta = ((low as i32 * 2 + 1) * mul) >> 3;
            prev[c] = (prev[c] + if v & 8 != 0 { -delta } else { delta }).clamp(-32768, 32767);
            out.extend_from_slice(&(prev[c] as i16).to_le_bytes());
        }
    }
    Ok(Audio::pcm(channels, rate, 16, out))
}
pub fn triangle(d: &[u8], limit: usize) -> Result<Audio> {
    let channels = u16at(d, 6)?;
    ensure!(channels > 0 && channels <= 32, "WADY channels");
    let rate = u32at(d, 8)?;
    let data = at(d, 48, d.len().saturating_sub(48))?;
    ensure!(
        data.len() % channels as usize == 0 && data.len() <= limit / 2,
        "WADY size/channel limit"
    );
    let mut held = vec![0i16; channels as usize];
    let mul = d[5] as i16;
    let mut out = Vec::with_capacity(data.len() * 2);
    for (i, &v) in data.iter().enumerate() {
        let s = &mut held[i % channels as usize];
        *s = if v & 128 != 0 {
            (v as i16).wrapping_shl(9)
        } else if v & 64 != 0 {
            s.wrapping_sub(mul.wrapping_mul(WADY[(v & 63) as usize]))
        } else {
            s.wrapping_add(mul.wrapping_mul(WADY[(v & 63) as usize]))
        };
        out.extend_from_slice(&s.to_le_bytes());
    }
    Ok(Audio::pcm(channels, rate, 16, out))
}
pub fn ivory(d: &[u8], limit: usize) -> Result<Audio> {
    let channels = u16at(d, 6)? as usize;
    ensure!(channels == 1 || channels == 2, "Ivory WADY channels");
    let rate = u32at(d, 8)?;
    let count = u32at(d, 12)? as usize;
    let per = u32at(d, 16)? as usize;
    ensure!(
        per.checked_mul(channels) == Some(count)
            && count.checked_mul(2) == Some(u32at(d, 20)? as usize),
        "WADY sample counts"
    );
    ensure!(
        u16at(d, 34)? as usize == channels && u32at(d, 36)? == rate,
        "WADY repeated format mismatch"
    );
    let data = at(d, 48, d.len().saturating_sub(48))?;
    let version2 = (|| -> Result<bool> {
        let mut c = Cursor::new(data);
        if channels == 2 {
            for _ in 0..2 {
                let n = c.le32()? as usize;
                c.take(n)?;
            }
        } else {
            c.take(4)?;
            let n = c.le32()?;
            c.take(2)?;
            for _ in 0..n {
                if c.u8()? & 1 == 0 {
                    c.u8()?;
                }
            }
        }
        Ok(c.left() == 0)
    })()
    .unwrap_or(false);
    // Upstream allocates sample_count * channels despite sample_count being scalar.
    // Keep that byte contract, including its trailing zero samples.
    let units = count
        .checked_mul(channels)
        .context("WADY sample overflow")?;
    ensure!(units <= limit / 2, "WADY output limit");
    let mut samples = vec![0i16; units];
    if !version2 {
        let mul = u16at(d, 44)? as i16;
        let mut prev = [0i16; 2];
        for (i, &v) in data.iter().take(units).enumerate() {
            let c = i % channels;
            prev[c] = if v & 128 != 0 {
                (v as i16).wrapping_shl(9)
            } else {
                let delta = WADY[(v & 63) as usize].wrapping_mul(mul);
                if v & 64 != 0 {
                    prev[c].wrapping_sub(delta)
                } else {
                    prev[c].wrapping_add(delta)
                }
            };
            samples[i] = prev[c];
        }
    } else {
        const TABLE: [i16; 32] = [
            0, 4, 8, 12, 19, 24, 30, 38, 47, 59, 74, 92, 115, 144, 180, 225, 281, 352, 440, 550,
            687, 859, 1073, 1342, 1678, 2097, 2621, 3277, 4096, 5120, 6400, 8000,
        ];
        const REPEAT: [usize; 8] = [3, 4, 5, 6, 8, 16, 32, 256];
        let mut c = Cursor::new(data);
        for ch in 0..channels {
            let n = if channels == 1 {
                c.left()
            } else {
                c.le32()? as usize
            };
            let mut s = Cursor::new(c.take(n)?);
            s.take(4)?;
            let mut left = s.le32()?;
            let mut prev = s.le16()? as i16;
            samples[0] = prev;
            let mut p = channels + ch;
            while left > 0 && p < units {
                if left == 300 {
                    prev = 0;
                }
                let b = s.u8()?;
                if b & 1 != 0 {
                    prev = if b & 128 != 0 {
                        ((b & !1) as i16).wrapping_shl(9)
                    } else {
                        let delta = TABLE[((b >> 1) & 31) as usize];
                        if b & 64 != 0 {
                            prev.wrapping_sub(delta)
                        } else {
                            prev.wrapping_add(delta)
                        }
                    };
                    samples[p] = prev;
                    p += channels;
                } else {
                    let v = ((s.u8()? as u16) << 8) | b as u16;
                    let repeat = REPEAT[((v >> 1) & 7) as usize];
                    let target = (v & 0xfff0) as i16;
                    let mut sample = prev as f32;
                    let delta = (target as f32 - sample) / repeat as f32;
                    for _ in 0..repeat {
                        if p >= units {
                            break;
                        }
                        sample += delta;
                        samples[p] = sample as i16;
                        p += channels;
                    }
                    prev = target;
                }
                left -= 1;
            }
        }
    }
    let mut a = Audio::pcm(channels as u16, rate, u16at(d, 46)?, bytes(samples));
    a.align = u16at(d, 44)?;
    a.byte_rate = u32at(d, 40)?;
    Ok(a)
}
struct Lsb<'a> {
    d: &'a [u8],
    bit: usize,
}
impl Lsb<'_> {
    fn get(&mut self, n: usize) -> Result<u32> {
        ensure!(
            self.bit
                .checked_add(n)
                .is_some_and(|e| e <= self.d.len() * 8),
            "truncated NWA block"
        );
        let mut v = 0;
        for i in 0..n {
            v |= (((self.d[self.bit / 8] >> (self.bit % 8)) & 1) as u32) << i;
            self.bit += 1;
        }
        Ok(v)
    }
}
pub fn nwa(d: &[u8], limit: usize) -> Result<Audio> {
    let channels = u16at(d, 0)?;
    let width = u16at(d, 2)?;
    let rate = u32at(d, 4)?;
    let level = u32at(d, 8)? as i32;
    let run = u32at(d, 12)? != 0;
    let blocks = u32at(d, 16)? as usize;
    let n = u32at(d, 20)? as usize;
    let total = u32at(d, 28)? as usize;
    let block = u32at(d, 32)? as usize;
    let rest = u32at(d, 36)? as usize;
    ensure!(
        n <= limit && matches!(channels, 1 | 2) && matches!(width, 8 | 16),
        "NWA size/channel/width"
    );
    ensure!(n == total * width as usize / 8, "NWA sample byte count");
    let samples = if level == -1 {
        at(d, 40, n)?.to_vec()
    } else {
        ensure!(
            (0..=5).contains(&level) && blocks > 0,
            "NWA compression/blocks"
        );
        ensure!(
            (blocks - 1)
                .checked_mul(block)
                .and_then(|v| v.checked_add(rest))
                == Some(total),
            "NWA block sample count"
        );
        ensure!(blocks <= d.len().saturating_sub(44) / 4, "NWA block table");
        let mut out = Vec::with_capacity(n);
        for i in 0..blocks {
            let start = u32at(d, 44 + i * 4)? as usize;
            let end = if i + 1 < blocks {
                u32at(d, 48 + i * 4)? as usize
            } else {
                d.len()
            };
            ensure!(
                start >= 44 + blocks * 4 && end >= start,
                "NWA block offsets"
            );
            let mut c = Cursor::new(at(d, start, end - start)?);
            let mut prev = [0i16; 2];
            for p in prev.iter_mut().take(channels as usize) {
                *p = if width == 8 {
                    c.u8()? as i16
                } else {
                    c.le16()? as i16
                };
            }
            let mut bits = Lsb {
                d: c.take(c.left())?,
                bit: 0,
            };
            let count = if i + 1 < blocks { block } else { rest };
            let mut skip = 0;
            for j in 0..count {
                let ch = j % channels as usize;
                if skip > 0 {
                    skip -= 1;
                } else {
                    let ty = bits.get(3)? as i32;
                    if ty == 7 && bits.get(1)? != 0 {
                        prev[ch] = 0;
                    } else if ty > 0 {
                        let (b, s) = if ty == 7 {
                            if level < 3 {
                                (8 - level, 9 + level)
                            } else {
                                (8, 9)
                            }
                        } else if level >= 3 {
                            (3 + level, 1 + ty)
                        } else {
                            (5 - level, 2 + ty + level)
                        };
                        let v = bits.get(b as usize)?;
                        let magnitude = (v & ((1 << (b - 1)) - 1)) << s;
                        let delta = if v & (1 << (b - 1)) != 0 {
                            -(magnitude as i32)
                        } else {
                            magnitude as i32
                        };
                        prev[ch] = prev[ch].wrapping_add(delta as i16);
                    } else if run {
                        skip = bits.get(1)?;
                        if skip == 1 {
                            skip = bits.get(2)?;
                            if skip == 3 {
                                skip = bits.get(8)?;
                            }
                        }
                    }
                }
                if width == 8 {
                    out.push(prev[ch] as u8);
                } else {
                    out.extend_from_slice(&prev[ch].to_le_bytes());
                }
            }
        }
        ensure!(out.len() == n, "NWA decoded bytes mismatch");
        out
    };
    Ok(Audio::pcm(channels, rate, width, samples))
}
