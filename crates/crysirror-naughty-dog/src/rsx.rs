// SPDX-License-Identifier: MPL-2.0
//! RSX fragment programs of `MATERIAL_TABLE_1` shaders: instruction decode
//! and the recognition of material layering the programs compute.
//!
//! A shader entry is 16 bytes `{?, ?, vertex program, fragment program}`
//! from table +0x1c (count at +8); a pass's data word 0 holds the shader
//! index in its high half. A fragment program stores its total size at +0
//! and the microcode offset at +0x10. Instruction words are big-endian with
//! their 16-bit halves swapped; word 0 holds the opcode (bits 24–29),
//! destination (1–6, half precision bit 7), write mask (9–12), input
//! (13–16), texture unit (17–20), saturation (31) and end (0); words 1–3
//! the sources (type bits 0–1: temporary, input or constant; index 2–7,
//! half 8, swizzle 9–16, negate 17). An instruction with a constant source
//! is followed by the 16-byte constant.
use crate::mesh::Loaded;
use anyhow::{Context, Result, ensure};

pub const MOV: u8 = 0x01;
pub const MUL: u8 = 0x02;
pub const ADD: u8 = 0x03;
pub const MAD: u8 = 0x04;
pub const TEX: u8 = 0x17;
pub const NRM: u8 = 0x39;
pub const DIV: u8 = 0x3a;

/// Fragment inputs by index (`TEX2` is 6).
pub const INPUTS: [&str; 16] = ["WPOS", "COL0", "COL1", "FOGC", "TEX0", "TEX1", "TEX2", "TEX3", "TEX4", "TEX5", "TEX6", "TEX7", "TEX8", "TEX9", "SSA", "FACE"];
const INPUT_TEX2: u8 = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operand {
    /// Temporary register (`H` half or `R` full precision).
    Temp { half: bool, index: u8 },
    Input(u8),
    /// The instruction's constant.
    Const,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub operand: Operand,
    pub swizzle: [u8; 4],
    pub negate: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    pub op: u8,
    pub saturate: bool,
    pub half: bool,
    pub dst: u8,
    pub mask: [bool; 4],
    pub texture: u8,
    pub sources: [Source; 3],
    pub constant: Option<[f32; 4]>,
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    let w = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
    Some(w.rotate_left(16))
}

/// Decode microcode until the end bit (at most `bytes.len()`).
pub fn decode(bytes: &[u8]) -> Vec<Instruction> {
    let mut out = Vec::new();
    let mut at = 0;
    while let (Some(w0), Some(w1), Some(w2), Some(w3)) = (word(bytes, at), word(bytes, at + 4), word(bytes, at + 8), word(bytes, at + 12)) {
        at += 16;
        let input = ((w0 >> 13) & 0xf) as u8;
        let source = |w: u32| Source {
            operand: match w & 3 {
                0 => Operand::Temp { half: (w >> 8) & 1 != 0, index: ((w >> 2) & 0x3f) as u8 },
                1 => Operand::Input(input),
                _ => Operand::Const,
            },
            swizzle: std::array::from_fn(|k| ((w >> (9 + 2 * k)) & 3) as u8),
            negate: (w >> 17) & 1 != 0,
        };
        let sources = [source(w1), source(w2), source(w3)];
        let constant = if sources.iter().any(|s| s.operand == Operand::Const) {
            let c: Option<Vec<f32>> = (0..4).map(|k| word(bytes, at + 4 * k).map(f32::from_bits)).collect();
            at += 16;
            c.map(|c| [c[0], c[1], c[2], c[3]])
        } else {
            None
        };
        out.push(Instruction {
            op: ((w0 >> 24) & 0x3f) as u8,
            saturate: (w0 >> 31) & 1 != 0,
            half: (w0 >> 7) & 1 != 0,
            dst: ((w0 >> 1) & 0x3f) as u8,
            mask: std::array::from_fn(|k| (w0 >> (9 + k)) & 1 != 0),
            texture: ((w0 >> 17) & 0xf) as u8,
            sources,
            constant,
        });
        if w0 & 1 != 0 {
            break;
        }
    }
    out
}

