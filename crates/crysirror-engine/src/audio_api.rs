// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared Audio class and derived PCM playback. SceneState channels remain the
//! canonical legacy contract; all channel and pooled voices feed one mixer.
use crate::{
    context::EngineContext,
    object::{Class, ClassInfo, MethodInfo, ObjectId, PropertyInfo, TypeHint as T},
    value::Value,
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_audio::{
    decoder::{self, AudioSource, Clip, StereoFrame},
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

#[derive(Clone)]
pub(crate) struct Attachment {
    pub(crate) handle: VoiceHandle,
    pub(crate) follow: Option<EntityHandle>,
    pub(crate) spatial: Spatial,
    pub(crate) raycast: bool,
    pub(crate) acoustics: Option<crysirror_acoustics::AcousticSource>,
    pub(crate) binaural: bool,
    pub(crate) reflections: Option<crysirror_acoustics::reflections::ReflectionTrackingSpec>,
}
pub struct AudioRuntime {
    pub(crate) frame_pending: bool,
    pub mixer: Mixer,
    pub scene_audio: crysirror_audio::scene::SceneAudio,
    pub(crate) clips: BTreeMap<(u32, String), Clip>,
    pub(crate) clip_bytes: usize,
    pub(crate) jobs: crate::audio_async::AudioJobs,
    pub(crate) attachments: BTreeMap<(u32, u32, u64), Attachment>,
    pub acoustics: crate::acoustics_api::AcousticsRuntime,
    pub binaural: crate::binaural_api::BinauralRuntime,
    pub reverb: crate::reverb_api::ReverbRuntime,
    pub reflections: crate::reflections_api::ReflectionsRuntime,
    pub(crate) music: BTreeMap<(u32, String), VoiceHandle>,
    listeners: BTreeMap<u32, Listener>,
    restored: Option<u64>,
    remainder: u64,
    output: Vec<StereoFrame>,
}
impl Default for AudioRuntime {
    fn default() -> Self {
        Self {
            frame_pending: false,
            mixer: Default::default(),
            scene_audio: Default::default(),
            clips: BTreeMap::new(),
            clip_bytes: 0,
            jobs: Default::default(),
            attachments: BTreeMap::new(),
            acoustics: Default::default(),
            binaural: Default::default(),
            reverb: Default::default(),
            reflections: Default::default(),
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
        self.frame_pending = false;
        self.jobs.reset();
        self.mixer.clear();
        self.scene_audio.reset(&mut self.mixer);
        self.attachments.clear();
        self.music.clear();
        self.output.clear();
        self.remainder = 0;
        self.acoustics.reset();
        self.binaural.reset();
        self.reverb.reset();
        self.reflections.reset();
    }
}
pub(crate) fn sync_generation(state: &mut crate::EngineState) {
    let generation = state.runtime.restored;
    let a = &mut state.mixer_audio;
    if a.restored.is_some_and(|old| old != generation) {
        a.reset_playback();
    }
    a.restored = Some(generation);
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
        Some(file) => decoder::open_reader(Box::new(file), extension)?,
        None => {
            let bytes = s
                .runtime
                .host
                .fs
                .read(ClientId(client), &p)
                .map_err(|e| anyhow::anyhow!("audio provider {path}: {e}"))?;
            decoder::open_memory(Arc::from(bytes), extension)?
        }
    };
    Ok(stream)
}
fn clip(cx: &mut EngineContext, client: u32, path: &str) -> Result<Clip> {
    let k = (client, path.to_owned());
    if let Some(clip) = cx.state()?.mixer_audio.clips.get(&k) {
        return Ok(clip.clone());
    }
    ensure!(
        !cx.state()?.runtime.audio_is_nonblocking(),
        "uncached SFX requires Audio.playSfxAsync or preloadSfxAsync in a realtime host"
    );
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
pub(crate) fn prepare(cx: &mut EngineContext, dt_ms: u64) -> Result<usize> {
    ensure!(dt_ms <= 60000, "audio frame exceeds 60 seconds");
    sync_generation(cx.state()?);
    crate::audio_async::poll(cx.state()?)?;
    crate::acoustics_api::poll(cx.state()?)?;
    crate::binaural_api::poll(cx.state()?)?;
    crate::reverb_api::poll(cx.state()?)?;
    crate::reflections_api::poll(cx.state()?)?;
    {
        let s = cx.state()?;
        let runtime = &mut s.runtime;
        let pool = runtime.assets.io_tasks();
        let live = runtime.audio_is_nonblocking();
        let a = &mut s.mixer_audio;
        a.scene_audio.sync(
            &mut a.mixer,
            &runtime.host.scene,
            runtime.restored,
            &pool,
            live,
            |client| {
                runtime
                    .host
                    .fs
                    .storage(client)
                    .map_err(|e| anyhow::anyhow!("{e}"))
            },
        )?;
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
        s.runtime
            .physics
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
                && s.runtime
                    .physics
                    .raycast_in(
                        a.handle.client,
                        origin,
                        delta,
                        distance - 0.02,
                        a.follow.map(|h| (h.client, h.id)),
                        false,
                    )
                    .is_some_and(|hit| hit.distance > 0.05)
            {
                1.
            } else {
                0.
            };
        }
        if a.binaural {
            let state = cx.state()?;
            let listener = state
                .mixer_audio
                .listeners
                .get(&a.handle.client)
                .copied()
                .unwrap_or(auto);
            crate::binaural_api::queue(state, a.handle, a.spatial.position, listener)?;
        }
        if let Some(settings) = &a.acoustics {
            let s = cx.state()?;
            let listener = s
                .mixer_audio
                .listeners
                .get(&a.handle.client)
                .copied()
                .unwrap_or(auto);
            crate::acoustics_api::queue(
                s,
                a.handle,
                crysirror_acoustics::AcousticQuery {
                    source: a.spatial.position,
                    listener: listener.position,
                    settings: settings.clone(),
                },
            )?;
        }
        cx.state()?
            .mixer_audio
            .mixer
            .set_spatial(a.handle.client, a.handle, Some(a.spatial))?;
    }
    crate::acoustics_api::dispatch(cx.state()?)?;
    crate::binaural_api::dispatch(cx.state()?)?;
    crate::reflection_tracking::update(cx.state()?, dt_ms)?;
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
    a.output.clear();
    Ok(count)
}
/// Explicit synchronous/offline completion. Live hosts use App.begin_frame /
/// poll_frame, which move final DSP to a worker after this owner control phase.
pub fn update(cx: &mut EngineContext, dt_ms: u64) -> Result<()> {
    let count = prepare(cx, dt_ms)?;
    let s = cx.state()?;
    let a = &mut s.mixer_audio;
    a.output.resize(count, [0.; 2]);
    for block in a.output.chunks_mut(4096) {
        a.mixer.render(block)?;
    }
    let native = s.runtime.host.take_pcm_output();
    if native.len() > a.output.len() {
        a.output.resize(native.len(), [0.; 2]);
    }
    for (out, pcm) in a.output.iter_mut().zip(native) {
        out[0] += pcm[0];
        out[1] += pcm[1];
    }
    Ok(())
}
pub(crate) fn publish(cx: &mut EngineContext, pcm: Vec<StereoFrame>) -> Result<()> {
    let s = cx.state()?;
    ensure!(
        !s.mixer_audio.mixer.is_rendering(),
        "audio publication still pending"
    );
    s.mixer_audio.output = pcm;
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
fn playback_json(info: crysirror_core::service::AudioPlaybackInfo) -> Value {
    Value::from_json(
        serde_json::json!({"age_frames":info.age_frames,"source_playing":info.source_playing,"paused":info.paused,"looping":info.looping,"position_frames":info.position_frames,"source_rate":info.source_rate,"source_frames":info.source_frames,"output_rate":info.output_rate,"fade_out_frames":info.fade_out_frames,"resume_delay_frames":info.resume_delay_frames}),
    )
}
fn info() -> &'static ClassInfo {
    static INFO: OnceLock<ClassInfo> = OnceLock::new();
    INFO.get_or_init(||ClassInfo{name:"Audio",doc:"Scoped voice pool, streaming music and deterministic CPU mixing; classic channels share this output.",properties:vec![PropertyInfo{name:"sampleRate",ty:T::Int,writable:false,doc:"Mixer output frames per second."},PropertyInfo{name:"rendering",ty:T::Bool,writable:false,doc:"A prepared or running audio frame awaits owner publication; playback queries retain the previous published state and controls require completion."}],methods:vec![
        method("nativeChannelInfo",vec![("channel",T::Int)],T::Any,"Shared native/media channel AudioPlaybackInfo or null when ended. Scoped to the caller; source frame units, output-clock fades/delays and source completion remain distinct from processing tails."),
        method("playbackInfo",vec![("handle",T::Map)],T::Map,"Exact shared AudioPlaybackInfo for a live pooled voice: decoded-source position/rate/length, paused/loop state and output-clock fade-out/resume-delay counters."),
        method("scheduleVoiceResume",vec![("handle",T::Map),("delay_ms",T::Number),("fade_ms",T::Number)],T::Void,"Freeze a source, then resume after a sample-clock delay with a fresh fade-in. Later pause/stop/resume replaces this schedule. This schedules transport, not asset readiness."),
        method("channelStatus",vec![("channel",T::Int)],T::Map,"Desired SceneState epoch and derived queued/loading/playing/paused/ended/failed readiness, position and error. Stopped declarations take precedence; frame synchronization prepares/publishes playback."),
        method("play",vec![("channel",T::Int),("voice",T::Map)],T::Void,"Classic {asset,looping?,gain?,bus?} channel, preserving SceneState trace/save semantics."),
        method("stop",vec![("channel",T::Int)],T::Void,"Stop a classic channel."),
        method("playAt",vec![("channel",T::Int),("voice",T::Map),("where",T::Map)],T::Void,"Legacy distance-gain API. Use playSfx with spatial/follow for stereo 3D and occlusion."),
        method("trackVoiceReflections",vec![("handle",T::Map),("settings",T::Map)],T::Void,"Automatically refresh a spatial voice's reflection paths on bounded workers using ReflectionTrackingSpec. Movement coalesces; fixed delay history is preserved and tap plans crossfade. Initial work holds the source clock, including HRTF dependency. clearVoiceReflections disables tracking. Playback options may include reflections to register before a short source runs."),
        method("beginReflectionProbe",vec![("query",T::Map)],T::Int,"Trace bounded single-bounce specular ReflectionQuery paths on workers against this client's prepared acoustic scene. Includes material pressure gains, arrival directions and physical/excess delays. Exceeding budgets fails visibly."),
        method("beginVoiceReflections",vec![("handle",T::Map),("options",T::Map)],T::Int,"Prepare captured-pose early echoes for a spatial voice on workers. Initial preparation holds its source clock; replacement keeps the old effect. Uses existing HRTF bridge latency after HRTF is ready. Pose/scene/latency changes invalidate these static snapshots; prepare again or opt into trackVoiceReflections."),
        method("reflectionStatus",vec![("ticket",T::Int)],T::Map,"Client/restore-scoped pending/ready/failed query/preparation receipt. Ready follows publication. Release using cancelReflectionJob."),
        method("cancelReflectionJob",vec![("ticket",T::Int)],T::Void,"Cancel/release reflection work and release an initial clock hold. Releasing a published receipt keeps its effect."),
        method("clearVoiceReflections",vec![("handle",T::Map)],T::Void,"Cancel pending reflection work and clear this voice's early echo history, preserving independent direct/HRTF/late-reverb processing."),
        method("reflectionsVoice",vec![("handle",T::Map)],T::Map,"Current captured paths, scene revision, pending/error state and optional automatic tracking snapshots. Static effects require re-preparation after pose changes; tracked effects coalesce updates and expose waiting/draining state."),
        method("beginReverb",vec![("settings",T::Map)],T::Int,"Prepare a client ReverbSettings parametric late-reverb bus on bounded CPU workers. Publishes matched resources in pooled/canonical and native/media mixers. Old effects remain until success. Replacing a room cuts its old tail; this does not simulate geometry reflections."),
        method("reverbStatus",vec![("ticket",T::Int)],T::Map,"Client/restore-scoped pending/ready/failed preparation. Ready includes published revision, settings and both mixer rates. Release with cancelReverbJob."),
        method("cancelReverbJob",vec![("ticket",T::Int)],T::Void,"Cancel/release a scoped reverb preparation ticket. Releasing a published ticket keeps its installed effect."),
        method("clearReverb",vec![],T::Void,"Cancel pending room preparation and remove this client's effect and tail from both mixers. Other clients remain unchanged."),
        method("reverbSettings",vec![],T::Any,"Published ReverbSettings for this client, or null. Sends are post source-bus fader/duck/spatial and pre-Master; the wet return uses its own bus fader and Master once."),
        method("beginHrtf",vec![("asset",T::String),("options",T::Map)],T::Int,"Prepare a client asset SOFA SimpleFreeFieldHRIR or HrirAsset JSON on workers, including resampling/FFT plans. HrtfOptions defaults are accepted with {}. Ready publication replaces the client profile and re-prepares enabled voices; profile replacement is not a seamless transition."),
        method("hrtfStatus",vec![("ticket",T::Int)],T::Map,"Client/restore-scoped pending/ready/failed dataset status; ready result contains revision, format, sample rate, measurements, FIR taps and bridge latency. Release with cancelHrtfJob."),
        method("cancelHrtfJob",vec![("ticket",T::Int)],T::Void,"Cancel/release a dataset preparation ticket; a successfully published profile remains installed."),
        method("clearHrtf",vec![],T::Void,"Remove this client's profile and pending preparation, cancel its filter work, and restore stereo panning on enabled voices. Acoustic direct-path filtering remains independent."),
        method("hrtfStats",vec![],T::Map,"Client profile metadata, voice/pending counts, and host admission reservations. Reservations are estimates, not measured process memory."),
        method("hrtfVoice",vec![("handle",T::Map)],T::Map,"Live client voice HRTF state: enabled plus ready/pending preparation/revision/published_direction/error; publication does not claim the audible crossfade has completed. Initial failure stays muted; update failure retains the last ready filter. Disable/re-enable to retry."),
        method("setVoiceHrtf",vec![("handle",T::Map),("enabled",T::Bool)],T::Void,"Enable/disable binaural rendering on a spatial voice. Enabling requires a prepared profile; source/envelope clocks wait for worker preparation. Repeated enable is idempotent. Pose updates crossfade with the profile policy."),
        method("beginAcousticScene",vec![("asset",T::String)],T::Int,"Read a client-scoped AcousticSceneSpec JSON (up to 32 MiB) and build its CPU BVH on workers. Old geometry remains until successful owner publication. Options live in the JSON; obvhs requires its host feature."),
        method("beginAcousticProbe",vec![("query",T::Map)],T::Int,"Simulate a captured source/listener/settings query on workers. Requires a prepared scene; returns a ticket for acousticStatus. Gains exclude the mixer's existing distance curve."),
        method("acousticStatus",vec![("ticket",T::Int)],T::Map,"Nonblocking client/generation-scoped pending/ready/failed status. Ready result contains scene metadata or a direct path. Release tickets with cancelAcousticJob."),
        method("cancelAcousticJob",vec![("ticket",T::Int)],T::Void,"Cancel/release a retained acoustic operation; a published scene remains installed."),
        method("clearAcousticScene",vec![],T::Void,"Remove this client's scene and pending build, and disable acoustic processing on its active voices. Explicit probes retain their captured scene."),
        method("acousticStats",vec![],T::Map,"Client scene/revision/backend, source queue counts and host geometry reservations. Scene reservations are conservative, not an allocator/VRAM measurement."),
        method("acousticVoice",vec![("handle",T::Map)],T::Map,"Read pending/result/error for a client-owned live voice. Automatic queries use latest queued poses and preserve the last ready coefficients while busy."),
        method("beginPrepareSource",vec![("asset",T::String),("options",T::Optional(Box::new(T::Map)))],T::Int,"Snapshot a client asset and probe its decoder on bounded workers. Options: measure_frames=false (scan when the header has no count), frame_limit. At most 128 MiB encoded per resource; bounded host bytes and handles. sourcePreparation publishes an immutable resource handle, without playback."),
        method("sourcePreparation",vec![("ticket",T::Int)],T::Map,"Client/restore-scoped pending/ready/failed preparation; ready result has resource:{client,id} and AudioResourceInfo. Release the receipt separately from its resource."),
        method("cancelSourcePreparation",vec![("ticket",T::Int)],T::Void,"Cancel/release preparation without publishing an unobserved result. Already published resources remain retained until releaseSource or restore."),
        method("sourceInfo",vec![("resource",T::Map)],T::Map,"Read a live client's immutable AudioResourceInfo. Released/foreign/obsolete handles fail."),
        method("releaseSource",vec![("resource",T::Map)],T::Void,"Invalidate future lookups. Admitted playback and existing independent decoder leases retain encoded bytes until their actual last owner drops. Handles are not save data."),
        method("beginPlaySource",vec![("resource",T::Map),("voice",T::Map)],T::Int,"Prepare an independent buffered decoder on workers from an immutable resource, without reopening VFS. VoiceSpec and spatial/acoustic/HRTF/reflection options match beginPlaySfx, excluding asset. playbackStatus returns result.handle."),
        method("beginPlaySfx",vec![("voice",T::Map)],T::Int,"Prepare SFX on bounded workers and publish a shared mixer voice; playbackStatus returns result.handle."),
        method("beginPreloadSfx",vec![("asset",T::String)],T::Int,"Decode and cache SFX on workers without starting a voice."),
        method("beginPlayMusic",vec![("name",T::String),("voice",T::Map),("crossfade_ms",T::Number)],T::Int,"Open and prewarm bounded streaming music on workers; old music fades only after successful publication."),
        method("playbackStatus",vec![("ticket",T::Int)],T::Map,"Client/restore-scoped pending/ready/failed audio preparation or seek; owner polling publishes ready playback."),
        method("cancelPlaybackJob",vec![("ticket",T::Int)],T::Void,"Cancel/release an audio job. Pending playback cannot publish; already published voices keep playing. Pending seek restores the prior logical position asynchronously."),
        method("beginSeekVoice",vec![("handle",T::Map),("source_frame",T::Int)],T::Int,"Exact nonblocking seek for resident or worker-buffered sources. A newer seek supersedes an older one; pending seeks freeze the voice clock."),
        method("sourceStatus",vec![("handle",T::Map)],T::Any,"Worker stream queue, decoded frame count, seek generation and errors, or null for a resident source."),
        method("clearSfxCache",vec![],T::Void,"Drop this client's cached clips. Playing voices retain their own PCM and residency receipts."),
        method("playSfx",vec![("voice",T::Map)],T::Map,"{asset,bus?,mix_group?,gain?,rate?,pitch_semitones?,priority?,looping?,loop_points?:{start,end},fade_in_ms?,paused?,spatial?,follow?:Entity,raycast_occlusion?,acoustics?:AcousticSource,binaural?:boolean,reflections?:ReflectionTrackingSpec}; acoustics requires a ready scene; binaural requires a ready HRTF profile. These use background preparation; reflections automatically follow source/listener poses. Realtime hosts require a preloaded clip or playSfxAsync. Returns scoped generation handle."),
        method("playRandom",vec![("name",T::String),("container",T::Map),("voice",T::Map)],T::Map,"Seeded weighted variants [{asset,weight?}], gain:[min,max], pitch_semitones:[min,max], avoid_repeat; optional VoiceSpec. Realtime hosts require preloaded variants."),
        method("playMusic",vec![("name",T::String),("voice",T::Map),("crossfade_ms",T::Number)],T::Map,"Stream music by name; optional sample-frame loop_points; crossfade_ms fades old/new together. Looping defaults true. Offline-only; realtime hosts use playMusicAsync."),
        method("stopVoice",vec![("handle",T::Map),("fade_ms",T::Number)],T::Void,"Stop immediately or sample-accurate fade out."),
        method("pauseVoice",vec![("handle",T::Map),("paused",T::Bool)],T::Void,"Pause without advancing the source or fade clock."),
        method("seekVoice",vec![("handle",T::Map),("frame",T::Int)],T::Void,"Synchronous exact seek. Worker-buffered streams require beginSeekVoice/seekVoiceAsync."),
        method("setVoice",vec![("handle",T::Map),("options",T::Map)],T::Void,"Update {gain?,pan?:0..1,fade_ms?,rate?,pitch_semitones?,paused?,spatial?}. Fade applies to gain/pan. Pitch/rate change resampling speed."),
        method("voice",vec![("handle",T::Map)],T::Map,"{playing,source_playing,position_seconds?}; playing includes effect tails, source_playing excludes them. Stale/stolen/ended handles are inactive."),
        method("setMixGroup",vec![("name",T::String),("settings",T::Map),("fade_ms",T::Optional(Box::new(T::Number)))],T::Void,"Create/update a client logical gain group in pooled and native/media mixers after validating both. Partial settings: parent?:string|null, gain?:0..4, muted?:boolean; new defaults parent=null,gain=1,muted=false. Parent must exist in both namespaces; cycles/depth>8/budgets fail atomically. Fade uses each output clock, including silence/paused sources. This is gain control, not an effect submix."),
        method("mixGroup",vec![("name",T::String)],T::Any,"Pooled AudioMixGroupInfo for the next output frame, or null. Controls multiply voice gain and ancestors before bus/master and reverb sends. Host controls persist through playback clear/restore; DSP history is not saved."),
        method("nativeMixGroup",vec![("name",T::String)],T::Any,"Native/media mixer AudioMixGroupInfo or null. Native adapters may define scoped groups independently of pooled voices."),
        method("removeMixGroup",vec![("name",T::String)],T::Void,"Remove from both namespaces after validating neither has child groups or live voice/effect instances. Unobserved playback resolves its logical group name again at publication."),
        method("setVoiceMixGroup",vec![("handle",T::Map),("group",T::Optional(Box::new(T::String)))],T::Void,"Route a live pooled voice to an existing client group, or null to bypass groups; preserves source position and independent voice fades."),
        method("setBus",vec![("bus",T::String),("settings",T::Map)],T::Void,"master/music/sfx/voice/ambient/ui {gain:0..4,muted:boolean}."),
        method("bus",vec![("bus",T::String)],T::Map,"Read this client's bus settings."),
        method("setDucking",vec![("config",T::Map)],T::Void,"Dialogue-driven music ducking {gain,attack_ms,release_ms}."),
        method("setVoiceLimit",vec![("limit",T::Int)],T::Void,"Set this client's pool limit within the host's global budget; stop voices before lowering below the active count."),
        method("setSeed",vec![("seed",T::Int)],T::Void,"Reset this client's deterministic variant generator and repeat history."),
        method("setListener",vec![("listener",T::Any)],T::Void,"{position,forward,up}; null restores automatic camera listener."),
        method("stats",vec![],T::Map,"Mixer active/stolen/rejected voices, rendered frames, per-voice underrun frames and unclipped peak."),
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
pub(crate) fn voice_options(
    cx: &mut EngineContext,
    value: &Value,
    asset_required: bool,
) -> Result<(Option<String>, VoiceSpec, Option<Attachment>)> {
    let Value::Map(map) = value else {
        bail!("voice must be an object")
    };
    let mut map = map.clone();
    let binaural = map
        .remove("binaural")
        .map(|v| v.as_bool())
        .transpose()?
        .unwrap_or(false);
    let reflections = map
        .remove("reflections")
        .filter(|v| !matches!(v, Value::Null))
        .map(|v| parse::<crysirror_acoustics::reflections::ReflectionTrackingSpec>(&v))
        .transpose()?;
    if let Some(spec) = &reflections {
        spec.validate()?;
    }
    let acoustics = map
        .remove("acoustics")
        .filter(|v| !matches!(v, Value::Null))
        .map(|v| parse::<crysirror_acoustics::AcousticSource>(&v))
        .transpose()?;
    if let Some(settings) = &acoustics {
        settings.validate()?;
    }
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
        (acoustics.is_none() && reflections.is_none()) || spec.spatial.is_some(),
        "acoustics/reflections require a spatial voice or follow entity"
    );
    ensure!(
        acoustics.is_none() || !raycast,
        "choose acoustic geometry or physics raycast_occlusion"
    );
    let state = cx.state()?;
    ensure!(
        !binaural || spec.spatial.is_some(),
        "binaural requires a spatial voice or follow entity"
    );
    if binaural {
        crate::binaural_api::poll(state)?;
        state.mixer_audio.binaural.admit_voice(state.client.0)?;
    }
    ensure!(
        (acoustics.is_none() && reflections.is_none())
            || state.mixer_audio.acoustics.has_scene(state.client.0),
        "prepare an acoustic scene before playing an acoustic source"
    );
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
        acoustics,
        binaural,
        reflections,
    });
    Ok((asset, spec, attachment))
}
fn attach(
    cx: &mut EngineContext,
    handle: VoiceHandle,
    attachment: Option<Attachment>,
) -> Result<()> {
    attach_state(cx.state()?, handle, attachment)
}
pub(crate) fn attach_state(
    state: &mut crate::EngineState,
    handle: VoiceHandle,
    attachment: Option<Attachment>,
) -> Result<()> {
    if let Some(mut a) = attachment {
        a.handle = handle;
        let runtime = &mut state.mixer_audio;
        runtime
            .attachments
            .retain(|_, a| runtime.mixer.is_playing(a.handle.client, a.handle));
        let binaural = a.binaural;
        let reflections = a.reflections.clone();
        runtime.attachments.insert(key(handle), a);
        if runtime.attachments[&key(handle)].acoustics.is_some() {
            runtime
                .mixer
                .set_acoustic_gains(handle.client, handle, Some([0.; 3]))?;
        }
        if binaural {
            if let Err(error) = crate::binaural_api::enable(state, handle) {
                let a = &mut state.mixer_audio;
                a.attachments.remove(&key(handle));
                a.mixer.stop(handle.client, handle, 0.)?;
                return Err(error);
            }
        }
        if let Some(spec) = reflections {
            if let Err(error) = crate::reflection_tracking::track(state, handle, spec) {
                state.mixer_audio.attachments.remove(&key(handle));
                state.mixer_audio.mixer.stop(handle.client, handle, 0.)?;
                return Err(error);
            }
        }
    }
    Ok(())
}
impl Class for AudioClass {
    fn info(&self) -> &ClassInfo {
        info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        sync_generation(cx.state()?);
        match p {
            "rendering" => {
                let s = cx.state()?;
                Ok(Value::Bool(
                    s.mixer_audio.frame_pending
                        || s.mixer_audio.mixer.is_rendering()
                        || s.runtime.host.channel_audio.mixer.is_rendering(),
                ))
            }
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
        sync_generation(cx.state()?);
        if cx.state()?.mixer_audio.frame_pending
            || cx.state()?.mixer_audio.mixer.is_rendering()
            || cx.state()?.runtime.host.channel_audio.mixer.is_rendering()
        {
            ensure!(
                matches!(
                    name,
                    "playbackInfo"
                        | "nativeChannelInfo"
                        | "voice"
                        | "stats"
                        | "bus"
                        | "mixGroup"
                        | "nativeMixGroup"
                        | "channelStatus"
                        | "sourceStatus"
                        | "playbackStatus"
                        | "sourceInfo"
                        | "sourcePreparation"
                        | "cancelSourcePreparation"
                        | "beginPrepareSource"
                        | "clearSfxCache"
                ),
                "audio frame pending; poll frame completion before this control"
            );
        }
        if let Some(result) = crate::audio_groups_api::call(cx, name, args)? {
            return Ok(result);
        }
        if let Some(result) = crate::audio_resources_api::call(cx, name, args)? {
            return Ok(result);
        }
        if let Some(result) = crate::audio_async::call(cx, name, args)? {
            return Ok(result);
        }
        if let Some(result) = crate::reflections_api::call(cx.state()?, name, args)? {
            return Ok(result);
        }
        if let Some(result) = crate::reverb_api::call(cx.state()?, name, args)? {
            return Ok(result);
        }
        if let Some(result) = crate::binaural_api::call(cx.state()?, name, args)? {
            return Ok(result);
        }
        if let Some(result) = crate::acoustics_api::call(cx.state()?, name, args)? {
            return Ok(result);
        }
        let client = cx.state()?.client.0;
        match name {
            "nativeChannelInfo" => {
                let channel = u32::try_from(arg(args, 0, "channel")?.as_i64()?)?;
                return Ok(cx
                    .state()?
                    .runtime
                    .host
                    .channel_audio
                    .playback_info(client, channel)?
                    .map(playback_json)
                    .unwrap_or(Value::Null));
            }
            "playbackInfo" => {
                let h = handle(cx, args)?;
                return Ok(playback_json(
                    cx.state()?.mixer_audio.mixer.playback_info(client, h)?,
                ));
            }
            "scheduleVoiceResume" => {
                let h = handle(cx, args)?;
                cx.state()?.mixer_audio.mixer.schedule_resume(
                    client,
                    h,
                    arg(args, 1, "delay")?.as_f32()?,
                    arg(args, 2, "fade")?.as_f32()?,
                )?;
            }
            "channelStatus" => {
                let channel = u32::try_from(arg(args, 0, "channel")?.as_i64()?)?;
                let s = cx.state()?;
                return Ok(Value::from_json(serde_json::to_value(
                    s.mixer_audio.scene_audio.status(
                        &s.mixer_audio.mixer,
                        &s.runtime.host.scene,
                        ChannelId {
                            client: ClientId(client),
                            local: channel,
                        },
                    ),
                )?));
            }
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
                    cx.state()?.mixer_audio.scene_audio.set_route(
                        ChannelId {
                            client: ClientId(client),
                            local: ch,
                        },
                        route,
                    )?;
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
                ensure!(
                    !cx.state()?.runtime.audio_is_nonblocking(),
                    "realtime music requires Audio.playMusicAsync"
                );
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
                ensure!(
                    opts.fade_ms.is_finite() && (0.0..=600000.).contains(&opts.fade_ms),
                    "invalid voice fade"
                );
                ensure!(
                    opts.pan
                        .is_none_or(|p| p.is_finite() && (0.0..=1.).contains(&p)),
                    "invalid voice pan"
                );
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
                if let Some(pan) = opts.pan {
                    a.mixer.set_pan(client, h, pan, opts.fade_ms)?;
                }
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
                    serde_json::json!({"playing":true,"source_playing":mixer.source_is_playing(client,h),"position_seconds":mixer.position(client,h)?})
                } else {
                    serde_json::json!({"playing":false,"source_playing":false})
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
                cx.state()?
                    .runtime
                    .host
                    .channel_audio
                    .mixer
                    .set_bus(client, b, settings)?;
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
                cx.state()?
                    .runtime
                    .host
                    .channel_audio
                    .set_ducking(client, config)?;
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
                )?));
            }
            _ => bail!("Audio has no method {name}"),
        }
        Ok(Value::Null)
    }
}
