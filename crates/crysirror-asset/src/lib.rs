// SPDX-License-Identifier: MPL-2.0
//! Asset importers that produce validated, device-free world data (ADR 0007).
//!
//! glTF 2.0 is the interchange format for Blender and other DCC tools. Blender
//! `.blend` files are not parsed directly: they are converted at authoring time
//! by Blender itself (tools/blender_export_gltf.py), because the `.blend` layout
//! is an undocumented, version-specific dump of Blender's memory structures.
#![forbid(unsafe_code)]
pub mod anim;
pub mod cluster;
pub mod indices;
pub mod pbr;
pub mod terrain;
use anyhow::{bail, ensure, Context, Result};
use crysirror_scene::{
    model::Geometry,
    world::{EntitySpec, Transform},
};
use glam::{EulerRot, Quat};
use serde::Serialize;

pub const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ENTITIES: usize = 4096;
pub const MAX_TRIANGLES: usize = 1_000_000 / 3;

#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// First world-local entity id; nodes and primitives are numbered upward.
    pub first_id: u32,
    /// Parent for the model's root nodes, e.g. an existing anchor entity.
    pub parent: Option<u32>,
    /// Prefix added to every entity name so several imports stay distinct.
    pub name_prefix: String,
    /// When set, meshes become `Geometry::Model { asset, mesh }` references
    /// drawn with full PBR materials, instead of baked untextured triangles.
    pub reference_asset: Option<String>,
}
impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            first_id: 1,
            parent: None,
            name_prefix: String::new(),
            reference_asset: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ImportReport {
    pub nodes: usize,
    pub primitives: usize,
    pub triangles: usize,
    /// Source features the current engine cannot represent. They are reported,
    /// never silently claimed as imported.
    pub dropped: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ImportedModel {
    pub entities: Vec<EntitySpec>,
    pub report: ImportReport,
}

fn linear_to_srgb_u8(c: f32) -> u8 {
    let c = c.clamp(0., 1.);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    };
    (s * 255. + 0.5) as u8
}

