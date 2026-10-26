// SPDX-License-Identifier: MPL-2.0
//! CPU-side PBR model and environment decoding for the renderer (ADR 0010).
//! Everything here is plain data rebuilt from asset bytes; nothing is saved.
use anyhow::{bail, ensure, Context, Result};

pub const MAX_TEXTURE_DIM: u32 = 8192;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct PbrVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// xyz tangent, w handedness.
    pub tangent: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct PbrPrimitive {
    /// Per-vertex joint indices and weights for skinned primitives.
    pub skin_attrs: Option<Vec<([u16; 4], [f32; 4])>>,
    pub vertices: Vec<PbrVertex>,
    pub indices: Vec<u32>,
    pub material: usize,
    pub bounds: ([f32; 3], [f32; 3]),
    /// Morph targets (blend shapes), sparse: only displaced vertices.
    pub morphs: Vec<MorphTarget>,
}

/// One morph target of a primitive: displaced vertex indices and their
/// position/normal deltas.
#[derive(Clone, Debug, Default)]
pub struct MorphTarget {
    pub name: String,
    pub indices: Vec<u32>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

/// Upper bound on morph targets per primitive.
pub const MAX_MORPH_TARGETS: usize = 512;

impl PbrPrimitive {
    /// Vertices with the named morph weights applied, or `None` when no
    /// target of this primitive has a non-zero weight.
    pub fn morphed(&self, weight: impl Fn(&str) -> f32) -> Option<Vec<PbrVertex>> {
        let active: Vec<(&MorphTarget, f32)> = self
            .morphs
            .iter()
            .map(|t| (t, weight(&t.name)))
            .filter(|(_, w)| *w != 0.)
            .collect();
        if active.is_empty() {
            return None;
        }
        let mut out = self.vertices.clone();
        for (t, w) in active {
            for (k, i) in t.indices.iter().enumerate() {
                let v = &mut out[*i as usize];
                for a in 0..3 {
                    v.position[a] += t.positions[k][a] * w;
                }
                if let Some(n) = t.normals.get(k) {
                    for a in 0..3 {
                        v.normal[a] += n[a] * w;
                    }
                }
            }
        }
        for v in &mut out {
            let n = glam::Vec3::from(v.normal);
            if n.length_squared() > 1e-12 {
                v.normal = n.normalize().to_array();
            }
        }
        Some(out)
    }
}

pub use crysirror_scene::material::AlphaMode;
pub type PbrMaterial = crysirror_scene::material::Material<usize>;

/// Decoded RGBA8 image with a full mip chain (level 0 first).
#[derive(Clone, Debug)]
pub struct MipImage {
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
    /// True for color data (base color, emissive) that is sRGB-encoded.
    pub srgb: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trs {
    pub translation: glam::Vec3,
    pub rotation: glam::Quat,
    pub scale: glam::Vec3,
}
impl Trs {
    pub fn matrix(&self) -> glam::Mat4 {
        glam::Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}
#[derive(Clone, Debug)]
pub struct SceneNode {
    pub name: String,
    pub children: Vec<usize>,
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
    pub rest: Trs,
}
#[derive(Clone, Debug)]
pub struct Skin {
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<glam::Mat4>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelPath {
    Translation,
    Rotation,
    Scale,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
    CubicSpline,
}
#[derive(Clone, Debug)]
pub struct Channel {
    pub node: usize,
    pub path: ChannelPath,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    /// xyz for T/S, xyzw quaternion for R. Cubic splines store
    /// (in-tangent, value, out-tangent) triplets per key.
    pub values: Vec<[f32; 4]>,
}
#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, Default)]
pub struct PbrModel {
    /// glTF meshes, each a list of primitives.
    pub meshes: Vec<Vec<PbrPrimitive>>,
    pub nodes: Vec<SceneNode>,
    /// Root nodes of the default scene.
    pub roots: Vec<usize>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Clip>,
    pub materials: Vec<PbrMaterial>,
    pub images: Vec<MipImage>,
    pub dropped: Vec<String>,
}

impl PbrModel {
    /// World matrices of every node in the rest pose (default scene).
    pub fn rest_world(&self) -> Vec<glam::Mat4> {
        let mut world = vec![glam::Mat4::IDENTITY; self.nodes.len()];
        let mut stack: Vec<(usize, glam::Mat4)> = self.roots.iter().map(|r| (*r, glam::Mat4::IDENTITY)).collect();
        let mut seen = vec![false; self.nodes.len()];
        while let Some((i, parent)) = stack.pop() {
            if i >= self.nodes.len() || std::mem::replace(&mut seen[i], true) {
                continue;
            }
            world[i] = parent * self.nodes[i].rest.matrix();
            stack.extend(self.nodes[i].children.iter().map(|c| (*c, world[i])));
        }
        world
    }
    /// Axis-aligned bounds of the default scene in its rest pose. Skinned
    /// vertices go through their joints (joint world × inverse bind), as the
    /// renderer poses them; other meshes use their node's world matrix.
    pub fn rest_bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let world = self.rest_world();
        let mut lo = glam::Vec3::splat(f32::MAX);
        let mut hi = glam::Vec3::splat(f32::MIN);
        let mut any = false;
        for (i, n) in self.nodes.iter().enumerate() {
            let Some(mesh) = n.mesh else { continue };
            let skin = n.skin.and_then(|s| self.skins.get(s));
            for prim in self.meshes.get(mesh).into_iter().flatten() {
                match (skin, &prim.skin_attrs) {
                    (Some(skin), Some(attrs)) => {
                        let joint: Vec<glam::Mat4> = skin
                            .joints
                            .iter()
                            .zip(&skin.inverse_bind)
                            .map(|(j, ib)| world.get(*j).copied().unwrap_or(glam::Mat4::IDENTITY) * *ib)
                            .collect();
                        for (v, (j, w)) in prim.vertices.iter().zip(attrs) {
                            let p = glam::Vec3::from_array(v.position);
                            let mut q = glam::Vec3::ZERO;
                            for k in 0..4 {
                                if w[k] > 0. {
                                    q += joint.get(j[k] as usize).copied().unwrap_or(glam::Mat4::IDENTITY).transform_point3(p) * w[k];
                                }
                            }
                            lo = lo.min(q);
                            hi = hi.max(q);
                            any = true;
                        }
                    }
                    _ => {
                        let (a, b) = prim.bounds;
                        for c in 0..8 {
                            let corner = glam::Vec3::new(
                                if c & 1 == 0 { a[0] } else { b[0] },
                                if c & 2 == 0 { a[1] } else { b[1] },
                                if c & 4 == 0 { a[2] } else { b[2] },
                            );
                            let q = world[i].transform_point3(corner);
                            lo = lo.min(q);
                            hi = hi.max(q);
                            any = true;
                        }
                    }
                }
            }
        }
        any.then(|| (lo.to_array(), hi.to_array()))
    }
}

/// Box-filtered mip chain. Color images are averaged in linear space.
pub fn mip_chain(width: u32, height: u32, rgba: Vec<u8>, srgb: bool) -> Vec<Vec<u8>> {
    fn to_lin(c: u8) -> f32 {
        let c = c as f32 / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn to_srgb(c: f32) -> u8 {
        let c = c.clamp(0., 1.);
        let s = if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1. / 2.4) - 0.055
        };
        (s * 255. + 0.5) as u8
    }
    let lut: Vec<f32> = (0..=255u8).map(to_lin).collect();
    let mut levels = vec![rgba];
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let src = levels.last().unwrap();
        let mut dst = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut acc = 0.;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        let v = src[((sy * w + sx) * 4 + c) as usize];
                        acc += if srgb && c < 3 {
                            lut[v as usize]
                        } else {
                            v as f32 / 255.
                        };
                    }
                    acc /= 4.;
                    dst[((y * nw + x) * 4 + c) as usize] = if srgb && c < 3 {
                        to_srgb(acc)
                    } else {
                        (acc * 255. + 0.5) as u8
                    };
                }
            }
        }
        levels.push(dst);
        (w, h) = (nw, nh);
    }
    levels
}