/// The fragment program of pass data `data` (`MATERIAL_TABLE_1` shader).
pub fn pass_program(p: &Loaded, data: usize) -> Result<Vec<Instruction>> {
    let shader = (p.u32(data)? >> 16) as usize;
    let table = p.directory.items.iter().find(|i| i.kind == "MATERIAL_TABLE_1").context("ND material table")?.payload_offset as usize;
    ensure!(shader < p.u32(table + 8)? as usize, "ND shader index");
    let fp = p.ptr(p.ptr(table + 0x1c)? + 16 * shader + 12)?;
    let (size, code) = (p.u32(fp)? as usize, p.u32(fp + 0x10)? as usize);
    ensure!(code < size && size <= 1 << 16, "ND fragment program size");
    Ok(decode(p.data.get(fp + code..fp + size).context("ND fragment program outside package")?))
}

/// The upper layer of a two-layer blend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Top {
    /// The pass sampler with this index.
    Sampler(u8),
    /// A colour the program computes (the `cliffwall-snow` programs: a
    /// light grey of 0.5 plus saturated terms).
    Computed,
}

/// What weighs the side layer against the top layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Weight {
    /// `saturate(1 − n.y)` of the normalised world normal input `TEX2`:
    /// the side layer on walls (the `train-wreck` snow programs).
    Slope,
    /// `saturate((mask.g − (1 + s)·m + 2s) / 2s)` with `m` the vertex
    /// mask (`COL0.x`, attribute 0x15) and `mask` the green channel of a
    /// pass sampler (`trn-wide-mountains-snow/rock-to-snow`, `s` =
    /// `g_fBlendSoftness`).
    Mask { sampler: u8, softness: f32 },
}

/// `colour = mix(top, side, weight)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerBlend {
    pub side: u8,
    pub top: Top,
    pub weight: Weight,
}

/// Texture units 12 and up hold the light pre-pass buffers.
pub const FIRST_GLOBAL_UNIT: u8 = 12;

/// The texture unit of each pass sampler, from the pass data's parameter
/// bindings (+0x14, count in the low half of word 2; 16-byte records `{u16
/// register, u16, u32 class, u32 value, u32 name}`): the k-th material
/// sampler binding (class low half 0x0202) gives sampler k + 1 its unit in
/// the high half of the record's first word; class 0x0201 binds global
/// buffers (pre-pass 12/13, the screen shadow). Sampler 0 is the shared
/// placeholder (`rock-to-snow`: rock → 2, mask → 0, snow → 3).
pub fn sampler_units(p: &Loaded, data: usize) -> Result<Vec<(u8, u8)>> {
    let n = (p.u32(data + 8)? & 0xffff) as usize;
    ensure!(n <= 256, "ND parameter binding count");
    let table = p.ptr(data + 0x14)?;
    let mut out = Vec::new();
    for r in 0..n {
        let at = table + 16 * r;
        if p.u32(at + 4)? & 0xffff == 0x0202 {
            let sampler = u8::try_from(out.len() + 1)?;
            out.push(((p.u32(at)? >> 16) as u8, sampler));
        }
    }
    Ok(out)
}

/// A weight term: the world normal's y, the vertex mask, or a texture
/// channel (unit, component).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Symbol {
    NormalY,
    VertexMask,
    Texture(u8, u8),
}

/// `Σ kᵢ·symbolᵢ + c`, saturated or not.
#[derive(Clone, Debug, PartialEq)]
struct Affine {
    terms: Vec<(Symbol, f32)>,
    c: f32,
    saturated: bool,
}

