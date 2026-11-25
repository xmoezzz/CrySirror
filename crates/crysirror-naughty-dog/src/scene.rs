// SPDX-License-Identifier: MPL-2.0
//! A package's prototype meshes as one glTF document for the shared world
//! renderer: one glTF mesh per prototype (a primitive per fragment with its
//! group's base-colour texture), placed once per native instance. Mesh
//! units follow [`mesh::unit_of_grid`]; instance matrices are in metres.
use crate::{
    mesh::{self, Mesh},
    package::Directory,
    texture::VramDesc,
};
use anyhow::Result;
use crysirror_asset::interchange::glb::{Document, Material, Node, Primitive};
use crysirror_core::service::AssetReader;
use std::collections::HashMap;

/// A level start camera from `CAMERA_TABLE_1`, in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct StartCamera {
    pub name: String,
    pub position: [f32; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
    /// Horizontal and vertical field of view in radians.
    pub fov: [f32; 2],
    pub near: f32,
    pub far: f32,
}

/// An entity spawner with an art group (`SPAWNER_GROUP`): position and
/// rotation (xyzw) in metres, as laid out in icemesh's
/// `pak-entries/spawner-group.h` at `3e3025c`.
#[derive(Clone, Debug, PartialEq)]
pub struct Spawner {
    pub name: String,
    pub art_group: String,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
}

/// Any `SPAWNER_GROUP` spawner with its process type (+0x30) and schema
/// properties (+0x44: `{property array, count, 0}`; a property is
/// `{StringId, value}` and a value `{u16 type, u16 size, data}` with types
/// 1 integer, 4 float, 5 string (data is a text pointer) and 0xd
/// `StringId`; other types are skipped).
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnerRecord {
    pub name: String,
    pub art_group: Option<String>,
    pub process_type: u32,
    pub flags: u32,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub properties: Vec<(u32, crate::ss::Property)>,
    /// The parent spawner (+0x40) whose frame placed this one.
    pub parent: Option<String>,
}

