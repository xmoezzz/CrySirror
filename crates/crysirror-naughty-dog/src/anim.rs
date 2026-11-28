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
//! relative to the program (block + 0x10). Commands write a joint-parameter buffer of 48
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
//!   [`range_record`]), plus the predictor the width flags select.
//!
//! - `PostDecompressQuatLog`/`LogPca`: the range vectors are half-angle
//!   logarithms turned into `e = (s(t)·v, c(t))`, `t = v·v`, with the
//!   decoder's cubic sine and cosine series, then `q = q0 ⊗ e` (Log) or
//!   `q = P ⊗ e ⊗ Q` (LogPca) with per-item quaternions from the
//!   parameter block (see [`log_params`]).
//!
//! The op handlers live in the `CalcInfluences` SPU job (loaded at LS
//! 0x4000; 62-entry handler table at listing 0xA850, op 27 at 0x30C0, op
//! 29 at 0x6C80, op 30 at 0x6E90); the formulas were checked by running
//! those handlers on controlled keys.
//!
//! Not decoded: the non-uniform key caches; those joints keep the rest
//! pose.
use crate::mesh::Loaded;
use anyhow::{Context, Result, ensure};
use glam::{Quat, Vec3};

pub const MAGIC: u32 = 0x4344_0107;
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
    /// Horizontal root-motion speed (metres per second) of the `align`
    /// channel, 0 without one.
    pub root_speed: f32,
    /// Per-frame `align` (the character root) and `apReference` (the
    /// action-pack reference) transforms in clip space: translation,
    /// rotation. Empty without the channel.
    pub align: Vec<(Vec3, Quat)>,
    pub ap_reference: Vec<(Vec3, Quat)>,
    /// Commands whose channels were skipped or decoded with an inferred rule.
    pub unsupported: Vec<&'static str>,
}

enum Value {
    Vec(Vec3),
    Quat(Quat),
}

