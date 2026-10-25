// SPDX-License-Identifier: MPL-2.0
//! Per-frame built-in systems: level streaming and spatial audio (ADR 0014).
use crate::{builtin, value::Value, EngineContext};
use anyhow::{Context, Result};
use crysirror_core::{service::HostServices, state::Command};
use crysirror_scene::world::WorldCommand;
use glam::Vec3;
use std::collections::BTreeMap;

fn focus(cx: &mut EngineContext) -> Result<Option<Vec3>> {
    let w = &cx.state()?.runtime.host.engine.world;
    Ok(w.view.camera.as_ref().map(|c| Vec3::from_array(c.eye)))
}

/// Load/unload streaming levels around the camera.
pub fn streaming(cx: &mut EngineContext) -> Result<()> {
    let Some(f) = focus(cx)? else { return Ok(()) };
    let levels = cx.state()?.runtime.host.engine.world.streaming.clone();
    for (name, mut level) in levels {
        let d = (Vec3::from_array(level.center) - f).length();
        match level.loaded {
            None if d <= level.load_radius => {
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
                builtin::submit(cx, vec![WorldCommand::Spawn { entity: root }])?;
                let s = cx.state()?;
                let h = s.runtime.host.engine.world.handle(s.client, id)?;
                let parent = builtin::entity_object(cx, id, h.generation)?;
                let opts = Value::from_json(
                    serde_json::json!({"pbr": level.pbr, "prefix": format!("{name}:")}),
                );
                let mut opts = match opts {
                    Value::Map(m) => m,
                    _ => unreachable!(),
                };
                opts.insert("parent".into(), parent);
                let r = builtin::import_model(cx, &level.source, Some(&Value::Map(opts)))
                    .with_context(|| format!("stream level {name}"))?;
                if level.collide {
                    if let Some(Value::List(roots)) = r.field("roots") {
                        let collider: crysirror_scene::components::ColliderSpec =
                            serde_json::from_value(serde_json::json!({"shape": {"kind": "mesh"}}))?;
                        let mut cmds = Vec::new();
                        for root in roots {
                            let oid = root.as_object()?;
                            let s = cx.state()?;
                            let e = s.objects.get(oid)?;
                            let rid = e.payload.field("id").context("id")?.as_i64()? as u32;
                            cmds.push(WorldCommand::SetCollider {
                                id: rid,
                                collider: Some(collider.clone()),
                            });
                        }
                        builtin::submit(cx, cmds)?;
                    }
                }
                level.loaded = Some(id);
                builtin::submit(
                    cx,
                    vec![WorldCommand::SetStreaming {
                        name,
                        level: Some(level),
                    }],
                )?;
            }
            Some(root) if d > level.unload_radius => {
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
