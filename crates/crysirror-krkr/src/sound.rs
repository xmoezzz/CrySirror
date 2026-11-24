// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native sound declarations/continuations over shared prepared audio resources.
use crate::natives::*;
use crysirror_core::{
    path::{Mount, VfsPath},
    service::*,
    state::ClientId,
};
use crysirror_tjs2::{
    TjsError,
    runtime::{ObjectHandle, Variant},
};
use std::collections::{BTreeMap, VecDeque};
#[derive(Clone)]
pub struct Buffer {
    channel: u32,
    resource: Option<AudioResourceHandle>,
    // Reopening the same unfiltered mounted asset can restart this buffer
    // with its existing immutable resource and an independent play cursor.
    prepared_path: Option<VfsPath>,
    info: Option<AudioResourceInfo>,
    pub status: &'static str,
    volume: i64,
    volume2: i64,
    pan: i64,
    paused: bool,
    looping: bool,
    pub start: u64,
    pub fading: bool,
    installed_gain: f32,
    /// `<storage>.sli` loop information, read at open (WaveLoopManager).
    sli: Option<crysirror_resource::sli::LoopInfo>,
    /// An open queued from a non-resumable callback whose preparation has not
    /// completed; metadata reads fail visibly until it has.
    opening: bool,
    /// Script-set playback frequency (krkrz `frequency`), applied at play as
    /// a rate relative to the source sample rate.
    frequency: Option<i64>,
    /// The `filters` chain the open resource was prepared with.
    filtered: Vec<WaveFilter>,
    /// wumultitrack stems the open resource was prepared with.
    multitrack: Option<std::sync::Arc<[crysirror_core::service::MultiTrackStem]>>,
}
impl Buffer {
    pub(crate) fn playing_resource(&self) -> Option<AudioResourceHandle> {
        if self.status == "play" { self.resource } else { None }
    }
    fn group(&self) -> String {
        format!("krkr.wave.{}", self.channel)
    }
    fn gain(&self) -> f32 {
        (self.volume as f64 * self.volume2 as f64 / 1e10) as f32
    }
}
pub enum Op {
    Control(AudioRequest),
    Open { object: usize, path: VfsPath },
    Play(usize),
    Release(usize),
}
pub enum Pending {
    Resource { object: usize, ticket: u64, path: Option<VfsPath> },
    Voice { object: usize, ticket: u64 },
}
#[derive(Default)]
pub struct Sounds {
    pub buffers: BTreeMap<usize, Buffer>,
    ops: VecDeque<Op>,
    pending: Option<Pending>,
    global: i64,
    smooth_logged: bool,
}
impl Sounds {
    /// Native instance cleanup also runs when a script overrides finalize
    /// without calling its base. At most the bounded owned buffers can retire.
    pub fn invalidate(&mut self, object: usize) {
        if self.buffers.contains_key(&object)
            && !self
                .ops
                .iter()
                .any(|op| matches!(op, Op::Release(id) if *id == object))
        {
            self.ops.push_back(Op::Release(object));
        }
    }
}
fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(e.to_string())
}
fn buffer(rt: &Rt, t: Option<ObjectHandle>) -> crysirror_tjs2::Result<(ObjectHandle, Buffer)> {
    let h = this_of(rt, t, "WaveSoundBuffer")?;
    Ok((
        h,
        rt.host()
            .sounds
            .buffers
            .get(&h.0)
            .cloned()
            .ok_or_else(|| err("WaveSoundBuffer not constructed"))?,
    ))
}
pub(crate) fn control(rt: &mut Rt, r: AudioRequest) -> crysirror_tjs2::Result<()> {
    let s = &mut rt.host_mut().sounds;
    if s.ops.len() >= 4096 {
        return Err(err("native audio operation budget"));
    }
    s.ops.push_back(Op::Control(r));
    Ok(())
}
/// The engine channel of an open WaveSoundBuffer.
pub(crate) fn channel(rt: &Rt, object: usize) -> Option<u32> {
    rt.host().sounds.buffers.get(&object).map(|b| b.channel)
}
/// Queue an operation that needs host completion. At a resumable boundary the
/// VM waits for it; inside a non-resumable callback (input events) it is
/// queued in order and completes after the callback returns.
fn park(rt: &mut Rt, op: Op) -> crysirror_tjs2::Result<Variant> {
    let suspend = rt.host().can_suspend;
    let s = &mut rt.host_mut().sounds;
    if s.ops.len() >= 4096 {
        return Err(err("native audio operation budget"));
    }
    if let (false, Op::Open { object, .. }) = (suspend, &op) {
        if let Some(b) = s.buffers.get_mut(object) {
            b.opening = true;
        }
    }
    s.ops.push_back(op);
    if suspend {
        rt.request_suspend();
    }
    Ok(Variant::Void)
}
fn notify(rt: &mut Rt, t: Option<ObjectHandle>, name: &str) -> crysirror_tjs2::Result<Variant> {
    let h = this_of(rt, t, name)?;
    let member = rt.resolve_object_member(h, name)?;
    if rt.variant_is_native_function(&member) && rt.call_secondary_class_method(h, name, vec![])? {
        return Ok(Variant::Void);
    }
    rt.call_object_method(h, name, vec![])
}
/// Filters are prepared into the resource at open (the native chain is
/// rebuilt then). A chain changed after open would play unfiltered.
fn active_filter_check(rt: &Rt, h: crysirror_tjs2::runtime::ObjectHandle) -> crysirror_tjs2::Result<()> {
    let chain = crate::wf_plugins::chain(rt, h)?;
    let b = &rt.host().sounds.buffers[&h.0];
    if b.opening || chain == b.filtered {
        return Ok(());
    }
    Err(unsupported(
        "WaveSoundBuffer.filters changed after open",
        "re-preparing an open sound resource with a new filter chain",
    ))
}
/// `WaveSoundBuffer.PhaseVocoder`: krkrz's built-in WaveFilter parameters.
fn install_phase_vocoder(rt: &mut Rt, wave: crysirror_tjs2::runtime::ObjectHandle) {
    let c = rt.alloc_native_constructor(|rt: &mut Rt, this: Option<crysirror_tjs2::runtime::ObjectHandle>, _| {
        let h = this
            .map(|h| rt.bound_this(h).unwrap_or(h))
            .filter(|h| *h != rt.global_handle())
            .unwrap_or_else(|| rt.alloc_ordinary_object());
        rt.add_object_class_info(h, "PhaseVocoder");
        // Instances resolve `window`/`overlap` through the class object.
        let class = rt.object_member(rt.global_handle(), "WaveSoundBuffer").object_handle()
            .and_then(|w| rt.object_member(w, "PhaseVocoder").object_handle());
        if let Some(class) = class.filter(|c| *c != h && rt.object_super_class(h).is_none()) {
            rt.set_object_super_class(h, class);
        }
        rt.set_object_member(h, "__vocoder", Variant::Integer(1));
        // krkrz defaults: window 4096, overlap 0 (automatic), pitch 1, time 1.
        rt.set_object_member(h, "__window", Variant::Integer(4096));
        rt.set_object_member(h, "__overlap", Variant::Integer(0));
        rt.set_object_member(h, "pitch", Variant::Real(1.));
        rt.set_object_member(h, "time", Variant::Real(1.));
        Ok(Variant::Object(h))
    });
    rt.add_object_class_info(c, "PhaseVocoder");
    rt.register_object_native_property(
        c,
        "window",
        |rt, t| Ok(rt.object_member(this_of(rt, t, "PhaseVocoder.window")?, "__window")),
        |rt, t, v| {
            let h = this_of(rt, t, "PhaseVocoder.window")?;
            let n = v.to_integer()?;
            if !(64..=32768).contains(&n) || n & (n - 1) != 0 {
                return Err(err("PhaseVocoder.window must be a power of two in 64..32768"));
            }
            rt.set_object_member(h, "__window", Variant::Integer(n));
            Ok(())
        },
    );
    rt.register_object_native_property(
        c,
        "overlap",
        |rt, t| Ok(rt.object_member(this_of(rt, t, "PhaseVocoder.overlap")?, "__overlap")),
        |rt, t, v| {
            let h = this_of(rt, t, "PhaseVocoder.overlap")?;
            let n = v.to_integer()?;
            if !matches!(n, 0 | 2 | 4 | 8 | 16 | 32) {
                return Err(err("PhaseVocoder.overlap must be 0, 2, 4, 8, 16 or 32"));
            }
            rt.set_object_member(h, "__overlap", Variant::Integer(n));
            Ok(())
        },
    );
    rt.register_object_native(c, "finalize", |_: &mut Rt, _, _| Ok(Variant::Void));
    rt.set_object_member(wave, "PhaseVocoder", Variant::Object(c));
}