impl Affine {
    fn constant(c: f32) -> Self {
        Self { terms: Vec::new(), c, saturated: false }
    }
    fn symbol(s: Symbol) -> Self {
        Self { terms: vec![(s, 1.)], c: 0., saturated: false }
    }
    fn as_constant(&self) -> Option<f32> {
        self.terms.is_empty().then_some(self.c)
    }
    fn add(&self, o: &Self) -> Self {
        let mut terms = self.terms.clone();
        for (s, k) in &o.terms {
            match terms.iter_mut().find(|(t, _)| t == s) {
                Some(t) => t.1 += k,
                None => terms.push((*s, *k)),
            }
        }
        terms.retain(|(_, k)| k.abs() > 1e-7);
        Self { terms, c: self.c + o.c, saturated: false }
    }
    fn scale(&self, f: f32) -> Self {
        Self { terms: self.terms.iter().map(|(s, k)| (*s, k * f)).collect(), c: self.c * f, saturated: false }
    }
    fn coefficient(&self, s: Symbol) -> f32 {
        self.terms.iter().find(|(t, _)| *t == s).map_or(0., |t| t.1)
    }
}

/// What the recogniser knows of one register component.
#[derive(Clone, Debug, PartialEq)]
enum Value {
    Unknown,
    Affine(Affine),
    /// Component `k` of the normalised world normal.
    Normal(u8),
    Texture(u8, u8),
    /// `texture − other` (`None`: a computed value).
    Difference(u8, Option<u8>),
    Blend(LayerBlend),
}

/// Find a two-layer blend: a texture difference scaled by a recognised
/// weight and added back to its second term. Layers are reported as pass
/// sampler indices through `units` (texture unit, sampler).
pub fn layer_blend(code: &[Instruction], units: &[(u8, u8)]) -> Option<LayerBlend> {
    let sampler = |unit: u8| units.iter().find(|(u, _)| *u == unit).map(|(_, s)| *s);
    let blend = blend_units(code)?;
    Some(LayerBlend {
        side: sampler(blend.side)?,
        top: match blend.top {
            Top::Sampler(u) => Top::Sampler(sampler(u)?),
            Top::Computed => Top::Computed,
        },
        weight: match blend.weight {
            Weight::Mask { sampler: u, softness } => Weight::Mask { sampler: sampler(u)?, softness },
            w => w,
        },
    })
}

/// The weight an affine saturated value stands for (texture units).
fn weight_of(w: &Affine) -> Option<Weight> {
    if !w.saturated {
        return None;
    }
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4 * (1. + b.abs());
    if w.terms.len() == 1 && near(w.coefficient(Symbol::NormalY), -1.) && near(w.c, 1.) {
        return Some(Weight::Slope);
    }
    // k·(mask − (1 + s)·m + 2s) with k = 1 / 2s.
    let (mask, k) = w.terms.iter().find_map(|(s, k)| match s {
        Symbol::Texture(u, 1) => Some((*u, *k)),
        _ => None,
    })?;
    let s = 1. / (2. * k);
    (w.terms.len() == 2 && k > 0. && near(w.coefficient(Symbol::VertexMask), -k * (1. + s)) && near(w.c, k * 2. * s)).then_some(Weight::Mask { sampler: mask, softness: s })
}

