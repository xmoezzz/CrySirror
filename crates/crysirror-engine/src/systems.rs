// SPDX-License-Identifier: MPL-2.0
//! Per-frame built-in systems: level streaming and spatial audio (ADR 0014).
use crate::{EngineContext, builtin};
use anyhow::{Context, Result};
use crysirror_core::{service::HostServices, state::Command};
use crysirror_scene::world::WorldCommand;
use glam::Vec3;
use std::collections::BTreeMap;

struct PendingLevel {
    spec: crysirror_scene::world::StreamingLevel,
    lease: crysirror_runtime::assets::AssetLease,
    revision: u64,
}
#[derive(Default)]
pub struct StreamingRuntime {
    epoch: Option<(u64, u64)>,
    pending: BTreeMap<String, PendingLevel>,
    loaded: BTreeMap<String, (u32, crysirror_runtime::assets::AssetLease)>,
    pub committed: u64,
    pub cancelled: u64,
}

fn focus(cx: &mut EngineContext) -> Result<Option<Vec3>> {
    let w = &cx.state()?.runtime.host.engine.world;
    Ok(w.view.camera.as_ref().map(|c| Vec3::from_array(c.eye)))
}

/// Tool/capture readiness without advancing scripts, input, physics or clocks.
/// Never called by the interactive window loop.
pub fn finish_streaming_offline(cx: &mut EngineContext) -> Result<()> {
    for _ in 0..4096 {
        let frame = cx.state()?.runtime.profiler.frame();
        streaming_at(cx, frame)?;
        let s = cx.state()?;
        let tickets: Vec<_> = s
            .streaming
            .pending
            .values()
            .map(|p| p.lease.ticket())
            .collect();
        if tickets.is_empty() {
            return Ok(());
        }
        for ticket in tickets {
            s.runtime.assets.wait_task(s.client, ticket)?;
        }
    }
    anyhow::bail!("offline streaming completion budget exceeded")
}

