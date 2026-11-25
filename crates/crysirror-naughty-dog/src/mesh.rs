// SPDX-License-Identifier: MPL-2.0
//! Relocated PAK 0xa79 contents and background prototype meshes
//! (`PM_COLLECTION_2`).
//!
//! Pointers inside pages are page-relative words listed by the header's
//! fix-up table (`page, target page, offset`). [`Loaded`] resolves them to
//! file offsets and refuses reads of words that are not listed pointers.
//!
//! Mesh layout (offsets into the relocated item payloads):
//! - collection: `u32 count`, then `count` record pointers. A record holds
//!   the mesh header, a pointer to its instance-pointer list and the
//!   instance count (twice). An instance carries a culling sphere at +0x30
//!   and a row-major 4×4 world matrix in metres at +0x40 (translation in
//!   the last row); positions are in [`unit_of_grid`] units;
//! - mesh header: `u16` group and fragment counts at +4/+6, fragment
//!   pointer array at +0x14;
//! - fragment: vertex count block at +0x24, stream descriptor at +0x28,
//!   attribute count byte at +0x2f, triangle count `u16` at +0x40, index
//!   blob at +0x48, per-attribute stream pointers at +0x50 (stride 8);
//! - descriptor: 8-byte attributes from +8 (`id`, kind << 4 | components,
//!   `u16` parameter offset, bit depth per component); each attribute is
//!   its own MSB-first bit stream. Kind 4 stores N floats `a`, then N
//!   floats `b`: `value = a · (1 + v · 2⁻²¹) + b`. Kind 6 adds N `u32`
//!   offsets `I`: `value = a · (1 + (4v + I) · 2⁻²³) + b`. Kind 0 is signed
//!   normalised: `v · 2 / (2ᵈ − 1) − 1`. Positions come out in metres in
//!   the prototype's own frame and texture coordinates in textures. These
//!   formulas follow the public Uncharted 3 PS3 actor decoder
//!   (`bc160201983/Uncharted-The-Nathan-Drake-Collection-PC-port`
//!   `u3mp/paktools/ps3geom.py` at `1294635`, checked there against the PS4
//!   assets). On Uncharted 2 they put the `train-wreck-b` box car roof at
//!   local x −2.98…2.92 m and y 0.37…7.57 m against its Havok collision's
//!   −2.94…2.92 and 0.35…7.54, the 1 m crate on the floor and the
//!   `train-wreck-wide` valley floor at the river. Earlier fits (a
//!   2^(N − 22) grid unit, origins fitted to fragment spheres, UVs centred
//!   by +0.5) approximated this rule;
//! - fragment +0x0c: skin information (null for rigid fragments), see
//!   [`skin`];
//! - mesh groups at header +0x10 (count `u16` at +4), 16 bytes each:
//!   pointer to a group whose word 0 is the material, then `u16` first
//!   fragment and fragment count;
//! - material: name, pass count at +0xc, pass pointers at +0x10. Pass
//!   data (pass +0x10) holds the sampler count at +0xc and samplers at +0x18,
//!   16 bytes each, whose word 0 points to a texture ID equal to word 3 of
//!   the matching `VRAM_DESC`. Sampler 1 of pass 0 is the base colour on the
//!   installed materials; slot 0 and the last slot are runtime
//!   placeholders.
use crate::{geometry::decode_indices, package::Directory};
use anyhow::{Context, Result, ensure};
use crysirror_core::service::AssetReader;
use crysirror_resource::read_exact_at;
use std::collections::HashSet;

pub const ATTR_POSITION: u8 = 1;
pub const ATTR_UV: u8 = 0x11;
pub const ATTR_NORMAL: u8 = 0x02;
/// Three unsigned bytes per vertex with 7–24 distinct values per fragment on
/// `menu-temple`: layer blend masks of two-layer materials.
pub const ATTR_LAYER_MASK: u8 = 0x15;
pub struct Loaded {
    pub data: Vec<u8>,
    pub directory: Directory,
    pointers: HashSet<usize>,
}

fn be32(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(b.get(at..at + 4).context("ND read outside package")?.try_into()?))
}