fn text_at(p: &mesh::Loaded, at: usize) -> String {
    let bytes = p.data.get(at..).unwrap_or_default();
    let end = bytes.iter().take(256).position(|b| *b == 0).unwrap_or(0);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// A spawner's world placement: its locator composed with the parent
/// spawner's (+0x40) world placement, recursively.
fn spawner_world(p: &mesh::Loaded, s: usize, depth: u32) -> Result<Option<(glam::Vec3, glam::Quat)>> {
    let f = |k: usize| p.f32(s + 4 * k);
    let position = [f(0)?, f(1)?, f(2)?];
    let rotation = [f(4)?, f(5)?, f(6)?, f(7)?];
    if !position.iter().chain(&rotation).all(|v| v.is_finite()) {
        return Ok(None);
    }
    let (t, q) = (glam::Vec3::from_array(position), glam::Quat::from_array(rotation).normalize());
    match p.ptr(s + 0x40) {
        Ok(parent) if parent != s && depth < 16 => Ok(spawner_world(p, parent, depth + 1)?.map(|(pt, pq)| (pt + pq * t, (pq * q).normalize()))),
        _ => Ok(Some((t, q))),
    }
}

/// Every spawner of a `SPAWNER_GROUP` payload (`u32 0`, spawner pointer
/// table, count) with finite placement. Positions and rotations are in the
/// world: a spawner with a parent (+0x40) is placed in its parent's frame.
pub fn spawner_records(p: &mesh::Loaded, payload: usize) -> Result<Vec<SpawnerRecord>> {
    use crate::ss::Property;
    let count = p.u32(payload + 8)? as usize;
    anyhow::ensure!(count <= 65536, "ND spawner count");
    let mut out = Vec::new();
    if count == 0 {
        return Ok(out);
    }
    let table = p.ptr(payload + 4)?;
    for i in 0..count {
        let s = p.ptr(table + 4 * i)?;
        let Some((t, q)) = spawner_world(p, s, 0)? else { continue };
        let (position, rotation) = (t.to_array(), q.to_array());
        let mut properties = Vec::new();
        if let Ok(schema) = p.ptr(s + 0x44) {
            let n = p.u32(schema + 4)? as usize;
            anyhow::ensure!(n <= 4096, "ND spawner property count");
            if let Ok(props) = p.ptr(schema) {
                for k in 0..n {
                    let id = p.u32(props + 8 * k)?;
                    let Ok(v) = p.ptr(props + 8 * k + 4) else { continue };
                    let Ok(data) = p.ptr(v + 4) else { continue };
                    let value = match p.u16(v)? {
                        1 => Property::Int(p.u32(data)? as i32),
                        4 => Property::Float(p.f32(data)?),
                        // The string value holds a pointer to the text.
                        5 => match p.ptr(data) {
                            Ok(t) => Property::Str(text_at(p, t)),
                            Err(_) => continue,
                        },
                        0xd => Property::Sid(p.u32(data)?),
                        8 => Property::Vec4([p.f32(data)?, p.f32(data + 4)?, p.f32(data + 8)?, p.f32(data + 12)?]),
                        _ => continue,
                    };
                    properties.push((id, value));
                }
            }
        }
        out.push(SpawnerRecord {
            name: p.ptr(s + 0x20).map(|a| text_at(p, a)).unwrap_or_default(),
            art_group: p.ptr(s + 0x34).ok().map(|a| text_at(p, a)),
            process_type: p.u32(s + 0x30)?,
            flags: p.u32(s + 0x38)?,
            position,
            rotation,
            properties,
            parent: p.ptr(s + 0x40).ok().filter(|a| *a != s).and_then(|a| p.ptr(a + 0x20).ok()).map(|a| text_at(p, a)),
        });
    }
    Ok(out)
}

/// Spawners of a `SPAWNER_GROUP` payload that name an art group.
pub fn spawners(p: &mesh::Loaded, payload: usize) -> Result<Vec<Spawner>> {
    Ok(spawner_records(p, payload)?
        .into_iter()
        .filter_map(|s| Some(Spawner { art_group: s.art_group?, name: s.name, position: s.position, rotation: s.rotation }))
        .collect())
}

/// The level's directional light and ambient term from `LIGHT_TABLE_1`.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelLight {
    pub color: [f32; 3],
    /// Unit direction the light travels.
    pub direction: [f32; 3],
    pub intensity: f32,
    /// First RGB spherical-harmonic term of the table's probe record, when
    /// non-zero.
    pub ambient: Option<[f32; 3]>,
}

/// `LIGHT_TABLE_1`: the light record pointer at +0x28 holds colour,
/// position, direction, position, zero, range (`f32::MAX` for directional)
/// and intensity; the probe record at +0x30 holds a position followed by
/// RGB spherical-harmonic coefficients.
pub fn level_light(p: &mesh::Loaded, payload: usize) -> Result<Option<LevelLight>> {
    // Some installed tables (for example `museum-sewer-end`) carry no light
    // record.
    let Ok(record) = p.ptr(payload + 0x28) else { return Ok(None) };
    let f = |i: usize| p.f32(record + 4 * i);
    let color = [f(0)?, f(1)?, f(2)?];
    let direction = [f(6)?, f(7)?, f(8)?];
    let (range, intensity) = (f(13)?, f(14)?);
    let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
    if range < 1e30 || !(0.9..1.1).contains(&length) || !color.iter().chain([&intensity]).all(|v| v.is_finite() && *v >= 0.) {
        return Ok(None);
    }
    let ambient = match p.ptr(payload + 0x30) {
        Ok(probe) => {
            let sh = [p.f32(probe + 12)?, p.f32(probe + 16)?, p.f32(probe + 20)?];
            (sh.iter().all(|v| v.is_finite() && *v >= 0.) && sh.iter().any(|v| *v > 0.)).then_some(sh)
        }
        Err(_) => None,
    };
    Ok(Some(LevelLight { color, direction: direction.map(|v| v / length), intensity, ambient }))
}

