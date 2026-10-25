// SPDX-License-Identifier: MPL-2.0
//! Service classes: Layers (2D retained layers), Ui (widgets), Audio and
//! Save. Specs are parsed with the engine's own serde types, so every field
//! the runtime accepts is reachable from Rust and scripts alike.
use crate::{
    context::EngineContext,
    object::{Class, ClassInfo, ClassRegistry, MethodInfo, ObjectId, PropertyInfo, TypeHint as T},
    value::Value,
};
use anyhow::{Context, Result, bail};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{FileSystem, HostServices},
    state::{AudioVoice, Command},
};
use crysirror_scene::{ServiceBatch, layer::LayerCommand, ui::UiCommand};
use std::sync::{Arc, OnceLock};

/// Every typed engine command and the API method that issues it.
/// `tests/api_coverage.rs` fails when a command variant has no entry.
pub const COMMAND_API: &[(&str, &str, &str)] = &[
    ("world", "spawn", "World.spawn"),
    ("world", "despawn", "Entity.destroy"),
    ("world", "set_transform", "Entity.position"),
    ("world", "set_velocity", "Entity.velocity"),
    ("world", "set_angular_velocity", "Entity.angularVelocity"),
    ("world", "set_physics", "Physics.gravity"),
    ("world", "set_parent", "Entity.setParent"),
    ("world", "set_visible", "Entity.visible"),
    ("world", "tween", "Entity.tween"),
    ("world", "pause", "World.pause"),
    ("world", "set_camera", "World.setCamera"),
    ("world", "set_environment", "World.setEnvironment"),
    ("world", "set_material", "Entity.setMaterial"),
    ("world", "set_animation", "Entity.play"),
    ("world", "set_collider", "Entity.setCollider"),
    ("world", "set_gameplay", "Entity.applyEffect"),
    ("world", "define_gameplay", "World.defineGameplay"),
    ("world", "define_animation", "World.defineAnimation"),
    ("world", "define_ai", "World.defineBehaviorTree"),
    ("world", "set_ai", "Entity.runBehaviorTree"),
    ("world", "set_particles", "Entity.setParticles"),
    ("world", "set_light", "Entity.setLight"),
    ("world", "set_reflection_probe", "Entity.setReflectionProbe"),
    (
        "world",
        "set_irradiance_volume",
        "Entity.setIrradianceVolume",
    ),
    ("world", "set_streaming", "World.streamLevel"),
    ("world", "set_video", "World.playVideo"),
    ("world", "play_sequence", "World.playSequence"),
    ("world", "stop_sequence", "World.stopSequence"),
    ("layer", "put", "Layers.put"),
    ("layer", "remove", "Layers.remove"),
    ("layer", "set_transform", "Layers.setTransform"),
    ("layer", "set_visible", "Layers.setVisible"),
    ("layer", "complete_reveal", "Layers.completeReveal"),
    ("layer", "set_opacity", "Layers.setOpacity"),
    ("layer", "set_order", "Layers.setOrder"),
    ("layer", "set_clip", "Layers.setClip"),
    ("layer", "set_filter", "Layers.setFilter"),
    ("layer", "transition", "Layers.transition"),
    ("layer", "end_transition", "Layers.endTransition"),
    ("layer", "puppet", "Puppets.setParameter"),
    ("canvas", "create", "Canvases.apply"),
    ("canvas", "clear", "Canvases.apply"),
    ("canvas", "draw", "Canvases.apply"),
    ("canvas", "set", "Canvases.apply"),
    ("canvas", "copy", "Canvases.apply"),
    ("canvas", "present", "Canvases.apply"),
    ("canvas", "hide", "Canvases.apply"),
    ("ui", "put", "Ui.put"),
    ("ui", "remove", "Ui.remove"),
    ("ui", "set_visible", "Ui.setVisible"),
    ("ui", "set_enabled", "Ui.setEnabled"),
    ("ui", "set_value", "Ui.setValue"),
    ("ui", "set_text", "Ui.setText"),
    ("ui", "set_title", "Ui.setTitle"),
    ("ui", "set_cursor_visible", "Ui.setCursorVisible"),
    ("ui", "insert_text", "Ui.insertText"),
    ("ui", "backspace", "Ui.backspace"),
    ("ui", "focus", "Ui.focusTextField"),
    ("media", "play", "Media.play"),
    ("media", "stop", "Media.stop"),
    ("service", "media_status", "Media.status"),
    ("service", "play_audio", "Audio.play"),
    ("service", "stop_audio", "Audio.stop"),
    ("runtime", "snapshot", "Save.save"),
    ("runtime", "restore", "Save.load"),
];