/// Load/unload streaming levels around the camera.
pub fn streaming(cx: &mut EngineContext) -> Result<()> {
    let frame = cx.state()?.runtime.profiler.frame() + 1;
    streaming_at(cx, frame)
}
fn streaming_at(cx: &mut EngineContext, frame: u64) -> Result<()> {
    let Some(f) = focus(cx)? else { return Ok(()) };
    let profiler = cx.state()?.runtime.profiler();
    let _span = profiler.span(frame, "streaming.commit");
    let s = cx.state()?;
    let epoch = (s.runtime.assets.store_id(), s.runtime.restored);
    if s.streaming.epoch != Some(epoch) {
        s.streaming.cancelled += s.streaming.pending.len() as u64;
        s.streaming.pending.clear();
        s.streaming.loaded.clear();
        s.streaming.epoch = Some(epoch);
    }
    let levels = cx.state()?.runtime.host.engine.world.streaming.clone();
    let obsolete: Vec<_> = cx
        .state()?
        .streaming
        .pending
        .iter()
        .filter(|(name, p)| {
            !levels.get(*name).is_some_and(|l| {
                l == &p.spec && (Vec3::from_array(l.center) - f).length() <= l.unload_radius
            })
        })
        .map(|(n, _)| n.clone())
        .collect();
    for name in obsolete {
        cx.state()?.streaming.pending.remove(&name);
        cx.state()?.streaming.cancelled += 1;
    }
    cx.state()?
        .streaming
        .loaded
        .retain(|name, (root, _)| levels.get(name).is_some_and(|s| s.loaded == Some(*root)));
    let mut commits = 0;
    for (name, mut level) in levels {
        let d = (Vec3::from_array(level.center) - f).length();
        match level.loaded {
            None if d <= level.load_radius => {
                let s = cx.state()?;
                let revision = s.runtime.assets.revision(s.client);
                if s.streaming.pending.get(&name).is_some_and(|p| {
                    p.revision != revision || p.lease.ticket().store != s.runtime.assets.store_id()
                }) {
                    s.streaming.pending.remove(&name);
                    s.streaming.cancelled += 1;
                }
                if !s.streaming.pending.contains_key(&name) {
                    let kind = if level.pbr {
                        crysirror_runtime::assets::AssetKind::Level
                    } else {
                        crysirror_runtime::assets::AssetKind::Scene
                    };
                    let lease = match s.runtime.assets.lease(s.client, &level.source, kind) {
                        Ok(t) => t,
                        Err(e) if e.is::<crysirror_tasks::CapacityExceeded>() => continue,
                        Err(e) => return Err(e),
                    };
                    s.streaming.pending.insert(
                        name.clone(),
                        PendingLevel {
                            spec: level.clone(),
                            lease,
                            revision,
                        },
                    );
                }
                let ticket = s.streaming.pending[&name].lease.ticket();
                let status = s.runtime.assets.task_status(s.client, ticket)?;
                match status.state {
                    crysirror_runtime::assets::AssetState::Loading => continue,
                    crysirror_runtime::assets::AssetState::Ready => {}
                    _ => anyhow::bail!(
                        "stream level {name}: {:?}: {}",
                        status.state,
                        status.error.unwrap_or_default()
                    ),
                }
                if commits >= 2 {
                    continue;
                }
                let baked = Some(s.runtime.assets.ready_scene(s.client, ticket)?);
                let id = builtin::next_entity_id(cx)?;
                let root = crysirror_scene::world::EntitySpec {
                    id,
                    name: format!("level:{name}"),
                    parent: None,
                    transform: level.transform.clone(),
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
                let mut commands = vec![WorldCommand::Spawn { entity: root }];
                if let Some(baked) = baked {
                    for old in &baked.entities {
                        let mut entity = old.clone();
                        entity.name = format!("{name}:{}", entity.name);
                        entity.id = id
                            .checked_add(old.id)
                            .context("streamed entity id overflow")?;
                        entity.parent = Some(match old.parent {
                            Some(p) => id.checked_add(p).context("streamed parent id overflow")?,
                            None => id,
                        });
                        if level.collide && entity.geometry.is_some() {
                            entity.collider = Some(serde_json::from_value(
                                serde_json::json!({"shape":{"kind":"mesh"}}),
                            )?);
                        }
                        commands.push(WorldCommand::Spawn { entity });
                    }
                }
                level.loaded = Some(id);
                commands.push(WorldCommand::SetStreaming {
                    name: name.clone(),
                    level: Some(level.clone()),
                });
                builtin::submit(cx, commands)?;
                let pending = cx.state()?.streaming.pending.remove(&name).unwrap();
                cx.state()?
                    .streaming
                    .loaded
                    .insert(name.clone(), (id, pending.lease));
                cx.state()?.streaming.committed += 1;
                commits += 1;
                continue;
            }
            Some(root) if d > level.unload_radius => {
                cx.state()?.streaming.loaded.remove(&name);
                level.loaded = None;
                builtin::submit(
                    cx,
                    vec![
                        WorldCommand::Despawn { id: root },
                        WorldCommand::SetStreaming {
                            name,
                            level: Some(level),
                        },
                    ],
                )?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// A positional voice: gain falls off with listener (camera) distance.
#[derive(Clone, Debug)]
pub struct Spatial {
    pub gain: f32,
    pub position: [f32; 3],
    pub follow: Option<u32>,
    pub min_distance: f32,
    pub radius: f32,
    pub applied: u16,
}
#[derive(Default)]
pub struct SpatialAudio {
    pub voices: BTreeMap<u32, Spatial>,
}
pub fn attenuation(d: f32, min: f32, radius: f32) -> f32 {
    if d <= min {
        1.
    } else if d >= radius {
        0.
    } else {
        // Smooth inverse-square-like rolloff that reaches zero at the radius.
        let t = (d - min) / (radius - min);
        (1. - t) * (1. - t)
    }
}
pub fn spatial_audio(cx: &mut EngineContext) -> Result<()> {
    let Some(listener) = focus(cx)? else {
        return Ok(());
    };
    let s = cx.state()?;
    let client = s.client;
    let playing: std::collections::BTreeSet<u32> = s
        .runtime
        .host
        .scene
        .voices()
        .keys()
        .filter(|k| k.client == client)
        .map(|k| k.local)
        .collect();
    s.audio.voices.retain(|ch, _| playing.contains(ch));
    let world = &s.runtime.host.engine.world;
    let mut cmds = Vec::new();
    for (ch, v) in s.audio.voices.iter_mut() {
        if let Some(id) = v.follow {
            if let Ok(h) = world.handle(client, id) {
                v.position = world.global_matrix(h)?.w_axis.truncate().to_array();
            }
        }
        let g = (v.gain
            * attenuation(
                (Vec3::from_array(v.position) - listener).length(),
                v.min_distance,
                v.radius,
            )
            * 1000.)
            .round()
            .clamp(0., 1000.) as u16;
        if g.abs_diff(v.applied) >= 2 {
            v.applied = g;
            cmds.push(Command::SetAudioGain {
                channel: *ch,
                gain_milli: g,
            });
        }
    }
    if !cmds.is_empty() {
        s.runtime
            .host
            .submit(client, &cmds)
            .map_err(|e| anyhow::anyhow!("spatial audio: {e}"))?;
    }
    Ok(())
}