/// The first camera of a `CAMERA_TABLE_1` payload: count, pointer to
/// records of a name pointer followed by position, forward, up, two field
/// of view angles, near and far.
pub fn start_camera(p: &mesh::Loaded, payload: usize) -> Result<Option<StartCamera>> {
    if p.u32(payload)? == 0 {
        return Ok(None);
    }
    let record = p.ptr(payload + 4)?;
    let name_at = p.ptr(record)?;
    let end = p.data[name_at..].iter().take(256).position(|b| *b == 0).unwrap_or(0);
    let f = |i: usize| p.f32(record + 4 + 4 * i);
    let c = StartCamera {
        name: String::from_utf8_lossy(&p.data[name_at..name_at + end]).into_owned(),
        position: [f(0)?, f(1)?, f(2)?],
        forward: [f(3)?, f(4)?, f(5)?],
        up: [f(6)?, f(7)?, f(8)?],
        fov: [f(9)?, f(10)?],
        near: f(11)?,
        far: f(12)?,
    };
    anyhow::ensure!(c.position.iter().chain(&c.forward).chain(&c.fov).all(|v| v.is_finite()), "ND camera values");
    Ok(Some(c))
}

/// Every render mesh of a package (collision collections excluded).
/// The prototype meshes of an already relocated package.
pub fn meshes_of(loaded: &mesh::Loaded, directory: &Directory) -> Result<Vec<Mesh>> {
    let mut meshes = Vec::new();
    for item in directory.items.iter().filter(|i| i.kind == "PM_COLLECTION_2") {
        meshes.extend(mesh::prototype_meshes(loaded, item.payload_offset as usize)?);
    }
    Ok(meshes)
}

pub fn package_meshes(source: &dyn AssetReader, directory: &Directory, limit: u64) -> Result<Vec<Mesh>> {
    let collections: Vec<_> = directory.items.iter().filter(|i| i.kind == "PM_COLLECTION_2").collect();
    if collections.is_empty() {
        return Ok(Vec::new());
    }
    let loaded = mesh::Loaded::read(source, limit)?;
    let mut meshes = Vec::new();
    for item in collections {
        meshes.extend(mesh::prototype_meshes(&loaded, item.payload_offset as usize)?);
    }
    Ok(meshes)
}

/// Art-group (actor) meshes carry no instance records: each is placed once
/// in the spawner's frame with an identity instance and no culling sphere.
/// Their decoded positions are already in the model frame (the crate
/// stands on the floor, the hero's feet are at y = 0).
pub fn place_art_group(meshes: &mut [Mesh]) {
    const IDENTITY: [f32; 16] = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
    for m in meshes.iter_mut().filter(|m| m.instances.is_empty()) {
        m.instances.push(IDENTITY);
        m.spheres.clear();
    }
}

/// The `vis_sphere` tag (`TAG_VEC4`) of an art-group package.
pub fn vis_sphere(loaded: &mesh::Loaded, directory: &Directory) -> Option<[f32; 4]> {
    let item = directory.items.iter().find(|i| i.kind == "TAG_VEC4" && i.name == "vis_sphere")?;
    let at = item.payload_offset as usize;
    let v: [f32; 4] = std::array::from_fn(|k| loaded.f32(at + 4 * k).unwrap_or(f32::NAN));
    v.iter().all(|x| x.is_finite()).then_some(v)
}

/// Axis-aligned bounds of all positions, in metres in the mesh frame.
pub fn bounds(meshes: &[Mesh]) -> Option<([f32; 3], [f32; 3])> {
    let mut points = meshes.iter().flat_map(|m| &m.fragments).flat_map(|f| &f.positions);
    let first = *points.next()?;
    Some(points.fold((first, first), |(lo, hi), p| (std::array::from_fn(|k| lo[k].min(p[k])), std::array::from_fn(|k| hi[k].max(p[k])))))
}