/// Import a `.glb` or `.gltf` document. `external` resolves non-embedded buffer
/// URIs relative to the document (through the caller's VFS); data URIs are
/// rejected because the engine does not carry a base64 importer on this path.
pub fn import_gltf(
    bytes: &[u8],
    options: &ImportOptions,
    mut external: impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<ImportedModel> {
    ensure!(
        bytes.len() <= MAX_IMPORT_BYTES,
        "glTF document exceeds 64 MiB"
    );
    let doc = gltf::Gltf::from_slice(bytes).context("parse glTF")?;
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    let mut total = 0usize;
    for buffer in doc.buffers() {
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => doc.blob.clone().context("GLB BIN chunk missing")?,
            gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => {
                bail!("embedded data-URI buffers are unsupported; export as .glb")
            }
            gltf::buffer::Source::Uri(uri) => {
                external(uri).with_context(|| format!("glTF external buffer '{uri}'"))?
            }
        };
        ensure!(
            data.len() >= buffer.length(),
            "glTF buffer shorter than declared"
        );
        total = total.saturating_add(data.len());
        ensure!(total <= MAX_IMPORT_BYTES, "glTF buffers exceed 64 MiB");
        buffers.push(data);
    }
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .context("glTF has no scene")?;

    let mut report = ImportReport::default();
    if doc.textures().len() > 0 && options.reference_asset.is_none() {
        report.dropped.push(format!(
            "{} texture(s): engine meshes are untextured",
            doc.textures().len()
        ));
    }
    if doc.animations().len() > 0 {
        report.dropped.push(format!(
            "{} animation(s): skeletal/keyframe import pending",
            doc.animations().len()
        ));
    }
    if doc.skins().len() > 0 {
        report
            .dropped
            .push(format!("{} skin(s)", doc.skins().len()));
    }
    if doc.cameras().len() > 0 {
        report
            .dropped
            .push(format!("{} camera(s)", doc.cameras().len()));
    }

    let mut entities = Vec::new();
    let mut next = options.first_id;
    let mut stack: Vec<(gltf::Node, Option<u32>)> = scene
        .nodes()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|n| (n, options.parent))
        .collect();
    while let Some((node, parent)) = stack.pop() {
        ensure!(entities.len() < MAX_ENTITIES, "glTF entity limit");
        let id = next;
        next = next.checked_add(1).context("entity id overflow")?;
        report.nodes += 1;
        let (t, r, s) = node.transform().decomposed();
        let (rx, ry, rz) = Quat::from_array(r).normalize().to_euler(EulerRot::XYZ);
        let transform = Transform {
            translation: t,
            rotation_degrees: [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
            scale: s,
        };
        transform
            .validate()
            .with_context(|| format!("glTF node {} transform", node.index()))?;
        let name = format!(
            "{}{}",
            options.name_prefix,
            node.name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("node{}", node.index()))
        );
        let mut node_entity = EntitySpec {
            id,
            name: name.clone(),
            parent,
            transform,
            geometry: None,
            color: [255; 4],
            visible: true,
            velocity: [0.; 3],
            angular_velocity: [0.; 3],
            tweens: vec![],
            material: None,
            animation: None,
            collider: None,
            gameplay: None,
            ai: None,
            particles: None,
            light: None,
            reflection_probe: None,
            irradiance_volume: None,
        };
        if let Some(light) = node.light() {
            node_entity.light = punctual_light(&light, &mut report);
        }
        let mut children = Vec::new();
        if let (Some(mesh), Some(asset)) = (node.mesh(), &options.reference_asset) {
            node_entity.geometry = Some(Geometry::Model {
                asset: asset.clone(),
                mesh: mesh.index() as u32,
            });
            report.primitives += mesh.primitives().len();
        } else if let Some(mesh) = node.mesh() {
            let prims: Vec<_> = mesh.primitives().collect();
            for (pi, prim) in prims.iter().enumerate() {
                let (geometry, color) = primitive(prim, &buffers, &mut report)?;
                if prims.len() == 1 {
                    node_entity.geometry = Some(geometry);
                    node_entity.color = color;
                } else {
                    ensure!(
                        entities.len() + children.len() < MAX_ENTITIES,
                        "glTF entity limit"
                    );
                    children.push(EntitySpec {
                        id: next,
                        name: format!("{name}/primitive{pi}"),
                        parent: Some(id),
                        transform: Transform::identity(),
                        geometry: Some(geometry),
                        color,
                        visible: true,
                        velocity: [0.; 3],
                        angular_velocity: [0.; 3],
                        tweens: vec![],
                        material: None,
                        animation: None,
                        collider: None,
                        gameplay: None,
                        ai: None,
                        particles: None,
                        light: None,
                        reflection_probe: None,
                        irradiance_volume: None,
                    });
                    next = next.checked_add(1).context("entity id overflow")?;
                }
            }
        }
        entities.push(node_entity);
        entities.extend(children);
        for child in node.children().collect::<Vec<_>>().into_iter().rev() {
            stack.push((child, Some(id)));
        }
    }
    ensure!(!entities.is_empty(), "glTF scene has no nodes");
    Ok(ImportedModel { entities, report })
}

/// KHR_lights_punctual → engine light. Point/spot intensity is candela in both;
/// glTF spots also emit along local -Z. A directional light is lux on a
/// single global sun, so it is reported instead of being mapped.
fn punctual_light(
    light: &gltf::khr_lights_punctual::Light,
    report: &mut ImportReport,
) -> Option<crysirror_scene::components::LightSpec> {
    use crysirror_scene::components::{LightShape, LightSpec, LightUnits};
    use gltf::khr_lights_punctual::Kind;
    let shape = match light.kind() {
        Kind::Directional => {
            report.dropped.push(format!(
                "directional light {} (use World.setEnvironment sun)",
                light.index()
            ));
            return None;
        }
        Kind::Point => LightShape::Point,
        Kind::Spot { inner_cone_angle, outer_cone_angle } => LightShape::Spot {
            inner_degrees: inner_cone_angle.to_degrees().min(89.),
            outer_degrees: outer_cone_angle.to_degrees().clamp(0.1, 89.),
        },
    };
    let intensity = light.intensity().clamp(0., 1e9);
    // glTF range is optional (infinite). Without one, end where illuminance
    // falls to 0.01 lux, which the smooth window makes invisible.
    let range = light
        .range()
        .unwrap_or_else(|| (intensity / 0.01).sqrt())
        .clamp(0.01, 100_000.);
    let spec = LightSpec {
        shape,
        color: light.color(),
        intensity,
        units: LightUnits::Candela,
        falloff: Default::default(),
        range,
        shadows: false,
        enabled: true,
    };
    match spec.validate() {
        Ok(()) => Some(spec),
        Err(e) => {
            report.dropped.push(format!("light {}: {e}", light.index()));
            None
        }
    }
}