fn decode_image(bytes: &[u8], srgb: bool) -> Result<MipImage> {
    let img = image::load_from_memory(bytes)
        .context("decode glTF image")?
        .to_rgba8();
    let (w, h) = img.dimensions();
    ensure!(
        w <= MAX_TEXTURE_DIM && h <= MAX_TEXTURE_DIM,
        "texture exceeds {MAX_TEXTURE_DIM}"
    );
    Ok(MipImage {
        width: w,
        height: h,
        levels: mip_chain(w, h, img.into_raw(), srgb),
        srgb,
    })
}

/// Per-vertex tangents from UV derivatives (Lengyel), used when the asset
/// has normal maps but no TANGENT attribute.
fn generate_tangents(v: &mut [PbrVertex], indices: &[u32]) {
    let mut tan = vec![[0f32; 3]; v.len()];
    let mut bit = vec![[0f32; 3]; v.len()];
    for f in indices.chunks_exact(3) {
        let (a, b, c) = (f[0] as usize, f[1] as usize, f[2] as usize);
        let (p0, p1, p2) = (v[a].position, v[b].position, v[c].position);
        let (t0, t1, t2) = (v[a].uv, v[b].uv, v[c].uv);
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let (du1, dv1, du2, dv2) = (t1[0] - t0[0], t1[1] - t0[1], t2[0] - t0[0], t2[1] - t0[1]);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-12 {
            continue;
        }
        let r = 1. / det;
        let t = [
            (e1[0] * dv2 - e2[0] * dv1) * r,
            (e1[1] * dv2 - e2[1] * dv1) * r,
            (e1[2] * dv2 - e2[2] * dv1) * r,
        ];
        let bt = [
            (e2[0] * du1 - e1[0] * du2) * r,
            (e2[1] * du1 - e1[1] * du2) * r,
            (e2[2] * du1 - e1[2] * du2) * r,
        ];
        for i in [a, b, c] {
            for k in 0..3 {
                tan[i][k] += t[k];
                bit[i][k] += bt[k];
            }
        }
    }
    for (i, vx) in v.iter_mut().enumerate() {
        let n = glam::Vec3::from_array(vx.normal);
        let t = glam::Vec3::from_array(tan[i]);
        let t = (t - n * n.dot(t)).normalize_or_zero();
        let t = if t == glam::Vec3::ZERO {
            n.any_orthonormal_vector()
        } else {
            t
        };
        let w = if n.cross(t).dot(glam::Vec3::from_array(bit[i])) < 0. {
            -1.
        } else {
            1.
        };
        vx.tangent = [t.x, t.y, t.z, w];
    }
}