pub fn register(r: &mut ClassRegistry) -> Result<()> {
    r.register(Arc::new(LayersClass))?;
    r.register(Arc::new(UiClass))?;
    r.register(Arc::new(crate::audio_api::AudioClass))?;
    r.register(Arc::new(SaveClass))?;
    r.register(Arc::new(crate::input_api::InputClass))?;
    r.register(Arc::new(crate::localization::LocalizationClass))?;
    r.register(Arc::new(crate::text_api::TextClass))?;
    r.register(Arc::new(crate::bitmap_api::BitmapClass))?;
    r.register(Arc::new(crate::resource_api::ResourcesClass))?;
    r.register(Arc::new(crate::puppet_api::PuppetsClass))?;
    Ok(())
}

fn m(
    name: &'static str,
    params: Vec<(&'static str, T)>,
    returns: T,
    doc: &'static str,
) -> MethodInfo {
    MethodInfo {
        name,
        params,
        returns,
        doc,
    }
}
fn arg<'a>(args: &'a [Value], i: usize, what: &str) -> Result<&'a Value> {
    args.get(i)
        .with_context(|| format!("missing argument '{what}'"))
}
fn id(args: &[Value]) -> Result<u32> {
    Ok(u32::try_from(arg(args, 0, "id")?.as_i64()?)?)
}
fn json<T: serde::de::DeserializeOwned>(v: &Value, what: &str) -> Result<T> {
    serde_json::from_value(v.to_json()).with_context(|| format!("invalid {what}"))
}
fn submit(cx: &mut EngineContext, batch: ServiceBatch) -> Result<()> {
    let s = cx.state()?;
    let client = s.client;
    s.runtime.host.engine.submit(client, &batch)
}