fn primitive(
    prim: &gltf::Primitive,
    buffers: &[Vec<u8>],
    report: &mut ImportReport,
) -> Result<(Geometry, [u8; 4])> {
    ensure!(
        prim.mode() == gltf::mesh::Mode::Triangles,
        "glTF primitive mode {:?} is unsupported; triangulate on export",
        prim.mode()
    );
    let reader = prim.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
    let vertices: Vec<[f32; 3]> = reader
        .read_positions()
        .context("glTF primitive has no POSITION")?
        .collect();
    ensure!(!vertices.is_empty(), "glTF primitive has no vertices");
    let indices: Vec<u32> = match reader.read_indices() {
        Some(i) => i.into_u32().collect(),
        None => (0..vertices.len() as u32).collect(),
    };
    ensure!(
        indices.len().is_multiple_of(3) && indices.iter().all(|i| (*i as usize) < vertices.len()),
        "glTF primitive indices invalid"
    );
    report.primitives += 1;
    report.triangles += indices.len() / 3;
    ensure!(report.triangles <= MAX_TRIANGLES, "glTF triangle limit");
    if prim.morph_targets().len() > 0 {
        report
            .dropped
            .push(format!("morph targets on primitive {}", prim.index()));
    }
    let material = prim.material();
    let pbr = material.pbr_metallic_roughness();
    let f = pbr.base_color_factor();
    if f[3] < 1. || material.alpha_mode() != gltf::material::AlphaMode::Opaque {
        report.dropped.push(format!(
            "transparency on material {:?}: renderer is opaque-only",
            material.name().unwrap_or("default")
        ));
    }
    let color = [
        linear_to_srgb_u8(f[0]),
        linear_to_srgb_u8(f[1]),
        linear_to_srgb_u8(f[2]),
        255,
    ];
    Ok((Geometry::Triangles { vertices, indices }, color))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built minimal glTF (one triangle, external buffer) to test the
    /// parser path independently of any Blender export.
    fn triangle_gltf() -> (Vec<u8>, Vec<u8>) {
        let mut bin = Vec::new();
        for v in [[0f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]] {
            for c in v {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        for i in [0u16, 1, 2] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"name":"tri","mesh":0,"translation":[1,2,3],"rotation":[0,0.7071068,0,0.7071068],"children":[1]}},{{"name":"kid","scale":[2,2,2]}}],
            "materials":[{{"pbrMetallicRoughness":{{"baseColorFactor":[1,0.2158605,0,1]}}}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1,"material":0}}]}}],
            "buffers":[{{"uri":"tri.bin","byteLength":{len}}}],
            "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
                         {{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}]}}"#,
            len = bin.len()
        );
        (json.into_bytes(), bin)
    }

    #[test]
    fn imports_hierarchy_transform_material_and_external_buffer() {
        let (json, bin) = triangle_gltf();
        let model = import_gltf(
            &json,
            &ImportOptions {
                first_id: 10,
                parent: Some(3),
                name_prefix: "m:".into(),
                reference_asset: None,
            },
            |uri| {
                assert_eq!(uri, "tri.bin");
                Ok(bin.clone())
            },
        )
        .unwrap();
        assert_eq!(model.entities.len(), 2);
        let root = &model.entities[0];
        assert_eq!(
            (root.id, root.parent, root.name.as_str()),
            (10, Some(3), "m:tri")
        );
        assert_eq!(root.transform.translation, [1., 2., 3.]);
        assert!((root.transform.rotation_degrees[1] - 90.).abs() < 1e-3);
        assert_eq!(root.color, [255, 128, 0, 255]);
        let Some(Geometry::Triangles { vertices, indices }) = &root.geometry else {
            panic!("geometry")
        };
        assert_eq!((vertices.len(), indices.as_slice()), (3, &[0u32, 1, 2][..]));
        let kid = &model.entities[1];
        assert_eq!(
            (kid.id, kid.parent, kid.transform.scale),
            (11, Some(10), [2.; 3])
        );
        assert_eq!(model.report.triangles, 1);
        assert!(model.report.dropped.is_empty());
    }

    #[test]
    fn punctual_lights_become_entity_lights_and_directional_is_reported() {
        use crysirror_scene::components::LightShape;
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": ["KHR_lights_punctual"],
            "extensions": {"KHR_lights_punctual": {"lights": [
                {"type": "point", "color": [1, 0.5, 0.25], "intensity": 20, "range": 7},
                {"type": "spot", "intensity": 100, "spot": {"innerConeAngle": 0.2, "outerConeAngle": 0.6}},
                {"type": "directional", "intensity": 3}
            ]}},
            "scene": 0,
            "scenes": [{"nodes": [0, 1, 2]}],
            "nodes": [
                {"name": "bulb", "translation": [0, 3, 0], "extensions": {"KHR_lights_punctual": {"light": 0}}},
                {"name": "spot", "extensions": {"KHR_lights_punctual": {"light": 1}}},
                {"name": "sun", "extensions": {"KHR_lights_punctual": {"light": 2}}}
            ]
        });
        let model = import_gltf(&serde_json::to_vec(&json).unwrap(), &ImportOptions::default(), |_| {
            anyhow::bail!("no buffers")
        })
        .unwrap();
        let bulb = model.entities.iter().find(|e| e.name == "bulb").unwrap();
        let l = bulb.light.as_ref().unwrap();
        assert_eq!((l.shape, l.color, l.intensity, l.range), (LightShape::Point, [1., 0.5, 0.25], 20., 7.));
        let spot = model.entities.iter().find(|e| e.name == "spot").unwrap().light.as_ref().unwrap();
        let LightShape::Spot { inner_degrees, outer_degrees } = spot.shape else { panic!("spot") };
        assert!((inner_degrees - 0.2f32.to_degrees()).abs() < 1e-3 && (outer_degrees - 0.6f32.to_degrees()).abs() < 1e-3);
        // No glTF range: ends where illuminance reaches 0.01 lux.
        assert!((spot.range - 100.).abs() < 1e-3, "{}", spot.range);
        assert!(model.entities.iter().find(|e| e.name == "sun").unwrap().light.is_none());
        assert!(model.report.dropped.iter().any(|d| d.contains("directional")), "{:?}", model.report.dropped);
    }

    #[test]
    fn unsupported_inputs_fail_visibly() {
        let (json, _) = triangle_gltf();
        let text = String::from_utf8(json).unwrap();
        let data_uri = text.replace(
            "\"tri.bin\"",
            "\"data:application/octet-stream;base64,AAAA\"",
        );
        assert!(
            import_gltf(data_uri.as_bytes(), &ImportOptions::default(), |_| bail!(
                "x"
            ))
            .is_err()
        );
        let lines = text.replace("\"indices\":1,", "\"indices\":1,\"mode\":1,");
        let (_, bin) = triangle_gltf();
        let err = import_gltf(lines.as_bytes(), &ImportOptions::default(), |_| {
            Ok(bin.clone())
        })
        .unwrap_err();
        assert!(format!("{err:#}").contains("unsupported"));
        assert!(import_gltf(b"not gltf", &ImportOptions::default(), |_| bail!("x")).is_err());
    }
}