/// [`layer_blend`] in texture units.
fn blend_units(code: &[Instruction]) -> Option<LayerBlend> {
    let mut regs: std::collections::HashMap<(bool, u8), [Value; 4]> = std::collections::HashMap::new();
    let mut found = None;
    let mut found_colour: Option<LayerBlend> = None;
    for ins in code {
        // The component a source supplies to result component `k`, and
        // whether it is negated.
        let read = |s: &Source, k: usize| -> (Value, bool) {
            let c = usize::from(s.swizzle[k]);
            let v = match s.operand {
                Operand::Const => Value::Affine(Affine::constant(ins.constant.map_or(0., |c4| c4[c]))),
                Operand::Input(1) if c == 0 => Value::Affine(Affine::symbol(Symbol::VertexMask)),
                Operand::Input(_) => Value::Unknown,
                Operand::Temp { half, index } => regs.get(&(half, index)).map_or(Value::Unknown, |r| r[c].clone()),
            };
            (v, s.negate)
        };
        let signed = |(v, negate): (Value, bool)| match (v, negate) {
            (Value::Affine(a), true) => Value::Affine(a.scale(-1.)),
            (Value::Texture(..) | Value::Difference(..) | Value::Normal(_) | Value::Blend(_), true) => Value::Unknown,
            (v, _) => v,
        };
        let mut result: [Value; 4] = std::array::from_fn(|_| Value::Unknown);
        for (k, slot) in result.iter_mut().enumerate() {
            if !ins.mask[k] {
                continue;
            }
            let (a, b, c) = (signed(read(&ins.sources[0], k)), signed(read(&ins.sources[1], k)), signed(read(&ins.sources[2], k)));
            // `DIV` divides by the divisor's first component.
            let divisor = signed(read(&ins.sources[1], 0));
            let affine = |v: &Value| match v {
                Value::Normal(1) => Some(Affine::symbol(Symbol::NormalY)),
                Value::Texture(u, ch) => Some(Affine::symbol(Symbol::Texture(*u, *ch))),
                Value::Affine(a) => Some(a.clone()),
                _ => None,
            };
            *slot = match ins.op {
                MOV => a,
                TEX => Value::Texture(ins.texture, k as u8),
                NRM if ins.sources[0].operand == Operand::Input(INPUT_TEX2) => Value::Normal(ins.sources[0].swizzle[k]),
                NRM => match a {
                    Value::Normal(_) => Value::Normal(k as u8),
                    _ => Value::Unknown,
                },
                ADD => match (&a, read(&ins.sources[1], k)) {
                    (Value::Texture(t, _), (Value::Texture(u, _), true)) => Value::Difference(*t, Some(u)),
                    (Value::Texture(t, _), (Value::Unknown, true)) => Value::Difference(*t, None),
                    _ => match (affine(&a), affine(&b)) {
                        (Some(x), Some(y)) => Value::Affine(x.add(&y)),
                        _ => Value::Unknown,
                    },
                },
                MUL | DIV => {
                    let (x, y) = (affine(&a), affine(if ins.op == DIV { &divisor } else { &b }));
                    match (x, y) {
                        (Some(x), Some(y)) if ins.op == DIV => y.as_constant().filter(|d| *d != 0.).map_or(Value::Unknown, |d| Value::Affine(x.scale(1. / d))),
                        (Some(x), Some(y)) => match (x.as_constant(), y.as_constant()) {
                            (_, Some(f)) => Value::Affine(x.scale(f)),
                            (Some(f), _) => Value::Affine(y.scale(f)),
                            _ => Value::Unknown,
                        },
                        _ => Value::Unknown,
                    }
                }
                MAD => match (&a, &b, &c) {
                    (Value::Affine(w), Value::Difference(side, top), third) if weight_of(w).is_some() => {
                        let top = match (top, third) {
                            (Some(u), Value::Texture(v, _)) if u == v => Some(Top::Sampler(*u)),
                            (None, Value::Unknown) => Some(Top::Computed),
                            _ => None,
                        };
                        top.zip(weight_of(w)).map_or(Value::Unknown, |(top, weight)| Value::Blend(LayerBlend { side: *side, top, weight }))
                    }
                    _ => match (affine(&a), affine(&b), affine(&c)) {
                        (Some(x), Some(y), Some(z)) => match (x.as_constant(), y.as_constant()) {
                            (_, Some(f)) => Value::Affine(x.scale(f).add(&z)),
                            (Some(f), _) => Value::Affine(y.scale(f).add(&z)),
                            _ => Value::Unknown,
                        },
                        _ => Value::Unknown,
                    },
                },
                _ => Value::Unknown,
            };
            if ins.saturate {
                if let Value::Affine(x) = slot {
                    x.saturated = true;
                }
            }
            // A colour blend (written to x, y and z) wins over the
            // single-channel ones (the gold programs also blend their
            // specular mask).
            if let Value::Blend(s) = slot {
                let colour = ins.mask[..3].iter().all(|m| *m);
                if found.is_none() || colour && found_colour.is_none() {
                    found = Some(*s);
                }
                if colour {
                    found_colour.get_or_insert(*s);
                }
            }
        }
        let entry = regs.entry((ins.half, ins.dst)).or_insert_with(|| std::array::from_fn(|_| Value::Unknown));
        for (k, v) in result.into_iter().enumerate() {
            if ins.mask[k] {
                entry[k] = v;
            }
        }
    }
    found_colour.or(found)
}

