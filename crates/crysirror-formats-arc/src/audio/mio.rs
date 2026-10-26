// SPDX-License-Identifier: GPL-3.0-or-later
// Entis MIO translated from arc_unpacker, Copyright (C) 2016 rr-.
// The shared Entis adaptive Huffman implementation is also used by ERI images.
use super::{at, u32at, Audio};
use crate::{common::Cursor, images_a::eri_entropy::Entropy};
use anyhow::{ensure, Context, Result};
const PI: f64 = 3.141592653589;
fn sections(d: &[u8]) -> Result<Vec<(&str, &[u8])>> {
    let mut c = Cursor::new(d);
    let mut v = Vec::new();
    while c.left() > 0 {
        ensure!(v.len() < 65536, "MIO section count limit");
        let name = std::str::from_utf8(c.take(8)?)?.trim_end_matches([' ', '\0']);
        let n = usize::try_from(c.le64()?).context("MIO section size")?;
        v.push((name, c.take(n)?));
    }
    Ok(v)
}
fn one<'a>(s: &[(&str, &'a [u8])], name: &str) -> Result<&'a [u8]> {
    let mut i = s.iter().filter(|v| v.0 == name);
    let d = i.next().with_context(|| format!("MIO missing {name}"))?.1;
    ensure!(i.next().is_none(), "MIO duplicate {name}");
    Ok(d)
}
pub fn decode(d: &[u8], limit: usize) -> Result<Audio> {
    let top = sections(at(d, 64, d.len().saturating_sub(64))?)?;
    let hs = sections(one(&top, "Header")?)?;
    let h = one(&hs, "SoundInf")?;
    let transformation = u32at(h, 4)?;
    let architecture = u32at(h, 8)? as i32;
    let channels = u32at(h, 12)? as usize;
    let rate = u32at(h, 16)?;
    let degree = u32at(h, 24)? as usize;
    let bits = u32at(h, 36)?;
    ensure!(channels == 1 || channels == 2, "MIO channel count");
    ensure!(bits == 16, "MIO only PCM16 is supported upstream");
    let lossless = transformation == 0x03020000;
    ensure!(
        lossless || matches!(transformation, 5 | 0x105),
        "unsupported MIO transformation"
    );
    ensure!(
        architecture == -4 || (!lossless && architecture == -1),
        "unsupported MIO architecture"
    );
    if !lossless {
        ensure!(
            (8..=12).contains(&degree) && u32at(h, 32)? == 1,
            "MIO subband/lapped degree"
        );
    }
    let ss = sections(one(&top, "Stream")?)?;
    let chunks: Vec<_> = ss
        .iter()
        .filter(|v| v.0 == "SoundStm")
        .map(|v| v.1)
        .collect();
    ensure!(!chunks.is_empty(), "MIO no sound chunks");
    let mut entropy = Entropy::new(&[], -4)?;
    let mut ready = false;
    let mut out = Vec::new();
    let mut lap = if lossless {
        Vec::new()
    } else {
        vec![0.0; (1 << degree) * channels]
    };
    for c in chunks {
        let n = u32at(c, 4)? as usize;
        let initial = at(c, 1, 1)?[0] != 0;
        entropy.set_input(at(c, 8, c.len().saturating_sub(8))?);
        if lossless {
            if initial {
                entropy.reset()?;
                ready = true;
            }
            ensure!(ready, "MIO first chunk must initialize entropy state");
            let units = n
                .checked_mul(channels)
                .context("MIO sample count overflow")?;
            ensure!(
                units <= limit.saturating_sub(out.len()) / 2,
                "MIO output limit"
            );
            let decoded = entropy.decode(units * 2)?;
            let mut pcm = vec![0i16; units];
            for ch in 0..channels {
                let base = ch * n * 2;
                let mut value = 0i16;
                let mut delta = 0i16;
                for i in 0..n {
                    let low = decoded[base + n + i] as i8;
                    let high = decoded[base + i] ^ (low >> 7) as u8;
                    let raw = i16::from_le_bytes([low as u8, high]);
                    delta = delta.wrapping_add(raw);
                    value = value.wrapping_add(delta);
                    pcm[i * channels + ch] = value;
                }
            }
            for s in pcm {
                out.extend_from_slice(&s.to_le_bytes());
            }
        } else {
            let mss = channels == 2 && transformation == 0x105;
            let decoded = lossy_chunk(
                &mut entropy,
                &mut ready,
                initial,
                n,
                channels,
                degree,
                mss,
                &mut lap,
                limit.saturating_sub(out.len()),
            )?;
            for s in decoded {
                out.extend_from_slice(&s.to_le_bytes());
            }
        }
    }
    Ok(Audio::pcm(channels as u16, rate, 16, out))
}
#[derive(Clone, Copy)]
struct Param {
    weight: i32,
    coefficient: u32,
    rotation: u8,
}
fn param(d: &mut Entropy<'_>, rotation: usize) -> Result<Param> {
    Ok(Param {
        rotation: if rotation == 0 {
            0
        } else {
            d.bits.get(rotation)? as u8
        },
        weight: d.bits.get(32)? as i32,
        coefficient: d.bits.get(16)?,
    })
}
#[derive(Clone, Copy)]
enum Block {
    Lead,
    Internal,
    Post,
}
#[derive(Clone, Copy)]
struct Action {
    kind: Block,
    degree: usize,
    channel: usize,
    param: Param,
}
fn lossy_chunk(
    e: &mut Entropy<'_>,
    ready: &mut bool,
    initial: bool,
    n: usize,
    channels: usize,
    degree: usize,
    mss: bool,
    lap: &mut [f32],
    limit: usize,
) -> Result<Vec<i16>> {
    let width = 1usize << degree;
    let padded = n
        .checked_add(width - 1)
        .context("MIO padded sample overflow")?
        & !(width - 1);
    let units = padded
        .checked_mul(channels)
        .context("MIO sample overflow")?;
    ensure!(units <= limit / 2, "MIO padded output limit");
    let count = padded >> degree;
    ensure!(e.bits.get(1)? == 0, "MIO leading separator");
    let mut actions = Vec::new();
    let mut divisions = vec![None; channels];
    for i in 0..count {
        for ch in 0..if mss { 1 } else { channels } {
            let div = e.bits.get(2)? as usize;
            let lead = divisions[ch] != Some(div);
            if lead {
                if i > 0 {
                    let old = divisions[ch].context("MIO division state")?;
                    actions.push(Action {
                        kind: Block::Post,
                        degree: degree - old,
                        channel: ch,
                        param: param(e, if mss { 2 } else { 0 })?,
                    });
                }
                divisions[ch] = Some(div);
            }
            for k in 0..1 << div {
                let first = lead && k == 0;
                actions.push(Action {
                    kind: if first { Block::Lead } else { Block::Internal },
                    degree: degree - div,
                    channel: ch,
                    param: param(
                        e,
                        if mss {
                            if first {
                                2
                            } else {
                                4
                            }
                        } else {
                            0
                        },
                    )?,
                });
            }
        }
    }
    if count > 0 {
        for ch in 0..if mss { 1 } else { channels } {
            actions.push(Action {
                kind: Block::Post,
                degree: degree - divisions[ch].context("MIO division state")?,
                channel: ch,
                param: param(e, if mss { 2 } else { 0 })?,
            });
        }
    }
    ensure!(e.bits.get(1)? == 0, "MIO trailing separator");
    if initial {
        e.reset()?;
        *ready = true;
    }
    ensure!(*ready, "MIO first chunk must initialize entropy state");
    let raw = e.decode(units * 2)?;
    let mut quant = vec![0i32; units];
    let stride = width * if mss { 2 } else { 1 };
    let bands = count * if mss { 1 } else { channels };
    let mut p = 0;
    for i in 0..stride {
        for j in 0..bands {
            let low = raw[units + p] as i8 as i32;
            let high = raw[p] as i8 as i32 ^ (low >> 8);
            quant[i + j * stride] = (low & 255) | (high << 8);
            p += 1;
        }
    }
    let mut source = 0;
    let mut output = vec![0i16; units];
    let mut positions = vec![0usize; channels];
    let mut left = vec![n; channels];
    for action in actions {
        let size = 1usize << action.degree;
        let joint = if mss { 2 } else { 1 };
        let mut matrix = Vec::with_capacity(joint * size);
        for _ in 0..joint {
            let q = if matches!(action.kind, Block::Internal) {
                let data = quant
                    .get(source..source + size)
                    .context("MIO transform samples")?;
                source += size;
                data.to_vec()
            } else {
                let half = size / 2;
                let data = quant
                    .get(source..source + half)
                    .context("MIO lapped samples")?;
                source += half;
                let mut data2 = vec![0; size];
                for (i, v) in data.iter().enumerate() {
                    data2[i * 2 + 1] = *v;
                }
                data2
            };
            matrix.extend(dequantize(&q, action.param));
        }
        if mss {
            let code = action.param.rotation;
            let step = if matches!(action.kind, Block::Internal) {
                2
            } else {
                1
            };
            for offset in 0..step {
                let rot = if step == 1 {
                    code
                } else if offset == 0 {
                    (code >> 2) & 3
                } else {
                    code & 3
                };
                let sin = (rot as f64 * PI / 8.0).sin() as f32;
                let cos = (rot as f64 * PI / 8.0).cos() as f32;
                for i in (offset..size).step_by(step) {
                    let (a, b) = (matrix[i], matrix[size + i]);
                    matrix[i] = a * cos - b * sin;
                    matrix[size + i] = a * sin + b * cos;
                }
            }
        }
        let ch = action.channel;
        let samples = if matches!(action.kind, Block::Lead) {
            0
        } else {
            left[ch].min(size)
        };
        for joint_ch in 0..joint {
            let start = joint_ch * size;
            let current = &mut matrix[start..start + size];
            odd_givens(current, action.degree)?;
            if matches!(action.kind, Block::Lead | Block::Post) {
                for i in (0..size).step_by(2) {
                    current[i] = if matches!(action.kind, Block::Lead) {
                        current[i + 1]
                    } else {
                        -current[i + 1]
                    };
                }
            }
            for i in (0..size).step_by(2) {
                let (a, b) = (current[i], current[i + 1]);
                current[i] = 0.5 * (a + b);
                current[i + 1] = 0.5 * (a - b);
            }
            let lap_at = if mss { joint_ch * size } else { ch * width };
            let last = lap
                .get_mut(lap_at..lap_at + size)
                .context("MIO lapped buffer")?;
            if matches!(action.kind, Block::Lead) {
                last.copy_from_slice(current);
            } else {
                let mut overlap = vec![0.0; size];
                for i in (0..size).step_by(2) {
                    overlap[i] = last[i] + current[i + 1];
                    overlap[i + 1] = last[i] - current[i + 1];
                }
                if matches!(action.kind, Block::Internal) {
                    last.copy_from_slice(current);
                }
                let pcm = idct(&overlap);
                let out_ch = if mss { joint_ch } else { ch };
                for (i, v) in pcm.iter().take(samples).enumerate() {
                    ensure!(v.is_finite(), "MIO nonfinite inverse transform");
                    let index = (positions[ch] + i) * channels + out_ch;
                    let target = output.get_mut(index).context("MIO output sample range")?;
                    *target = v.round().clamp(-32768.0, 32767.0) as i16;
                }
            }
        }
        if !matches!(action.kind, Block::Lead) {
            positions[ch] += samples;
            left[ch] -= samples;
        }
    }
    ensure!(
        left.iter()
            .take(if mss { 1 } else { channels })
            .all(|v| *v == 0),
        "MIO incomplete lapped output"
    );
    Ok(output)
}
fn cos_table(n: usize) -> Vec<f32> {
    let dr = PI / (2.0 * n as f64);
    let mut angle = PI / (4.0 * n as f64);
    (0..n)
        .map(|_| {
            let c = angle.cos() as f32;
            angle += dr;
            c
        })
        .collect()
}
fn dct(input: &[f32]) -> Vec<f32> {
    let n = input.len();
    let rc = (PI / 4.0).cos() as f32;
    let mut out = vec![0.0; n];
    if n == 4 {
        let a = input[0] + input[3];
        let b = input[1] + input[2];
        let c = cos_table(2);
        let x = c[0] * (input[0] - input[3]);
        let y = c[1] * (input[1] - input[2]);
        out[0] = (a + b) * 0.5;
        out[2] = (a - b) * rc;
        out[1] = x + y;
        out[3] = (x - y) * (2.0 * rc) - out[1];
        return out;
    }
    let half = n / 2;
    let even: Vec<_> = (0..half).map(|i| input[i] + input[n - i - 1]).collect();
    let cos = cos_table(half);
    let odd: Vec<_> = (0..half)
        .map(|i| (input[i] - input[n - i - 1]) * cos[i])
        .collect();
    let a = dct(&even);
    let b = dct(&odd);
    for i in 0..half {
        out[i * 2] = a[i];
        out[i * 2 + 1] = b[i] * 2.0;
    }
    for i in 1..half {
        out[i * 2 + 1] -= out[(i - 1) * 2 + 1];
    }
    out
}
fn idct(input: &[f32]) -> Vec<f32> {
    let n = input.len();
    let rc = (PI / 4.0).cos() as f32;
    let mut out = vec![0.0; n];
    if n == 4 {
        let c = cos_table(2);
        let x = rc * input[2];
        let a = input[0] + x;
        let b = input[0] - x;
        let y = c[0] * input[1];
        let z = c[1] * input[3];
        let c = y + z;
        let d = 2.0 * rc * (y - z) - c;
        out[0] = a + c;
        out[3] = a - c;
        out[1] = b + d;
        out[2] = b - d;
        return out;
    }
    let half = n / 2;
    let even: Vec<_> = input.iter().step_by(2).copied().collect();
    let a = idct(&even);
    let cos = cos_table(half);
    let odd: Vec<_> = (0..half).map(|i| input[i * 2 + 1] * cos[i]).collect();
    let mut b = dct(&odd);
    for v in &mut b {
        *v += *v;
    }
    for i in 1..half {
        b[i] -= b[i - 1];
    }
    for i in 0..half / 2 {
        out[i] = a[i] + b[i];
        out[half - 1 - i] = a[half - 1 - i] + b[half - 1 - i];
        out[half + i] = a[half - 1 - i] - b[half - 1 - i];
        out[n - 1 - i] = a[i] - b[i];
    }
    out
}
fn dequantize(q: &[i32], p: Param) -> Vec<f32> {
    let n = q.len();
    let degree = n.ilog2() as i32;
    let mut points = [0usize; 7];
    let mut pos = 0;
    for (i, width) in [-6, -6, -5, -4, -3, -2, -1].into_iter().enumerate() {
        let w = 1usize << ((degree + width) as usize);
        points[i] = pos + w / 2;
        pos += w;
    }
    let mut ratios = [1.0f64; 7];
    for (i, r) in ratios.iter_mut().take(6).enumerate() {
        let power = (((p.weight >> (i * 5)) & 31) - 15) as f64 * 0.5;
        *r = 1.0 / 2.0f64.powf(power);
    }
    let mut weights = vec![0.0f32; n];
    let mut pos = 0;
    while pos < points[0] {
        weights[pos] = ratios[0] as f32;
        pos += 1;
    }
    for j in 1..7 {
        let a = ratios[j - 1];
        let k = (ratios[j] - a) / (points[j] - points[j - 1]) as f64;
        while pos < points[j] {
            weights[pos] = (k * (pos - points[j - 1]) as f64 + a) as f32;
            pos += 1;
        }
    }
    while pos < n {
        weights[pos] = 1.0;
        pos += 1;
    }
    let odd = (((p.weight >> 30) & 3) + 2) as f32 / 2.0;
    for i in (15..n).step_by(16) {
        weights[i] *= odd;
    }
    weights[n - 1] = p.coefficient as f32;
    let ratio = (2.0 / n as f64).sqrt() * p.coefficient as f64;
    q.iter()
        .zip(weights)
        .map(|(q, w)| (ratio * (1.0f32 / w) as f64 * (*q as f64)) as f32)
        .collect()
}
fn rotate_params(degree: usize) -> Vec<[f32; 2]> {
    let n = 1usize << degree;
    let mut lc = 1;
    let mut v = n / 2;
    while v >= 8 {
        lc += 1;
        v /= 8;
    }
    let mut out = vec![[0.0; 2]; lc * 8];
    let k = PI / (n * 2) as f64;
    let mut p = 0;
    let mut step = 2;
    loop {
        for i in 0..7 {
            let mut ws = 1.0;
            let mut a = 0.0;
            for j in 0..i {
                a += step as f64;
                ws = ws * out[p + j][0] as f64 + out[p + j][1] as f64 * (a * k).cos();
            }
            let r = ws.atan2(((a + step as f64) * k).cos());
            out[p + i] = [r.sin() as f32, r.cos() as f32];
        }
        p += 7;
        step *= 8;
        if step >= n {
            break;
        }
    }
    out
}
fn odd_givens(input: &mut [f32], degree: usize) -> Result<()> {
    let n = input.len();
    let params = rotate_params(degree);
    let (mut p, mut index, mut step, mut lc) = (0usize, 1usize, 2usize, (n / 2) / 8);
    loop {
        p += 7;
        index += step * 7;
        step *= 8;
        if lc <= 8 {
            break;
        }
        lc /= 8;
    }
    let mut k = index + step * lc.saturating_sub(2);
    for j in (0..lc.saturating_sub(1)).rev() {
        let [a, b] = *params.get(p + j).context("MIO rotation parameters")?;
        let (r1, r2) = (
            *input.get(k).context("MIO rotation index")?,
            *input.get(k + step).context("MIO rotation index")?,
        );
        input[k] = r1 * b + r2 * a;
        input[k + step] = r2 * b - r1 * a;
        k = k.saturating_sub(step);
    }
    loop {
        if lc > (n / 2) / 8 {
            break;
        }
        p = p
            .checked_sub(7)
            .context("MIO rotation parameter position")?;
        step /= 8;
        index -= step * 7;
        for i in 0..lc {
            let mut k = i * (step * 8) + index + step * 6;
            for j in (0..7).rev() {
                let [sin, cos] = params[p + j];
                let (r1, r2) = (input[k], input[k + step]);
                input[k] = r1 * cos + r2 * sin;
                input[k + step] = r2 * cos - r1 * sin;
                k = k.saturating_sub(step);
            }
        }
        lc *= 8;
    }
    Ok(())
}
