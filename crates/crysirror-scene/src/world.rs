// SPDX-License-Identifier: MPL-2.0
//! Dynamic, scoped world state and render extraction. No device/VM pointers.
use crate::{
    animation::IntEvent,
    model::{Camera, Geometry, Mesh, Scene3d},
};
use anyhow::{ensure, Context, Result};
use crysirror_core::state::ClientId;
use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
fn one() -> [f32; 3] {
    [1.; 3]
}
fn white() -> [u8; 4] {
    [255; 4]
}
fn yes() -> bool {
    true
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityHandle {
    pub client: u32,
    pub id: u32,
    pub generation: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    /// Position (world units).
    #[serde(default)]
    pub translation: [f32; 3],
    /// Euler rotation, XYZ order, degrees.
    #[serde(default)]
    pub rotation_degrees: [f32; 3],
    /// Per-axis scale (non-zero).
    #[serde(default = "one")]
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}
impl Transform {
    pub fn identity() -> Self {
        Self {
            translation: [0.; 3],
            rotation_degrees: [0.; 3],
            scale: one(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.translation
                .iter()
                .chain(&self.rotation_degrees)
                .chain(&self.scale)
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.),
            "invalid world transform"
        );
        ensure!(
            self.scale.iter().all(|v| v.abs() > 1e-6),
            "singular world transform"
        );
        Ok(())
    }
    pub fn matrix(&self) -> Mat4 {
        let r = self.rotation_degrees.map(f32::to_radians);
        Mat4::from_scale_rotation_translation(
            Vec3::from_array(self.scale),
            Quat::from_euler(EulerRot::XYZ, r[0], r[1], r[2]),
            Vec3::from_array(self.translation),
        )
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Property {
    X,
    Y,
    Z,
    RotationX,
    RotationY,
    RotationZ,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tween {
    pub property: Property,
    pub from: i32,
    pub to: i32,
    pub duration_ms: i32,
    #[serde(default)]
    pub delay_ms: i32,
    #[serde(default)]
    pub mode: i32,
    #[serde(default)]
    pub easing: i32,
    #[serde(default)]
    pub real_time: bool,
}
impl Tween {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.from.abs_diff(0) <= 100_000_000
                && self.to.abs_diff(0) <= 100_000_000
                && (1..=86_400_000).contains(&self.duration_ms)
                && (0..=86_400_000).contains(&self.delay_ms)
                && (0..=2).contains(&self.mode)
                && (0..=2).contains(&self.easing),
            "invalid tween"
        );
        Ok(())
    }
    fn event(&self) -> Result<IntEvent> {
        self.validate()?;
        let mut event = IntEvent::new(self.from);
        match self.mode {
            0 => event.set_event(
                self.to,
                self.duration_ms,
                self.delay_ms,
                self.easing,
                i32::from(self.real_time),
            ),
            1 => event.loop_event(
                self.from,
                self.to,
                self.duration_ms,
                self.delay_ms,
                self.easing,
                i32::from(self.real_time),
            ),
            2 => event.turn_event(
                self.from,
                self.to,
                self.duration_ms,
                self.delay_ms,
                self.easing,
                i32::from(self.real_time),
            ),
            _ => unreachable!(),
        };
        Ok(event)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitySpec {
    /// Client-local entity id.
    pub id: u32,
    /// Display/debug name.
    pub name: String,
    /// Parent entity id (same client); transforms are relative to it.
    #[serde(default)]
    pub parent: Option<u32>,
    /// Local transform.
    #[serde(default = "Transform::identity")]
    pub transform: Transform,
    /// Cube, triangles, model or scene geometry; `null` for an empty node.
    #[serde(default)]
    pub geometry: Option<Geometry>,
    /// Material override of the geometry's own materials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<crate::material::MaterialSpec>,
    /// RGBA8 tint for simple geometry.
    #[serde(default = "white")]
    pub color: [u8; 4],
    /// Inherited visibility.
    #[serde(default = "yes")]
    pub visible: bool,
    /// Linear velocity (units/s) integrated on the fixed step.
    #[serde(default)]
    pub velocity: [f32; 3],
    /// Angular velocity (degrees/s per axis).
    #[serde(default)]
    pub angular_velocity: [f32; 3],
    /// Active property tweens.
    #[serde(default)]
    pub tweens: Vec<Tween>,
    /// Skeletal animation state for `Geometry::Scene` entities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<crate::components::AnimationState>,
    /// Physics collider/body (ADR 0012).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collider: Option<crate::components::ColliderSpec>,
    /// Attributes, active effects, tags and abilities (ADR 0014).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gameplay: Option<crate::gameplay::GameplayState>,
    /// Behaviour tree instance and blackboard (ADR 0014).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<crate::ai::AiState>,
    /// Particle emitter (visual; ADR 0014).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub particles: Option<crate::components::EmitterSpec>,
    /// Local light (ADR 0018).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<crate::components::LightSpec>,
    /// Box reflection capture (ADR 0024).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflection_probe: Option<crate::components::ReflectionProbeSpec>,
    /// Dynamic diffuse GI volume (ADR 0028).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irradiance_volume: Option<crate::components::IrradianceVolumeSpec>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entity {
    spec: EntitySpec,
    generation: u32,
    events: Vec<IntEvent>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientWorld {
    entities: BTreeMap<u32, Entity>,
    generations: BTreeMap<u32, u32>,
    paused: bool,
    /// Named video playbacks (ADR 0020); decoding happens in the runtime.
    #[serde(default)]
    videos: BTreeMap<String, VideoSpec>,
    /// Named level sequences (ADR 0040).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sequences: BTreeMap<String, crate::sequence::SequencePlayback>,
}
/// First dynamic-surface slot used for video frames (adapter surfaces use low slots).
pub const VIDEO_SLOT_BASE: u32 = 1000;
pub const MAX_VIDEOS_PER_CLIENT: usize = 8;
/// Saved video playback state: the presented frame is a pure function of
/// `asset` and `time_ms`, so saves/replays restore the same picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoSpec {
    /// Client-scoped asset path (OMV, Ogg/Theora, MPEG-1/2, ASF/WMV).
    pub asset: String,
    #[serde(default)]
    pub looping: bool,
    #[serde(default = "yes_stream")]
    pub playing: bool,
    /// Playback position; advances with world time while playing.
    #[serde(default)]
    pub time_ms: u64,
    /// Dynamic surface slot receiving frames (assigned by the world).
    #[serde(default)]
    pub slot: u32,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    #[serde(default)]
    pub physics: crate::physics::PhysicsScene,
    /// Camera and lighting environment (ADR 0010); the last client to set
    /// them owns the environment asset reference.
    #[serde(default)]
    pub view: crate::view::View,
    /// Gameplay effect/ability definitions shared by all entities (ADR 0014).
    #[serde(default)]
    pub gameplay: crate::gameplay::GameplayDefs,
    /// State machines and montages (ADR 0014).
    #[serde(default)]
    pub animation: crate::animgraph::AnimDefs,
    /// Behaviour trees (ADR 0014).
    #[serde(default)]
    pub ai: crate::ai::AiDefs,
    /// Streaming volumes (ADR 0014).
    #[serde(default)]
    pub streaming: BTreeMap<String, StreamingLevel>,
    clients: BTreeMap<u32, ClientWorld>,
    pub elapsed_ms: u64,
    remainder_ms: u64,
    #[serde(default)]
    pub revision: u64,
}
fn yes_stream() -> bool {
    true
}
/// A sub-level loaded when the streaming focus is within `load_radius` of
/// `center` and unloaded beyond `unload_radius` (hysteresis).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamingLevel {
    /// glTF/GLB asset path.
    pub source: String,
    pub center: [f32; 3],
    pub load_radius: f32,
    pub unload_radius: f32,
    /// Placement of the level root.
    #[serde(default)]
    pub transform: Transform,
    #[serde(default = "yes_stream")]
    pub pbr: bool,
    /// Give imported roots triangle-mesh colliders.
    #[serde(default)]
    pub collide: bool,
    /// Root entity while loaded.
    #[serde(default)]
    pub loaded: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorldCommand {
    Spawn {
        entity: EntitySpec,
    },
    Despawn {
        id: u32,
    },
    SetTransform {
        id: u32,
        transform: Transform,
    },
    SetVelocity {
        id: u32,
        velocity: [f32; 3],
    },
    SetAngularVelocity {
        id: u32,
        velocity: [f32; 3],
    },
    SetPhysics {
        physics: crate::physics::ClientPhysicsScene,
    },
    SetParent {
        id: u32,
        parent: Option<u32>,
    },
    SetVisible {
        id: u32,
        visible: bool,
    },
    Tween {
        id: u32,
        tween: Tween,
    },
    Pause {
        paused: bool,
    },
    SetCamera {
        camera: Camera,
    },
    /// Start (or restart) a named level sequence owned by this client.
    PlaySequence {
        name: String,
        sequence: crate::sequence::SequenceSpec,
        #[serde(default)]
        start_ms: f32,
    },
    /// Remove a named sequence; tracks keep their last applied values.
    StopSequence {
        name: String,
    },
    SetMaterial {
        id: u32,
        material: Option<crate::material::MaterialSpec>,
    },
    SetAnimation {
        id: u32,
        animation: Option<crate::components::AnimationState>,
    },
    SetCollider {
        id: u32,
        collider: Option<crate::components::ColliderSpec>,
    },
    SetGameplay {
        id: u32,
        gameplay: Option<crate::gameplay::GameplayState>,
    },
    DefineGameplay {
        defs: crate::gameplay::GameplayDefs,
    },
    DefineAnimation {
        defs: crate::animgraph::AnimDefs,
    },
    DefineAi {
        defs: crate::ai::AiDefs,
    },
    SetAi {
        id: u32,
        ai: Option<crate::ai::AiState>,
    },
    SetParticles {
        id: u32,
        particles: Option<crate::components::EmitterSpec>,
    },
    SetLight {
        id: u32,
        light: Option<crate::components::LightSpec>,
    },
    SetReflectionProbe {
        id: u32,
        probe: Option<crate::components::ReflectionProbeSpec>,
    },
    SetIrradianceVolume {
        id: u32,
        volume: Option<crate::components::IrradianceVolumeSpec>,
    },
    SetStreaming {
        name: String,
        level: Option<StreamingLevel>,
    },
    /// Create, replace (keeping its surface slot) or remove a named video.
    SetVideo {
        name: String,
        video: Option<VideoSpec>,
    },
    SetEnvironment {
        environment: crate::view::Environment,
    },
}
impl World {
    pub fn client_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.clients
            .keys()
            .chain(self.physics.clients.keys())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
    }
    pub fn client_paused(&self, client: u32) -> bool {
        self.clients.get(&client).is_some_and(|owner| owner.paused)
    }
    /// Save simulation results without cloning the entire world or changing
    /// authored collider configuration. Validate all writes before committing.
    pub fn write_physics(
        &mut self,
        states: &[(EntityHandle, Transform, [f32; 3], [f32; 3])],
    ) -> Result<()> {
        ensure!(states.len() <= 4096, "physics state count");
        let revision = if states.is_empty() {
            self.revision
        } else {
            self.revision
                .checked_add(1)
                .context("physics world revision overflow")?
        };
        for (h, transform, linear, angular) in states {
            self.entity(*h)?;
            transform.validate()?;
            ensure!(
                linear
                    .iter()
                    .chain(angular)
                    .all(|v| v.is_finite() && v.abs() <= 100_000.),
                "invalid simulated velocity"
            );
        }
        for (h, transform, linear, angular) in states {
            let entity = self
                .clients
                .get_mut(&h.client)
                .unwrap()
                .entities
                .get_mut(&h.id)
                .unwrap();
            entity.spec.transform = transform.clone();
            entity.spec.velocity = *linear;
            entity.spec.angular_velocity = *angular;
        }
        self.revision = revision;
        Ok(())
    }
    /// Every video as (client, name, spec).
    pub fn videos(&self) -> impl Iterator<Item = (u32, &str, &VideoSpec)> + '_ {
        self.clients
            .iter()
            .flat_map(|(c, w)| w.videos.iter().map(move |(n, v)| (*c, n.as_str(), v)))
    }
    pub fn video(&self, client: ClientId, name: &str) -> Option<&VideoSpec> {
        self.clients.get(&client.0)?.videos.get(name)
    }
    pub fn install(&mut self, client: ClientId, specs: &[EntitySpec]) -> Result<()> {
        ensure!(specs.len() <= 4096, "world entity limit");
        let commands = specs
            .iter()
            .cloned()
            .map(|entity| WorldCommand::Spawn { entity })
            .collect::<Vec<_>>();
        self.apply_batch(client, &commands)
    }
    pub fn handle(&self, client: ClientId, id: u32) -> Result<EntityHandle> {
        let e = self
            .clients
            .get(&client.0)
            .and_then(|c| c.entities.get(&id))
            .context("world entity missing")?;
        Ok(EntityHandle {
            client: client.0,
            id,
            generation: e.generation,
        })
    }
    pub fn entity(&self, h: EntityHandle) -> Result<&EntitySpec> {
        let e = self
            .clients
            .get(&h.client)
            .and_then(|c| c.entities.get(&h.id))
            .context("world entity missing")?;
        ensure!(e.generation == h.generation, "stale entity handle");
        Ok(&e.spec)
    }
    /// (id, generation, spec) for one client's entities in id order.
    pub fn entities(&self, client: ClientId) -> Vec<(u32, u32, &EntitySpec)> {
        self.clients
            .get(&client.0)
            .map(|c| {
                c.entities
                    .iter()
                    .map(|(id, e)| (*id, e.generation, &e.spec))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn len(&self) -> usize {
        self.clients.values().map(|c| c.entities.len()).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn apply_batch(&mut self, client: ClientId, commands: &[WorldCommand]) -> Result<()> {
        ensure!(commands.len() <= 4096, "world command batch limit");
        let mut staged = self.clone();
        let mut staged_view = staged.view.clone();
        let mut staged_defs = staged.gameplay.clone();
        let mut staged_anim = staged.animation.clone();
        let mut staged_ai = staged.ai.clone();
        let mut staged_streaming = staged.streaming.clone();
        let owner = staged.clients.entry(client.0).or_default();
        for command in commands {
            match command {
                WorldCommand::SetPhysics { physics } => {
                    staged.physics.clients.insert(client.0, physics.clone());
                }
                WorldCommand::Spawn { entity } => {
                    ensure!(
                        !owner.entities.contains_key(&entity.id),
                        "duplicate world entity"
                    );
                    let generation = owner
                        .generations
                        .get(&entity.id)
                        .copied()
                        .unwrap_or(0)
                        .checked_add(1)
                        .context("entity generation exhausted")?;
                    let events = entity
                        .tweens
                        .iter()
                        .map(Tween::event)
                        .collect::<Result<Vec<_>>>()?;
                    owner.generations.insert(entity.id, generation);
                    owner.entities.insert(
                        entity.id,
                        Entity {
                            spec: entity.clone(),
                            generation,
                            events,
                        },
                    );
                }
                WorldCommand::Despawn { id } => {
                    ensure!(owner.entities.contains_key(id), "despawn entity missing");
                    let mut remove = BTreeSet::from([*id]);
                    loop {
                        let old = remove.len();
                        for (eid, e) in &owner.entities {
                            if e.spec.parent.is_some_and(|p| remove.contains(&p)) {
                                remove.insert(*eid);
                            }
                        }
                        if old == remove.len() {
                            break;
                        }
                    }
                    owner.entities.retain(|id, _| !remove.contains(id));
                }
                WorldCommand::Pause { paused } => owner.paused = *paused,
                WorldCommand::SetVideo { name, video } => {
                    ensure!(!name.is_empty() && name.len() <= 128, "video name");
                    match video {
                        Some(v) => {
                            crysirror_core::path::VfsPath::new(
                                crysirror_core::path::Mount::Asset,
                                &v.asset,
                            )
                            .map_err(|e| anyhow::anyhow!("video asset: {e}"))?;
                            let slot = match owner.videos.get(name) {
                                Some(old) => old.slot,
                                None => {
                                    ensure!(
                                        owner.videos.len() < MAX_VIDEOS_PER_CLIENT,
                                        "video limit per client"
                                    );
                                    (VIDEO_SLOT_BASE..)
                                        .find(|s| owner.videos.values().all(|o| o.slot != *s))
                                        .context("video slot")?
                                }
                            };
                            owner
                                .videos
                                .insert(name.clone(), VideoSpec { slot, ..v.clone() });
                        }
                        None => {
                            ensure!(owner.videos.remove(name).is_some(), "video {name} missing");
                        }
                    }
                }
                WorldCommand::SetCamera { camera } => staged_view.camera = Some(camera.clone()),
                WorldCommand::PlaySequence {
                    name,
                    sequence,
                    start_ms,
                } => {
                    ensure!(!name.is_empty() && name.len() <= 128, "sequence name");
                    ensure!(
                        owner.sequences.contains_key(name)
                            || owner.sequences.len() < crate::sequence::MAX_SEQUENCES_PER_CLIENT,
                        "sequence limit per client"
                    );
                    for t in &sequence.tracks {
                        let e = owner.entities.get(&t.entity).with_context(|| {
                            format!("sequence track entity {} missing", t.entity)
                        })?;
                        ensure!(
                            !t.property.needs_light() || e.spec.light.is_some(),
                            "sequence light track on entity {} without a light",
                            t.entity
                        );
                    }
                    ensure!(
                        sequence.camera.is_none() || staged_view.camera.is_some(),
                        "camera sequences need a camera (set_camera first)"
                    );
                    owner.sequences.insert(
                        name.clone(),
                        crate::sequence::SequencePlayback::new(sequence.clone(), *start_ms)?,
                    );
                }
                WorldCommand::StopSequence { name } => {
                    ensure!(
                        owner.sequences.remove(name).is_some(),
                        "sequence {name} missing"
                    );
                }
                WorldCommand::DefineAi { defs } => staged_ai.trees.extend(defs.trees.clone()),
                WorldCommand::SetStreaming { name, level } => match level {
                    Some(l) => {
                        ensure!(
                            l.load_radius.is_finite()
                                && l.unload_radius >= l.load_radius
                                && !l.source.is_empty()
                                && name.len() <= 128,
                            "invalid streaming level"
                        );
                        staged_streaming.insert(name.clone(), l.clone());
                    }
                    None => {
                        staged_streaming.remove(name);
                    }
                },
                WorldCommand::DefineAnimation { defs } => {
                    staged_anim.machines.extend(defs.machines.clone());
                    staged_anim.montages.extend(defs.montages.clone());
                }
                WorldCommand::DefineGameplay { defs } => {
                    staged_defs.effects.extend(defs.effects.clone());
                    staged_defs.abilities.extend(defs.abilities.clone());
                }
                WorldCommand::SetEnvironment { environment } => {
                    staged_view.environment = Some((client.0, environment.clone()))
                }
                other => {
                    let id = match other {
                        WorldCommand::SetTransform { id, .. }
                        | WorldCommand::SetVelocity { id, .. }
                        | WorldCommand::SetAngularVelocity { id, .. }
                        | WorldCommand::SetParent { id, .. }
                        | WorldCommand::SetVisible { id, .. }
                        | WorldCommand::Tween { id, .. }
                        | WorldCommand::SetMaterial { id, .. }
                        | WorldCommand::SetAnimation { id, .. }
                        | WorldCommand::SetCollider { id, .. }
                        | WorldCommand::SetGameplay { id, .. }
                        | WorldCommand::SetAi { id, .. }
                        | WorldCommand::SetParticles { id, .. }
                        | WorldCommand::SetLight { id, .. }
                        | WorldCommand::SetReflectionProbe { id, .. }
                        | WorldCommand::SetIrradianceVolume { id, .. } => *id,
                        _ => unreachable!(),
                    };
                    let entity = owner
                        .entities
                        .get_mut(&id)
                        .context("world entity missing")?;
                    match other {
                        WorldCommand::SetTransform { transform, .. } => {
                            entity.spec.transform = transform.clone()
                        }
                        WorldCommand::SetVelocity { velocity, .. } => {
                            entity.spec.velocity = *velocity
                        }
                        WorldCommand::SetAngularVelocity { velocity, .. } => {
                            entity.spec.angular_velocity = *velocity
                        }
                        WorldCommand::SetParent { parent, .. } => entity.spec.parent = *parent,
                        WorldCommand::SetVisible { visible, .. } => entity.spec.visible = *visible,
                        WorldCommand::SetMaterial { material, .. } => {
                            entity.spec.material = material.clone()
                        }
                        WorldCommand::SetAnimation { animation, .. } => {
                            entity.spec.animation = animation.clone()
                        }
                        WorldCommand::SetCollider { collider, .. } => {
                            entity.spec.collider = collider.clone()
                        }
                        WorldCommand::SetGameplay { gameplay, .. } => {
                            entity.spec.gameplay = gameplay.clone()
                        }
                        WorldCommand::SetAi { ai, .. } => entity.spec.ai = ai.clone(),
                        WorldCommand::SetParticles { particles, .. } => {
                            entity.spec.particles = particles.clone()
                        }
                        WorldCommand::SetLight { light, .. } => entity.spec.light = light.clone(),
                        WorldCommand::SetReflectionProbe { probe, .. } => {
                            entity.spec.reflection_probe = probe.clone()
                        }
                        WorldCommand::SetIrradianceVolume { volume, .. } => {
                            entity.spec.irradiance_volume = volume.clone()
                        }
                        WorldCommand::Tween { tween, .. } => {
                            entity.spec.tweens.push(tween.clone());
                            entity.events.push(tween.event()?);
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        staged.view = staged_view;
        staged.gameplay = staged_defs;
        staged.animation = staged_anim;
        staged.ai = staged_ai;
        staged.streaming = staged_streaming;
        if !commands
            .iter()
            .any(|c| matches!(c, WorldCommand::SetPhysics { .. }))
        {
            let mut physics = std::mem::take(&mut staged.physics);
            physics.retain_live(&staged);
            staged.physics = physics;
        }
        staged.validate()?;
        staged.revision = staged
            .revision
            .checked_add(1)
            .context("world revision overflow")?;
        *self = staged;
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.physics.validate(self)?;
        self.view.validate()?;
        self.gameplay.validate()?;
        self.animation.validate()?;
        self.ai.validate()?;
        ensure!(
            self.clients.len() <= 64 && self.len() <= 4096 && self.remainder_ms < 16,
            "world size/clock limit"
        );
        for owner in self.clients.values() {
            ensure!(
                owner.generations.len() <= 65536,
                "world generation history limit"
            );
            ensure!(
                owner.sequences.len() <= crate::sequence::MAX_SEQUENCES_PER_CLIENT,
                "sequence limit per client"
            );
            for seq in owner.sequences.values() {
                seq.spec.validate()?;
                ensure!(
                    seq.time_ms.is_finite() && (0.0..=seq.spec.duration_ms).contains(&seq.time_ms),
                    "sequence time"
                );
            }
            for (id, e) in &owner.entities {
                ensure!(
                    *id == e.spec.id
                        && e.generation > 0
                        && owner.generations.get(id) == Some(&e.generation),
                    "world handle identity"
                );
                ensure!(
                    !e.spec.name.is_empty() && e.spec.name.len() <= 256,
                    "world entity name"
                );
                e.spec.transform.validate()?;
                if let Some(m) = &e.spec.material {
                    m.validate_paths()?;
                }
                if let Some(a) = &e.spec.animation {
                    a.validate()?;
                }
                if let Some(c) = &e.spec.collider {
                    c.validate()?;
                }
                if let Some(g) = &e.spec.gameplay {
                    g.validate()?;
                }
                if let Some(a) = &e.spec.ai {
                    a.validate()?;
                }
                if let Some(p) = &e.spec.particles {
                    p.validate()?;
                }
                if let Some(l) = &e.spec.light {
                    l.validate()?;
                }
                if let Some(p) = &e.spec.reflection_probe {
                    p.validate()?;
                }
                if let Some(v) = &e.spec.irradiance_volume {
                    v.validate()?;
                }
                ensure!(
                    e.spec
                        .velocity
                        .iter()
                        .chain(&e.spec.angular_velocity)
                        .all(|v| v.is_finite() && v.abs() <= 100_000.),
                    "invalid world velocity"
                );
                ensure!(e.spec.color[3] == 255, "world opaque material required");
                if let Some(g) = &e.spec.geometry {
                    Scene3d {
                        camera: default_camera(),
                        meshes: vec![Mesh {
                            name: e.spec.name.clone(),
                            geometry: g.clone(),
                            translation: [0.; 3],
                            rotation_degrees: [0.; 3],
                            scale: one(),
                            color: e.spec.color,
                        }],
                    }
                    .validate()?;
                }
                ensure!(
                    e.spec.tweens.len() <= 64 && e.events.len() == e.spec.tweens.len(),
                    "world tween count"
                );
                for (t, event) in e.spec.tweens.iter().zip(&e.events) {
                    t.validate()?;
                    ensure!(
                        event.cur_time >= 0
                            && event.end_time == t.duration_ms
                            && event.delay_time == t.delay_ms
                            && event.start_value == t.from
                            && event.end_value == t.to
                            && (event.loop_type == t.mode
                                || (t.mode == 0 && event.loop_type == -1))
                            && event.speed_type == t.easing
                            && event.real_flag == i32::from(t.real_time)
                            && event.def_value == t.from
                            && event.value == if t.mode == 0 { t.to } else { t.from },
                        "invalid saved tween"
                    );
                }
                let mut seen = BTreeSet::from([*id]);
                let mut parent = e.spec.parent;
                while let Some(p) = parent {
                    ensure!(
                        seen.len() <= 64 && seen.insert(p),
                        "world parent cycle/depth"
                    );
                    parent = owner
                        .entities
                        .get(&p)
                        .context("world parent missing")?
                        .spec
                        .parent;
                }
            }
        }
        Ok(())
    }
    pub fn tick(&mut self, delta_ms: u64) -> Result<()> {
        self.tick_with(delta_ms, |_, _| Ok(()))
    }
    /// Execute an external CPU simulation once inside each authoritative 16 ms
    /// world step, after animation/tweens and before the next world step.
    pub fn tick_with(
        &mut self,
        delta_ms: u64,
        mut physics: impl FnMut(&mut Self, f32) -> Result<()>,
    ) -> Result<()> {
        if self.is_empty()
            && self
                .clients
                .values()
                .all(|c| c.videos.is_empty() && c.sequences.is_empty())
        {
            return Ok(());
        }
        ensure!(delta_ms <= 60_000, "world step exceeds 60 seconds");
        let mut staged = self.clone();
        let total = staged
            .remainder_ms
            .checked_add(delta_ms)
            .context("world clock overflow")?;
        let steps = total / 16;
        let defs = staged.gameplay.clone();
        let anim_defs = staged.animation.clone();
        staged.remainder_ms = total % 16;
        for _ in 0..steps {
            let mut camera_pose: Option<([f32; 3], [f32; 3], Option<f32>)> = None;
            for owner in staged.clients.values_mut() {
                if !owner.paused {
                    for seq in owner.sequences.values_mut() {
                        seq.advance(16.);
                        let t = seq.time_ms;
                        if let Some(c) = &seq.spec.camera {
                            camera_pose = Some(c.value_at(t));
                        }
                        for track in &seq.spec.tracks {
                            let Some(e) = owner.entities.get_mut(&track.entity) else {
                                continue;
                            };
                            let v = track.value_at(t);
                            use crate::sequence::TrackProperty as P;
                            match track.property {
                                P::Translation => e.spec.transform.translation = [v[0], v[1], v[2]],
                                P::Rotation => {
                                    e.spec.transform.rotation_degrees = [v[0], v[1], v[2]]
                                }
                                P::Scale => {
                                    e.spec.transform.scale = [v[0], v[1], v[2]].map(|x| {
                                        if x.abs() < 1e-6 {
                                            1e-6
                                        } else {
                                            x
                                        }
                                    })
                                }
                                P::Visible => e.spec.visible = v[0] > 0.5,
                                P::LightIntensity | P::LightColor | P::LightRange => {
                                    if let Some(l) = e.spec.light.as_mut() {
                                        match track.property {
                                            P::LightIntensity => l.intensity = v[0].max(0.),
                                            P::LightColor => {
                                                l.color = [v[0].max(0.), v[1].max(0.), v[2].max(0.)]
                                            }
                                            _ => l.range = v[0].max(1e-3),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let (Some((eye, target, fov)), Some(cam)) =
                (camera_pose, staged.view.camera.as_mut())
            {
                cam.eye = eye;
                cam.target = target;
                if let Some(f) = fov {
                    cam.fov_y_degrees = f;
                }
            }
            for owner in staged.clients.values_mut() {
                if !owner.paused {
                    for v in owner.videos.values_mut().filter(|v| v.playing) {
                        v.time_ms = v.time_ms.checked_add(16).context("video clock overflow")?;
                    }
                }
                for entity in owner.entities.values_mut() {
                    if let (false, Some(a)) = (owner.paused, entity.spec.animation.as_mut()) {
                        a.advance_with(&anim_defs, 0.016);
                    }
                    if let (false, Some(g)) = (owner.paused, entity.spec.gameplay.as_mut()) {
                        g.tick(&defs, 0.016);
                    }
                    // Physics-driven bodies are moved by the physics system.
                    let physics = entity
                        .spec
                        .collider
                        .as_ref()
                        .is_some_and(|c| c.body != crate::components::BodyKind::Static);
                    if !owner.paused && !physics {
                        for i in 0..3 {
                            entity.spec.transform.translation[i] += entity.spec.velocity[i] * 0.016;
                            entity.spec.transform.rotation_degrees[i] +=
                                entity.spec.angular_velocity[i] * 0.016;
                        }
                    }
                    for (t, event) in entity.spec.tweens.iter().zip(&mut entity.events) {
                        event.update_time(if owner.paused { 0 } else { 16 }, 16);
                        event.frame();
                        let value = event.get_total_value() as f32 / 1000.;
                        match t.property {
                            Property::X => entity.spec.transform.translation[0] = value,
                            Property::Y => entity.spec.transform.translation[1] = value,
                            Property::Z => entity.spec.transform.translation[2] = value,
                            Property::RotationX => {
                                entity.spec.transform.rotation_degrees[0] = value
                            }
                            Property::RotationY => {
                                entity.spec.transform.rotation_degrees[1] = value
                            }
                            Property::RotationZ => {
                                entity.spec.transform.rotation_degrees[2] = value
                            }
                        }
                    }
                }
            }
            staged.revision = staged
                .revision
                .checked_add(1)
                .context("fixed world revision overflow")?;
            physics(&mut staged, 0.016)?;
            staged.elapsed_ms = staged
                .elapsed_ms
                .checked_add(16)
                .context("world clock overflow")?;
        }
        staged.validate()?;
        if steps > 0 {
            staged.revision = staged
                .revision
                .checked_add(1)
                .context("world revision overflow")?;
        }
        *self = staged;
        Ok(())
    }
    /// A client's named sequence.
    pub fn sequence(&self, client: u32, name: &str) -> Option<&crate::sequence::SequencePlayback> {
        self.clients.get(&client)?.sequences.get(name)
    }
    /// Names of a client's sequences.
    pub fn sequence_names(&self, client: u32) -> Vec<String> {
        self.clients
            .get(&client)
            .map_or(Vec::new(), |c| c.sequences.keys().cloned().collect())
    }
    /// Events fired by a sequence since the last call (consumed).
    pub fn take_sequence_events(&mut self, client: u32, name: &str) -> Result<Vec<String>> {
        let seq = self
            .clients
            .get_mut(&client)
            .and_then(|c| c.sequences.get_mut(name))
            .with_context(|| format!("sequence {name} missing"))?;
        Ok(std::mem::take(&mut seq.fired))
    }
    /// Visibility including every ancestor.
    pub fn effective_visible(&self, client: u32, id: u32) -> bool {
        let Some(owner) = self.clients.get(&client) else {
            return false;
        };
        let mut current = Some(id);
        let mut depth = 0;
        while let Some(i) = current {
            depth += 1;
            let Some(e) = owner.entities.get(&i) else {
                return false;
            };
            if !e.spec.visible || depth > 64 {
                return false;
            }
            current = e.spec.parent;
        }
        true
    }
    pub fn global_matrix(&self, h: EntityHandle) -> Result<Mat4> {
        self.entity(h)?;
        let owner = &self.clients[&h.client];
        let mut current = Some(h.id);
        let mut result = Mat4::IDENTITY;
        let mut count = 0;
        while let Some(id) = current {
            count += 1;
            ensure!(count <= 64, "world hierarchy depth");
            let e = owner.entities.get(&id).context("world parent missing")?;
            result = e.spec.transform.matrix() * result;
            current = e.spec.parent;
        }
        ensure!(result.is_finite(), "world matrix overflow");
        Ok(result)
    }
    pub fn render_scene(&self, base: Option<&Scene3d>) -> Result<Option<Scene3d>> {
        self.validate()?;
        if self.is_empty() {
            return Ok(base.cloned());
        }
        let mut scene = base.cloned().unwrap_or(Scene3d {
            camera: default_camera(),
            meshes: vec![],
        });
        for (client, owner) in &self.clients {
            for (id, e) in &owner.entities {
                let Some(geometry) = &e.spec.geometry else {
                    continue;
                };
                // glTF models are drawn by the PBR path, not as flat triangles.
                if matches!(
                    geometry,
                    Geometry::Model { .. } | Geometry::Scene { .. } | Geometry::Terrain(_)
                ) {
                    continue;
                }
                let mut visible = e.spec.visible;
                let mut parent = e.spec.parent;
                while let Some(p) = parent {
                    let e = &owner.entities[&p];
                    visible &= e.spec.visible;
                    parent = e.spec.parent;
                }
                if !visible {
                    continue;
                }
                let matrix = self.global_matrix(EntityHandle {
                    client: *client,
                    id: *id,
                    generation: e.generation,
                })?;
                let (vertices, mut indices) = geometry_data(geometry);
                let vertices = vertices
                    .into_iter()
                    .map(|p| matrix.transform_point3(Vec3::from_array(p)).to_array())
                    .collect();
                if matrix.determinant() < 0. {
                    for face in indices.chunks_exact_mut(3) {
                        face.swap(1, 2);
                    }
                }
                scene.meshes.push(Mesh {
                    name: format!("world:{client}:{id}:{}", e.spec.name),
                    geometry: Geometry::Triangles { vertices, indices },
                    translation: [0.; 3],
                    rotation_degrees: [0.; 3],
                    scale: one(),
                    color: e.spec.color,
                });
            }
        }
        scene.validate()?;
        Ok(Some(scene))
    }
}
pub fn geometry_data(g: &Geometry) -> (Vec<[f32; 3]>, Vec<u32>) {
    match g {
        Geometry::Model { .. } | Geometry::Scene { .. } | Geometry::Terrain(_) => {
            (Vec::new(), Vec::new())
        }
        Geometry::Triangles { vertices, indices } => (vertices.clone(), indices.clone()),
        Geometry::Cube => (
            vec![
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            vec![
                4, 5, 6, 4, 6, 7, 0, 3, 2, 0, 2, 1, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 3, 7, 6, 3,
                6, 2, 0, 1, 5, 0, 5, 4,
            ],
        ),
    }
}
pub fn default_camera() -> Camera {
    Camera {
        eye: [7., 5., 9.],
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 50.,
        near: 0.1,
        far: 1000.,
    }
}