/// IEEE half to single precision.
pub(crate) fn half(h: u16) -> f32 {
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

/// The decoder's `exp` of a half-angle log `v`: cubic series in `t = v·v`
/// for `cos|v|` and `sin|v| / |v|` (constants at `CalcInfluences` listing
/// 0xA7F0 and 0xA800). It is not renormalised.
fn quat_exp(v: Vec3) -> Quat {
    let t = v.length_squared();
    let c = 1. + t * (-0.5 + t * (0.041_523_72 + t * -0.001_271_407_7));
    let s = 1. + t * (-0.166_666_67 + t * (0.008_317_291 + t * -0.000_185_261_76));
    Quat::from_xyzw(v.x * s, v.y * s, v.z * s, c)
}

/// Per-item quaternions of a `PostDecompressQuatLog` (`pca` false: `q0`)
/// or `LogPca` (`(P, Q)`) parameter block, as `s16 / 32768`. Log blocks
/// are 16 bytes per item pair with interleaved halfwords
/// `[x0 x1 y0 y1 z0 z1 w0 w1]`; LogPca blocks are 16 bytes per item,
/// `[Px Qx Py Qy Pz Qz Pw Qw]`.
fn log_params(data: &[u8], at: usize, count: usize, pca: bool) -> Result<Vec<(Quat, Quat)>> {
    let h = |o: usize| -> Result<f32> { Ok(f32::from(i16::from_be_bytes(data.get(o..o + 2).context("ND log parameters")?.try_into()?)) / 32768.) };
    (0..count)
        .map(|i| {
            if pca {
                let b = at + 16 * i;
                let c = |k: usize| h(b + 4 * k);
                let d = |k: usize| h(b + 4 * k + 2);
                Ok((Quat::from_xyzw(c(0)?, c(1)?, c(2)?, c(3)?), Quat::from_xyzw(d(0)?, d(1)?, d(2)?, d(3)?)))
            } else {
                let b = at + 16 * (i / 2) + 2 * (i % 2);
                Ok((Quat::from_xyzw(h(b)?, h(b + 4)?, h(b + 8)?, h(b + 12)?), Quat::IDENTITY))
            }
        })
        .collect()
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
/// widths `[a, b, a + b, c]` follow the records (their low six bits sum to
/// every item's size; the top two bits are flags, set only on items a
/// `PostDecompressQuatLog` later converts). A flag `f` (1–3) on the
/// width byte of component x, y or z (bytes 0, 1, 3) adds component `f − 1`
/// of the previous item's decoded vector of the same frame: a predictor
/// chaining the log rotations. Which field steps which component is inferred.
fn range_record(r: u128) -> ([f32; 3], [f32; 3]) {
    let header = ((r >> 107) & 0x1f_ffff) as u32;
    let s0 = f32::from_bits((header >> 13) << 23 | (header & 0x1fff) << 10);
    ([s0, small_float(r, 21, 21), small_float(r, 42, 22)], [small_float(r, 64, 22), small_float(r, 86, 21), small_float(r, 107, 21)])
}

/// Bit zero of the ANIM flags is copied to the playing state's looping
/// bit by `sub_2DCB18` and checked by the original seam validator.
pub(crate) fn looping(p: &Loaded, payload: usize) -> Result<bool> {
    Ok(p.u32(payload + 4)? & 1 != 0)
}

/// sub_2EAD8C multiplies the authored final sample index by its seconds
/// per frame; a closing duplicate key does not extend the period.
pub(crate) fn duration(p: &Loaded, payload: usize) -> Result<f32> {
    let data = p.ptr(payload + 8)?;
    let seconds = p.f32(data + 0x28)? * p.f32(data + 0x2c)?;
    ensure!(seconds.is_finite() && seconds >= 0., "invalid ND animation duration");
    Ok(seconds)
}

/// The `JOINT_HIERARCHY` hash an `ANIM` payload's clip animates.
pub fn hierarchy(p: &Loaded, payload: usize) -> Result<u32> {
    let cd = p.ptr(payload + 8)?;
    ensure!(p.u32(cd)? == MAGIC, "ND clip magic");
    p.u32(cd + 4)
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
    // Data offsets are relative to the program (the block + 0x10): 0x60
    // in the player rig's clips, 0x50 in `anim-train-wreck-climb` props.
    let base = program_block + 0x10;
    // Per frame, per joint-parameter offset: the value written.
    let mut constants: Vec<(usize, Value)> = Vec::new();
    let mut keys: Vec<Vec<Option<Value>>> = Vec::new();
    let mut outputs: Vec<(usize, usize)> = Vec::new();
    let mut unsupported = Vec::new();
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
                // Lists are padded to a multiple of 8 with a repeated
                // output whose values may lie past the clip: stop there.
                for (i, o) in outs.into_iter().enumerate() {
                    let width = [12, 6, 16, 6][usize::from(op - 21)];
                    if values + width * (i + 1) > size {
                        break;
                    }
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
                    let (wa, wb, wc) = (u32::from(w[0] & 0x3f), u32::from(w[1] & 0x3f), u32::from(w[3] & 0x3f));
                    if wa + wb + wc != u32::from(sizes.get(first + i).copied().unwrap_or(0)) {
                        continue;
                    }
                    let r = u128::from_be_bytes(data.get(records + 16 * i..records + 16 * i + 16).context("ND range record")?.try_into()?);
                    let (step, min) = range_record(r);
                    let flags = [w[0] >> 6, w[1] >> 6, w[3] >> 6];
                    for (f, k) in keys.iter_mut().enumerate() {
                        let v = item_bits[f].get(first + i).copied().unwrap_or(0);
                        let q = [(v >> (wb + wc)) & ((1 << wa) - 1), (v >> wc) & ((1 << wb) - 1), v & ((1 << wc) - 1)];
                        let previous = match (i > 0).then(|| k.get(first + i - 1)).flatten() {
                            Some(Some(Value::Vec(p))) => *p,
                            _ => Vec3::ZERO,
                        };
                        let value = Vec3::from_array(std::array::from_fn(|c| {
                            let predicted = if flags[c] == 0 { 0. } else { previous[usize::from(flags[c]) - 1] };
                            min[c] + q[c] as f32 * step[c] + predicted
                        }));
                        if let Some(slot) = k.get_mut(first + i) {
                            *slot = Some(Value::Vec(value));
                        }
                    }
                }
            }
            // PostDecompressQuatLog / LogPca(count, buffer, parameters, key
            // slot): `q0 ⊗ exp(v)` / `P ⊗ exp(v) ⊗ Q` in place.
            29 | 30 => {
                let first = args[3] / SLOT;
                let params = log_params(data, base + args[2], count, op == 30)?;
                for k in keys.iter_mut() {
                    for (i, (p, q)) in params.iter().enumerate() {
                        if let Some(slot) = k.get_mut(first + i) {
                            if let Some(Value::Vec(v)) = slot {
                                let e = quat_exp(*v);
                                *slot = Some(Value::Quat(if op == 29 { *p * e } else { *p * e * *q }));
                            }
                        }
                    }
                }
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
    unsupported.sort_unstable();
    unsupported.dedup();
    let align = raw_channel(p, payload, crate::dc::sid("align")).unwrap_or_default();
    let ap_reference = raw_channel(p, payload, crate::dc::sid("apReference")).unwrap_or_default();
    let root_speed = match (align.first(), align.last()) {
        (Some(a), Some(b)) if align.len() > 1 => {
            let d = b.0 - a.0;
            (d.x * d.x + d.z * d.z).sqrt() / ((align.len() - 1) as f32 / fps.max(1.))
        }
        _ => 0.,
    };
    Ok(Clip { name: name.to_owned(), hierarchy: u32_at(4)?, frames, fps, joints, root_speed, align, ap_reference, unsupported })
}

/// `ANIM_RAWCHANNELS` (`ANIM` +0xc): `{count, fps, entry pointers}`, an
/// entry `{StringId, data}`. A channel's data holds the first and last
/// transforms (two 11-float `{quaternion, scale, translation, 0}` records)
/// and at +88 `{samples, u16 record bytes, u16 components, u16 count, u16
/// flags}`: one record per frame, 12 bytes (translation; components 4) or
/// 28 bytes (quaternion then translation; components 6). `walk` advances
/// 0.0633 m per frame (1.9 m/s), `move` 4.0 m/s, `move^turn-180^move`
/// turns 180°, `jump-long` drops 3.25 m over 6.98 m.
fn raw_channel(p: &Loaded, payload: usize, name: u32) -> Option<Vec<(Vec3, Quat)>> {
    let raw = p.ptr(payload + 0xc).ok()?;
    let count = p.u32(raw).ok()? as usize;
    if count > 64 {
        return None;
    }
    for i in 0..count {
        let entry = p.ptr(raw + 8 + 4 * i).ok()?;
        if p.u32(entry).ok()? != name {
            continue;
        }
        let data = p.ptr(entry + 4).ok()?;
        let samples = p.ptr(data + 88).ok()?;
        let (stride, components) = (usize::from(p.u16(data + 92).ok()?), p.u16(data + 94).ok()?);
        let frames = usize::from(p.u16(data + 98).ok()?);
        let rotation = components & 2 != 0;
        if frames == 0 || frames > 1 << 14 || stride != if rotation { 28 } else { 12 } {
            return None;
        }
        let f = |at: usize| p.f32(at).ok();
        return (0..frames)
            .map(|k| {
                let r = samples + stride * k;
                if rotation {
                    let q = Quat::from_xyzw(f(r)?, f(r + 4)?, f(r + 8)?, f(r + 12)?).normalize();
                    Some((Vec3::new(f(r + 16)?, f(r + 20)?, f(r + 24)?), q))
                } else {
                    Some((Vec3::new(f(r)?, f(r + 4)?, f(r + 8)?), Quat::IDENTITY))
                }
            })
            .collect();
    }
    None
}

/// The names of an `ANIM` payload's raw channels with their sample
/// layout `(stride, components, frames)`.
pub fn raw_channels(p: &Loaded, payload: usize) -> Vec<(u32, u16, u16, u16)> {
    let Ok(raw) = p.ptr(payload + 0xc) else { return Vec::new() };
    let count = p.u32(raw).map_or(0, |c| c as usize).min(64);
    (0..count)
        .filter_map(|i| {
            let entry = p.ptr(raw + 8 + 4 * i).ok()?;
            let data = p.ptr(entry + 4).ok()?;
            Some((p.u32(entry).ok()?, p.u16(data + 92).ok()?, p.u16(data + 94).ok()?, p.u16(data + 98).ok()?))
        })
        .collect()
}

/// The float samples of raw channel `name` (`stride / 4` floats each).
pub fn raw_samples(p: &Loaded, payload: usize, name: u32) -> Vec<Vec<f32>> {
    let found = || -> Option<Vec<Vec<f32>>> {
        let raw = p.ptr(payload + 0xc).ok()?;
        let count = (p.u32(raw).ok()? as usize).min(64);
        let entry = (0..count).filter_map(|i| p.ptr(raw + 8 + 4 * i).ok()).find(|e| p.u32(*e).ok() == Some(name))?;
        let data = p.ptr(entry + 4).ok()?;
        let samples = p.ptr(data + 88).ok()?;
        let (stride, frames) = (usize::from(p.u16(data + 92).ok()?), usize::from(p.u16(data + 98).ok()?));
        if stride == 0 || stride % 4 != 0 || frames > 1 << 14 {
            return None;
        }
        (0..frames).map(|k| (0..stride / 4).map(|j| p.f32(samples + stride * k + 4 * j).ok()).collect()).collect()
    };
    found().unwrap_or_default()
}

/// The clip name of a clip mirrored by [`mirror`].
pub const MIRROR_SUFFIX: &str = "@mirror";

/// The left/right partner of each joint by name (`l_…`/`r_…`, `…_l`/`…_r`);
/// central joints are their own partners.
pub fn joint_partners(names: &[String]) -> Vec<usize> {
    let swap = |n: &str| -> Option<String> {
        if let Some(rest) = n.strip_prefix("l_") {
            Some(format!("r_{rest}"))
        } else if let Some(rest) = n.strip_prefix("r_") {
            Some(format!("l_{rest}"))
        } else if let Some(stem) = n.strip_suffix("_l") {
            Some(format!("{stem}_r"))
        } else {
            n.strip_suffix("_r").map(|stem| format!("{stem}_l"))
        }
    };
    names.iter().enumerate().map(|(j, n)| swap(n).and_then(|m| names.iter().position(|x| *x == m)).unwrap_or(j)).collect()
}

/// A clip mirrored across the character's YZ plane (x is left): each
/// joint takes its partner's pose reflected, `W′ⱼ = R·W_p·R·Cⱼ` with `R`
/// the x reflection and `Cⱼ = (R·B_p·R)⁻¹·Bⱼ` from the bind pose, so the
/// bind pose maps onto itself whatever the joints' local axes. `align`
/// and `apReference` are reflected the same way. The mirrored states'
/// clips (`ledge-shimmy-left` plays `ledge-grab-shuffle-r-hurt`) are
/// built this way; the native `SKELETON_FLIPDATA` joint-pair programs are
/// not decoded (SATD-nd-retail-runtime).
pub fn mirror(clip: &Clip, skel: &crate::skeleton::Skeleton) -> Clip {
    use glam::Mat4;
    let n = skel.names.len();
    let partner = joint_partners(&skel.names);
    let order = skel.order();
    let r = Mat4::from_scale(Vec3::new(-1., 1., 1.));
    let world_of = |local: &dyn Fn(usize) -> (Vec3, Quat, Vec3)| {
        let mut w = vec![Mat4::IDENTITY; n];
        for &j in &order {
            let (t, q, sc) = local(j);
            let l = Mat4::from_scale_rotation_translation(sc, q, t);
            w[j] = skel.parents[j].map_or(l, |p| w[p] * l);
        }
        w
    };
    let bind = world_of(&|j| skel.rest[j]);
    let correction: Vec<Mat4> = (0..n).map(|j| (r * bind[partner[j]] * r).inverse() * bind[j]).collect();
    let frames = clip.frames.max(1);
    let track = |j: usize| clip.joints.get(j);
    let animated: Vec<bool> = (0..n).map(|j| [j, partner[j]].iter().any(|&k| track(k).is_some_and(|t| t.translation.is_some() || t.rotation.is_some() || t.scale.is_some()))).collect();
    let mut joints: Vec<JointTrack> = (0..clip.joints.len().max(n)).map(|_| JointTrack::default()).collect();
    for f in 0..frames {
        let local = |j: usize| {
            let (mut t, mut q, mut sc) = skel.rest[j];
            if let Some(tr) = track(j) {
                t = tr.translation.as_ref().and_then(|v| v.get(f.min(v.len().saturating_sub(1)))).copied().unwrap_or(t);
                q = tr.rotation.as_ref().and_then(|v| v.get(f.min(v.len().saturating_sub(1)))).copied().unwrap_or(q);
                sc = tr.scale.as_ref().and_then(|v| v.get(f.min(v.len().saturating_sub(1)))).copied().unwrap_or(sc);
            }
            (t, q, sc)
        };
        let world = world_of(&local);
        let mirrored: Vec<Mat4> = (0..n).map(|j| r * world[partner[j]] * r * correction[j]).collect();
        for j in (0..n).filter(|j| animated[*j]) {
            let l = skel.parents[j].map_or(mirrored[j], |p| mirrored[p].inverse() * mirrored[j]);
            let (sc, q, t) = l.to_scale_rotation_translation();
            let jt = &mut joints[j];
            jt.translation.get_or_insert_with(Vec::new).push(t);
            jt.rotation.get_or_insert_with(Vec::new).push(q.normalize());
            if track(j).is_some_and(|t| t.scale.is_some()) || track(partner[j]).is_some_and(|t| t.scale.is_some()) {
                jt.scale.get_or_insert_with(Vec::new).push(sc);
            }
        }
    }
    let reflect = |(t, q): (Vec3, Quat)| (Vec3::new(-t.x, t.y, t.z), Quat::from_xyzw(q.x, -q.y, -q.z, q.w));
    Clip {
        name: format!("{}{MIRROR_SUFFIX}", clip.name),
        joints,
        align: clip.align.iter().copied().map(reflect).collect(),
        ap_reference: clip.ap_reference.iter().copied().map(reflect).collect(),
        ..clip.clone()
    }
}

/// Every `ANIM` item of a relocated package, by name.
pub fn items(p: &Loaded) -> impl Iterator<Item = (&str, usize)> {
    p.directory.items.iter().filter(|i| i.kind == "ANIM").map(|i| (i.name.as_str(), i.payload_offset as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::Skeleton;

    fn rig() -> Skeleton {
        // A root and two arms whose local axes are not mirror images.
        let rest = vec![
            (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
            (Vec3::new(0.3, 1.4, 0.), Quat::from_rotation_z(0.4), Vec3::ONE),
            (Vec3::new(-0.3, 1.4, 0.), Quat::from_rotation_z(-0.4) * Quat::from_rotation_x(std::f32::consts::PI), Vec3::ONE),
        ];
        Skeleton { hash: 1, names: vec!["root".into(), "l_arm".into(), "r_arm".into()], parents: vec![None, Some(0), Some(0)], animated: 3, inverse_bind: vec![[0.; 16]; 3], rest }
    }

    fn clip(rotations: [Quat; 2]) -> Clip {
        Clip {
            name: "lift".into(),
            hierarchy: 1,
            frames: 2,
            fps: 30.,
            joints: vec![JointTrack::default(), JointTrack { rotation: Some(rotations.to_vec()), ..Default::default() }, JointTrack::default()],
            root_speed: 0.,
            align: vec![(Vec3::new(0.2, 0., 0.), Quat::from_rotation_y(0.3)); 2],
            ap_reference: Vec::new(),
            unsupported: Vec::new(),
        }
    }

    #[test]
    fn mirroring_keeps_the_bind_pose_and_inverts_itself() {
        let skel = rig();
        let lift = Quat::from_rotation_z(0.4) * Quat::from_rotation_y(0.8);
        let c = clip([skel.rest[1].1, lift]);
        let m = mirror(&c, &skel);
        assert_eq!(m.name, "lift@mirror");
        // Frame 0 is the bind pose: the right arm keeps its rest rotation.
        let r = m.joints[2].rotation.as_ref().expect("right arm track")[0];
        assert!(r.dot(skel.rest[2].1).abs() > 0.9999);
        // The left arm's motion moves to the right arm, and back again.
        assert!(m.joints[2].rotation.as_ref().unwrap()[1].dot(skel.rest[2].1).abs() < 0.999);
        let back = mirror(&Clip { name: "lift".into(), ..m.clone() }, &skel);
        assert!(back.joints[1].rotation.as_ref().unwrap()[1].dot(lift).abs() > 0.9999);
        assert!((m.align[0].0.x + 0.2).abs() < 1e-6);
    }
}