/// Flat-shaded vertices for untextured triangles (cubes, legacy meshes).
pub fn flat_primitive(positions: &[[f32; 3]], indices: &[u32]) -> PbrPrimitive {
    let mut vertices = Vec::with_capacity(indices.len());
    for f in indices.chunks_exact(3) {
        let p: Vec<glam::Vec3> = f
            .iter()
            .map(|i| glam::Vec3::from_array(positions[*i as usize]))
            .collect();
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        let t = n.any_orthonormal_vector();
        for q in p {
            vertices.push(PbrVertex {
                position: q.to_array(),
                normal: n.to_array(),
                uv: [0.; 2],
                tangent: [t.x, t.y, t.z, 1.],
            });
        }
    }
    let indices = (0..vertices.len() as u32).collect();
    let bounds = bounds(&vertices);
    PbrPrimitive {
        skin_attrs: None,
        vertices,
        indices,
        material: 0,
        bounds,
        morphs: Vec::new(),
    }
}

/// Area-weighted smooth normals for primitives without a NORMAL attribute.
fn smooth_normals(pos: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut n = vec![glam::Vec3::ZERO; pos.len()];
    for f in indices.chunks_exact(3) {
        let p: Vec<glam::Vec3> = f
            .iter()
            .map(|i| glam::Vec3::from_array(pos[*i as usize]))
            .collect();
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        for i in f {
            n[*i as usize] += face;
        }
    }
    n.into_iter()
        .map(|v| v.normalize_or(glam::Vec3::Y).to_array())
        .collect()
}
fn bounds(v: &[PbrVertex]) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for x in v {
        for k in 0..3 {
            lo[k] = lo[k].min(x.position[k]);
            hi[k] = hi[k].max(x.position[k]);
        }
    }
    (lo, hi)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialEnvelope {
    schema: u32,
    material: PbrMaterial,
}
fn engine_material(m: &gltf::Material) -> Result<Option<PbrMaterial>> {
    if let Some(raw) = m.extras() {
        let extra: serde_json::Value = serde_json::from_str(raw.get())?;
        if let Some(value) = extra.get("crysirror_material") {
            let e: MaterialEnvelope =
                serde_json::from_value(value.clone()).context("engine material extras")?;
            ensure!(
                e.schema == 1,
                "unsupported engine material schema {}",
                e.schema
            );
            e.material.validate()?;
            return Ok(Some(e.material));
        }
    }
    Ok(None)
}

