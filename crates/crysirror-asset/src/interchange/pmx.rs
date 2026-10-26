// SPDX-License-Identifier: MPL-2.0
//! MikuMikuDance PMX 2.0/2.1 models → glTF.
//!
//! Parsed: header, vertices (BDEF1/2/4, SDEF, QDEF weights), faces, texture
//! table, materials and bones. MMD space is left-handed, Y-up, with 1 unit
//! ≈ 8 cm; it is converted to glTF metres by negating Z, scaling by
//! [`MMD_UNIT_METRES`] and reversing triangle winding. Bones become glTF
//! joints whose rest pose is the PMX bone position.
use super::{glb, Textures};
use anyhow::{bail, ensure, Context, Result};
use glam::{Mat4, Vec3};

/// Conventional MMD unit: a 20-unit model is about 1.6 m tall.
pub const MMD_UNIT_METRES: f32 = 0.08;
const MAX_COUNT: usize = 4_000_000;

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
    utf8: bool,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).context("PMX offset overflow")?;
        let s = self
            .data
            .get(self.at..end)
            .with_context(|| format!("PMX truncated at byte {}", self.at))?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn f32(&mut self) -> Result<f32> {
        let v = f32::from_le_bytes(self.take(4)?.try_into()?);
        ensure!(
            v.is_finite(),
            "PMX non-finite float at byte {}",
            self.at - 4
        );
        Ok(v)
    }
    fn vec<const N: usize>(&mut self) -> Result<[f32; N]> {
        let mut out = [0.; N];
        for v in &mut out {
            *v = self.f32()?;
        }
        Ok(out)
    }
    fn count(&mut self) -> Result<usize> {
        let n = self.i32()?;
        ensure!(
            n >= 0 && (n as usize) <= MAX_COUNT,
            "PMX count {n} out of range"
        );
        Ok(n as usize)
    }
    fn text(&mut self) -> Result<String> {
        let n = self.count()?;
        let bytes = self.take(n)?;
        if self.utf8 {
            Ok(String::from_utf8_lossy(bytes).into_owned())
        } else {
            ensure!(n % 2 == 0, "PMX odd UTF-16 text length");
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            Ok(String::from_utf16_lossy(&units))
        }
    }
    /// Signed index of 1, 2 or 4 bytes (-1 means none).
    fn index(&mut self, size: u8) -> Result<i32> {
        Ok(match size {
            1 => self.u8()? as i8 as i32,
            2 => self.u16()? as i16 as i32,
            4 => self.i32()?,
            _ => bail!("PMX index size {size}"),
        })
    }
    /// Vertex indices of 1 or 2 bytes are unsigned.
    fn vertex_index(&mut self, size: u8) -> Result<u32> {
        Ok(match size {
            1 => self.u8()? as u32,
            2 => self.u16()? as u32,
            4 => u32::try_from(self.i32()?).context("negative PMX vertex index")?,
            _ => bail!("PMX vertex index size {size}"),
        })
    }
}

pub struct Bone {
    pub name: String,
    pub position: [f32; 3],
    pub parent: i32,
}

pub struct MaterialRange {
    pub name: String,
    pub diffuse: [f32; 4],
    pub flags: u8,
    pub texture: i32,
    pub sphere: i32,
    pub toon_shared: bool,
    pub index_count: usize,
}