/// An instance's world matrix (row-major, metres). Decoded positions
/// already carry the prototype's fitted local origin.
pub fn placement(mesh: &Mesh, instance: usize) -> [f32; 16] {
    mesh.instances[instance]
}

/// World-space bounds in metres of every placed instance's mesh bounds.
pub fn placed_bounds(meshes: &[Mesh]) -> Option<([f32; 3], [f32; 3])> {
    let mut out: Option<([f32; 3], [f32; 3])> = None;
    for mesh in meshes {
        let Some((lo, hi)) = bounds(std::slice::from_ref(mesh)) else { continue };
        for i in 0..mesh.instances.len() {
            let m = placement(mesh, i);
            for corner in 0..8 {
                let p: [f32; 3] = std::array::from_fn(|k| if corner >> k & 1 == 0 { lo[k] } else { hi[k] });
                let w: [f32; 3] = std::array::from_fn(|c| p[0] * m[c] + p[1] * m[4 + c] + p[2] * m[8 + c] + m[12 + c]);
                out = Some(match out {
                    None => (w, w),
                    Some((a, b)) => (std::array::from_fn(|k| a[k].min(w[k])), std::array::from_fn(|k| b[k].max(w[k]))),
                });
            }
        }
    }
    out
}

/// Translation, rotation (xyzw) and scale of a row-major matrix.
fn trs(m: &[f32; 16]) -> ([f32; 3], [f32; 4], [f32; 3]) {
    // Row-major with row vectors equals glam's column-major layout.
    let (s, r, t) = glam::Mat4::from_cols_array(m).to_scale_rotation_translation();
    (t.to_array(), r.to_array(), s.to_array())
}

/// A skeleton and its decoded clips for skinned art groups.
pub struct Rig<'a> {
    pub skeleton: &'a crate::skeleton::Skeleton,
    pub clips: &'a [crate::anim::Clip],
}

/// Build the document. Textures decode from the package's VRAM data;
/// `max_texture` halves oversized base-colour images by box filtering.
pub fn document(name: &str, meshes: &[Mesh], source: &dyn AssetReader, directory: &Directory, max_texture: u32) -> Result<Document> {
    document_lit(name, meshes, source, directory, max_texture, None, None)
}

/// Up to four influences of a vertex, renormalised; joints outside the
/// skeleton are dropped.
fn influences(v: &[(u16, f32)], joints: usize) -> ([u16; 4], [f32; 4]) {
    let mut list: Vec<(u16, f32)> = v.iter().copied().filter(|(j, w)| usize::from(*j) < joints && *w > 0.).collect();
    list.sort_by(|a, b| b.1.total_cmp(&a.1));
    list.truncate(4);
    let total: f32 = list.iter().map(|x| x.1).sum();
    let (mut j, mut w) = ([0u16; 4], [0f32; 4]);
    for (k, (jj, ww)) in list.iter().enumerate() {
        j[k] = *jj;
        w[k] = if total > 0. { ww / total } else { 0. };
    }
    if total <= 0. {
        w[0] = 1.;
    }
    (j, w)
}

/// Texture role from the source file name stored in `VRAM_DESC`
/// (`…/name-color.tga/….ndb`): 2 colour, 0 not a colour map (normal, mask,
/// blend, specular, reflection, translucency, lightmap, missing texture),
/// 1 unknown.
fn texture_role(name: &str) -> u8 {
    let file = name.rsplit('/').find(|p| p.contains('.')).unwrap_or(name).to_ascii_lowercase();
    let stem = file.split('.').next().unwrap_or("");
    if name.contains("missingtexture") || name.contains("indirect") || ["normal", "mask", "blend", "spec", "reflection", "trans", "lightmap", "gloss", "bump"].iter().any(|r| stem.contains(r)) {
        0
    } else if ["color", "diffuse", "albedo"].iter().any(|r| stem.contains(r)) {
        2
    } else {
        1
    }
}

