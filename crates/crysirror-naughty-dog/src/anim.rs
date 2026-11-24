// SPDX-License-Identifier: MPL-2.0
//! `ANIM` clips: ICE "CD" clip data (magic `0x43440107`).
//!
//! The `ANIM` item holds the clip pointer at +8 and the skeleton's
//! hierarchy hash in the clip at +4. Clip header words: total size +8,
//! frame count (`u16` +0xe), frames per second (`f32` +0x1c), program block
//! offset +0x30 and the first frame record +0x38. After the program block's
//! 16-byte validity mask comes a command program whose opcodes are the
//! EBOOT's animation command table (`sub_2D7058` names them): each command
//! is `u8 argument count, u8 opcode, u16 arguments…`, and data offsets are
//! relative to clip + 0x60. Commands write a joint-parameter buffer of 48
//! bytes per animated joint (scale +0, quaternion +0x10, translation
//! +0x20); outputs past the animated joints go to a scratch slot.
//!
//! Decoded here:
//! - constant channels (`Const3VecF16`: `u16` output list padded to 16
//!   bytes, then three halves per item; `Const3VecUncompressed`: three
//!   floats; `ConstQuatUncompressed`: four floats; `ConstQuat48SmallestThree`:
//!   48 bits `[a:15][spare:1][b:15][c:15][index:2]`, components
//!   `(q / 32767 · 2 − 1) / √2` stored as x y z with the reconstructed
//!   largest in w, then swapped with the `index` slot — all 5,565 sampled
//!   quaternions are valid with the reconstructed component largest, and
//!   where the index is not 3, 1,719 of 2,072 match the skeleton's default
//!   pose against 193 for inserting at the index);
//! - uniform keys (`CopyUniformDataToKeyCache`: a byte per item giving its
//!   bit size; frame records of `stride` bytes hold the items MSB first;
//!   key-cache slots are 32 bytes): `Dec3VecFloat16` items are three
//!   halves, `DecompressQuatSmallestThree` items use 4-byte descriptors
//!   `{bits a, bits b, a + b, bits c}` plus two index bits, components
//!   `(q − 2ⁿ⁻¹) / 2ⁿ⁻¹ / √2`, the largest stored in w and swapped with the
//!   index slot (smooth on every 30 fps clip sampled);
//! - `KeyCacheUniformLerp`/`Slerp` output lists (one `u16` offset per
//!   item, from the command's key-cache slot).
//!
//! - `Dec3VecRange` items: per-component widths from a `[a, b, a + b, c]`
//!   table after the 16-byte records, `minimum + q · step` (see
//!   [`range_record`]).
//!
//! - `PostDecompressQuatLog`/`LogPca`: the range vectors are half-angle
//!   logarithms, `q = exp(v)` (on `idle^roll-fw` they give a 116° elbow
//!   bend and 30° finger curls). Their per-item parameters (8 and 16 bytes:
//!   not a 48-bit or 16-bit-per-component centre quaternion) are not
//!   applied (SATD-nd-retail-runtime).
//!
//! Not decoded: the non-uniform key caches; those joints keep the rest
//! pose.
use crate::mesh::Loaded;
use anyhow::{Context, Result, ensure};
use glam::{Quat, Vec3};

pub const MAGIC: u32 = 0x4344_0107;
/// Offsets in the clip program are relative to clip + 0x60.
const DATA_BASE: usize = 0x60;
const SLOT: usize = 32;

/// One joint's local channels per frame; `None` where the clip leaves the
/// channel to the rest pose.
#[derive(Clone, Debug, Default)]
pub struct JointTrack {
    pub translation: Option<Vec<Vec3>>,
    pub rotation: Option<Vec<Quat>>,
    pub scale: Option<Vec<Vec3>>,
}

#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub hierarchy: u32,
    pub frames: usize,
    pub fps: f32,
    /// Per animated joint.
    pub joints: Vec<JointTrack>,
    /// Commands whose channels were skipped or decoded with an inferred rule.
    pub unsupported: Vec<&'static str>,
}

enum Value {
    Vec(Vec3),
    Quat(Quat),
}

/// IEEE half to single precision.
fn half(h: u16) -> f32 {
    let (sign, exp, man) = (u32::from(h >> 15), u32::from(h >> 10) & 0x1f, u32::from(h) & 0x3ff);
    let bits = match (exp, man) {
        (0, 0) => sign << 31,
        (0, _) => return if sign == 1 { -1. } else { 1. } * man as f32 * 2f32.powi(-24),
        (0x1f, _) => sign << 31 | 0xff << 23 | man << 13,
        _ => sign << 31 | (exp + 112) << 23 | man << 13,
    };
    f32::from_bits(bits)
}