/// Load every mesh, material and texture of a glTF/GLB document.
pub fn load_pbr(bytes: &[u8], external: impl FnMut(&str) -> Result<Vec<u8>>) -> Result<PbrModel> {
    load_pbr_with(bytes, external, true, None)
}
/// Inject the host's shared CPU image registry for external or embedded textures.
pub fn load_pbr_with_decoder(bytes:&[u8],external:impl FnMut(&str)->Result<Vec<u8>>,mut decoder:impl FnMut(&str,&[u8])->Result<crysirror_image::ImageSurface>)->Result<PbrModel>{
    load_pbr_with(bytes,external,true,Some(&mut decoder))
}
/// Geometry, nodes, skins and animations only (no texture decoding), e.g.
/// for physics colliders.
pub fn load_geometry(
    bytes: &[u8],
    external: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<PbrModel> {
    load_pbr_with(bytes, external, false, None)
}
fn load_pbr_with(
    bytes: &[u8],
    mut external: impl FnMut(&str) -> Result<Vec<u8>>,
    decode_images: bool,
    mut decoder:Option<&mut dyn FnMut(&str,&[u8])->Result<crysirror_image::ImageSurface>>,
) -> Result<PbrModel> {
    ensure!(
        bytes.len() <= crate::MAX_IMPORT_BYTES,
        "glTF document exceeds 64 MiB"
    );
    let converted = crate::interchange::to_gltf(bytes, &mut external)?;
    let doc = gltf::Gltf::from_slice(&converted).context("parse glTF")?;
    let mut buffers = Vec::new();
    for b in doc.buffers() {
        buffers.push(match b.source() {
            gltf::buffer::Source::Bin => doc.blob.clone().context("GLB BIN chunk missing")?,
            gltf::buffer::Source::Uri(u) if u.starts_with("data:") => {
                bail!("data-URI buffers unsupported")
            }
            gltf::buffer::Source::Uri(u) => external(u)?,
        });
    }
    let mut model = PbrModel::default();
    // Which images are color (sRGB) is decided by material usage.
    let mut srgb = vec![false; doc.images().len()];
    for m in doc.materials() {
        let p = m.pbr_metallic_roughness();
        if let Some(extra) = engine_material(&m)? {
            for index in extra.textures().into_iter().flatten() {
                ensure!(
                    *index < srgb.len(),
                    "engine material image index outside model"
                );
            }
            for i in [extra.base_color_texture, extra.emissive_texture, extra.specular_texture]
                .into_iter()
                .flatten()
            {
                srgb[i] = true;
            }
        }
        for t in [
            p.base_color_texture().map(|t| t.texture()),
            m.emissive_texture().map(|t| t.texture()),
        ]
        .into_iter()
        .flatten()
        {
            srgb[t.source().index()] = true;
        }
    }
    for (i, img) in doc.images().enumerate() {
        if !decode_images {
            break;
        }
        let name=match img.source(){gltf::image::Source::Uri{uri,..}=>uri,_=>"embedded.png"};
        let data = match img.source() {
            gltf::image::Source::View { view, .. } => {
                let buf = &buffers[view.buffer().index()];
                buf.get(view.offset()..view.offset() + view.length())
                    .context("image view out of range")?
                    .to_vec()
            }
            gltf::image::Source::Uri { uri, .. } if uri.starts_with("data:") => {
                bail!("data-URI images unsupported")
            }
            gltf::image::Source::Uri { uri, .. } => external(uri)?,
        };
        let image=if let Some(ref mut decoder)=decoder {
            let surface=decoder(name,&data)?;surface.validate()?;
            MipImage{width:surface.width,height:surface.height,levels:mip_chain(surface.width,surface.height,surface.rgba,srgb[i]),srgb:srgb[i]}
        }else{decode_image(&data,srgb[i])?};
        model.images.push(image);
    }
    let tex = |t: Option<gltf::texture::Texture>| t.map(|t| t.source().index());
    for m in doc.materials() {
        let p = m.pbr_metallic_roughness();
        let material = if let Some(extra) = engine_material(&m)? {
            extra
        } else {
            PbrMaterial {
                unlit: m.unlit(),
                base_color_factor: p.base_color_factor(),
                metallic_factor: p.metallic_factor(),
                roughness_factor: p.roughness_factor(),
                emissive_factor: m.emissive_factor(),
                emissive_strength: m.emissive_strength().unwrap_or(1.),
                normal_scale: m.normal_texture().map(|n| n.scale()).unwrap_or(1.),
                occlusion_strength: m.occlusion_texture().map(|o| o.strength()).unwrap_or(1.),
                alpha_mode: match m.alpha_mode() {
                    gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                    gltf::material::AlphaMode::Mask => AlphaMode::Mask,
                    gltf::material::AlphaMode::Blend => AlphaMode::Blend,
                },
                alpha_cutoff: m.alpha_cutoff().unwrap_or(0.5),
                double_sided: m.double_sided(),
                base_color_texture: tex(p.base_color_texture().map(|t| t.texture())),
                metallic_roughness_texture: tex(p
                    .metallic_roughness_texture()
                    .map(|t| t.texture())),
                normal_texture: tex(m.normal_texture().map(|t| t.texture())),
                occlusion_texture: tex(m.occlusion_texture().map(|t| t.texture())),
                emissive_texture: tex(m.emissive_texture().map(|t| t.texture())),
                ..Default::default()
            }
        };
        model.materials.push(material);
    }
    for m in &model.materials {
        m.validate()?;
    }
    let default_material = model.materials.len();
    model.materials.push(PbrMaterial::default());
    for mesh in doc.meshes() {
        let mut prims = Vec::new();
        // Morph target names: the `targetNames` extras convention, on the
        // mesh or (UniVRM) on its primitives.
        let names_of = |extras: &gltf::json::Extras| -> Option<Vec<String>> {
            let raw = extras.as_ref()?;
            serde_json::from_str::<serde_json::Value>(raw.get())
                .ok()?
                .get("targetNames")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
        };
        let target_names: Vec<String> = names_of(mesh.extras())
            .or_else(|| mesh.primitives().find_map(|p| names_of(p.extras())))
            .unwrap_or_default();
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                model.dropped.push(format!(
                    "mesh {} primitive {}: mode {:?}",
                    mesh.index(),
                    prim.index(),
                    prim.mode()
                ));
                continue;
            }
            let r = prim.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
            let pos: Vec<[f32; 3]> = r.read_positions().context("POSITION")?.collect();
            let indices: Vec<u32> = match r.read_indices() {
                Some(i) => i.into_u32().collect(),
                None => (0..pos.len() as u32).collect(),
            };
            ensure!(
                indices.iter().all(|i| (*i as usize) < pos.len()),
                "index out of range"
            );
            let normals = match r.read_normals() {
                Some(n) => n.collect::<Vec<_>>(),
                None => smooth_normals(&pos, &indices),
            };
            let uvs: Vec<[f32; 2]> = r
                .read_tex_coords(0)
                .map(|t| t.into_f32().collect())
                .unwrap_or_else(|| vec![[0.; 2]; pos.len()]);
            let tangents: Option<Vec<[f32; 4]>> = r.read_tangents().map(|t| t.collect());
            let mut vertices: Vec<PbrVertex> = (0..pos.len())
                .map(|i| PbrVertex {
                    position: pos[i],
                    normal: normals[i],
                    uv: uvs[i],
                    tangent: tangents.as_ref().map(|t| t[i]).unwrap_or([1., 0., 0., 1.]),
                })
                .collect();
            if tangents.is_none() {
                generate_tangents(&mut vertices, &indices);
            }
            let skin_attrs = match (r.read_joints(0), r.read_weights(0)) {
                (Some(j), Some(w)) => {
                    let j: Vec<[u16; 4]> = j.into_u16().collect();
                    let w: Vec<[f32; 4]> = w.into_f32().collect();
                    ensure!(
                        j.len() == pos.len() && w.len() == pos.len(),
                        "skin attribute count"
                    );
                    Some(
                        j.into_iter()
                            .zip(w)
                            .map(|(j, w)| {
                                let sum = w.iter().sum::<f32>().max(1e-6);
                                (j, w.map(|x| x / sum))
                            })
                            .collect(),
                    )
                }
                _ => None,
            };
            let bounds = bounds(&vertices);
            let mut morphs = Vec::new();
            for (ti, (p, n, _)) in r.read_morph_targets().enumerate() {
                ensure!(ti < MAX_MORPH_TARGETS, "primitive exceeds {MAX_MORPH_TARGETS} morph targets");
                let p: Vec<[f32; 3]> = p.map(|x| x.collect()).unwrap_or_default();
                let n: Vec<[f32; 3]> = n.map(|x| x.collect()).unwrap_or_default();
                ensure!(
                    (p.is_empty() || p.len() == pos.len()) && (n.is_empty() || n.len() == pos.len()),
                    "morph target attribute count"
                );
                let mut t = MorphTarget {
                    name: target_names
                        .get(ti)
                        .cloned()
                        .unwrap_or_else(|| format!("{}#{ti}", mesh.name().unwrap_or("mesh"))),
                    ..Default::default()
                };
                for i in 0..pos.len() {
                    let dp = p.get(i).copied().unwrap_or([0.; 3]);
                    let dn = n.get(i).copied().unwrap_or([0.; 3]);
                    if dp.iter().chain(&dn).any(|v| *v != 0.) {
                        ensure!(dp.iter().chain(&dn).all(|v| v.is_finite()), "non-finite morph delta");
                        t.indices.push(i as u32);
                        t.positions.push(dp);
                        t.normals.push(dn);
                    }
                }
                morphs.push(t);
            }
            prims.push(PbrPrimitive {
                skin_attrs,
                vertices,
                indices,
                material: prim.material().index().unwrap_or(default_material),
                bounds,
                morphs,
            });
        }
        model.meshes.push(prims);
    }
    for n in doc.nodes() {
        let (t, r, sc) = n.transform().decomposed();
        model.nodes.push(SceneNode {
            name: n.name().unwrap_or("").to_owned(),
            children: n.children().map(|c| c.index()).collect(),
            mesh: n.mesh().map(|m| m.index()),
            skin: n.skin().map(|s| s.index()),
            rest: Trs {
                translation: t.into(),
                rotation: glam::Quat::from_array(r).normalize(),
                scale: sc.into(),
            },
        });
    }
    model.roots = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .map(|s| s.nodes().map(|n| n.index()).collect())
        .unwrap_or_default();
    for skin in doc.skins() {
        let r = skin.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
        let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
        let inverse_bind = match r.read_inverse_bind_matrices() {
            Some(m) => m.map(|m| glam::Mat4::from_cols_array_2d(&m)).collect(),
            None => vec![glam::Mat4::IDENTITY; joints.len()],
        };
        ensure!(inverse_bind.len() == joints.len(), "skin matrices");
        ensure!(
            joints.len() <= crate::anim::MAX_JOINTS,
            "skin exceeds {} joints",
            crate::anim::MAX_JOINTS
        );
        model.skins.push(Skin {
            joints,
            inverse_bind,
        });
    }
    for a in doc.animations() {
        let mut channels = Vec::new();
        let mut duration = 0f32;
        for c in a.channels() {
            let r = c.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
            let times: Vec<f32> = r.read_inputs().context("animation input")?.collect();
            use gltf::animation::util::ReadOutputs as O;
            let (path, values): (ChannelPath, Vec<[f32; 4]>) =
                match r.read_outputs().context("animation output")? {
                    O::Translations(t) => (
                        ChannelPath::Translation,
                        t.map(|v| [v[0], v[1], v[2], 0.]).collect(),
                    ),
                    O::Scales(t) => (
                        ChannelPath::Scale,
                        t.map(|v| [v[0], v[1], v[2], 0.]).collect(),
                    ),
                    O::Rotations(q) => (ChannelPath::Rotation, q.into_f32().collect()),
                    O::MorphTargetWeights(_) => {
                        model
                            .dropped
                            .push(format!("animation {:?}: morph weights", a.name()));
                        continue;
                    }
                };
            let interpolation = match c.sampler().interpolation() {
                gltf::animation::Interpolation::Linear => Interpolation::Linear,
                gltf::animation::Interpolation::Step => Interpolation::Step,
                gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
            };
            let per_key = if interpolation == Interpolation::CubicSpline {
                3
            } else {
                1
            };
            ensure!(
                !times.is_empty() && values.len() == times.len() * per_key,
                "animation key count"
            );
            duration = duration.max(*times.last().unwrap());
            channels.push(Channel {
                node: c.target().node().index(),
                path,
                interpolation,
                times,
                values,
            });
        }
        model.animations.push(Clip {
            name: a
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("clip{}", a.index())),
            duration,
            channels,
        });
    }
    Ok(model)
}

