// SPDX-License-Identifier: MPL-2.0
//! Shared Audio class and derived PCM playback. SceneState channels remain the
//! canonical legacy contract; all channel and pooled voices feed one mixer.
use crate::{
    context::EngineContext,
    object::{Class, ClassInfo, MethodInfo, ObjectId, PropertyInfo, TypeHint as T},
    value::Value,
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_audio::{
    decoder::{self, AudioSource, Clip, StereoFrame, Stream},
    mixer::{
        Bus, Ducking, Listener, Mixer, RandomContainer, Spatial, Variant, VoiceHandle, VoiceSpec,
    },
};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::FileSystem,
    state::{ClientId, ObjectId as ChannelId},
};
use crysirror_scene::world::EntityHandle;
use glam::Vec3;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

struct Legacy {
    epoch: u64,
    handle: VoiceHandle,
}
#[derive(Clone)]
struct Attachment {
    handle: VoiceHandle,
    follow: Option<EntityHandle>,
    spatial: Spatial,
    raycast: bool,
}
pub struct AudioRuntime {
    pub mixer: Mixer,
    legacy: BTreeMap<ChannelId, Legacy>,
    routes: BTreeMap<ChannelId, Bus>,
    clips: BTreeMap<(u32, String), Clip>,
    clip_bytes: usize,
    attachments: BTreeMap<(u32, u32, u64), Attachment>,
    music: BTreeMap<(u32, String), VoiceHandle>,
    listeners: BTreeMap<u32, Listener>,
    restored: Option<u64>,
    remainder: u64,
    output: Vec<StereoFrame>,
}
impl Default for AudioRuntime {
    fn default() -> Self {
        Self {
            mixer: Default::default(),
            legacy: BTreeMap::new(),
            routes: BTreeMap::new(),
            clips: BTreeMap::new(),
            clip_bytes: 0,
            attachments: BTreeMap::new(),
            music: BTreeMap::new(),
            listeners: BTreeMap::new(),
            restored: None,
            remainder: 0,
            output: Vec::new(),
        }
    }
}
impl AudioRuntime {
    /// Last frame's PCM, drained by one host device/export consumer.
    pub fn take_output(&mut self) -> Vec<StereoFrame> {
        std::mem::take(&mut self.output)
    }
    pub fn reset_playback(&mut self) {
        self.mixer.clear();
        self.legacy.clear();
        self.attachments.clear();
        self.music.clear();
        self.output.clear();
        self.remainder = 0;
    }
}
fn key(h: VoiceHandle) -> (u32, u32, u64) {
    (h.client, h.slot, h.generation)
}
fn source(cx: &mut EngineContext, client: u32, path: &str) -> Result<Box<dyn AudioSource>> {
    let p = VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
    ensure!(p.relative() == path, "non-canonical audio asset path");
    let s = cx.state()?;
    let extension = path.rsplit_once('.').map(|(_, x)| x);
    let stream = match s
        .runtime
        .host
        .fs
        .open_asset_file(ClientId(client), &p)
        .map_err(|e| anyhow::anyhow!("audio asset {path}: {e}"))?
    {
        Some(file) => Stream::open(Box::new(file), extension)?,
        None => {
            let bytes = s
                .runtime
                .host
                .fs
                .read(ClientId(client), &p)
                .map_err(|e| anyhow::anyhow!("audio provider {path}: {e}"))?;
            Stream::from_bytes(Arc::from(bytes), extension)?
        }
    };
    Ok(Box::new(stream))
}
fn clip(cx: &mut EngineContext, client: u32, path: &str) -> Result<Clip> {
    let k = (client, path.to_owned());
    if let Some(clip) = cx.state()?.mixer_audio.clips.get(&k) {
        return Ok(clip.clone());
    }
    let clip = decoder::decode_clip(source(cx, client, path)?)?;
    let bytes = clip.frames() * 8;
    let s = cx.state()?;
    ensure!(
        s.mixer_audio.clip_bytes + bytes <= 128 * 1024 * 1024,
        "SFX cache exceeds 128 MiB; use streaming music"
    );
    s.mixer_audio.clip_bytes += bytes;
    s.mixer_audio.clips.insert(k, clip.clone());
    Ok(clip)
}
/// Called after world/native VM updates and late update, once per App frame.
pub fn update(cx: &mut EngineContext, dt_ms: u64) -> Result<()> {
    ensure!(dt_ms <= 60000, "audio frame exceeds 60 seconds");
    let restored = cx.state()?.runtime.restored;
    {
        let a = &mut cx.state()?.mixer_audio;
        if a.restored.is_some_and(|n| n != restored) {
            a.reset_playback()
        }
        a.restored = Some(restored);
    }
    let current = {
        let s = cx.state()?;
        s.runtime
            .host
            .scene
            .voices()
            .iter()
            .map(|(id, v)| (*id, v.clone(), s.runtime.host.scene.voice_epoch(*id)))
            .collect::<Vec<_>>()
    };
    let stale = {
        let s = cx.state()?;
        s.mixer_audio
            .legacy
            .keys()
            .filter(|id| !current.iter().any(|(i, _, _)| i == *id))
            .copied()
            .collect::<Vec<_>>()
    };
    for id in stale {
        let a = &mut cx.state()?.mixer_audio;
        if let Some(old) = a.legacy.remove(&id) {
            if a.mixer.is_playing(id.client.0, old.handle) {
                a.mixer.stop(id.client.0, old.handle, 0.)?
            }
        }
        a.routes.remove(&id);
    }
    for (id, v, epoch) in current {
        let change = cx
            .state()?
            .mixer_audio
            .legacy
            .get(&id)
            .is_none_or(|old| old.epoch != epoch);
        if change {
            let src = source(cx, id.client.0, v.asset.relative())?;
            let a = &mut cx.state()?.mixer_audio;
            if let Some(old) = a.legacy.remove(&id) {
                if a.mixer.is_playing(id.client.0, old.handle) {
                    a.mixer.stop(id.client.0, old.handle, 0.)?
                }
            }
            let bus = a.routes.get(&id).copied().unwrap_or(Bus::Sfx);
            let handle = a.mixer.play(
                id.client.0,
                src,
                VoiceSpec {
                    bus,
                    gain: v.gain_milli as f32 / 1000.,
                    looping: v.looping,
                    ..Default::default()
                },
            )?;
            a.legacy.insert(id, Legacy { epoch, handle });
        } else {
            let a = &mut cx.state()?.mixer_audio;
            let handle = a.legacy[&id].handle;
            if a.mixer.is_playing(id.client.0, handle) {
                a.mixer
                    .set_gain(id.client.0, handle, v.gain_milli as f32 / 1000.)?
            }
        }
    }
    let (camera, clients) = {
        let s = cx.state()?;
        let camera = s
            .runtime
            .host
            .engine
            .world
            .view
            .camera
            .clone()
            .or_else(|| s.runtime.project.scene3d.as_ref().map(|v| v.camera.clone()));
        (camera, s.runtime.project.clients.len())
    };
    let auto = camera
        .map(|c| Listener {
            position: c.eye,
            forward: (Vec3::from_array(c.target) - Vec3::from_array(c.eye)).to_array(),
            up: c.up,
        })
        .unwrap_or_default();
    for client in 1..=clients as u32 {
        let a = &mut cx.state()?.mixer_audio;
        let listener = a.listeners.get(&client).copied().unwrap_or(auto);
        a.mixer.set_listener(client, listener)?;
    }
    let attachments = cx
        .state()?
        .mixer_audio
        .attachments
        .values()
        .cloned()
        .collect::<Vec<_>>();
    if attachments.iter().any(|v| v.raycast) {
        let s = cx.state()?;
        s.runtime.physics
            .sync(&s.runtime.host.engine.world, &mut s.runtime.host.fs)?;
    }
    for mut a in attachments {
        if !cx
            .state()?
            .mixer_audio
            .mixer
            .is_playing(a.handle.client, a.handle)
        {
            cx.state()?.mixer_audio.attachments.remove(&key(a.handle));
            continue;
        }
        if let Some(h) = a.follow {
            let s = cx.state()?;
            if s.runtime.host.engine.world.entity(h).is_err() {
                s.mixer_audio.mixer.stop(h.client, a.handle, 0.)?;
                s.mixer_audio.attachments.remove(&key(a.handle));
                continue;
            }
            let position = s
                .runtime
                .host
                .engine
                .world
                .global_matrix(h)?
                .w_axis
                .truncate()
                + Vec3::from_array(a.spatial.position);
            a.spatial.position = position.to_array();
        }
        if a.raycast {
            let s = cx.state()?;
            let listener = s
                .mixer_audio
                .listeners
                .get(&a.handle.client)
                .copied()
                .unwrap_or(auto);
            let origin = Vec3::from_array(listener.position);
            let delta = Vec3::from_array(a.spatial.position) - origin;
            let distance = delta.length();
            a.spatial.occlusion = if distance > 0.05
                && s.runtime.physics
                    .raycast(
                        origin,
                        delta,
                        distance - 0.02,
                        a.follow.map(|h| (h.client, h.id)),
                    )
                    .is_some_and(|hit| hit.distance > 0.05)
            {
                1.
            } else {
                0.
            };
        }
        cx.state()?
            .mixer_audio
            .mixer
            .set_spatial(a.handle.client, a.handle, Some(a.spatial))?;
    }
    let s = cx.state()?;
    let a = &mut s.mixer_audio;
    a.attachments
        .retain(|_, v| a.mixer.is_playing(v.handle.client, v.handle));
    a.music
        .retain(|(client, _), h| a.mixer.is_playing(*client, *h));
    let numerator = dt_ms
        .checked_mul(a.mixer.sample_rate() as u64)
        .and_then(|n| n.checked_add(a.remainder))
        .context("audio clock overflow")?;
    let count = (numerator / 1000) as usize;
    a.remainder = numerator % 1000;
    a.output.resize(count, [0.; 2]);
    for block in a.output.chunks_mut(4096) {
        a.mixer.render(block)?
    }
    let native = s.runtime.host.take_pcm_output();
    if native.len() > a.output.len() { a.output.resize(native.len(), [0.; 2]); }
    for (out, pcm) in a.output.iter_mut().zip(native) {
        out[0] += pcm[0];
        out[1] += pcm[1];
    }
    Ok(())
}