impl Loaded {
    /// Read a package of at most `limit` bytes and resolve its pointers.
    pub fn read(source: &dyn AssetReader, limit: u64) -> Result<Self> {
        let directory = Directory::read(source)?;
        let len = source.len()?;
        ensure!(len <= limit, "ND package exceeds the load limit");
        let mut data = vec![0; len as usize];
        read_exact_at(source, 0, &mut data)?;
        let pages = (0..directory.page_count)
            .map(|i| be32(&data, 44 + 12 * i).map(|v| v as usize))
            .collect::<Result<Vec<_>>>()?;
        let (groups, table) = (be32(&data, 24)? as usize, be32(&data, 28)? as usize);
        ensure!(groups <= 64, "ND fix-up groups");
        let mut entries = 0usize;
        for g in 0..groups {
            entries += be32(&data, table + 12 * g + 8)? as usize;
        }
        let first = be32(&data, table + 4)? as usize;
        ensure!(entries <= 4 << 20, "ND fix-up count");
        let mut pointers = HashSet::with_capacity(entries);
        for e in 0..entries {
            let at = first + 8 * e;
            let row = data.get(at..at + 8).context("ND fix-up outside package")?;
            let (from, to) = (u16::from_be_bytes([row[0], row[1]]) as usize, u16::from_be_bytes([row[2], row[3]]) as usize);
            let offset = u32::from_be_bytes(row[4..8].try_into()?) as usize;
            let slot = pages.get(from).context("ND fix-up page")? + offset;
            let target = pages.get(to).context("ND fix-up target page")? + be32(&data, slot)? as usize;
            ensure!(target < directory.texture_base as usize, "ND pointer outside pages");
            data[slot..slot + 4].copy_from_slice(&u32::try_from(target)?.to_be_bytes());
            pointers.insert(slot);
        }
        Ok(Self { data, directory, pointers })
    }

    /// Offsets of the relocated pointer words.
    pub fn pointers(&self) -> impl Iterator<Item = usize> + '_ {
        self.pointers.iter().copied()
    }

    pub fn ptr(&self, at: usize) -> Result<usize> {
        ensure!(self.pointers.contains(&at), "ND word at {at:#x} is not a pointer");
        Ok(be32(&self.data, at)? as usize)
    }
    pub fn u32(&self, at: usize) -> Result<u32> {
        be32(&self.data, at)
    }
    pub fn u16(&self, at: usize) -> Result<u16> {
        Ok(u16::from_be_bytes(self.data.get(at..at + 2).context("ND read outside package")?.try_into()?))
    }
    pub fn f32(&self, at: usize) -> Result<f32> {
        Ok(f32::from_bits(be32(&self.data, at)?))
    }
}

#[derive(Debug)]
#[derive(Default)]
pub struct Fragment {
    /// Metres in the prototype's frame.
    pub positions: Vec<[f32; 3]>,
    /// Unit normals, or empty when the fragment's normal kind is unknown.
    pub normals: Vec<[f32; 3]>,
    /// Layer masks (attribute 0x15) in 0..1, or empty when not stored per
    /// vertex.
    pub masks: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u16; 3]>,
    /// Per vertex up to four  influences;
    /// empty for rigid fragments.
    pub skin: Vec<Vec<(u16, f32)>>,
}

#[derive(Debug)]
pub struct Material {
    pub name: String,
    /// Texture IDs of pass 0's samplers (`VRAM_DESC` word 3).
    pub samplers: Vec<u32>,
    /// Every pass's samplers, pass 0 first.
    pub passes: Vec<Vec<u32>>,
    /// The two-layer blend pass 0's fragment program computes, if any.
    pub layers: Option<crate::rsx::LayerBlend>,
    /// Pass 0's named fragment-program constants.
    pub parameters: Vec<(String, [f32; 4])>,
}

impl Material {
    /// A named fragment-program constant.
    pub fn parameter(&self, name: &str) -> Option<[f32; 4]> {
        self.parameters.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }

    /// Base-colour texture: the first sampler that is not a placeholder
    /// (`skip`: single-colour textures and lightmaps). Background materials
    /// bind a shared placeholder at sampler 0 and the albedo at 1, prop
    /// materials placeholders at 0 and 1 and the albedo at 2, character
    /// materials the albedo at 0 and a normal map at 1 (survey of
    /// `ice-cave-temple`, `train-wreck-car-b` and 46 actor packages).
    pub fn base_color_with(&self, skip: impl Fn(u32) -> bool) -> Option<u32> {
        self.samplers.iter().copied().find(|id| !skip(*id))
    }