/// Albedo of the snow the `cliffwall-snow` fragment programs compute:
/// about 0.75 in the programs' gamma-space arithmetic (0.5 plus three
/// saturated terms), 0.55 linear (approximated).
const COMPUTED_SNOW: f32 = 0.55;

/// [`document`] with the level's baked probes: meshes placed once get
/// per-vertex irradiance in `_AUX` and the `baked_vertex_irradiance` effect.
pub fn document_lit(name: &str, meshes: &[Mesh], source: &dyn AssetReader, directory: &Directory, max_texture: u32, probes: Option<&crate::probes::ProbeField>, rig: Option<&Rig>) -> Result<Document> {
    let mut skinned_nodes = Vec::new();
    let mut doc = Document { generator: "CrySirror Naughty Dog background importer".into(), ..Default::default() };
    let mut textures = HashMap::new();
    for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
        let desc = VramDesc::read(source, item.payload_offset)?;
        textures.insert(desc.id, desc);
    }
    let mut images: HashMap<u32, usize> = HashMap::new();
    // Placeholder textures hold one colour in every texel; textures are
    // decoded for this test only when they are base-colour candidates.
    let uniform: std::cell::RefCell<HashMap<u32, bool>> = Default::default();
    for (index, mesh) in meshes.iter().enumerate() {
        let mut primitives = Vec::new();
        for group in &mesh.groups {
            let tiny = |id: u32| {
                if let Some(v) = uniform.borrow().get(&id) {
                    return *v;
                }
                let flat = match textures.get(&id) {
                    None => true,
                    // Baked lighting (`indirect.rgbmax`, `…rgbmaxdelta`).
                    Some(d) if d.name.contains("indirect") => true,
                    // Small textures are checked; larger ones are content.
                    Some(d) if d.width * d.height > 64 * 64 => false,
                    Some(d) => d.decode(source, directory.texture_base).map_or(true, |s| s.rgba.chunks_exact(4).all(|t| t == &s.rgba[..4])),
                };
                uniform.borrow_mut().insert(id, flat);
                flat
            };
            // A sampler named as a colour map wins; otherwise the first
            // sampler that is neither a non-colour map nor a placeholder.
            // Pass 0 of some snow materials binds no colour map; their
            // single-texture pass (5) holds it.
            let role = |id: u32| textures.get(&id).map_or(0, |d: &VramDesc| texture_role(&d.name));
            let colour = |samplers: &[u32]| samplers.iter().copied().find(|id| role(*id) == 2 && !tiny(*id));
            let layers = group.material.layers;
            let sampler = |i: u8| group.material.samplers.get(usize::from(i)).copied().filter(|id| !tiny(*id));
            let chosen = match layers {
                Some(s) => sampler(s.side),
                None => colour(&group.material.samplers).or_else(|| group.material.base_color_with(|id| role(id) == 0 || tiny(id))),
            }
            .or_else(|| group.material.passes.iter().skip(1).find_map(|p| colour(p)));
            let mut image_of = |id: Option<u32>| -> Result<Option<usize>> {
                let Some(desc) = id.and_then(|id| textures.get(&id)) else { return Ok(None) };
                if let Some(&i) = images.get(&desc.id) {
                    return Ok(Some(i));
                }
                let mut surface = desc.decode(source, directory.texture_base)?;
                while surface.width > max_texture.max(1) || surface.height > max_texture.max(1) {
                    surface = halve(&surface);
                }
                doc.images.push((crysirror_image::encode_png(&surface)?, "image/png"));
                images.insert(desc.id, doc.images.len() - 1);
                Ok(Some(doc.images.len() - 1))
            };
            let image = image_of(chosen)?;
            // The upper layer: a sampler, or the light grey the
            // `cliffwall-snow` programs compute (0.5 plus lighting terms;
            // approximated, SATD-nd-retail-runtime); a height-mask blend
            // also binds its mask.
            let layer = match layers {
                Some(l) => {
                    let (top, tint) = match l.top {
                        crate::rsx::Top::Sampler(i) => (image_of(sampler(i))?, [1.; 3]),
                        // `g_cDiffuseIn`, the computed snow's colour (gamma
                        // space), when the material sets it.
                        crate::rsx::Top::Computed => (None, group.material.parameter("g_cDiffuseIn").map_or([COMPUTED_SNOW; 3], |c| [c[0], c[1], c[2]].map(|v| v.clamp(0., 1.).powf(2.2)))),
                    };
                    let mask = match l.weight {
                        // The material's `g_fBlendSoftness` replaces the
                        // program's inline default.
                        crate::rsx::Weight::Mask { sampler: m, softness } => Some((image_of(group.material.samplers.get(usize::from(m)).copied())?, group.material.parameter("g_fBlendSoftness_fBlendOffset").map_or(softness, |v| v[0].max(1e-3)))),
                        crate::rsx::Weight::Slope => None,
                    };
                    Some(crate::probes::Layer { top, tint, mask })
                }
                None => None,
            };
            // Baked lighting applies to meshes placed once (per-vertex data
            // cannot differ between instances of one glTF mesh).
            let bake = probes.filter(|p| !p.is_empty() && mesh.instances.len() == 1);
            let mut material = Material { name: group.material.name.clone(), base_color_texture: image, ..Default::default() };
            let mut engine = if bake.is_some() {
                crate::probes::baked_material(image, layer.as_ref())
            } else if let Some(l) = layer {
                // Without per-vertex data a height-mask blend falls back to
                // the slope layering.
                crysirror_asset::pbr::PbrMaterial {
                    base_color_texture: image,
                    emissive_texture: l.top,
                    effect: Some(crysirror_scene::effect::ShaderEffect::SlopeLayerBlend { tint: l.tint }),
                    ..Default::default()
                }
            } else {
                crysirror_asset::pbr::PbrMaterial { base_color_texture: image, ..Default::default() }
            };
            // The native passes shade diffuse light (pre-pass and probes)
            // and add specular only through a `…-spec-mask` map: without
            // one the surface reflects nothing. The first `…-normal` map of
            // the passes perturbs the normal (DXT5 maps hold X in alpha and
            // Y in green).
            // `default-…` maps are the shared flat placeholders. Water
            // binds an environment reflection map (`…-env-reflection`) that
            // its program adds at 0.6 along the reflection vector: here a
            // smooth specular reflects the environment instead.
            let named = |role: &str| {
                group.material.passes.iter().flatten().copied().find(|id| {
                    textures.get(id).is_some_and(|d: &VramDesc| {
                        let file = d.name.rsplit('/').find(|p| p.contains('.')).unwrap_or(&d.name).to_ascii_lowercase();
                        file.contains(role) && !file.starts_with("default-")
                    })
                })
            };
            let normal = named("-normal");
            let spec = named("spec");
            let reflective = named("env-reflection").is_some();
            // `g_fSpecularScale_fSpecularExponentMax_fMetalness` scales the
            // spec-mask highlight; the Blinn exponent maps to roughness
            // `√(2 / (n + 2))` (kept above 0.25: the mask modulates it).
            let specular = group.material.parameter("g_fSpecularScale_fSpecularExponentMax_fMetalness");
            engine.metallic_factor = specular.map_or(0., |v| v[2].clamp(0., 1.));
            engine.roughness_factor = if reflective {
                0.12
            } else {
                match (spec, specular) {
                    (Some(_), Some(v)) => (2. / (v[1].max(1.) + 2.)).sqrt().max(0.25),
                    (Some(_), None) => 0.55,
                    _ => 1.,
                }
            };
            engine.specular_factor = match (spec.is_some() || reflective, specular) {
                (true, Some(v)) => v[0].clamp(0., 1.),
                (true, None) => 1.,
                _ => 0.,
            };
            engine.specular_texture = image_of(spec)?;
            if let Some(id) = normal {
                engine.normal_encoding = if textures.get(&id).is_some_and(|d| d.format == crate::texture::Format::Dxt5) { crysirror_scene::material::NormalEncoding::Ag } else { crysirror_scene::material::NormalEncoding::Rgb };
                engine.normal_texture = image_of(Some(id))?;
            }
            material.engine = Some(engine);
            doc.materials.push(material);
            let material = doc.materials.len() - 1;
            for f in &mesh.fragments[group.fragments.clone()] {
                if f.positions.is_empty() || f.triangles.is_empty() {
                    continue;
                }
                let normals = if f.normals.len() == f.positions.len() { f.normals.clone() } else { Vec::new() };
                let (joints, weights): (Vec<[u16; 4]>, Vec<[f32; 4]>) = match rig {
                    Some(r) if f.skin.len() == f.positions.len() => f.skin.iter().map(|v| influences(v, r.skeleton.names.len())).unzip(),
                    _ => (Vec::new(), Vec::new()),
                };
                let mut aux = match bake {
                    Some(field) => crate::probes::bake(field, &placement(mesh, 0), &f.positions, &normals, &f.triangles),
                    None => Vec::new(),
                };
                // A height-mask blend reads the vertex mask (attribute
                // 0x15 x) in `_AUX` w.
                if group.material.layers.is_some_and(|l| matches!(l.weight, crate::rsx::Weight::Mask { .. })) && f.masks.len() == aux.len() {
                    for (a, m) in aux.iter_mut().zip(&f.masks) {
                        a[3] = m[0];
                    }
                }
                primitives.push(Primitive {
                    positions: f.positions.clone(),
                    normals,
                    uvs: if f.uvs.len() == f.positions.len() { f.uvs.clone() } else { Vec::new() },
                    indices: f.triangles.iter().flatten().map(|&i| u32::from(i)).collect(),
                    material: Some(material),
                    aux,
                    joints,
                    weights,
                    ..Default::default()
                });
            }
        }
        if primitives.is_empty() || mesh.instances.is_empty() {
            continue;
        }
        doc.meshes.push((format!("{name}/{index}"), primitives));
        let mesh_index = doc.meshes.len() - 1;
        for i in 0..mesh.instances.len() {
            let (translation, rotation, scale) = trs(&placement(mesh, i));
            let mut geometry = Node::named(format!("{name}/{index}/{i}/geometry"));
            geometry.mesh = Some(mesh_index);
            if rig.is_some() && mesh.skinned {
                skinned_nodes.push(doc.nodes.len());
            }
            doc.nodes.push(geometry);
            let mut placed = Node::named(format!("{name}/{index}/{i}"));
            placed.translation = translation;
            placed.rotation = rotation;
            placed.scale = scale;
            placed.children = vec![doc.nodes.len() - 1];
            doc.roots.push(doc.nodes.len());
            doc.nodes.push(placed);
        }
    }
    if let Some(rig) = rig.filter(|_| !skinned_nodes.is_empty()) {
        add_rig(&mut doc, rig, &skinned_nodes);
    }
    Ok(doc)
}