// ---------------------------------------------------------------- Layers
pub struct LayersClass;
fn layers_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Layers",
        doc: "Retained 2D layer hierarchy (sprites, text, solid fills) composited over 3D.",
        properties: vec![
            PropertyInfo { name: "count", ty: T::Int, writable: false, doc: "Layers owned by this client." },
            PropertyInfo { name: "activeTransition", ty: T::Any, writable: false, doc: "Active screen transition {code, progress, elapsed_ms, duration_ms, mine} or null." },
        ],
        methods: vec![
            m("put", vec![("layer", T::Map)], T::Void, "Insert/replace {id, parent?, rect, paint, transform?, order?, clip?, opacity?, visible?, blend?, uv?}."),
            m("remove", vec![("id", T::Int)], T::Void, "Remove a layer and its descendants."),
            m("setTransform", vec![("id", T::Int), ("transform", T::Map)], T::Void, "Pivot/anchor/rotation/scale/z transform."),
            m("setVisible", vec![("id", T::Int), ("visible", T::Bool)], T::Void, "Inherited visibility."),
            m("revealStatus", vec![("id", T::Int)], T::Optional(Box::new(T::Map)), "Engine-owned glyph reveal {visible,total,complete}, or null for an absent/non-glyph layer."),
            m("completeReveal", vec![("id", T::Int)], T::Void, "Show the complete positioned text run without a VM-private reveal clock."),
            m("setOpacity", vec![("id", T::Int), ("opacity", T::Int)], T::Void, "Inherited opacity 0..255."),
            m("setOrder", vec![("id", T::Int), ("order", T::List(Box::new(T::Int)))], T::Void, "Explicit [major, minor] draw order."),
            m("setClip", vec![("id", T::Int), ("clip", T::Optional(Box::new(T::List(Box::new(T::Number)))))], T::Void, "Clip rectangle [x,y,w,h] or null."),
            m("setFilter", vec![("id", T::Int), ("filter", T::Optional(Box::new(T::Map)))], T::Void,
                "Siglus-style 2D filter {mono, invert, bright, dark, color_rate, color, add, tone_curve?:{asset,row,saturation}, mask?:{asset,gray_alpha}, alpha_test, distortion?:{kind:'mosaic'|'raster'|'radial_blur'|'threshold_fade',...}} or null."),
            m("transition", vec![("transition", T::Map)], T::Void,
                "Screen transition from the last presented frame: {effect:{kind:'crossfade'|'mask'{asset,fade}|'slide'{direction}|'mosaic'{size}|'ordered_dither'{cell:[x,y],diagonal?,steps}|'siglus'{code,options?,mask?}}, duration_ms, easing?:'linear'|'ease_in'|'ease_out'|'smooth'}."),
            m("endTransition", vec![], T::Void, "Finish this client's transition now (skip)."),
        ],
    })
}
impl Class for LayersClass {
    fn info(&self) -> &ClassInfo {
        layers_info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        let s = cx.state()?;
        if p == "activeTransition" {
            let layers = &s.runtime.host.engine.layers;
            let (Some(t), Some(f)) = (layers.transition(), layers.transition_frame()) else {
                return Ok(Value::Null);
            };
            return Ok(Value::from_json(serde_json::json!({
                "code": f.code,
                "progress": f.progress,
                "elapsed_ms": t.elapsed_ms,
                "duration_ms": t.spec.duration_ms,
                "mine": t.owner == s.client.0,
            })));
        }
        let frame = s
            .runtime
            .host
            .engine
            .layers
            .frame(s.runtime.project.canvas)?;
        let c = s.client.0;
        Ok((frame.iter().filter(|p| p.owner == c && p.sub == 0).count() as u64).into())
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        method: &str,
        args: &[Value],
    ) -> Result<Value> {
        if method == "revealStatus" {
            let id = id(args)?;
            let state = cx.state()?;
            return Ok(state
                .runtime
                .host
                .engine
                .layers
                .reveal_status(state.client, id)
                .map(|status| {
                    Value::from_json(serde_json::to_value(status).expect("plain reveal status"))
                })
                .unwrap_or(Value::Null));
        }
        let cmd = match method {
            "put" => LayerCommand::Put {
                layer: json(arg(args, 0, "layer")?, "layer spec")?,
            },
            "remove" => LayerCommand::Remove { id: id(args)? },
            "setTransform" => LayerCommand::SetTransform {
                id: id(args)?,
                transform: json(arg(args, 1, "transform")?, "transform")?,
            },
            "setVisible" => LayerCommand::SetVisible {
                id: id(args)?,
                visible: arg(args, 1, "visible")?.as_bool()?,
            },
            "completeReveal" => LayerCommand::CompleteReveal { id: id(args)? },
            "setOpacity" => LayerCommand::SetOpacity {
                id: id(args)?,
                opacity: u8::try_from(arg(args, 1, "opacity")?.as_i64()?)?,
            },
            "setOrder" => LayerCommand::SetOrder {
                id: id(args)?,
                order: json(arg(args, 1, "order")?, "order")?,
            },
            "setClip" => LayerCommand::SetClip {
                id: id(args)?,
                clip: json(args.get(1).unwrap_or(&Value::Null), "clip")?,
            },
            "setFilter" => LayerCommand::SetFilter {
                id: id(args)?,
                filter: json(args.get(1).unwrap_or(&Value::Null), "filter")?,
            },
            "transition" => LayerCommand::Transition {
                transition: json(arg(args, 0, "transition")?, "transition")?,
            },
            "endTransition" => LayerCommand::EndTransition,
            _ => unreachable!(),
        };
        submit(
            cx,
            ServiceBatch {
                layers: vec![cmd],
                ..Default::default()
            },
        )?;
        Ok(Value::Null)
    }
}