/// A pass's named fragment-program constants (binding class 0x0102) from
/// its constant block (pass data +0x1c): `g_fSpecularScale_fSpecularExponentMax_fMetalness`,
/// `g_fBlendSoftness_fBlendOffset`, `g_cDiffuseIn`, … The names are
/// offsets into the package page holding the binding table.
pub fn pass_parameters(p: &Loaded, data: usize) -> Result<Vec<(String, [f32; 4])>> {
    let n = (p.u32(data + 8)? & 0xffff) as usize;
    ensure!(n <= 256, "ND parameter binding count");
    let table = p.ptr(data + 0x14)?;
    let Ok(constants) = p.ptr(data + 0x1c) else { return Ok(Vec::new()) };
    let pages: Vec<usize> = (0..p.directory.page_count).filter_map(|i| p.u32(44 + 12 * i).ok().map(|v| v as usize)).collect();
    let page = pages.iter().copied().filter(|s| *s <= table).max().context("ND binding table page")?;
    let mut out = Vec::new();
    for r in 0..n {
        let at = table + 16 * r;
        if p.u32(at + 4)? & 0xffff != 0x0102 {
            continue;
        }
        let value = p.u32(at + 8)? as usize;
        let name = binding_name(p, at, page)?;
        let v: Vec<f32> = (0..4).map(|k| p.f32(constants + value + 4 * k)).collect::<Result<_>>()?;
        out.push((name, [v[0], v[1], v[2], v[3]]));
    }
    Ok(out)
}

/// A material-constant binding's value: class low half 0x0002 holds an
/// offset into the pass constant block (+0x1c), like 0x0102 (the
/// `godrays-fog` material's `g_fDepthBlend` 0.5 at offset 0,
/// `g_fTransparencyFogScroll` (0, 0.075, 0, 0.35) at 0x10).
pub fn material_constant(p: &Loaded, data: usize, wanted: &str) -> Result<Option<[f32; 4]>> {
    let names = binding_names(p, data)?;
    let Some(r) = names.iter().position(|n| n == wanted) else { return Ok(None) };
    let at = p.ptr(data + 0x14)? + 16 * r;
    if !matches!(p.u32(at + 4)? & 0xffff, 0x0002 | 0x0102) {
        return Ok(None);
    }
    let base = p.ptr(data + 0x1c)? + p.u32(at + 8)? as usize;
    Ok(Some([p.f32(base)?, p.f32(base + 4)?, p.f32(base + 8)?, p.f32(base + 12)?]))
}

/// The names of every parameter binding of a pass (`{u16 register, u16,
/// u32 class, u32 value, u32 name}` at pass data +0x14).
pub fn binding_names(p: &Loaded, data: usize) -> Result<Vec<String>> {
    let n = (p.u32(data + 8)? & 0xffff) as usize;
    ensure!(n <= 256, "ND parameter binding count");
    let table = p.ptr(data + 0x14)?;
    let pages: Vec<usize> = (0..p.directory.page_count).filter_map(|i| p.u32(44 + 12 * i).ok().map(|v| v as usize)).collect();
    let page = pages.iter().copied().filter(|s| *s <= table).max().context("ND binding table page")?;
    (0..n).map(|r| binding_name(p, table + 16 * r, page)).collect()
}