/// An animation library: the rig's joint nodes and its clips, no meshes.
/// Entities bind its clips by joint name (`AnimationLayer::source`).
pub fn animation_library(rig: &Rig) -> Document {
    let mut doc = Document::default();
    add_rig(&mut doc, rig, &[]);
    doc
}

/// Joint nodes in their rest pose, a skin for the skinned mesh nodes and
/// one glTF animation per clip (keys at the clip's frame times; channels a
/// clip leaves undefined keep the rest pose).
fn add_rig(doc: &mut Document, rig: &Rig, skinned: &[usize]) {
    use crysirror_asset::interchange::glb::{Animation, Channel, Skin, Track};
    let skel = rig.skeleton;
    let base = doc.nodes.len();
    for (j, name) in skel.names.iter().enumerate() {
        let (t, r, s) = skel.rest[j];
        let mut node = Node::named(name.clone());
        node.translation = t.to_array();
        node.rotation = r.to_array();
        node.scale = s.to_array();
        doc.nodes.push(node);
    }
    for (j, parent) in skel.parents.iter().enumerate() {
        match parent {
            Some(p) => doc.nodes[base + p].children.push(base + j),
            None => doc.roots.push(base + j),
        }
    }
    doc.skins.push(Skin { joints: (base..base + skel.names.len()).collect(), inverse_bind: skel.inverse_bind.clone(), skeleton: None });
    let skin = doc.skins.len() - 1;
    for &n in skinned {
        doc.nodes[n].skin = Some(skin);
    }
    for clip in rig.clips {
        let times: Vec<f32> = (0..clip.frames).map(|f| f as f32 / clip.fps.max(1.)).collect();
        let mut tracks = Vec::new();
        for (j, track) in clip.joints.iter().enumerate().filter(|(j, _)| *j < skel.names.len()) {
            if let Some(v) = &track.translation {
                tracks.push(Track { node: base + j, times: times.clone(), values: Channel::Translation(v.iter().map(|x| x.to_array()).collect()), tangents: None });
            }
            if let Some(v) = &track.rotation {
                tracks.push(Track { node: base + j, times: times.clone(), values: Channel::Rotation(v.iter().map(|x| x.to_array()).collect()), tangents: None });
            }
            if let Some(v) = &track.scale {
                tracks.push(Track { node: base + j, times: times.clone(), values: Channel::Scale(v.iter().map(|x| x.to_array()).collect()), tangents: None });
            }
        }
        if !tracks.is_empty() {
            doc.animations.push(Animation { name: clip.name.clone(), tracks });
        }
    }
}