// ---------------------------------------------------------------- Ui
pub struct UiClass;
fn ui_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Ui",
        doc: "Retained widgets (panel/label/button/toggle/slider/text_field) with focus, capture and modal routing.",
        properties: vec![
            PropertyInfo { name: "events", ty: T::List(Box::new(T::Map)), writable: false, doc: "Events produced by the last UI tick: {client, id, kind, value}." },
            PropertyInfo { name: "focus", ty: T::Any, writable: false, doc: "Focused widget or null." },
        ],
        methods: vec![
            m("put", vec![("widget", T::Map)], T::Void, "Insert/replace a widget spec {id, parent?, rect, kind, flow?, text?, ...}."),
            m("remove", vec![("id", T::Int)], T::Void, "Remove a widget and its children."),
            m("setVisible", vec![("id", T::Int), ("visible", T::Bool)], T::Void, "Show/hide."),
            m("setEnabled", vec![("id", T::Int), ("enabled", T::Bool)], T::Void, "Enable/disable input."),
            m("setValue", vec![("id", T::Int), ("value", T::Number)], T::Void, "Toggle/slider value."),
            m("setCursorVisible",vec![("visible",T::Bool)],T::Void,"Set engine cursor presentation visibility."),
            m("setTitle",vec![("text",T::String)],T::Void,"Set engine presentation title."),
            m("focusTextField", vec![("id", T::Int)], T::Void, "Focus an enabled, visible text field owned by this client."),
            m("insertText", vec![("text", T::String)], T::Void, "Append committed single-line Unicode to the focused text field, up to max_length Unicode scalars."),
            m("backspace", vec![("id", T::Int)], T::Void, "Remove the final Unicode scalar from a client-owned text field."),
            m("setText", vec![("id", T::Int), ("text", T::String), ("font", T::Optional(Box::new(T::String)))], T::Void, "Label/button text and font alias."),
        ],
    })
}
impl Class for UiClass {
    fn info(&self) -> &ClassInfo {
        ui_info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        let ui = &cx.state()?.runtime.host.engine.ui;
        Ok(Value::from_json(match p {
            "events" => serde_json::to_value(&ui.events)?,
            "focus" => serde_json::to_value(ui.focus)?,
            _ => unreachable!(),
        }))
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        method: &str,
        args: &[Value],
    ) -> Result<Value> {
        let cmd = match method {
            "put" => UiCommand::Put {
                widget: json(arg(args, 0, "widget")?, "widget spec")?,
            },
            "remove" => UiCommand::Remove { id: id(args)? },
            "setVisible" => UiCommand::SetVisible {
                id: id(args)?,
                visible: arg(args, 1, "visible")?.as_bool()?,
            },
            "setEnabled" => UiCommand::SetEnabled {
                id: id(args)?,
                enabled: arg(args, 1, "enabled")?.as_bool()?,
            },
            "setValue" => UiCommand::SetValue {
                id: id(args)?,
                value: arg(args, 1, "value")?.as_f32()?,
            },
            "setCursorVisible" => UiCommand::SetCursorVisible {
                visible: arg(args, 0, "visible")?.as_bool()?,
            },
            "setTitle" => UiCommand::SetTitle {
                text: arg(args, 0, "text")?.as_str()?.into(),
            },
            "focusTextField" => UiCommand::Focus { id: id(args)? },
            "insertText" => UiCommand::InsertText {
                text: arg(args, 0, "text")?.as_str()?.to_owned(),
            },
            "backspace" => UiCommand::Backspace { id: id(args)? },
            "setText" => UiCommand::SetText {
                id: id(args)?,
                text: arg(args, 1, "text")?.as_str()?.to_owned(),
                font: args
                    .get(2)
                    .filter(|v| !matches!(v, Value::Null))
                    .map(|v| v.as_str().map(str::to_owned))
                    .transpose()?,
            },
            _ => unreachable!(),
        };
        submit(
            cx,
            ServiceBatch {
                ui: vec![cmd],
                ..Default::default()
            },
        )?;
        Ok(Value::Null)
    }
}