pub struct AudioClass;
fn method(
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
fn info() -> &'static ClassInfo {
    static INFO: OnceLock<ClassInfo> = OnceLock::new();
    INFO.get_or_init(||ClassInfo{name:"Audio",doc:"Scoped voice pool, streaming music and deterministic CPU mixing; classic channels share this output.",properties:vec![PropertyInfo{name:"sampleRate",ty:T::Int,writable:false,doc:"Mixer output frames per second."}],methods:vec![
        method("play",vec![("channel",T::Int),("voice",T::Map)],T::Void,"Classic {asset,looping?,gain?,bus?} channel, preserving SceneState trace/save semantics."),
        method("stop",vec![("channel",T::Int)],T::Void,"Stop a classic channel."),
        method("playAt",vec![("channel",T::Int),("voice",T::Map),("where",T::Map)],T::Void,"Legacy distance-gain API. Use playSfx with spatial/follow for stereo 3D and occlusion."),
        method("playSfx",vec![("voice",T::Map)],T::Map,"{asset,bus?,gain?,rate?,pitch_semitones?,priority?,looping?,loop_points?:{start,end},fade_in_ms?,paused?,spatial?,follow?:Entity,raycast_occlusion?}; returns scoped generation handle."),
        method("playRandom",vec![("name",T::String),("container",T::Map),("voice",T::Map)],T::Map,"Seeded weighted variants [{asset,weight?}], gain:[min,max], pitch_semitones:[min,max], avoid_repeat; optional VoiceSpec."),
        method("playMusic",vec![("name",T::String),("voice",T::Map),("crossfade_ms",T::Number)],T::Map,"Stream music by name; optional sample-frame loop_points; crossfade_ms fades old/new together. Looping defaults true."),
        method("stopVoice",vec![("handle",T::Map),("fade_ms",T::Number)],T::Void,"Stop immediately or sample-accurate fade out."),
        method("pauseVoice",vec![("handle",T::Map),("paused",T::Bool)],T::Void,"Pause without advancing the source or fade clock."),
        method("seekVoice",vec![("handle",T::Map),("frame",T::Int)],T::Void,"Seek exactly to a decoded source frame."),
        method("setVoice",vec![("handle",T::Map),("options",T::Map)],T::Void,"Update {gain?,pan?:0..1,fade_ms?,rate?,pitch_semitones?,paused?,spatial?}. Fade applies to gain/pan. Pitch/rate change resampling speed."),
        method("voice",vec![("handle",T::Map)],T::Map,"{playing,position_seconds?}; stale/stolen/ended handles are inactive."),
        method("setBus",vec![("bus",T::String),("settings",T::Map)],T::Void,"master/music/sfx/voice/ambient/ui {gain:0..4,muted:boolean}."),
        method("bus",vec![("bus",T::String)],T::Map,"Read this client's bus settings."),
        method("setDucking",vec![("config",T::Map)],T::Void,"Dialogue-driven music ducking {gain,attack_ms,release_ms}."),
        method("setVoiceLimit",vec![("limit",T::Int)],T::Void,"Set this client's pool limit within the host's global budget; stop voices before lowering below the active count."),
        method("setSeed",vec![("seed",T::Int)],T::Void,"Reset this client's deterministic variant generator and repeat history."),
        method("setListener",vec![("listener",T::Any)],T::Void,"{position,forward,up}; null restores automatic camera listener."),
        method("stats",vec![],T::Map,"Mixer active/stolen/rejected voices, rendered frames and unclipped peak."),
    ]})
}
fn arg<'a>(args: &'a [Value], i: usize, name: &str) -> Result<&'a Value> {
    args.get(i).with_context(|| format!("missing {name}"))
}
fn parse<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T> {
    serde_json::from_value(v.to_json()).map_err(Into::into)
}
fn bus(v: &Value) -> Result<Bus> {
    parse(v)
}
fn handle(cx: &mut EngineContext, args: &[Value]) -> Result<VoiceHandle> {
    let h: VoiceHandle = parse(arg(args, 0, "voice handle")?)?;
    ensure!(
        h.client == cx.state()?.client.0,
        "audio handle belongs to another client"
    );
    Ok(h)
}
fn entity(cx: &mut EngineContext, value: &Value) -> Result<EntityHandle> {
    let oid = value.as_object()?;
    let s = cx.state()?;
    let e = s.objects.get(oid)?;
    ensure!(e.class == "Entity", "follow requires Entity");
    let h = EntityHandle {
        client: s.client.0,
        id: u32::try_from(e.payload.field("id").context("entity id")?.as_i64()?)?,
        generation: u32::try_from(
            e.payload
                .field("generation")
                .context("entity generation")?
                .as_i64()?,
        )?,
    };
    s.runtime.host.engine.world.entity(h)?;
    Ok(h)
}
fn voice_options(
    cx: &mut EngineContext,
    value: &Value,
    asset_required: bool,
) -> Result<(Option<String>, VoiceSpec, Option<Attachment>)> {
    let Value::Map(map) = value else {
        bail!("voice must be an object")
    };
    let mut map = map.clone();
    let asset = map
        .remove("asset")
        .map(|v| v.as_str().map(str::to_owned))
        .transpose()?;
    ensure!(!asset_required || asset.is_some(), "voice.asset required");
    let follow = map
        .remove("follow")
        .filter(|v| !matches!(v, Value::Null))
        .map(|v| entity(cx, &v))
        .transpose()?;
    let raycast = map
        .remove("raycast_occlusion")
        .map(|v| v.as_bool())
        .transpose()?
        .unwrap_or(false);
    let mut spec: VoiceSpec = parse(&Value::Map(map))?;
    if follow.is_some() && spec.spatial.is_none() {
        spec.spatial = Some(Spatial::default())
    }
    spec.validate()?;
    ensure!(
        !raycast || spec.spatial.is_some(),
        "raycast_occlusion requires spatial audio"
    );
    let attachment = spec.spatial.as_ref().map(|spatial| Attachment {
        handle: VoiceHandle {
            client: 0,
            slot: 0,
            generation: 0,
        },
        follow,
        spatial: spatial.clone(),
        raycast,
    });
    Ok((asset, spec, attachment))
}
fn attach(
    cx: &mut EngineContext,
    handle: VoiceHandle,
    attachment: Option<Attachment>,
) -> Result<()> {
    if let Some(mut a) = attachment {
        a.handle = handle;
        let runtime = &mut cx.state()?.mixer_audio;
        runtime
            .attachments
            .retain(|_, a| runtime.mixer.is_playing(a.handle.client, a.handle));
        runtime.attachments.insert(key(handle), a);
    }
    Ok(())
}
impl Class for AudioClass {
    fn info(&self) -> &ClassInfo {
        info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        match p {
            "sampleRate" => Ok(Value::Int(
                cx.state()?.mixer_audio.mixer.sample_rate().into(),
            )),
            _ => bail!("Audio has no property {p}"),
        }
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        this: ObjectId,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        let client = cx.state()?.client.0;
        match name {
            "play" | "stop" | "playAt" => {
                let ch = u32::try_from(arg(args, 0, "channel")?.as_i64()?)?;
                let route = if name != "stop" {
                    arg(args, 1, "voice")?.field("bus").map(bus).transpose()?
                } else {
                    None
                };
                if let Some(route) = route {
                    ensure!(route != Bus::Master, "master is not a voice bus")
                }
                crate::services::AudioClass.call(cx, this, name, args)?;
                if let Some(route) = route {
                    cx.state()?.mixer_audio.routes.insert(
                        ChannelId {
                            client: ClientId(client),
                            local: ch,
                        },
                        route,
                    );
                }
                return Ok(Value::Null);
            }
            "playSfx" => {
                let (asset, spec, a) = voice_options(cx, arg(args, 0, "voice")?, true)?;
                let clip = clip(cx, client, &asset.unwrap())?;
                let h = cx
                    .state()?
                    .mixer_audio
                    .mixer
                    .play(client, clip.source(), spec)?;
                attach(cx, h, a)?;
                return Ok(Value::from_json(serde_json::to_value(h)?));
            }
            "playRandom" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Item {
                    asset: String,
                    #[serde(default = "one")]
                    weight: u32,
                }
                fn one() -> u32 {
                    1
                }
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Container {
                    variants: Vec<Item>,
                    #[serde(default = "gain_range")]
                    gain: [f32; 2],
                    #[serde(default)]
                    pitch_semitones: [f32; 2],
                    #[serde(default)]
                    avoid_repeat: bool,
                }
                fn gain_range() -> [f32; 2] {
                    [1.; 2]
                }
                let name = arg(args, 0, "container name")?.as_str()?.to_owned();
                let c: Container = parse(arg(args, 1, "container")?)?;
                ensure!(
                    !c.variants.is_empty() && c.variants.len() <= 256,
                    "random variant count"
                );
                let mut variants = Vec::new();
                for item in c.variants {
                    variants.push(Variant {
                        clip: clip(cx, client, &item.asset)?,
                        weight: item.weight,
                    });
                }
                let container = RandomContainer {
                    variants,
                    gain: c.gain,
                    pitch_semitones: c.pitch_semitones,
                    avoid_repeat: c.avoid_repeat,
                };
                let options = args
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| Value::Map(BTreeMap::new()));
                let (asset, spec, a) = voice_options(cx, &options, false)?;
                ensure!(asset.is_none(), "random voice asset comes from variants");
                let (h, selected) = cx
                    .state()?
                    .mixer_audio
                    .mixer
                    .play_random(client, &name, &container, spec)?;
                attach(cx, h, a)?;
                return Ok(Value::from_json(
                    serde_json::json!({"handle":h,"variant":selected.index,"gain":selected.gain,"pitch_semitones":selected.pitch_semitones}),
                ));
            }
            "playMusic" => {
                let name = arg(args, 0, "music name")?.as_str()?.to_owned();
                ensure!(!name.is_empty() && name.len() <= 256, "music name");
                let mut value = arg(args, 1, "voice")?.clone();
                if let Value::Map(map) = &mut value {
                    map.entry("looping".into()).or_insert(Value::Bool(true));
                }
                let (asset, mut spec, a) = voice_options(cx, &value, true)?;
                ensure!(
                    a.is_none(),
                    "music is non-spatial; use playSfx for positioned sources"
                );
                spec.bus = Bus::Music;
                let fade = args.get(2).map(Value::as_f32).transpose()?.unwrap_or(0.);
                ensure!(
                    fade.is_finite() && (0.0..=600000.).contains(&fade),
                    "music crossfade duration"
                );
                spec.fade_in_ms = fade;
                spec.validate()?;
                let src = source(cx, client, &asset.unwrap())?;
                let a = &mut cx.state()?.mixer_audio;
                let h = a.mixer.play(client, src, spec)?;
                if let Some(old) = a.music.insert((client, name), h) {
                    if a.mixer.is_playing(client, old) {
                        a.mixer.stop(client, old, fade)?
                    }
                }
                return Ok(Value::from_json(serde_json::to_value(h)?));
            }
            "stopVoice" => {
                let h = handle(cx, args)?;
                let fade = args.get(1).map(Value::as_f32).transpose()?.unwrap_or(0.);
                cx.state()?.mixer_audio.mixer.stop(client, h, fade)?;
            }
            "pauseVoice" => {
                let h = handle(cx, args)?;
                let paused = args.get(1).map(Value::as_bool).transpose()?.unwrap_or(true);
                cx.state()?.mixer_audio.mixer.pause(client, h, paused)?;
            }
            "seekVoice" => {
                let h = handle(cx, args)?;
                let frame = u64::try_from(arg(args, 1, "source frame")?.as_i64()?)?;
                cx.state()?.mixer_audio.mixer.seek(client, h, frame)?;
            }
            "setVoice" => {
                #[derive(Deserialize, Default)]
                #[serde(default, deny_unknown_fields)]
                struct Options {
                    gain: Option<f32>,
                    pan: Option<f32>,
                    fade_ms: f32,
                    rate: Option<f32>,
                    pitch_semitones: Option<f32>,
                    paused: Option<bool>,
                    spatial: Option<Spatial>,
                }
                let h = handle(cx, args)?;
                let opts: Options = parse(arg(args, 1, "voice options")?)?;
                ensure!(opts.fade_ms.is_finite() && (0.0..=600000.).contains(&opts.fade_ms), "invalid voice fade");
                ensure!(opts.pan.is_none_or(|p| p.is_finite() && (0.0..=1.).contains(&p)), "invalid voice pan");
                let mut spec = cx.state()?.mixer_audio.mixer.spec(client, h)?;
                if let Some(gain) = opts.gain {
                    spec.gain = gain
                }
                if let Some(rate) = opts.rate {
                    spec.rate = rate
                }
                if let Some(pitch) = opts.pitch_semitones {
                    spec.pitch_semitones = pitch
                }
                if let Some(spatial) = &opts.spatial {
                    spec.spatial = Some(spatial.clone())
                }
                spec.validate()?;
                let a = &mut cx.state()?.mixer_audio;
                a.mixer.position(client, h)?;
                if let Some(gain) = opts.gain {
                    a.mixer.fade_gain(client, h, gain, opts.fade_ms)?
                }
                if let Some(pan) = opts.pan { a.mixer.set_pan(client, h, pan, opts.fade_ms)?; }
                if opts.rate.is_some() || opts.pitch_semitones.is_some() {
                    a.mixer
                        .set_rate(client, h, spec.rate, spec.pitch_semitones)?
                }
                if let Some(paused) = opts.paused {
                    a.mixer.pause(client, h, paused)?
                }
                if let Some(spatial) = opts.spatial {
                    a.mixer.set_spatial(client, h, Some(spatial.clone()))?;
                    if let Some(attached) = a.attachments.get_mut(&key(h)) {
                        attached.spatial = spatial;
                    }
                }
            }
            "voice" => {
                let h = handle(cx, args)?;
                let mixer = &cx.state()?.mixer_audio.mixer;
                let playing = mixer.is_playing(client, h);
                return Ok(Value::from_json(if playing {
                    serde_json::json!({"playing":true,"position_seconds":mixer.position(client,h)?})
                } else {
                    serde_json::json!({"playing":false})
                }));
            }
            "setBus" => {
                let b = bus(arg(args, 0, "bus")?)?;
                #[derive(Deserialize, Default)]
                #[serde(default, deny_unknown_fields)]
                struct Change {
                    gain: Option<f32>,
                    muted: Option<bool>,
                }
                let change: Change = parse(arg(args, 1, "bus settings")?)?;
                let mut settings = cx.state()?.mixer_audio.mixer.bus(client, b);
                if let Some(gain) = change.gain {
                    settings.gain = gain
                }
                if let Some(muted) = change.muted {
                    settings.muted = muted
                }
                cx.state()?.mixer_audio.mixer.set_bus(client, b, settings)?;
                cx.state()?.runtime.host.channel_audio.mixer.set_bus(client, b, settings)?;
            }
            "bus" => {
                let b = bus(arg(args, 0, "bus")?)?;
                return Ok(Value::from_json(serde_json::to_value(
                    cx.state()?.mixer_audio.mixer.bus(client, b),
                )?));
            }
            "setDucking" => {
                let config: Ducking = parse(arg(args, 0, "ducking")?)?;
                cx.state()?.mixer_audio.mixer.set_ducking(client, config)?;
                cx.state()?.runtime.host.channel_audio.set_ducking(client, config)?;
            }
            "setVoiceLimit" => {
                let limit = usize::try_from(arg(args, 0, "voice limit")?.as_i64()?)?;
                cx.state()?
                    .mixer_audio
                    .mixer
                    .set_client_limit(client, limit)?;
            }
            "setSeed" => {
                let seed = u64::try_from(arg(args, 0, "random seed")?.as_i64()?)?;
                cx.state()?.mixer_audio.mixer.set_seed(client, seed)?;
            }
            "setListener" => {
                let value = arg(args, 0, "listener")?;
                if matches!(value, Value::Null) {
                    cx.state()?.mixer_audio.listeners.remove(&client);
                } else {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct L {
                        position: [f32; 3],
                        forward: [f32; 3],
                        up: [f32; 3],
                    }
                    let v: L = parse(value)?;
                    let l = Listener {
                        position: v.position,
                        forward: v.forward,
                        up: v.up,
                    };
                    let a = &mut cx.state()?.mixer_audio;
                    a.mixer.set_listener(client, l)?;
                    a.listeners.insert(client, l);
                }
            }
            "stats" => {
                return Ok(Value::from_json(serde_json::to_value(
                    cx.state()?.mixer_audio.mixer.stats(),
                )?))
            }
            _ => bail!("Audio has no method {name}"),
        }
        Ok(Value::Null)
    }
}
