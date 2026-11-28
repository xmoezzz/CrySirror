// SPDX-License-Identifier: MPL-2.0
//! Bind the native lamp meshes as individual CrySirror material instances.
use super::*;
use crysirror_core::path::{Mount, VfsPath};
use crysirror_unity::{
    classes::{Material, Mesh},
    gltf::GlbBuilder,
};
use glam::{Mat4, Quat, Vec3};
use std::sync::Arc;

const MODEL: u32 = MACHINE + 2;
const FIRST_ENTITY: u32 = MACHINE + 32;
const MASK_SHADER: &str = "crysirror/cm3d2/slot-mask.wgsl";
const MASK_SOURCE: &str = include_str!("slot_mask.wgsl");
const FADE_MS: f32 = 100.;

#[derive(Clone, Copy)]
enum Kind {
    Additive,
    Unlit,
    Reel(usize),
}

struct Lamp {
    name: String,
    material: serde_json::Value,
    kind: Kind,
    at: u64,
    on: bool,
    visible: bool,
    alpha: f32,
    offset: [f32; 2],
    reel_tint: [f32; 4],
    reel_settings: Option<(f32, [[f32; 2]; 3])>,
}

pub(super) struct Lighting {
    asset: VfsPath,
    lamps: Vec<Lamp>,
}

impl Lighting {
    pub fn load(
        layout: &Layout,
        assets: &mut Assets,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
    ) -> Result<Self> {
        let file = assets.unity.file(op, &layout.level)?;
        let mut root = None;
        for object in file
            .objects
            .iter()
            .filter(|o| o.class_id == class_id::GAME_OBJECT)
        {
            let bytes = file.bytes(object)?;
            let mut r = Reader::new(&bytes);
            let components = r.count(12)?;
            if components != 0 {
                let transform = r.pptr64()?.path_id;
                r.take((components - 1) * 12 + 4)?;
                if r.string()? == layout.machine {
                    root = Some(transform);
                    break;
                }
            }
        }
        let mut stack = vec![(root.context("missing slot-machine group")?, Mat4::IDENTITY)];
        let mut meshes = GlbBuilder::new("CrySirror slot lamps");
        let mut lamps = Vec::with_capacity(layout.all_lamps.len());
        let mut seen = BTreeSet::new();
        while let Some((transform, parent)) = stack.pop() {
            ensure!(seen.insert(transform), "cyclic slot-machine hierarchy");
            let transform_bytes = file.bytes_of(transform)?;
            let mut tr = Reader::new(&transform_bytes);
            let reference = tr.pptr64()?;
            let rotation = Quat::from_array(tr.f32s()?);
            let position = Vec3::from_array(tr.f32s()?);
            let scale = Vec3::from_array(tr.f32s()?);
            let matrix = parent * Mat4::from_scale_rotation_translation(scale, rotation, position);
            for _ in 0..tr.count(12)? {
                stack.push((tr.pptr64()?.path_id, matrix));
            }
            let object = file.bytes_of(reference.path_id)?;
            let mut r = Reader::new(&object);
            let mut filter = None;
            let mut renderer = None;
            for _ in 0..r.count(12)? {
                let component = r.pptr64()?.path_id;
                match file
                    .object(component)
                    .context("slot-machine component")?
                    .class_id
                {
                    class_id::MESH_FILTER => filter = Some(component),
                    class_id::MESH_RENDERER => renderer = Some(component),
                    _ => {}
                }
            }
            r.take(4)?;
            let name = r.string()?;
            if !layout.all_lamps.contains(&name) {
                continue;
            }
            let filter =
                file.bytes_of(filter.with_context(|| format!("missing mesh filter for {name}"))?)?;
            let renderer =
                file.bytes_of(renderer.with_context(|| format!("missing renderer for {name}"))?)?;
            let mut r = Reader::new(&filter);
            r.seek(12)?;
            let mesh_key = assets
                .unity
                .resolve(op, &layout.level, r.pptr64()?)?
                .context("missing slot lamp mesh")?;
            let (_, mesh) = assets.unity.bytes(op, &mesh_key)?;
            let mut mesh = Mesh::read_56(&mesh)?;
            ensure!(
                !mesh.positions.is_empty()
                    && !mesh.indices.is_empty()
                    && mesh
                        .indices
                        .iter()
                        .all(|&i| usize::from(i) < mesh.positions.len()),
                "invalid slot lamp geometry {name}"
            );
            let normal = matrix.inverse().transpose();
            for p in &mut mesh.positions {
                *p = matrix.transform_point3(Vec3::from_array(*p)).to_array();
            }
            for n in &mut mesh.normals {
                *n = normal
                    .transform_vector3(Vec3::from_array(*n))
                    .normalize_or_zero()
                    .to_array();
            }
            let node = meshes.node(&name, None, [0.; 3], [0., 0., 0., 1.], [1.; 3])?;
            meshes.mesh(node, &mesh, &[])?;
            let mut r = Reader::new(&renderer);
            r.seek(56)?;
            ensure!(r.count(12)? == 1, "slot lamp has several materials");
            let key = assets
                .unity
                .resolve(op, &layout.level, r.pptr64()?)?
                .context("slot lamp material")?;
            let (_, bytes) = assets.unity.bytes(op, &key)?;
            let material = Material::read_56(&bytes)?;
            let shader = assets
                .unity
                .resolve(op, &key.file, material.shader)?
                .context("slot lamp shader")?;
            let (_, bytes) = assets.unity.bytes(op, &shader)?;
            let kind =
                if let Some(reel) = layout.reel_lights.iter().position(|light| light == &name) {
                    ensure!(
                        Reader::new(&bytes).string()? == "AdditiveMaskUV",
                        "unknown slot reel mask shader"
                    );
                    Kind::Reel(reel)
                } else if club::builtin_shader(&bytes, "Particles/Additive") {
                    Kind::Additive
                } else {
                    ensure!(
                        club::builtin_shader(&bytes, "Unlit/Transparent"),
                        "unknown lamp shader for {name}"
                    );
                    Kind::Unlit
                };
            let main = material
                .texture("_MainTex")
                .context("slot lamp main texture")?;
            let key = assets
                .unity
                .resolve(op, &key.file, main.texture)?
                .context("slot lamp texture")?;
            let (texture, _, _) = assets.texture(op, host, client, key)?;
            let reel_tint = material
                .color("_TintColor")
                .unwrap_or([0.5; 4])
                .map(|v| v * 2.);
            let material = json!({
                "base_color_texture": texture,
                "unlit": true,
                "alpha_mode": if matches!(kind, Kind::Unlit) { "blend" } else { "additive" },
                "double_sided": !matches!(kind, Kind::Unlit),
                "uv_scale": main.scale,
            });
            lamps.push(Lamp {
                name,
                material,
                kind,
                at: 0,
                on: false,
                visible: false,
                alpha: 0.,
                offset: [main.offset[0], 1. - main.scale[1] - main.offset[1]],
                reel_tint,
                reel_settings: None,
            });
        }
        ensure!(
            lamps.len() == layout.all_lamps.len(),
            "missing slot lamp mesh"
        );
        let shader = VfsPath::new(Mount::Asset, MASK_SHADER)?;
        op.storage.retain_asset_bytes(
            &shader,
            Arc::from(MASK_SOURCE.as_bytes()),
            64 * 1024 * 1024,
        )?;
        let asset = host.upload_model(client, MODEL, &meshes.finish()?)?;
        let world = lamps.iter().enumerate().map(|(mesh, lamp)| json!({
            "op":"spawn","entity":{"id":FIRST_ENTITY + mesh as u32,"name":lamp.name,
            "geometry":{"kind":"model","asset":asset.relative(),"mesh":mesh},"visible":false}
        })).collect::<Vec<_>>();
        host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":world}))?)?;
        Ok(Self { asset, lamps })
    }