fn s3_48(v: u64) -> Quat {
    let field = |shift: u32| ((v >> shift) & 0x7fff) as f32 / 32767. * 2. - 1.;
    let x = [field(33), field(17), field(2)].map(|c| c * std::f32::consts::FRAC_1_SQRT_2);
    let big = (1. - x.iter().map(|c| c * c).sum::<f32>()).max(0.).sqrt();
    let mut q = [x[0], x[1], x[2], big];
    let idx = (v & 3) as usize;
    if idx < 3 {
        q.swap(idx, 3);
    }
    Quat::from_array(q).normalize()
}

fn s3_key(v: u64, a: u32, b: u32, c: u32) -> Quat {
    let field = |shift: u32, w: u32| -> f32 {
        if w == 0 {
            return 0.;
        }
        let q = ((v >> shift) & ((1u64 << w) - 1)) as f32;
        let half = (1u64 << (w - 1)) as f32;
        (q - half) / half * std::f32::consts::FRAC_1_SQRT_2
    };
    let x = [field(2 + c + b, a), field(2 + c, b), field(2, c)];
    let big = (1. - x.iter().map(|c| c * c).sum::<f32>()).max(0.).sqrt();
    let mut q = [x[0], x[1], x[2], big];
    let idx = (v & 3) as usize;
    if idx < 3 {
        q.swap(idx, 3);
    }
    Quat::from_array(q).normalize()
}

/// `exp` of a pure quaternion `(v, 0)`.
fn quat_exp(v: Vec3) -> Quat {
    let a = v.length();
    if a < 1e-9 {
        return Quat::from_xyzw(v.x, v.y, v.z, 1.).normalize();
    }
    let s = a.sin() / a;
    Quat::from_xyzw(v.x * s, v.y * s, v.z * s, a.cos())
}

/// Bits `p..p + w` (MSB first) of a 128-bit record as a float with sign,
/// eight exponent bits and `w − 9` mantissa bits.
fn small_float(r: u128, p: u32, w: u32) -> f32 {
    let v = ((r >> (128 - p - w)) & ((1u128 << w) - 1)) as u32;
    let (sign, exp, man) = (v >> (w - 1), (v >> (w - 9)) & 0xff, v & ((1 << (w - 9)) - 1));
    f32::from_bits(sign << 31 | exp << 23 | man << (23 - (w - 9)))
}

/// `Dec3VecRange` record: a 21-bit unsigned float (eight exponent bits,
/// 13 mantissa bits) and two signed floats (21 and 22 bits) as the
/// per-component steps, then three signed floats (22, 21, 21 bits) as the
/// minimum. The field starts (bits 21, 42, 64, 86, 107) are where 98–99.5 %
/// of 4,000 records have plausible exponents; the minimum lies within
/// millimetres of the joints' rest translations; the per-component bit
/// widths `[a, b, a + b, c]` follow the records (they sum to every item's
/// size). Which field steps which component is inferred.
fn range_record(r: u128) -> ([f32; 3], [f32; 3]) {
    let header = ((r >> 107) & 0x1f_ffff) as u32;
    let s0 = f32::from_bits((header >> 13) << 23 | (header & 0x1fff) << 10);
    ([s0, small_float(r, 21, 21), small_float(r, 42, 22)], [small_float(r, 64, 22), small_float(r, 86, 21), small_float(r, 107, 21)])
}