pub struct Model {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub joints: Vec<[i32; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub textures: Vec<String>,
    pub materials: Vec<MaterialRange>,
    pub bones: Vec<Bone>,
    pub sdef: usize,
    pub qdef: usize,
    pub ik_bones: usize,
    /// Vertex morphs (and group morphs made of vertex morphs): name and
    /// (vertex, MMD-space delta) offsets.
    pub vertex_morphs: Vec<(String, Vec<(u32, [f32; 3])>)>,
    /// Morphs of other kinds (bone, UV, material, flip, impulse).
    pub other_morphs: usize,
    pub additional_uvs: u8,
}

pub fn parse(bytes: &[u8]) -> Result<Model> {
    ensure!(bytes.starts_with(b"PMX "), "not a PMX file");
    let mut r = Reader {
        data: bytes,
        at: 4,
        utf8: false,
    };
    let version = r.f32()?;
    ensure!(
        version == 2.0 || version == 2.1,
        "PMX version {version} unsupported"
    );
    let globals = r.u8()? as usize;
    ensure!(globals >= 8, "PMX header has {globals} globals");
    let g = r.take(globals)?.to_vec();
    r.utf8 = match g[0] {
        0 => false,
        1 => true,
        e => bail!("PMX text encoding {e}"),
    };
    let (extra_uv, vsize, tsize, msize, bsize, morph_size, rigid_size) =
        (g[1], g[2], g[3], g[4], g[5], g[6], g[7]);
    ensure!(extra_uv <= 4, "PMX additional UV count {extra_uv}");
    let name = r.text()?;
    for _ in 0..3 {
        r.text()?;
    }
    let n = r.count()?;
    let mut m = Model {
        name,
        positions: Vec::with_capacity(n),
        normals: Vec::with_capacity(n),
        uvs: Vec::with_capacity(n),
        joints: Vec::with_capacity(n),
        weights: Vec::with_capacity(n),
        indices: vec![],
        textures: vec![],
        materials: vec![],
        bones: vec![],
        sdef: 0,
        qdef: 0,
        ik_bones: 0,
        vertex_morphs: vec![],
        other_morphs: 0,
        additional_uvs: extra_uv,
    };
    for _ in 0..n {
        m.positions.push(r.vec()?);
        m.normals.push(r.vec()?);
        m.uvs.push(r.vec()?);
        for _ in 0..extra_uv {
            r.vec::<4>()?;
        }
        let (joints, weights) = match r.u8()? {
            0 => ([r.index(bsize)?, -1, -1, -1], [1., 0., 0., 0.]),
            1 => {
                let (a, b, w) = (r.index(bsize)?, r.index(bsize)?, r.f32()?);
                ([a, b, -1, -1], [w, 1. - w, 0., 0.])
            }
            t @ (2 | 4) => {
                let j = [
                    r.index(bsize)?,
                    r.index(bsize)?,
                    r.index(bsize)?,
                    r.index(bsize)?,
                ];
                if t == 4 {
                    m.qdef += 1;
                }
                (j, r.vec()?)
            }
            3 => {
                let (a, b, w) = (r.index(bsize)?, r.index(bsize)?, r.f32()?);
                r.vec::<9>()?; // C, R0, R1
                m.sdef += 1;
                ([a, b, -1, -1], [w, 1. - w, 0., 0.])
            }
            t => bail!("PMX weight type {t}"),
        };
        m.joints.push(joints);
        m.weights.push(weights);
        r.f32()?; // edge scale
    }
    let faces = r.count()?;
    ensure!(faces % 3 == 0, "PMX face index count {faces}");
    m.indices.reserve(faces);
    for _ in 0..faces {
        let i = r.vertex_index(vsize)?;
        ensure!((i as usize) < n, "PMX face index {i} outside {n} vertices");
        m.indices.push(i);
    }
    for _ in 0..r.count()? {
        m.textures.push(r.text()?);
    }
    for _ in 0..r.count()? {
        let name = r.text()?;
        r.text()?;
        let diffuse = r.vec()?;
        r.vec::<3>()?; // specular
        r.f32()?; // specular strength
        r.vec::<3>()?; // ambient
        let flags = r.u8()?;
        r.vec::<4>()?; // edge colour
        r.f32()?; // edge size
        let texture = r.index(tsize)?;
        let sphere = r.index(tsize)?;
        r.u8()?; // sphere mode
        let toon_shared = r.u8()? != 0;
        if toon_shared {
            r.u8()?;
        } else {
            r.index(tsize)?;
        }
        r.text()?; // memo
        let index_count = r.count()?;
        m.materials.push(MaterialRange {
            name,
            diffuse,
            flags,
            texture,
            sphere,
            toon_shared,
            index_count,
        });
    }
    ensure!(
        m.materials.iter().map(|x| x.index_count).sum::<usize>() == faces,
        "PMX material face counts do not cover the index buffer"
    );
    for _ in 0..r.count()? {
        let name = r.text()?;
        r.text()?;
        let position = r.vec()?;
        let parent = r.index(bsize)?;
        r.i32()?; // transform layer
        let flags = r.u16()?;
        if flags & 0x0001 != 0 {
            r.index(bsize)?;
        } else {
            r.vec::<3>()?;
        }
        if flags & 0x0300 != 0 {
            r.index(bsize)?;
            r.f32()?;
        }
        if flags & 0x0400 != 0 {
            r.vec::<3>()?;
        }
        if flags & 0x0800 != 0 {
            r.vec::<6>()?;
        }
        if flags & 0x2000 != 0 {
            r.i32()?;
        }
        if flags & 0x0020 != 0 {
            m.ik_bones += 1;
            r.index(bsize)?;
            r.i32()?;
            r.f32()?;
            for _ in 0..r.count()? {
                r.index(bsize)?;
                if r.u8()? != 0 {
                    r.vec::<6>()?;
                }
            }
        }
        m.bones.push(Bone {
            name,
            position,
            parent,
        });
    }
    // Morphs: vertex morphs are kept; group morphs are expanded below.
    let mut groups: Vec<(String, Vec<(i32, f32)>)> = vec![];
    let mut vertex_of_morph: Vec<Option<usize>> = vec![];
    for _ in 0..r.count()? {
        let name = r.text()?;
        r.text()?;
        r.u8()?; // panel
        let kind = r.u8()?;
        let n = r.count()?;
        vertex_of_morph.push(None);
        match kind {
            0 | 9 => {
                let mut members = Vec::with_capacity(n);
                for _ in 0..n {
                    members.push((r.index(morph_size)?, r.f32()?));
                }
                if kind == 0 {
                    groups.push((name, members));
                } else {
                    m.other_morphs += 1;
                }
            }
            1 => {
                let mut offsets = Vec::with_capacity(n);
                for _ in 0..n {
                    let v = r.vertex_index(vsize)?;
                    ensure!((v as usize) < m.positions.len(), "PMX morph vertex {v} out of range");
                    offsets.push((v, r.vec()?));
                }
                *vertex_of_morph.last_mut().unwrap() = Some(m.vertex_morphs.len());
                m.vertex_morphs.push((name, offsets));
            }
            2 => {
                for _ in 0..n {
                    r.index(bsize)?;
                    r.vec::<7>()?;
                }
                m.other_morphs += 1;
            }
            3..=7 => {
                for _ in 0..n {
                    r.vertex_index(vsize)?;
                    r.vec::<4>()?;
                }
                m.other_morphs += 1;
            }
            8 => {
                for _ in 0..n {
                    r.index(msize)?;
                    r.u8()?;
                    r.vec::<28>()?;
                }
                m.other_morphs += 1;
            }
            10 => {
                for _ in 0..n {
                    r.index(rigid_size)?;
                    r.u8()?;
                    r.vec::<6>()?;
                }
                m.other_morphs += 1;
            }
            k => bail!("PMX morph type {k}"),
        }
    }
    // Group morphs whose members are vertex morphs become summed vertex morphs.
    for (name, members) in groups {
        let mut sum: std::collections::BTreeMap<u32, [f32; 3]> = Default::default();
        for (index, w) in members {
            let Some(vm) = usize::try_from(index).ok().and_then(|i| vertex_of_morph.get(i).copied().flatten()) else {
                continue;
            };
            for (v, d) in &m.vertex_morphs[vm].1 {
                let e = sum.entry(*v).or_insert([0.; 3]);
                for a in 0..3 {
                    e[a] += d[a] * w;
                }
            }
        }
        if sum.is_empty() {
            m.other_morphs += 1;
        } else {
            m.vertex_morphs.push((name, sum.into_iter().collect()));
        }
    }

    let bones = m.bones.len() as i32;
    for b in &m.bones {
        ensure!(
            b.parent < bones,
            "PMX bone parent {} out of range",
            b.parent
        );
    }
    for j in m.joints.iter().flatten() {
        ensure!(*j < bones, "PMX vertex bone {j} out of range");
    }
    Ok(m)
}

fn to_gltf_space(p: [f32; 3]) -> [f32; 3] {
    [
        p[0] * MMD_UNIT_METRES,
        p[1] * MMD_UNIT_METRES,
        -p[2] * MMD_UNIT_METRES,
    ]
}

pub fn convert(
    bytes: &[u8],
    external: &mut dyn FnMut(&str) -> Result<Vec<u8>>,
) -> Result<glb::Document> {
    let m = parse(bytes)?;
    let mut doc = glb::Document {
        generator: "CrySirror PMX importer".into(),
        ..Default::default()
    };
    let mut textures = Textures::new(external);

    // Skeleton: one node per bone, translation relative to its parent.
    let bone_base = 0usize;
    let has_bones = !m.bones.is_empty() && m.joints.iter().flatten().any(|j| *j >= 0);
    for b in &m.bones {
        let pos = Vec3::from(to_gltf_space(b.position));
        let parent =
            (b.parent >= 0).then(|| Vec3::from(to_gltf_space(m.bones[b.parent as usize].position)));
        let mut node = glb::Node::named(b.name.clone());
        node.translation = (pos - parent.unwrap_or(Vec3::ZERO)).to_array();
        doc.nodes.push(node);
    }
    for (i, b) in m.bones.iter().enumerate() {
        if b.parent >= 0 {
            ensure!(b.parent as usize != i, "PMX bone is its own parent");
            doc.nodes[bone_base + b.parent as usize]
                .children
                .push(bone_base + i);
        } else {
            doc.roots.push(bone_base + i);
        }
    }
    // Reject parent cycles: every bone must reach a root.
    for i in 0..m.bones.len() {
        let mut p = m.bones[i].parent;
        let mut steps = 0;
        while p >= 0 {
            steps += 1;
            ensure!(steps <= m.bones.len(), "PMX bone parent cycle");
            p = m.bones[p as usize].parent;
        }
    }

    let positions: Vec<[f32; 3]> = m.positions.iter().map(|p| to_gltf_space(*p)).collect();
    let normals: Vec<[f32; 3]> = m
        .normals
        .iter()
        .map(|n| {
            let v = Vec3::new(n[0], n[1], -n[2]);
            if v.length_squared() > 1e-12 {
                v.normalize().to_array()
            } else {
                [0., 1., 0.]
            }
        })
        .collect();
    // Skin joints are the bones some vertex weights; the engine palette is
    // bounded (anim::MAX_JOINTS) and PMX rigs carry many physics-only bones.
    let mut used: Vec<usize> = m
        .joints
        .iter()
        .zip(&m.weights)
        .flat_map(|(j, w)| {
            (0..4)
                .filter(move |k| j[*k] >= 0 && w[*k] > 0.)
                .map(move |k| j[k] as usize)
        })
        .collect();
    used.sort_unstable();
    used.dedup();
    ensure!(
        used.len() <= crate::anim::MAX_JOINTS,
        "PMX skin weights {} bones; the engine supports {}",
        used.len(),
        crate::anim::MAX_JOINTS
    );
    let mut palette = vec![0u16; m.bones.len()];
    for (slot, bone) in used.iter().enumerate() {
        palette[*bone] = slot as u16;
    }
    let mut prims = Vec::new();
    let mut start = 0;
    for mat in &m.materials {
        let range = &m.indices[start..start + mat.index_count];
        start += mat.index_count;
        if range.is_empty() {
            continue;
        }
        // Compact the vertices this material uses.
        let mut remap = std::collections::HashMap::new();
        let mut p = glb::Primitive::default();
        for tri in range.chunks_exact(3) {
            // Z negation mirrors the mesh; reverse winding to keep front faces.
            for &v in [tri[0], tri[2], tri[1]].iter() {
                let next = remap.len() as u32;
                let local = *remap.entry(v).or_insert_with(|| {
                    let v = v as usize;
                    p.positions.push(positions[v]);
                    p.normals.push(normals[v]);
                    p.uvs.push(m.uvs[v]);
                    if has_bones {
                        let mut j = [0u16; 4];
                        let mut w = [0f32; 4];
                        for k in 0..4 {
                            if m.joints[v][k] >= 0 && m.weights[v][k] > 0. {
                                j[k] = palette[m.joints[v][k] as usize];
                                w[k] = m.weights[v][k];
                            }
                        }
                        let sum: f32 = w.iter().sum();
                        if sum > 0. {
                            w.iter_mut().for_each(|x| *x /= sum);
                        } else {
                            w = [1., 0., 0., 0.];
                        }
                        p.joints.push(j);
                        p.weights.push(w);
                    }
                    next
                });
                p.indices.push(local);
            }
        }
        let texture = usize::try_from(mat.texture)
            .ok()
            .and_then(|t| m.textures.get(t))
            .and_then(|path| textures.embed(&mut doc, path, None));
        let alpha = mat.diffuse[3] < 0.999;
        let cutout = !alpha && texture.is_some_and(|t| Textures::has_alpha(&doc, t));
        doc.materials.push(glb::Material {
            name: mat.name.clone(),
            base_color: mat.diffuse,
            base_color_texture: texture,
            double_sided: mat.flags & 0x01 != 0,
            alpha_mode: if alpha {
                "BLEND"
            } else if cutout {
                "MASK"
            } else {
                "OPAQUE"
            },
            roughness: 0.7,
            ..Default::default()
        });
        p.material = Some(doc.materials.len() - 1);
        // Morph targets touching this material's vertices, as dense deltas.
        for (name, offsets) in &m.vertex_morphs {
            let mut delta = vec![[0f32; 3]; p.positions.len()];
            let mut touched = false;
            for (v, d) in offsets {
                if let Some(local) = remap.get(v) {
                    delta[*local as usize] = to_gltf_space(*d);
                    touched |= d.iter().any(|x| *x != 0.);
                }
            }
            if touched {
                p.targets.push(delta);
                p.target_names.push(name.clone());
            }
        }
        prims.push((mat.name.clone(), p));
    }
    ensure!(!prims.is_empty(), "PMX model has no drawable faces");
    // One mesh per material, so each carries only the morphs that touch it.
    let mut mesh_nodes = Vec::new();
    for (name, p) in prims {
        doc.meshes.push((name.clone(), vec![p]));
        let mut node = glb::Node::named(format!("{}_{name}", m.name));
        node.mesh = Some(doc.meshes.len() - 1);
        node.skin = has_bones.then_some(0);
        doc.nodes.push(node);
        mesh_nodes.push(doc.nodes.len() - 1);
    }
    if has_bones {
        let inverse_bind = used
            .iter()
            .map(|b| {
                Mat4::from_translation(-Vec3::from(to_gltf_space(m.bones[*b].position)))
                    .to_cols_array()
            })
            .collect();
        doc.skins.push(glb::Skin {
            joints: used.iter().map(|b| bone_base + b).collect(),
            inverse_bind,
            skeleton: None,
        });
    }
    doc.roots.extend(mesh_nodes);

    if m.sdef > 0 {
        doc.dropped
            .push(format!("{} SDEF vertices skinned as linear blends", m.sdef));
    }
    if m.qdef > 0 {
        doc.dropped
            .push(format!("{} QDEF vertices skinned as linear blends", m.qdef));
    }
    if m.ik_bones > 0 {
        doc.dropped.push(format!(
            "{} IK bone chains (no engine IK solver yet)",
            m.ik_bones
        ));
    }
    if m.other_morphs > 0 {
        doc.dropped.push(format!(
            "{} bone/UV/material/flip/impulse morphs",
            m.other_morphs
        ));
    }
    if m.additional_uvs > 0 {
        doc.dropped
            .push(format!("{} additional UV sets", m.additional_uvs));
    }
    let spheres = m.materials.iter().filter(|x| x.sphere >= 0).count();
    if spheres > 0 {
        doc.dropped.push(format!("{spheres} sphere maps"));
    }
    doc.dropped
        .push("MMD toon shading and edge outlines (approximated by PBR)".into());
    Ok(doc)
}