pub fn install(rt: &mut Rt) {
    rt.host_mut().sounds.global = 100000;
    rt.host_mut()
        .sounds
        .ops
        .push_back(Op::Control(AudioRequest::SetMixGroup {
            name: "krkr.wave".into(),
            settings: AudioMixGroupSettings::default(),
            fade_ms: 0.,
        }));
    let c = native_class(rt, "WaveSoundBuffer", |rt, t, _| {
        let h = this_of(rt, t, "WaveSoundBuffer")?;
        if rt.host().sounds.buffers.contains_key(&h.0) {
            return Ok(Variant::Void);
        }
        // Logical native buffers are not simultaneous playing voices. Keep
        // them bounded within the common 128-groups-per-client budget (one root) while allowing
        // ordinary KAG configurations with 100 SE slots plus music buffers.
        let channel = (0..127)
            .find(|n| !rt.host().sounds.buffers.values().any(|b| b.channel == *n))
            .ok_or_else(|| err("native sound buffer limit"))?;
        let b = Buffer {
            channel,
            resource: None,
            prepared_path: None,
            info: None,
            status: "unload",
            volume: 100000,
            volume2: 100000,
            pan: 0,
            paused: false,
            looping: false,
            start: 0,
            fading: false,
            installed_gain: 1.,
            sli: None,
            opening: false,
            frequency: None,
            filtered: vec![],
            multitrack: None,
        };
        let group = b.group();
        rt.host_mut().sounds.buffers.insert(h.0, b);
        // WaveSoundBuffer.filters: the ordered WaveFilter list (an Array).
        let filters = rt.alloc_array_object(vec![]);
        rt.set_object_member(h, "filters", Variant::Object(filters));
        // WaveSoundBuffer.flags: the loop-link condition flags (16 slots).
        let flags = rt.alloc_array_object(vec![Variant::Integer(0); crysirror_resource::sli::MAX_FLAGS]);
        rt.set_object_member(h, "flags", Variant::Object(flags));
        control(
            rt,
            AudioRequest::SetMixGroup {
                name: group,
                settings: AudioMixGroupSettings {
                    parent: Some("krkr.wave".into()),
                    ..Default::default()
                },
                fade_ms: 0.,
            },
        )?;
        Ok(Variant::Void)
    });
    rt.register_object_native_property(c,"useVisBuffer",
        |rt,t|{let h=this_of(rt,t,"WaveSoundBuffer.useVisBuffer")?;Ok(Variant::Integer(i64::from(rt.object_member(h,"__wave_vis_buffer").is_truthy())))},
        |rt,t,v|{let h=this_of(rt,t,"WaveSoundBuffer.useVisBuffer")?;rt.set_object_member(h,"__wave_vis_buffer",Variant::Integer(i64::from(v.is_truthy())));Ok(())});
    install_phase_vocoder(rt, c);
    methods(
        rt,
        c,
        &[
            ("__crysirror_notify_status", |rt, t, _| {
                notify(rt, t, "onStatusChanged")
            }),
            ("__crysirror_notify_fade", |rt, t, _| {
                notify(rt, t, "onFadeCompleted")
            }),
            ("open", |rt, t, a| {
                let (h, _) = buffer(rt, t)?;
                let name = string(&a, 0, "WaveSoundBuffer.open")?;
                if crate::storage::save_path(&name).map_err(err)?.is_some() {
                    return Err(unsupported(
                        "WaveSoundBuffer.open",
                        "audio preparation from the Save mount",
                    ));
                }
                // The filter chain is read now, as the native open connects it.
                let chain = crate::wf_plugins::chain(rt, h)?;
                rt.host_mut().sounds.buffers.get_mut(&h.0).unwrap().filtered = chain;
                rt.host_mut().multitrack.retire(h.0);
                if crate::multitrack::handles(rt, &name) {
                    let d = crate::multitrack::describe(rt, &name)?;
                    if !rt.host().sounds.buffers[&h.0].filtered.is_empty() {
                        return Err(unsupported(
                            "WaveSoundBuffer.filters on multi-track music",
                            "an unfiltered multi-track resource",
                        ));
                    }
                    let b = rt.host_mut().sounds.buffers.get_mut(&h.0).unwrap();
                    b.sli = None;
                    b.multitrack = Some(d.stems.clone().into());
                    crate::multitrack::opened(rt, h.0, &name, &d)?;
                    return park(
                        rt,
                        Op::Open {
                            object: h.0,
                            path: VfsPath::new(Mount::Asset, &d.main).map_err(err)?,
                        },
                    );
                }
                rt.host_mut().sounds.buffers.get_mut(&h.0).unwrap().multitrack = None;
                let storage = rt.host().storage()?;
                let path = storage
                    .resolve(&name)
                    .ok_or_else(|| err(format!("sound storage absent: {name}")))?;
                // krkrz reads loop information from `<placed name>.sli`.
                let sli_name = format!("{}.sli", path.rsplit('/').next().unwrap_or(&path));
                let sli = match storage.resolve(&format!("{path}.sli")).or_else(|| storage.resolve(&sli_name)) {
                    Some(sli) => {
                        let bytes = storage.read(&sli).map_err(err)?;
                        let text = String::from_utf8(bytes).map_err(|e| err(format!("{sli}: {e}")))?;
                        Some(crysirror_resource::sli::parse(&text).map_err(|e| err(format!("{sli}: {e:#}")))?)
                    }
                    None => None,
                };
                rt.host_mut().sounds.buffers.get_mut(&h.0).unwrap().sli = sli;
                park(
                    rt,
                    Op::Open {
                        object: h.0,
                        path: VfsPath::new(Mount::Asset, &path).map_err(err)?,
                    },
                )
            }),
            ("play", |rt, t, _| {
                let (h, b) = buffer(rt, t)?;
                if b.status == "play" {
                    return Ok(Variant::Void);
                }
                if b.resource.is_none() && !b.opening {
                    return Err(err("WaveSoundBuffer.play requires open"));
                }
                active_filter_check(rt, h)?;
                let result = park(rt, Op::Play(h.0))?;
                crate::multitrack::replay(rt, h.0)?;
                Ok(result)
            }),
            ("stop", |rt, t, _| {
                let (h, b) = buffer(rt, t)?;
                control(
                    rt,
                    AudioRequest::Stop {
                        channel: b.channel,
                        fade_ms: 0.,
                    },
                )?;
                let state = rt.host_mut().sounds.buffers.get_mut(&h.0).unwrap();
                state.fading = false;
                let changed = state.status == "play";
                if changed {
                    state.status = "stop";
                    notify(rt, Some(h), "onStatusChanged")?;
                }
                Ok(Variant::Void)
            }),
            ("fade", |rt, t, a| {
                let (h, mut b) = buffer(rt, t)?;
                let target = need_int(&a, 0, "fade volume")?.clamp(0, 100000);
                let ms = need_int(&a, 1, "fade time")?;
                if !(1..=600000).contains(&ms) || int(&a, 2, 0)? != 0 {
                    return Err(unsupported(
                        "WaveSoundBuffer.fade",
                        "positive <=600000 ms duration without delay",
                    ));
                }
                b.volume = target;
                b.fading = true;
                control(
                    rt,
                    AudioRequest::SetMixGroupGain {
                        name: b.group(),
                        gain: b.gain(),
                        fade_ms: ms as f32,
                    },
                )?;
                rt.host_mut().sounds.buffers.insert(h.0, b);
                Ok(Variant::Void)
            }),
            ("stopFade", |rt, t, _| {
                let (h, b) = buffer(rt, t)?;
                if b.fading {
                    control(
                        rt,
                        AudioRequest::SetMixGroupGain {
                            name: b.group(),
                            gain: b.installed_gain,
                            fade_ms: 0.,
                        },
                    )?;
                    let state = rt.host_mut().sounds.buffers.get_mut(&h.0).unwrap();
                    state.volume = if b.volume2 == 0 {
                        0
                    } else {
                        (b.installed_gain as f64 * 1e10 / b.volume2 as f64)
                            .round()
                            .clamp(0., 100000.) as i64
                    };
                    state.fading = false;
                }
                Ok(Variant::Void)
            }),
            ("finalize", |rt, t, _| {
                let (h, _) = buffer(rt, t)?;
                rt.host_mut().sounds.invalidate(h.0);
                rt.host_mut().get_sample.invalidate(h.0);
                Ok(Variant::Void)
            }),
            ("onStatusChanged", |_, _, _| Ok(Variant::Void)),
            ("onFadeCompleted", |_, _, _| Ok(Variant::Void)),
        ],
    );
    for name in [
        "status",
        "volume",
        "volume2",
        "pan",
        "paused",
        "looping",
        "position",
        "samplePosition",
        "totalTime",
        "totalSamples",
        "frequency",
        "labels",
    ] {
        rt.register_object_native_property(
            c,
            name,
            move |rt, t| {
                let (_, b) = buffer(rt, t)?;
                if b.opening && !matches!(name, "status" | "volume" | "volume2" | "pan" | "paused" | "looping" | "labels") {
                    return Err(unsupported(
                        &format!("WaveSoundBuffer.{name} while an open from a native callback is preparing"),
                        "synchronous audio metadata inside non-resumable callbacks",
                    ));
                }
                Ok(match name {
                    "status" => Variant::String(b.status.into()),
                    "volume" => Variant::Integer(b.volume),
                    "volume2" => Variant::Integer(b.volume2),
                    "pan" => Variant::Integer(b.pan),
                    "paused" => Variant::Integer(b.paused as i64),
                    "looping" => Variant::Integer(b.looping as i64),
                    "samplePosition" => Variant::Integer(b.start as i64),
                    "position" => Variant::Integer(
                        b.info
                            .map_or(0, |i| b.start.saturating_mul(1000) / i.source_rate as u64)
                            as i64,
                    ),
                    "totalSamples" => {
                        Variant::Integer(b.info.and_then(|i| i.source_frames).unwrap_or(0) as i64)
                    }
                    "totalTime" => Variant::Integer(b.info.map_or(0, |i| {
                        i.source_frames.unwrap_or(0) * 1000 / i.source_rate as u64
                    }) as i64),
                    "frequency" => Variant::Integer(
                        b.frequency.unwrap_or(b.info.map_or(0, |i| i.source_rate as i64)),
                    ),
                    // name -> %[samplePosition] from the .sli labels.
                    "labels" => {
                        let d = rt.alloc_dictionary_object();
                        for l in b.sli.iter().flat_map(|i| &i.labels) {
                            let e = rt.alloc_dictionary_object();
                            rt.set_object_member(e, "samplePosition", Variant::Integer(l.position as i64));
                            rt.set_object_member(d, l.name.clone(), Variant::Object(e));
                        }
                        Variant::Object(d)
                    }
                    _ => unreachable!(),
                })
            },
            move |rt, t, v| {
                let (h, mut b) = buffer(rt, t)?;
                match name {
                    "volume" | "volume2" => {
                        let n = v.to_integer()?.clamp(0, 100000);
                        if name == "volume" {
                            b.volume = n
                        } else {
                            b.volume2 = n
                        }
                        b.fading = false;
                        control(
                            rt,
                            AudioRequest::SetMixGroupGain {
                                name: b.group(),
                                gain: b.gain(),
                                fade_ms: 0.,
                            },
                        )?;
                    }
                    "pan" => {
                        b.pan = v.to_integer()?.clamp(-100000, 100000);
                        if b.status == "play" {
                            control(
                                rt,
                                AudioRequest::SetPan {
                                    channel: b.channel,
                                    pan: (b.pan as f32 / 200000.) + 0.5,
                                    fade_ms: 0.,
                                },
                            )?;
                        }
                    }
                    "paused" => {
                        b.paused = v.is_truthy();
                        if b.status == "play" {
                            control(
                                rt,
                                AudioRequest::Pause {
                                    channel: b.channel,
                                    paused: b.paused,
                                    fade_ms: 0.,
                                },
                            )?;
                        }
                    }
                    "looping" => {
                        b.looping = v.is_truthy();
                        if b.status == "play" {
                            control(
                                rt,
                                AudioRequest::SetLooped {
                                    channel: b.channel,
                                    looping: b.looping,
                                    loop_start: None,
                                },
                            )?;
                        }
                    }
                    "position" | "samplePosition" => {
                        if b.status == "play" {
                            return Err(unsupported(
                                "live sound seek",
                                "native seek continuation mapping",
                            ));
                        }
                        let n = u64::try_from(v.to_integer()?).map_err(err)?;
                        b.start = if name == "position" {
                            n.checked_mul(
                                b.info.ok_or_else(|| err("seek requires open"))?.source_rate as u64,
                            )
                            .ok_or_else(|| err("sound seek overflow"))?
                                / 1000
                        } else {
                            n
                        };
                    }
                    "frequency" => {
                        let f = v.to_integer()?;
                        if b.status == "play" && Some(f) != b.frequency {
                            return Err(unsupported(
                                "WaveSoundBuffer.frequency change during playback",
                                "live shared voice rate control",
                            ));
                        }
                        b.frequency = Some(f);
                    }
                    _ => return Err(err("read-only sound property")),
                }
                rt.host_mut().sounds.buffers.insert(h.0, b);
                Ok(())
            },
        );
    }
    rt.register_object_native_property(
        c,
        "globalVolume",
        |rt, _| Ok(Variant::Integer(rt.host().sounds.global)),
        |rt, _, v| {
            let n = v.to_integer()?.clamp(0, 100000);
            control(
                rt,
                AudioRequest::SetMixGroupGain {
                    name: "krkr.wave".into(),
                    gain: n as f32 / 100000.,
                    fade_ms: 0.,
                },
            )?;
            rt.host_mut().sounds.global = n;
            Ok(())
        },
    );
}
pub fn drive(rt: &mut Rt, h: &mut dyn HostServices, client: ClientId) -> anyhow::Result<bool> {
    for _ in 0..4096 {
        if let Some(p) = rt.host_mut().sounds.pending.take() {
            match p {
                Pending::Resource { object, ticket, path } => {
                    let status = h.audio_resource_preparation(client, ticket)?;
                    match status.phase {
                        AudioPreparePhase::Pending => {
                            rt.host_mut().sounds.pending =
                                Some(Pending::Resource { object, ticket, path });
                            return Ok(false);
                        }
                        AudioPreparePhase::Failed => {
                            h.release_audio_resource_preparation(client, ticket)?;
                            anyhow::bail!(
                                "native sound open failed: {}",
                                status.error.unwrap_or_default()
                            )
                        }
                        AudioPreparePhase::Ready => {
                            h.release_audio_resource_preparation(client, ticket)?;
                            let resource = status.resource.ok_or_else(|| {
                                anyhow::anyhow!("ready native audio preparation has no resource")
                            })?;
                            let info = status.info.ok_or_else(|| {
                                anyhow::anyhow!("ready native audio preparation has no metadata")
                            })?;
                            if rt.host().loaded_plugins.contains("wuvorbis.dll") && info.has_replay_gain {
                                h.release_audio_resource(client,resource)?;
                                anyhow::bail!("wuvorbis ReplayGain-tagged audio requires a shared explicit metadata gain policy; playback refused");
                            }
                            let s = rt
                                .host_mut()
                                .sounds
                                .buffers
                                .get_mut(&object)
                                .ok_or_else(|| anyhow::anyhow!("native sound owner invalidated"))?;
                            s.resource = Some(resource);
                            s.prepared_path = path;
                            s.info = Some(info);
                            s.opening = false;
                            s.status = "stop";
                            s.start = 0;
                            s.paused = false;
                        }
                    }
                }
                Pending::Voice { object, ticket } => {
                    let status = h.audio_preparation(client, ticket)?;
                    match status.phase {
                        AudioPreparePhase::Pending => {
                            rt.host_mut().sounds.pending = Some(Pending::Voice { object, ticket });
                            return Ok(false);
                        }
                        AudioPreparePhase::Failed => {
                            h.release_audio(client, ticket)?;
                            anyhow::bail!(
                                "native sound play failed: {}",
                                status.error.unwrap_or_default()
                            )
                        }
                        AudioPreparePhase::Ready => {
                            h.release_audio(client, ticket)?;
                            rt.host_mut()
                                .sounds
                                .buffers
                                .get_mut(&object)
                                .ok_or_else(|| anyhow::anyhow!("native sound owner invalidated"))?
                                .status = "play";
                            let old = rt.host().can_suspend;
                            rt.host_mut().can_suspend = false;
                            let called = rt.call_object_method_during_suspend(
                                ObjectHandle(object),
                                "__crysirror_notify_status",
                                vec![],
                            );
                            rt.host_mut().can_suspend = old;
                            called.map_err(|e| anyhow::anyhow!("{e}"))?;
                        }
                    }
                }
            }
        }
        let Some(op) = rt.host_mut().sounds.ops.pop_front() else {
            return Ok(true);
        };
        match op {
            Op::Control(r) => h.submit_audio(client, r)?,
            Op::Open { object, path } => {
                let mounted = rt.host().storage()?.is_mounted_asset(&path);
                let b = rt
                    .host_mut()
                    .sounds
                    .buffers
                    .get_mut(&object)
                    .ok_or_else(|| anyhow::anyhow!("sound owner absent"))?;
                h.submit_audio(
                    client,
                    AudioRequest::Stop {
                        channel: b.channel,
                        fade_ms: 0.,
                    },
                )?;
                let reusable = mounted && b.filtered.is_empty() && b.multitrack.is_none();
                if reusable && b.resource.is_some() && b.prepared_path.as_ref() == Some(&path) {
                    b.opening = false;
                    b.status = "stop";
                    b.start = 0;
                    b.paused = false;
                    continue;
                }
                b.prepared_path = None;
                if let Some(old) = b.resource.take() {
                    h.release_audio_resource(client, old)?;
                }
                b.status = "unload";
                let asset = AudioAsset {
                    path: path.clone(),
                    frame_limit: None,
                };
                let ticket = if let Some(stems) = b.multitrack.clone() {
                    h.begin_transformed_audio_resource(client, asset, AudioResourceTransform::MultiTrack(stems))?
                } else if b.filtered.is_empty() {
                    h.begin_audio_resource(client, asset, true)?
                } else {
                    let chain = b.filtered.clone().into();
                    h.begin_transformed_audio_resource(client, asset, AudioResourceTransform::WaveFilters(chain))?
                };
                rt.host_mut().sounds.pending = Some(Pending::Resource {
                    object, ticket, path: reusable.then_some(path),
                });
            }
            Op::Play(object) => {
                let mut flags = [0; crysirror_resource::sli::MAX_FLAGS];
                if let Some(f) = rt.object_member(ObjectHandle(object), "flags").object_handle() {
                    for (v, e) in flags.iter_mut().zip(rt.array_elements(f).unwrap_or_default()) {
                        *v = e.to_integer().unwrap_or(0);
                    }
                }
                let b = &rt.host().sounds.buffers[&object];
                let sli_loop = sli_loop(b, &flags)?;
                let smooth = b.sli.as_ref().is_some_and(|i| i.links.iter().any(|l| l.taken(&flags) && l.smooth));
                if smooth && !rt.host().sounds.smooth_logged {
                    rt.host_mut().sounds.smooth_logged = true;
                    rt.host_mut().push_log(
                        "WaveSoundBuffer: .sli smooth link loops without its 50 ms junction crossfade".into(),
                    );
                }
                let b = &rt.host().sounds.buffers[&object];
                let rate = match (b.frequency, b.info) {
                    (None, _) => 1.,
                    (Some(f), Some(i)) => {
                        let rate = f as f32 / i.source_rate as f32;
                        anyhow::ensure!(
                            (0.125..=4.).contains(&rate),
                            "WaveSoundBuffer.frequency {f} outside the shared voice rate range"
                        );
                        rate
                    }
                    (Some(_), None) => anyhow::bail!("sound frequency without source metadata"),
                };
                let ticket = h.begin_audio(
                    client,
                    AudioRequest::PlayResource {
                        channel: b.channel,
                        resource: b
                            .resource
                            .ok_or_else(|| anyhow::anyhow!("sound resource missing"))?,
                        parameters: AudioParameters {
                            rate,
                            looping: b.looping || sli_loop.is_some(),
                            loop_start: sli_loop.map_or(0, |l| l.0),
                            loop_end: sli_loop.map(|l| l.1),
                            paused: b.paused,
                            start_frame: Some(b.start),
                            pan: b.pan as f32 / 200000. + 0.5,
                            mix_group: Some(b.group()),
                            ..Default::default()
                        },
                    },
                )?;
                rt.host_mut().sounds.pending = Some(Pending::Voice { object, ticket });
            }
            Op::Release(object) => {
                rt.host_mut().multitrack.retire(object);
                if let Some(b) = rt.host_mut().sounds.buffers.remove(&object) {
                    h.submit_audio(
                        client,
                        AudioRequest::Stop {
                            channel: b.channel,
                            fade_ms: 0.,
                        },
                    )?;
                    if let Some(r) = b.resource {
                        h.release_audio_resource(client, r)?;
                    }
                }
            }
        }
    }
    anyhow::bail!("native sound operation budget")
}
pub fn observations(
    rt: &mut Rt,
    h: &dyn HostServices,
    client: ClientId,
) -> anyhow::Result<Vec<(usize, &'static str)>> {
    let mut callbacks = vec![];
    for (id, b) in &mut rt.host_mut().sounds.buffers {
        if b.status == "play" {
            match h.audio_info(client, b.channel)? {
                Some(info) => {
                    b.start = info.position_frames;
                    if !info.source_playing {
                        b.status = "stop";
                        callbacks.push((*id, "onStatusChanged"));
                    }
                }
                None => {
                    b.status = "stop";
                    callbacks.push((*id, "onStatusChanged"));
                }
            }
        }
        if let Some(g) = h.audio_mix_group(client, &b.group())? {
            b.installed_gain = g.gain;
            if b.fading && g.fade_frames == 0 {
                b.fading = false;
                callbacks.push((*id, "onFadeCompleted"));
            }
        }
    }
    Ok(callbacks)
}