    pub fn base_color(&self) -> Option<u32> {
        self.samplers.get(1).copied()
    }
}

#[derive(Debug)]
pub struct Group {
    pub material: Material,
    pub fragments: std::ops::Range<usize>,
}

#[derive(Debug)]
pub struct Mesh {
    pub fragments: Vec<Fragment>,
    pub groups: Vec<Group>,
    /// Row-major world matrices in metres (`p' = p · M`, translation in
    /// row 3), one per placed instance.
    pub instances: Vec<[f32; 16]>,
    /// Each instance's culling sphere (centre in metres, radius).
    pub spheres: Vec<[f32; 4]>,
    /// Some fragment carries joint streams beyond its described attributes.
    pub skinned: bool,
}

/// One quantized attribute before its mesh grid is known.
struct Raw<const N: usize> {
    kind: u8,
    depths: [u8; N],
    a: [f32; N],
    b: [f32; N],
    /// Kind 6: `4v + I` already applied.
    values: Vec<[u64; N]>,
}

/// Unpack `count` records of MSB-first unsigned fields.
fn unpack(data: &[u8], at: usize, count: usize, depths: &[u8]) -> Result<Vec<[u32; 4]>> {
    let bits: usize = depths.iter().map(|d| *d as usize).sum();
    let bytes = data.get(at..at + (count * bits).div_ceil(8)).context("ND vertex stream outside package")?;
    let mut pos = 0usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let mut row = [0u32; 4];
        for (k, &d) in depths.iter().enumerate() {
            let mut v = 0u32;
            for _ in 0..d {
                v = v << 1 | u32::from(bytes[pos >> 3] >> (7 - (pos & 7)) & 1);
                pos += 1;
            }
            row[k] = v;
        }
        out.push(row);
    }
    Ok(out)
}

/// Rows of the fragment's attribute descriptor (byte +0x20). Byte +0x2f
/// counts all vertex streams: skinned fragments add five streams of joint
/// data after the described attributes (the hero's 6 described of 11), and
/// the descriptor's parameter floats follow its rows.
fn described(p: &Loaded, f: usize) -> Result<usize> {
    let (rows, streams) = (p.data[f + 0x20] as usize, p.data[f + 0x2f] as usize);
    ensure!((1..=32).contains(&rows) && rows <= streams, "ND fragment attribute count");
    Ok(rows)
}

/// Attribute `id` with `N` components of fragment `f`, if present.
fn raw_attribute<const N: usize>(p: &Loaded, f: usize, id: u8, vertices: usize) -> Result<Option<Raw<N>>> {
    let desc = p.ptr(f + 0x28)?;
    for row_index in 0..described(p, f)? {
        let at = desc + 8 + 8 * row_index;
        let row: [u8; 8] = p.data.get(at..at + 8).context("ND descriptor outside package")?.try_into()?;
        if row[0] != id {
            continue;
        }
        let kind = row[1] >> 4;
        ensure!((row[1] & 15) as usize == N && matches!(kind, 0 | 4 | 6), "ND attribute {id:#x} format {:#x}", row[1]);
        let depths: [u8; N] = std::array::from_fn(|k| row[4 + k]);
        ensure!(depths.iter().all(|d| *d <= 24), "ND attribute bit depth");
        let param = desc + usize::from(u16::from_be_bytes([row[2], row[3]]));
        let float = |k: usize| if kind == 0 { Ok(0.) } else { p.f32(param + 4 * k) };
        let a = (0..N).map(float).collect::<Result<Vec<_>>>()?;
        let b = (0..N).map(|k| float(N + k)).collect::<Result<Vec<_>>>()?;
        let offset = (0..N).map(|k| if kind == 6 { p.u32(param + 4 * (2 * N + k)) } else { Ok(0) }).collect::<Result<Vec<_>>>()?;
        let values = unpack(&p.data, p.ptr(f + 0x50 + 8 * row_index)?, vertices, &depths)?
            .iter()
            .map(|r| std::array::from_fn(|k| if kind == 6 { 4 * u64::from(r[k]) + u64::from(offset[k]) } else { u64::from(r[k]) }))
            .collect();
        return Ok(Some(Raw { kind, depths, a: std::array::from_fn(|k| a[k]), b: std::array::from_fn(|k| b[k]), values }));
    }
    Ok(None)
}