/// Linear RGB float pixels of an equirectangular Radiance `.hdr` image.
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<[f32; 3]>,
}
pub fn load_hdr(bytes: &[u8]) -> Result<HdrImage> {
    let decoder = image::codecs::hdr::HdrDecoder::new(std::io::Cursor::new(bytes))
        .context("Radiance HDR header")?;
    let meta = decoder.metadata();
    ensure!(meta.width <= 16384 && meta.height <= 8192, "HDR too large");
    ensure!(
        meta.width == meta.height * 2,
        "environment must be 2:1 equirectangular"
    );
    let px = decoder.read_image_hdr().context("decode HDR")?;
    Ok(HdrImage {
        width: meta.width,
        height: meta.height,
        rgb: px.into_iter().map(|p| p.0).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mip_chain_reaches_one_pixel_and_averages_in_linear_space() {
        let rgba = [0u8, 0, 0, 255, 255, 255, 255, 255].repeat(2);
        let levels = mip_chain(2, 2, rgba, true);
        assert_eq!(levels.len(), 2);
        // Linear mean of black/white is 0.5 -> sRGB 188, not 128.
        assert_eq!(&levels[1][..4], &[188, 188, 188, 255]);
        let lin = mip_chain(2, 2, [0u8, 0, 0, 0, 255, 255, 255, 255].repeat(2), false);
        assert_eq!(&lin[1][..4], &[128, 128, 128, 128]);
    }
    #[test]
    fn flat_primitive_has_unit_normals_and_orthogonal_tangents() {
        let p = flat_primitive(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]], &[0, 1, 2]);
        assert_eq!(p.vertices[0].normal, [0., 0., 1.]);
        let t = glam::Vec3::from_slice(&p.vertices[0].tangent[..3]);
        assert!(t.dot(glam::Vec3::Z).abs() < 1e-6 && (t.length() - 1.).abs() < 1e-5);
    }
    #[test]
    fn hdr_rejects_non_equirect_and_garbage() {
        assert!(load_hdr(b"not hdr").is_err());
    }
}