/// A binding row's name: its +12 word is a relocated pointer to the string
/// in some packages (`particle.pak`) and an offset into the table's page
/// in others.
fn binding_name(p: &Loaded, row: usize, page: usize) -> Result<String> {
    let at = match p.ptr(row + 12) {
        Ok(ptr) => ptr,
        Err(_) => page + p.u32(row + 12)? as usize,
    };
    let bytes = p.data.get(at..).unwrap_or_default();
    let end = bytes.iter().take(128).position(|b| *b == 0).unwrap_or(0);
    Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(operand: Operand, swizzle: [u8; 4], negate: bool) -> Source {
        Source { operand, swizzle, negate }
    }
    fn ins(op: u8, saturate: bool, dst: u8, mask: [bool; 4], texture: u8, sources: [Source; 3], constant: Option<[f32; 4]>) -> Instruction {
        Instruction { op, saturate, half: true, dst, mask, texture, sources, constant }
    }

    #[test]
    fn recognises_two_texture_slope_blend() {
        let h = |i| Operand::Temp { half: true, index: i };
        let none = src(h(0), [0, 1, 2, 3], false);
        let xyz = [true, true, true, false];
        let w = [false, false, false, true];
        let x = [true, false, false, false];
        // From `trnwrk-snow-mod-cliff-cheap` (shader 34 of `train-wreck-b`).
        let code = vec![
            ins(TEX, false, 6, xyz, 1, [none; 3], None),
            ins(NRM, false, 5, xyz, 0, [src(Operand::Input(INPUT_TEX2), [0, 1, 2, 3], false), none, none], None),
            ins(ADD, false, 7, x, 0, [src(Operand::Const, [0; 4], false), src(Operand::Const, [1; 4], true), none], Some([0., 1., 0., 0.])),
            ins(MOV, false, 0, xyz, 0, [src(h(5), [0, 1, 2, 3], false), none, none], None),
            ins(NRM, false, 1, xyz, 0, [src(h(0), [0, 1, 2, 3], false), none, none], None),
            ins(ADD, false, 6, w, 0, [src(h(1), [1; 4], false), src(Operand::Const, [1; 4], true), none], Some([0., 1., 0., 0.])),
            ins(DIV, true, 6, w, 0, [src(h(6), [3; 4], false), src(h(7), [0; 4], false), none], None),
            ins(TEX, false, 5, xyz, 2, [none; 3], None),
            ins(ADD, false, 6, xyz, 0, [src(h(6), [0, 1, 2, 3], false), src(h(5), [0, 1, 2, 3], true), none], None),
            ins(MAD, false, 0, xyz, 0, [src(h(6), [3; 4], false), src(h(6), [0, 1, 2, 3], false), src(h(5), [0, 1, 2, 3], false)], None),
        ];
        assert_eq!(layer_blend(&code, &[(1, 1), (2, 2)]), Some(LayerBlend { side: 1, top: Top::Sampler(2), weight: Weight::Slope }));
    }

    #[test]
    fn decodes_swapped_halves() {
        // MOV R0.xyzw, TEX0 with the end bit: halves swapped in memory.
        let w0: u32 = (MOV as u32) << 24 | 0xf << 9 | 4 << 13 | 1;
        let w1: u32 = 1 | (0 << 9) | (1 << 11) | (2 << 13) | (3 << 15);
        let mut bytes = Vec::new();
        for w in [w0, w1, 0, 0] {
            bytes.extend_from_slice(&w.rotate_left(16).to_be_bytes());
        }
        let code = decode(&bytes);
        assert_eq!(code.len(), 1);
        assert_eq!(code[0].op, MOV);
        assert_eq!(code[0].sources[0].operand, Operand::Input(4));
        assert_eq!(code[0].sources[0].swizzle, [0, 1, 2, 3]);
    }
}