// ---------------------------------------------------------------- Audio
pub struct AudioClass;
fn audio_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Audio",
        doc: "Voice channels mixed by the host audio device (BGM/SE/voice are channel conventions).",
        properties: vec![],
        methods: vec![
            m("play", vec![("channel", T::Int), ("voice", T::Map)], T::Void, "Play {asset, looping?, gain?: 0..=1} on a channel, replacing it."),
            m("stop", vec![("channel", T::Int)], T::Void, "Stop a channel."),
            m("playAt", vec![("channel", T::Int), ("voice", T::Map), ("where", T::Map)], T::Void,
                "Positional voice: where {position?: [x,y,z], follow?: Entity, radius?: 20, min_distance?: 1}; gain falls off with camera distance."),
        ],
    })
}
impl Class for AudioClass {
    fn info(&self) -> &ClassInfo {
        audio_info()
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        bail!("Audio has no property {p}")
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        method: &str,
        args: &[Value],
    ) -> Result<Value> {
        let channel = u32::try_from(arg(args, 0, "channel")?.as_i64()?)?;
        if method == "playAt" {
            let w = arg(args, 2, "where")?;
            let follow = match w.field("follow") {
                Some(e) => {
                    let oid = e.as_object()?;
                    Some(
                        cx.state()?
                            .objects
                            .get(oid)?
                            .payload
                            .field("id")
                            .context("entity id")?
                            .as_i64()? as u32,
                    )
                }
                None => None,
            };
            let spatial = crate::systems::Spatial {
                gain: arg(args, 1, "voice")?
                    .field("gain")
                    .map(Value::as_f32)
                    .transpose()?
                    .unwrap_or(1.),
                position: w
                    .field("position")
                    .map(Value::as_vec3)
                    .transpose()?
                    .unwrap_or_default(),
                follow,
                min_distance: w
                    .field("min_distance")
                    .map(Value::as_f32)
                    .transpose()?
                    .unwrap_or(1.),
                radius: w
                    .field("radius")
                    .map(Value::as_f32)
                    .transpose()?
                    .unwrap_or(20.),
                applied: u16::MAX,
            };
            anyhow::ensure!(
                spatial.radius > spatial.min_distance && spatial.min_distance >= 0.,
                "radius must exceed min_distance"
            );
            // Start silent; the spatial system sets the gain this frame.
            let mut voice = arg(args, 1, "voice")?.clone();
            if let Value::Map(m) = &mut voice {
                m.insert("gain".into(), Value::Float(0.));
            }
            self.call(
                cx,
                ObjectId(0),
                "play",
                &[Value::Int(channel.into()), voice],
            )?;
            cx.state()?.audio.voices.insert(channel, spatial);
            return Ok(Value::Null);
        }
        let cmd = match method {
            "play" => {
                let v = arg(args, 1, "voice")?;
                let asset = v.field("asset").context("voice.asset")?.as_str()?;
                let gain = v
                    .field("gain")
                    .map(Value::as_f64)
                    .transpose()?
                    .unwrap_or(1.);
                anyhow::ensure!((0.0..=1.0).contains(&gain), "gain must be 0..=1");
                Command::PlayAudio {
                    channel,
                    voice: AudioVoice {
                        asset: VfsPath::new(Mount::Asset, asset)
                            .map_err(|e| anyhow::anyhow!("{asset}: {e}"))?,
                        looping: v
                            .field("looping")
                            .map(Value::as_bool)
                            .transpose()?
                            .unwrap_or(false),
                        gain_milli: (gain * 1000.).round() as u16,
                    },
                }
            }
            "stop" => Command::StopAudio { channel },
            _ => unreachable!(),
        };
        let s = cx.state()?;
        let client = s.client;
        s.runtime
            .host
            .submit(client, &[cmd])
            .map_err(|e| anyhow::anyhow!("audio: {e}"))?;
        Ok(Value::Null)
    }
}

// ---------------------------------------------------------------- Save
pub struct SaveClass;
fn save_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Save",
        doc: "Whole-runtime snapshots (world, layers, UI, clocks, every VM) in the client's save mount.",
        properties: vec![],
        methods: vec![
            m("save", vec![("path", T::String)], T::Int, "Write a validated snapshot; returns its size in bytes."),
            m("load", vec![("path", T::String)], T::Void, "Transactionally restore a snapshot (all-or-nothing). Entity objects from before are revalidated by generation."),
            m("exists", vec![("path", T::String)], T::Bool, "Whether a snapshot file exists."),
        ],
    })
}
impl Class for SaveClass {
    fn info(&self) -> &ClassInfo {
        save_info()
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        bail!("Save has no property {p}")
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        method: &str,
        args: &[Value],
    ) -> Result<Value> {
        let path = arg(args, 0, "path")?.as_str()?;
        let p = VfsPath::new(Mount::Save, path).map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
        let s = cx.state()?;
        let client = s.client;
        match method {
            "save" => {
                let bytes = s.runtime.snapshot()?;
                s.runtime
                    .host
                    .fs
                    .write_atomic(client, &p, &bytes)
                    .map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
                Ok((bytes.len() as u64).into())
            }
            "load" => {
                let bytes = s
                    .runtime
                    .host
                    .fs
                    .read(client, &p)
                    .map_err(|e| anyhow::anyhow!("{path}: {e}"))?;
                s.runtime.restore(&bytes)?;
                Ok(Value::Null)
            }
            "exists" => Ok(s.runtime.host.fs.read(client, &p).is_ok().into()),
            _ => unreachable!(),
        }
    }
}