#[cfg(test)]
mod material_contract_tests {
    use super::*;
    #[test]
    fn gltf_blend_mask_and_unlit_keep_their_distinct_semantics() {
        let bytes=br#"{"asset":{"version":"2.0"},"extensionsUsed":["KHR_materials_unlit"],"materials":[{"alphaMode":"BLEND","pbrMetallicRoughness":{"baseColorFactor":[1,0,0,0.25]},"extensions":{"KHR_materials_unlit":{}}},{"alphaMode":"MASK","alphaCutoff":0.3},{"alphaMode":"OPAQUE"}]}"#;
        let m = load_pbr(bytes, |_| anyhow::bail!("unexpected external resource")).unwrap();
        assert_eq!(m.materials[0].alpha_mode, AlphaMode::Blend);
        assert_eq!(m.materials[0].base_color_factor[3], 0.25);
        assert!(m.materials[0].unlit);
        assert_eq!(m.materials[1].alpha_mode, AlphaMode::Mask);
        assert_eq!(m.materials[1].alpha_cutoff, 0.3);
        assert_eq!(m.materials[2].alpha_mode, AlphaMode::Opaque);
        assert!(m.dropped.is_empty());
    }
}

#[cfg(test)]
mod engine_material_extras_tests {
    use super::*;
    #[test]
    fn extras_share_validation_and_keep_opacity_linear() {
        let json = serde_json::json!({"asset":{"version":"2.0"},"images":[{"uri":"mask.png"}],"materials":[{"extras":{"crysirror_material":{"schema":1,"material":{"alpha_mode":"blend","opacity_texture":0,"depth_fade":4.0,"unlit":true}}}}]});
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([128, 128, 128, 255]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
        let model = load_pbr(&serde_json::to_vec(&json).unwrap(), |path| {
            assert_eq!(path, "mask.png");
            Ok(png.clone())
        })
        .unwrap();
        assert_eq!(model.materials[0].depth_fade, 4.0);
        assert_eq!(model.materials[0].opacity_texture, Some(0));
        assert!(model.materials[0].unlit);
        assert!(!model.images[0].srgb);
        for (pointer, value) in [
            (
                "/materials/0/extras/crysirror_material/schema",
                serde_json::json!(2),
            ),
            (
                "/materials/0/extras/crysirror_material/material/depth_fade",
                serde_json::json!(-1),
            ),
            (
                "/materials/0/extras/crysirror_material/material/opacity_texture",
                serde_json::json!(1),
            ),
        ] {
            let mut bad = json.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(load_pbr(&serde_json::to_vec(&bad).unwrap(), |_| panic!(
                "invalid descriptor must fail before file access"
            ))
            .is_err());
        }
    }
}