/// Decode the clip of an `ANIM` payload for a skeleton with `animated`
/// animated joints.
pub fn decode(p: &Loaded, payload: usize, name: &str, animated: usize) -> Result<Clip> {
    let cd = p.ptr(payload + 8)?;
    ensure!(p.u32(cd)? == MAGIC, "ND clip magic");
    let size = p.u32(cd + 8)? as usize;
    let data = p.data.get(cd..cd + size).context("ND clip outside package")?;
    let u16_at = |at: usize| -> Result<u16> { Ok(u16::from_be_bytes(data.get(at..at + 2).context("ND clip read")?.try_into()?)) };
    let u32_at = |at: usize| -> Result<u32> { Ok(u32::from_be_bytes(data.get(at..at + 4).context("ND clip read")?.try_into()?)) };
    let frames = usize::from(u16_at(0xe)?).max(1);
    ensure!(frames <= 1 << 14, "ND clip frame count");
    let fps = f32::from_bits(u32_at(0x1c)?);
    let program_block = u32_at(0x30)? as usize;
    let first_frame = u32_at(0x38)? as usize;
    let base = DATA_BASE;
    // Per frame, per joint-parameter offset: the value written.
    let mut constants: Vec<(usize, Value)> = Vec::new();
    let mut keys: Vec<Vec<Option<Value>>> = Vec::new();
    let mut outputs: Vec<(usize, usize)> = Vec::new();
    let mut unsupported = Vec::new();
    let mut inferred: Vec<&'static str> = Vec::new();
    let mut sizes: Vec<u8> = Vec::new();
    let list = |off: usize, n: usize| -> Result<Vec<usize>> { (0..n).map(|i| Ok(usize::from(u16_at(base + off + 2 * i)?))).collect() };
    let mut at = program_block + 0x10;
    let mut item_bits: Vec<Vec<u128>> = Vec::new();
    loop {
        let (n, op) = (*data.get(at).context("ND clip program")? as usize, *data.get(at + 1).context("ND clip program")?);
        if n == 0 && op == 0 {
            break;
        }
        let args: Vec<usize> = (0..n).map(|i| u16_at(at + 2 + 2 * i).map(usize::from)).collect::<Result<_>>()?;
        at += 2 + 2 * n;
        let count = args.first().copied().unwrap_or(0) & 0xfff;
        match op {
            // Const3VecUncompressed / Float16 / ConstQuatUncompressed / Quat48.
            21..=24 => {
                let outs = list(args[2], count)?;
                let values = base + args[2] + (2 * count).div_ceil(16) * 16;
                for (i, o) in outs.into_iter().enumerate() {
                    let v = match op {
                        21 => Value::Vec(Vec3::new(f32::from_bits(u32_at(values + 12 * i)?), f32::from_bits(u32_at(values + 12 * i + 4)?), f32::from_bits(u32_at(values + 12 * i + 8)?))),
                        22 => Value::Vec(Vec3::new(half(u16_at(values + 6 * i)?), half(u16_at(values + 6 * i + 2)?), half(u16_at(values + 6 * i + 4)?))),
                        23 => Value::Quat(Quat::from_array(std::array::from_fn(|k| f32::from_bits(u32_at(values + 16 * i + 4 * k).unwrap_or(0)))).normalize()),
                        _ => {
                            let raw = (u64::from(u16_at(values + 6 * i)?) << 32) | (u64::from(u16_at(values + 6 * i + 2)?) << 16) | u64::from(u16_at(values + 6 * i + 4)?);
                            Value::Quat(s3_48(raw))
                        }
                    };
                    constants.push((o, v));
                }
            }
            // CopyUniformDataToKeyCache(count, buffer, sizes, ?, ?, stride).
            11 => {
                sizes = data.get(base + args[2]..base + args[2] + count).context("ND clip sizes")?.to_vec();
                let stride = *args.get(5).context("ND uniform stride")?;
                ensure!(stride > 0 && first_frame + stride * frames <= size + 16, "ND clip frames");
                item_bits = (0..frames)
                    .map(|f| {
                        let rec = data.get(first_frame + stride * f..(first_frame + stride * (f + 1)).min(size)).unwrap_or_default();
                        let mut pos = 0usize;
                        sizes
                            .iter()
                            .map(|&s| {
                                let mut v = 0u128;
                                for _ in 0..s {
                                    let bit = rec.get(pos >> 3).map_or(0, |b| (b >> (7 - (pos & 7))) & 1);
                                    v = v << 1 | u128::from(bit);
                                    pos += 1;
                                }
                                v
                            })
                            .collect()
                    })
                    .collect();
                keys = (0..frames).map(|_| (0..count).map(|_| None).collect()).collect();
            }
            // Dec3VecFloat16(count, buffer, ?, key slot).
            26 => {
                let first = args[3] / SLOT;
                for (f, k) in keys.iter_mut().enumerate() {
                    for i in 0..count {
                        let v = item_bits[f].get(first + i).copied().unwrap_or(0) as u64;
                        let h = |s: u32| half(((v >> s) & 0xffff) as u16);
                        if let Some(slot) = k.get_mut(first + i) {
                            *slot = Some(Value::Vec(Vec3::new(h(32), h(16), h(0))));
                        }
                    }
                }
            }
            // DecompressQuatSmallestThree(count, buffer, descriptors, key slot).
            28 => {
                let first = args[3] / SLOT;
                for i in 0..count {
                    let d = data.get(base + args[2] + 4 * i..base + args[2] + 4 * i + 4).context("ND quat descriptor")?;
                    let (a, b, c) = (u32::from(d[0]), u32::from(d[1]), u32::from(d[3]));
                    if a + b + c + 2 != u32::from(sizes.get(first + i).copied().unwrap_or(0)) {
                        continue;
                    }
                    for (f, k) in keys.iter_mut().enumerate() {
                        let v = item_bits[f].get(first + i).copied().unwrap_or(0) as u64;
                        if let Some(slot) = k.get_mut(first + i) {
                            *slot = Some(Value::Quat(s3_key(v, a, b, c)));
                        }
                    }
                }
            }
            // Dec3VecRange(count, buffer, records, key slot).
            27 => {
                let first = args[3] / SLOT;
                let records = base + args[2];
                let widths = records + 16 * count;
                for i in 0..count {
                    let w = data.get(widths + 4 * i..widths + 4 * i + 4).context("ND range widths")?;
                    let (wa, wb, wc) = (u32::from(w[0]), u32::from(w[1]), u32::from(w[3]));
                    if wa + wb + wc != u32::from(sizes.get(first + i).copied().unwrap_or(0)) {
                        continue;
                    }
                    let r = u128::from_be_bytes(data.get(records + 16 * i..records + 16 * i + 16).context("ND range record")?.try_into()?);
                    let (step, min) = range_record(r);
                    for (f, k) in keys.iter_mut().enumerate() {
                        let v = item_bits[f].get(first + i).copied().unwrap_or(0);
                        let q = [(v >> (wb + wc)) & ((1 << wa) - 1), (v >> wc) & ((1 << wb) - 1), v & ((1 << wc) - 1)];
                        if let Some(slot) = k.get_mut(first + i) {
                            *slot = Some(Value::Vec(Vec3::from_array(std::array::from_fn(|c| min[c] + q[c] as f32 * step[c]))));
                        }
                    }
                }
            }
            // PostDecompressQuatLog / LogPca(count, buffer, parameters, key
            // slot): range-decoded vectors become rotations `exp(v)` with
            // `v` the half-angle log. The per-item parameters (8 and 16
            // bytes) are not applied.
            29 | 30 => {
                let first = args[3] / SLOT;
                for k in keys.iter_mut() {
                    for i in 0..count {
                        if let Some(slot) = k.get_mut(first + i) {
                            if let Some(Value::Vec(v)) = slot {
                                *slot = Some(Value::Quat(quat_exp(*v)));
                            }
                        }
                    }
                }
                inferred.push(if op == 29 { "PostDecompressQuatLog parameters" } else { "PostDecompressQuatLogPca parameters" });
            }
            // KeyCacheUniformLerp / Slerp(count, buffer, outputs, key slot).
            15 | 16 => {
                let first = args[3] / SLOT;
                for (i, o) in list(args[2], count)?.into_iter().enumerate() {
                    outputs.push((first + i, o));
                }
            }
            12 | 13 | 14 | 17..=20 => unsupported.push("non-uniform or float keys"),
            _ => unsupported.push("other command"),
        }
    }
    let mut joints: Vec<JointTrack> = vec![JointTrack::default(); animated];
    let set_vec = |o: usize, f: usize, v: Vec3, joints: &mut Vec<JointTrack>| {
        let (j, slot) = (o / 48, o % 48);
        if j >= animated {
            return;
        }
        let track = match slot {
            0 => &mut joints[j].scale,
            0x20 => &mut joints[j].translation,
            _ => return,
        };
        track.get_or_insert_with(|| vec![v; frames])[f] = v;
    };
    let set_quat = |o: usize, f: usize, q: Quat, joints: &mut Vec<JointTrack>| {
        let j = o / 48;
        if j >= animated || o % 48 != 0x10 {
            return;
        }
        joints[j].rotation.get_or_insert_with(|| vec![q; frames])[f] = q;
    };
    for (o, v) in &constants {
        for f in 0..frames {
            match v {
                Value::Vec(v) => set_vec(*o, f, *v, &mut joints),
                Value::Quat(q) => set_quat(*o, f, *q, &mut joints),
            }
        }
    }
    for (item, o) in outputs {
        for f in 0..frames {
            match keys.get(f).and_then(|k| k.get(item)).and_then(|v| v.as_ref()) {
                Some(Value::Vec(v)) => set_vec(o, f, *v, &mut joints),
                Some(Value::Quat(q)) => set_quat(o, f, *q, &mut joints),
                None => {}
            }
        }
    }
    unsupported.extend(inferred);
    unsupported.sort_unstable();
    unsupported.dedup();
    Ok(Clip { name: name.to_owned(), hierarchy: u32_at(4)?, frames, fps, joints, unsupported })
}

/// Every `ANIM` item of a relocated package, by name.
pub fn items(p: &Loaded) -> impl Iterator<Item = (&str, usize)> {
    p.directory.items.iter().filter(|i| i.kind == "ANIM").map(|i| (i.name.as_str(), i.payload_offset as usize))
}