/// Dequantized values (module documentation).
fn dequantize<const N: usize>(raw: &Raw<N>) -> Vec<[f32; N]> {
    raw.values
        .iter()
        .map(|v| {
            std::array::from_fn(|k| {
                let (a, b, v) = (f64::from(raw.a[k]), f64::from(raw.b[k]), v[k] as f64);
                (match raw.kind {
                    0 => v * 2. / ((1u64 << raw.depths[k]) - 1).max(1) as f64 - 1.,
                    6 => a * (1. + v * 2f64.powi(-23)) + b,
                    _ => a * (1. + v * 2f64.powi(-21)) + b,
                }) as f32
            })
        })
        .collect()
}

/// Joint influences of a skinned fragment from its skin information
/// (fragment +0x0c): `u16` byte sizes of streams s1–s4, the palette range
/// count, then pointers to s1–s4 at +0xc, a null word and the palette
/// ranges `{u16 first joint, u16 count}` at +0x20. Vertices come in groups
/// of four; s1 holds one code per group: 4/6 read a joint list shared by
/// the four lanes from s2 (`u16` palette offsets of 48 bytes, +4 while more
/// follow), 2/10 read per-lane rows of four `u16` from s3 (lane order 1, 0,
/// 3, 2; the last row has no continuation flags), 0 repeats the previous
/// group and 8 keeps its joints with new weights. Codes 6, 8 and 10 read
/// one row of four `u8` weights per joint from s4; codes 2 and 4 give the
/// first joint the whole weight. Layout from the public Uncharted 3 PS3
/// decoder named in the module documentation; the stream sizes of every
/// installed Uncharted 2 skinned fragment are consumed exactly.
pub fn skin(p: &Loaded, info: usize, vertices: usize) -> Result<Vec<Vec<(u16, f32)>>> {
    let size = |k: usize| -> Result<usize> { Ok(usize::from(p.u16(info + 2 * k)?)) };
    let (s1, s2, s3, s4, ranges) = (size(0)?, size(1)?, size(2)?, size(3)?, size(4)?);
    ensure!(ranges <= 64, "ND skin palette ranges");
    let stream = |k: usize, len: usize| -> Result<&[u8]> {
        if len == 0 {
            return Ok(&[]);
        }
        let at = p.ptr(info + 0xc + 4 * k)?;
        p.data.get(at..at + len).context("ND skin stream outside package")
    };
    let (codes, list, rows, weights) = (stream(0, s1)?, stream(1, s2)?, stream(2, s3)?, stream(3, s4)?);
    let u16_at = |b: &[u8], i: usize| -> Result<u16> { Ok(u16::from_be_bytes(b.get(2 * i..2 * i + 2).context("ND skin stream overrun")?.try_into()?)) };
    let mut palette = Vec::new();
    if ranges > 0 {
        let at = p.ptr(info + 0x20)?;
        for r in 0..ranges {
            let (first, n) = (p.u16(at + 4 * r)?, p.u16(at + 4 * r + 2)?);
            palette.extend(first..first + n);
        }
    }
    let joint = |offset: u16| -> Result<u16> { palette.get(usize::from(offset / 48)).copied().context("ND skin palette index") };
    let groups = vertices.div_ceil(4);
    ensure!(groups <= codes.len(), "ND skin code stream");
    let (mut p2, mut p3, mut wp) = (0usize, 0usize, 0usize);
    let mut lanes: [Vec<u16>; 4] = Default::default();
    let mut previous: Vec<[u8; 4]> = Vec::new();
    let mut out = Vec::with_capacity(vertices);
    for (g, &code) in codes.iter().take(groups).enumerate() {
        ensure!(matches!(code, 0 | 2 | 4 | 6 | 8 | 10), "ND skin group code {code}");
        match code {
            2 | 10 => {
                let mut columns: [Vec<u16>; 4] = Default::default();
                loop {
                    let row = [u16_at(rows, p3)?, u16_at(rows, p3 + 1)?, u16_at(rows, p3 + 2)?, u16_at(rows, p3 + 3)?];
                    p3 += 4;
                    for (lane, raw) in [row[1], row[0], row[3], row[2]].into_iter().enumerate() {
                        columns[lane].push(joint(raw)?);
                    }
                    if row.iter().all(|x| x % 48 == 0) {
                        break;
                    }
                }
                lanes = columns;
            }
            4 | 6 => {
                let mut shared = Vec::new();
                loop {
                    let x = u16_at(list, p2)?;
                    p2 += 1;
                    shared.push(joint(x)?);
                    if x % 48 == 0 {
                        break;
                    }
                }
                lanes = [shared.clone(), shared.clone(), shared.clone(), shared];
            }
            _ => {}
        }
        if code != 0 {
            previous.clear();
            if matches!(code, 6 | 8 | 10) {
                for _ in 0..lanes[0].len() {
                    previous.push(weights.get(wp..wp + 4).context("ND skin weight overrun")?.try_into()?);
                    wp += 4;
                }
            }
        }
        for (lane, joints) in lanes.iter().enumerate() {
            if g * 4 + lane >= vertices {
                break;
            }
            let mut v: Vec<(u16, f32)> = Vec::with_capacity(4);
            if previous.is_empty() {
                v.push((*joints.first().context("ND skin lane without joints")?, 1.));
            } else {
                for (k, row) in previous.iter().enumerate() {
                    let w = f32::from(row[lane]) / 255.;
                    if w > 0. {
                        let j = *joints.get(k).context("ND skin weight without joint")?;
                        match v.iter_mut().find(|(x, _)| *x == j) {
                            Some(e) => e.1 += w,
                            None => v.push((j, w)),
                        }
                    }
                }
            }
            out.push(v);
        }
    }
    ensure!((2 * p2, 2 * p3, wp) == (s2, s3, s4), "ND skin streams not consumed exactly");
    Ok(out)
}