/// The loop a `.sli` file applies with the flag values at play time: one
/// backward link becomes the shared loop range (a smooth link's 50 ms
/// junction crossfade is not reproduced and is logged). Flag changes
/// during playback take effect at the next play (krkrz re-evaluates links
/// live). Other link profiles fail visibly.
fn sli_loop(b: &Buffer, flags: &[i64; crysirror_resource::sli::MAX_FLAGS]) -> anyhow::Result<Option<(u64, u64)>> {
    let Some(info) = &b.sli else { return Ok(None) };
    let taken: Vec<_> = info.links.iter().filter(|l| l.taken(flags)).collect();
    match taken.as_slice() {
        [] => Ok(None),
        [l] if l.to < l.from && !l.smooth => Ok(Some((l.to, l.from))),
        // krkrz crossfades a smooth link over TVP_WL_SMOOTH_TIME (50 ms)
        // around the junction; the shared voice loops at the same points
        // without that crossfade (logged by the caller).
        [l] if l.to < l.from => Ok(Some((l.to, l.from))),
        _ => anyhow::bail!(
            "WaveSoundBuffer .sli link profile is not supported by the CrySirror KiriKiri adapter: \
             requires one backward non-smooth link (taken: {:?})",
            taken
        ),
    }
}

impl Sounds {
    /// Objects with live native sound state or queued operations.
    pub(crate) fn gc_roots(&self, out: &mut Vec<ObjectHandle>) {
        out.extend(self.buffers.keys().map(|k| ObjectHandle(*k)));
        for op in &self.ops {
            match op {
                Op::Open { object, .. } | Op::Play(object) | Op::Release(object) => out.push(ObjectHandle(*object)),
                Op::Control(_) => {}
            }
        }
        if let Some(Pending::Resource { object, .. } | Pending::Voice { object, .. }) = &self.pending {
            out.push(ObjectHandle(*object));
        }
    }
}

/// Playing source and cursor frame of a WaveSoundBuffer, `None` when stopped.
pub(crate) fn sample_source(rt:&Rt,object:ObjectHandle)->crysirror_tjs2::Result<Option<(AudioResourceHandle,u64)>>{
    let buffer=rt.host().sounds.buffers.get(&object.0).ok_or_else(||TjsError::runtime("getSample source is not a WaveSoundBuffer"))?;
    Ok(buffer.playing_resource().map(|r|(r,buffer.start)))
}
pub(crate) fn visualization_source(rt:&Rt,object:ObjectHandle)->crysirror_tjs2::Result<(Option<AudioResourceHandle>,u64)>{
    let buffer=rt.host().sounds.buffers.get(&object.0).ok_or_else(||TjsError::runtime("spectrum source is not a WaveSoundBuffer"))?;
    Ok((if buffer.status=="play" && rt.object_member(object,"__wave_vis_buffer").is_truthy() {buffer.resource}else{None},buffer.start))
}