/// The sky image of a `sky-…` level package: sampler 0 of its mesh's
/// first material, when it is a 2:1 image. The sky's main-pass fragment
/// program (`sky-train-wreck` shader 0) samples that unit by view direction
/// with `u = atan2(z, x) / 2π` (an atan polynomial and quadrant fix-ups) and
/// `v = acos(y) · 2/π`, a hemispherical latitude–longitude map with the
/// horizon on the bottom row; it then blends two scrolling cloud layers
/// (units 1–4, not reproduced). `None` when the package holds no such
/// image (the mesh is then drawn as geometry).
pub fn sky_texture(meshes: &[Mesh], source: &dyn AssetReader, directory: &Directory) -> Result<Option<crysirror_image::ImageSurface>> {
    let Some(id) = meshes.first().and_then(|m| m.groups.first()).and_then(|g| g.material.samplers.first().copied()) else { return Ok(None) };
    for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
        let desc = VramDesc::read(source, item.payload_offset)?;
        if desc.id == id {
            return Ok((desc.width == 2 * desc.height).then(|| desc.decode(source, directory.texture_base)).transpose()?);
        }
    }
    Ok(None)
}

fn halve(s: &crysirror_image::ImageSurface) -> crysirror_image::ImageSurface {
    let (w, h) = ((s.width / 2).max(1), (s.height / 2).max(1));
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let mut sum = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = ((2 * x + dx).min(s.width - 1), (2 * y + dy).min(s.height - 1));
                    sum += u32::from(s.rgba[((sy * s.width + sx) * 4 + c) as usize]);
                }
                rgba[((y * w + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
            }
        }
    }
    crysirror_image::ImageSurface { width: w, height: h, rgba }
}