    pub fn animate(
        &mut self,
        now: u64,
        lit: &BTreeSet<String>,
        phases: [f32; 3],
        marks: &ReelMarks,
        commands: &mut Vec<serde_json::Value>,
    ) {
        for (index, lamp) in self.lamps.iter_mut().enumerate() {
            let id = FIRST_ENTITY + index as u32;
            let visible = match lamp.kind {
                Kind::Reel(reel) => {
                    let ranges = marks.ranges[reel];
                    let present = ranges.iter().any(|[lo, hi]| lo < hi);
                    let settings = (phases[reel], ranges);
                    if present && lamp.reel_settings != Some(settings) {
                        let mut material = lamp.material.clone();
                        material["uv_offset"] =
                            json!([lamp.offset[0], lamp.offset[1] - phases[reel]]);
                        material["effect"] = json!({
                            "kind":"custom","asset":MASK_SHADER,"parameters":[
                                [ranges[0][0], ranges[0][1], ranges[1][0], ranges[1][1]],
                                [ranges[2][0], ranges[2][1], 0., 0.], lamp.reel_tint, [0., 0., 0., 0.]
                            ]
                        });
                        lamp.reel_settings = Some(settings);
                        commands.push(json!({"op":"set_material","id":id,"material":material}));
                    }
                    present
                }
                Kind::Additive | Kind::Unlit => {
                    let on = lit.contains(&lamp.name);
                    if lamp.on != on {
                        lamp.at = now;
                        lamp.on = on;
                    }
                    let elapsed = now.saturating_sub(lamp.at) as f32;
                    let progress = (elapsed / FADE_MS).min(1.);
                    let alpha = if matches!(lamp.kind, Kind::Unlit) {
                        if lamp.on || (lamp.visible && elapsed < FADE_MS) {
                            1.
                        } else {
                            0.
                        }
                    } else if lamp.on {
                        progress
                    } else if lamp.visible {
                        1. - progress
                    } else {
                        0.
                    };
                    if alpha != lamp.alpha || !lamp.visible && lamp.on {
                        let mut material = lamp.material.clone();
                        material["uv_offset"] = json!(lamp.offset);
                        material["base_color_factor"] = json!([1., 1., 1., alpha]);
                        commands.push(json!({"op":"set_material","id":id,"material":material}));
                        lamp.alpha = alpha;
                    }
                    lamp.on || alpha > 0.
                }
            };
            if visible != lamp.visible {
                commands.push(json!({"op":"set_visible","id":id,"visible":visible}));
                lamp.visible = visible;
            }
        }
    }

    pub fn release(self, op: &Opener, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        let world = (0..self.lamps.len())
            .map(|i| json!({"op":"despawn","id":FIRST_ENTITY + i as u32}))
            .collect::<Vec<_>>();
        host.submit_engine(client, 1, &serde_json::to_vec(&json!({"world":world}))?)?;
        host.release_model(client, &self.asset)?;
        op.storage
            .release_asset_bytes(Some(&VfsPath::new(Mount::Asset, MASK_SHADER)?))?;
        Ok(())
    }
}