/// Attribute `id` dequantized, if present with `N` components.
fn attribute<const N: usize>(p: &Loaded, f: usize, id: u8, vertices: usize) -> Result<Option<Vec<[f32; N]>>> {
    Ok(raw_attribute::<N>(p, f, id, vertices)?.map(|r| dequantize(&r)))
}

fn cstring(p: &Loaded, at: usize) -> Result<String> {
    let bytes = p.data.get(at..).context("ND string outside package")?;
    let end = bytes.iter().take(1024).position(|b| *b == 0).context("unterminated ND string")?;
    Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

/// The material record at `m` (name, passes, samplers, layer blend and
/// constants).
pub fn material(p: &Loaded, m: usize) -> Result<Material> {
    let name = cstring(p, p.ptr(m)?)?;
    let passes = p.u32(m + 0xc)?;
    ensure!((1..=64).contains(&passes), "ND material pass count");
    let pass_samplers = |pass: usize| -> Result<Vec<u32>> {
        let data = p.ptr(pass + 0x10)?;
        let count = usize::from(p.u16(data + 0xc)?);
        ensure!(count <= 16, "ND sampler count");
        if count == 0 {
            return Ok(Vec::new());
        }
        let table = p.ptr(data + 0x18)?;
        (0..count).map(|s| p.u32(p.ptr(table + 16 * s)?)).collect()
    };
    let list = p.ptr(m + 0x10)?;
    let samplers = pass_samplers(p.ptr(list)?)?;
    let layers = p
        .ptr(p.ptr(list)? + 0x10)
        .and_then(|data| Ok((crate::rsx::pass_program(p, data)?, crate::rsx::sampler_units(p, data)?)))
        .ok()
        .and_then(|(code, units)| crate::rsx::layer_blend(&code, &units));
    let passes = (0..passes as usize).map(|i| p.ptr(list + 4 * i).and_then(pass_samplers)).collect::<Result<Vec<_>>>().unwrap_or_else(|_| vec![samplers.clone()]);
    let parameters = p.ptr(p.ptr(list)? + 0x10).and_then(|data| crate::rsx::pass_parameters(p, data)).unwrap_or_default();
    Ok(Material { name, samplers, passes, layers, parameters })
}

/// First word of the collision-mesh collections in actor packages (the
/// foreground login code tests the same value); these hold no render meshes.
pub const COLLISION_COLLECTION: usize = 0x1001;

/// Every mesh of a `PM_COLLECTION_2` payload. Fragments without stored
/// positions (two-triangle quads with only texture coordinates in the
/// installed `post-56` package) decode as empty fragments so group ranges
/// keep their native indices.
pub fn prototype_meshes(p: &Loaded, payload: usize) -> Result<Vec<Mesh>> {
    let count = p.u32(payload)? as usize;
    if count == COLLISION_COLLECTION {
        return Ok(Vec::new());
    }
    ensure!(count <= 4096, "ND prototype count");
    let mut meshes = Vec::with_capacity(count);
    for e in 0..count {
        let record = p.ptr(payload + 4 + 4 * e)?;
        let header = p.ptr(record)?;
        let placed = p.u32(record + 8)? as usize;
        ensure!(placed <= 65536, "ND instance count");
        let (mut instances, mut spheres) = (Vec::with_capacity(placed), Vec::with_capacity(placed));
        if placed > 0 {
            let list = p.ptr(record + 4)?;
            for i in 0..placed {
                let at = p.ptr(list + 4 * i)?;
                let m: [f32; 16] = std::array::from_fn(|k| p.f32(at + 0x40 + 4 * k).unwrap_or(f32::NAN));
                let s: [f32; 4] = std::array::from_fn(|k| p.f32(at + 0x30 + 4 * k).unwrap_or(f32::NAN));
                ensure!(m.iter().chain(&s).all(|v| v.is_finite()) && (m[15] - 1.).abs() < 1e-3, "ND instance matrix");
                instances.push(m);
                spheres.push(s);
            }
        }
        let (group_count, fragments) = (usize::from(p.u16(header + 4)?), usize::from(p.u16(header + 6)?));
        let array = p.ptr(header + 0x14)?;
        let mut decoded = Vec::with_capacity(fragments);
        for i in 0..fragments {
            let f = p.ptr(array + 4 * i)?;
            let vertices = p.u32(p.ptr(f + 0x24)?)? as usize;
            ensure!(vertices <= 0x10000, "ND fragment vertex count");
            let Some(positions) = attribute::<3>(p, f, ATTR_POSITION, vertices)? else {
                decoded.push(Fragment::default());
                continue;
            };
            // One installed prop (`ice-t-prayer-wheel-gears`) stores
            // coordinates in format 0xb8, not yet decoded: it keeps no UVs.
            let uvs = attribute::<2>(p, f, ATTR_UV, vertices).ok().flatten().unwrap_or_default();
            let normals = attribute::<3>(p, f, ATTR_NORMAL, vertices).ok().flatten().unwrap_or_default();
            let masks = attribute::<3>(p, f, ATTR_LAYER_MASK, vertices).ok().flatten().map(|m| m.into_iter().map(|v| v.map(|x| (x + 1.) * 0.5)).collect()).unwrap_or_default();
            let skin = match p.ptr(f + 0x0c) {
                Ok(info) => skin(p, info, vertices)?,
                Err(_) => Vec::new(),
            };
            let triangles = decode_indices(&p.data[p.ptr(f + 0x48)?..], usize::from(p.u16(f + 0x40)?), vertices)?;
            decoded.push(Fragment { positions, normals, masks, uvs, triangles, skin });
        }
        let fragments = decoded;
        let skinned = (0..usize::from(p.u16(header + 6)?)).any(|i| p.ptr(array + 4 * i).is_ok_and(|f| p.data[f + 0x2f] > p.data[f + 0x20]));
        let groups_at = p.ptr(header + 0x10)?;
        let mut groups = Vec::with_capacity(group_count);
        for g in 0..group_count {
            let row = groups_at + 16 * g;
            let (first, n) = (usize::from(p.u16(row + 4)?), usize::from(p.u16(row + 6)?));
            ensure!(first + n <= fragments.len(), "ND mesh group range");
            groups.push(Group { material: material(p, p.ptr(p.ptr(row)?)?)?, fragments: first..first + n });
        }
        meshes.push(Mesh { fragments, groups, instances, spheres, skinned });
    }
    Ok(meshes)
}