// ---------------------------------------------------------------- Input
/// Named action bindings (session configuration, not saved).
#[derive(Default)]
pub struct ActionMap {
    pub bindings: std::collections::BTreeMap<String, u64>,
    pub previous_down: u64,
    pub previous_pointer: (i32, i32),
}
pub struct InputClass;
fn input_info() -> &'static ClassInfo {
    static I: OnceLock<ClassInfo> = OnceLock::new();
    I.get_or_init(|| ClassInfo {
        name: "Input",
        doc: "Action maps over keyboard/mouse buttons: bind('jump', ['space']), isDown, pressed, axis.",
        properties: vec![
            PropertyInfo { name: "pointer", ty: T::List(Box::new(T::Int)), writable: false, doc: "Pointer position in canvas pixels." },
            PropertyInfo { name: "pointerDelta", ty: T::List(Box::new(T::Int)), writable: false, doc: "Pointer motion since the previous frame." },
            PropertyInfo { name: "keys", ty: T::List(Box::new(T::String)), writable: false, doc: "Bindable key names." },
        ],
        methods: vec![
            m("bind", vec![("action", T::String), ("keys", T::List(Box::new(T::String)))], T::Void, "Bind an action to one or more keys (replaces previous)."),
            m("isDown", vec![("action", T::String)], T::Bool, "Any bound key held this frame."),
            m("pressed", vec![("action", T::String)], T::Bool, "Any bound key went down this frame."),
            m("axis", vec![("negative", T::String), ("positive", T::String)], T::Number, "-1, 0 or 1 from two actions."),
        ],
    })
}
impl Class for InputClass {
    fn info(&self) -> &ClassInfo {
        input_info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        let s = cx.state()?;
        let i = s.input;
        Ok(match p {
            "pointer" => Value::List(vec![
                Value::Int(i.pointer_x.into()),
                Value::Int(i.pointer_y.into()),
            ]),
            "pointerDelta" => Value::List(vec![
                Value::Int((i.pointer_x - s.actions.previous_pointer.0).into()),
                Value::Int((i.pointer_y - s.actions.previous_pointer.1).into()),
            ]),
            "keys" => Value::List(
                crysirror_core::service::buttons::NAMES
                    .iter()
                    .map(|(n, _)| Value::Str((*n).into()))
                    .collect(),
            ),
            _ => unreachable!(),
        })
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        method: &str,
        args: &[Value],
    ) -> Result<Value> {
        let s = cx.state()?;
        let mask = |s: &crate::EngineState, a: &str| -> Result<u64> {
            s.actions
                .bindings
                .get(a)
                .copied()
                .with_context(|| format!("action '{a}' is not bound"))
        };
        let down = s.input.buttons_down | s.input.buttons_pressed;
        Ok(match method {
            "bind" => {
                let action = arg(args, 0, "action")?.as_str()?.to_owned();
                let Value::List(keys) = arg(args, 1, "keys")? else {
                    bail!("keys must be a list")
                };
                let mut bits = 0;
                for k in keys {
                    let k = k.as_str()?;
                    bits |= crysirror_core::service::buttons::NAMES
                        .iter()
                        .find(|(n, _)| *n == k)
                        .map(|(_, b)| *b)
                        .with_context(|| format!("unknown key '{k}'"))?;
                }
                s.actions.bindings.insert(action, bits);
                Value::Null
            }
            "isDown" => (down & mask(s, arg(args, 0, "action")?.as_str()?)? != 0).into(),
            "pressed" => {
                let m = mask(s, arg(args, 0, "action")?.as_str()?)?;
                ((s.input.buttons_pressed & m != 0)
                    || (down & m != 0 && s.actions.previous_down & m == 0))
                    .into()
            }
            "axis" => {
                let n = down & mask(s, arg(args, 0, "negative")?.as_str()?)? != 0;
                let p = down & mask(s, arg(args, 1, "positive")?.as_str()?)? != 0;
                Value::Float(f64::from(i32::from(p) - i32::from(n)))
            }
            _ => unreachable!(),
        })
    }
}
