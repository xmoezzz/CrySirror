// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `VideoOverlay` over the shared engine media service (`MediaRequest`, the
//! runtime's MPEG/ASF/OMV decoders). The facade keeps only native status and
//! event naming:
//!
//! - `open` resolves the storage; `prepare` checks the container is one the
//!   shared decoders accept and then reports `onPeriod(perPrepare)` (there is
//!   no separate pre-roll; decoding starts at `play`).
//! - `play`/`stop`/`close` become `MediaRequest::Play/Stop`; status changes
//!   reach the script as `onStatusChanged(status)` through the deferred
//!   dispatch queue, and media `Ended`/`Failed` becomes `"stop"`.
//! - Layer mode (`vomLayer`) paints the media texture into `layer1`/`layer2`
//!   (scaled to their image size); overlay/mixer modes show it in an engine
//!   layer above every KiriKiri layer at the overlay's bounds.
//! - `pause` freezes the shared media clock (status `"pause"`), `play`
//!   resumes it; `rewind` and `frame = n` seek the shared media (krkrz
//!   VideoOvlImpl.cpp Pause/Rewind/SetFrame), keeping the current state.
//!   `frame` reads the presented picture index. Segment loops and stop
//!   frames need media transport the shared service does not offer and fail
//!   visibly.
use crate::natives::*;
use crysirror_core::{
    deferred_dispatch::DispatchPhase,
    service::{HostServices, MediaPhase, MediaRequest},
    state::ClientId,
};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};

/// Engine media identities owned by this adapter (RealLive uses 0).
const MEDIA_BASE: u32 = 512;
/// Engine layer ids for overlay-mode output, above every KiriKiri layer.
const OVERLAY_LAYER: u32 = 0x7e00_0000;
const MAX_OVERLAYS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Status {
    Unload,
    Stop,
    Play,
    Pause,
}

impl Status {
    fn name(&self) -> &'static str {
        match self {
            Status::Unload => "unload",
            Status::Stop => "stop",
            Status::Play => "play",
            Status::Pause => "pause",
        }
    }
}

#[derive(Debug)]
pub(crate) struct Overlay {
    media: u32,
    asset: Option<String>,
    status: Status,
    texture: Option<String>,
    shown: bool,
    statuses: VecDeque<&'static str>,
    periods: VecDeque<i64>,
}

#[derive(Debug)]
enum Request {
    Play(usize),
    Stop(usize),
    Pause(usize),
    Resume(usize),
    /// Seek to a picture index.
    Seek(usize, i64),
}

#[derive(Debug, Default)]
pub struct Videos {
    overlays: BTreeMap<usize, Overlay>,
    requests: VecDeque<Request>,
}

fn overlay<'a>(rt: &'a mut Rt, h: ObjectHandle, what: &str) -> Result<&'a mut Overlay> {
    rt.host_mut()
        .videos
        .overlays
        .get_mut(&h.0)
        .ok_or_else(|| TjsError::runtime(format!("{what}: VideoOverlay is not constructed")))
}

/// Queue `onStatusChanged(status)` (deferred kind 2) or `onPeriod(type)`
/// (kind 3) for this object.
fn post(rt: &mut Rt, h: ObjectHandle, status: Option<&'static str>, period: Option<i64>) -> Result<()> {
    let o = overlay(rt, h, "VideoOverlay event")?;
    let kind = match (status, period) {
        (Some(s), _) => {
            o.statuses.push_back(s);
            2
        }
        (None, Some(p)) => {
            o.periods.push_back(p);
            3
        }
        _ => return Ok(()),
    };
    let client = rt.host().client;
    rt.host_mut()
        .deferred
        .post(
            client,
            crate::async_trigger::target(h.0, kind),
            DispatchPhase::Exclusive,
            false,
        )
        .map_err(|e| TjsError::runtime(e.to_string()))
}

/// krkrz `SetStatus` (VideoOvlIntf.cpp:84-108): the native call itself
/// fires `onStatusChanged` (TVP_EPT_IMMEDIATE) after cancelling this
/// object's undelivered events (TVPCancelSourceEvents).
fn set_status(rt: &mut Rt, h: ObjectHandle, status: Status) -> Result<()> {
    let o = overlay(rt, h, "VideoOverlay")?;
    if o.status == status {
        return Ok(());
    }
    o.status = status.clone();
    o.statuses.clear();
    o.periods.clear();
    let client = rt.host().client;
    for kind in [2, 3] {
        rt.host_mut()
            .deferred
            .cancel(client, crate::async_trigger::target(h.0, kind));
    }
    rt.call_object_method(h, "onStatusChanged", vec![Variant::String(status.name().into())])?;
    Ok(())
}

/// krkrz `SetStatusAsync` (:110-133): media-driven changes are posted.
fn set_status_async(rt: &mut Rt, h: ObjectHandle, status: Status) -> Result<()> {
    let o = overlay(rt, h, "VideoOverlay")?;
    if o.status == status {
        return Ok(());
    }
    o.status = status.clone();
    o.statuses.clear();
    o.periods.clear();
    let client = rt.host().client;
    for kind in [2, 3] {
        rt.host_mut()
            .deferred
            .cancel(client, crate::async_trigger::target(h.0, kind));
    }
    post(rt, h, Some(status.name()), None)
}

fn open(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "VideoOverlay.open")?;
    let name = string(&a, 0, "VideoOverlay.open")?;
    let path = rt
        .host()
        .storage()?
        .resolve(&name)
        .ok_or_else(|| TjsError::runtime(format!("video storage absent: {name}")))?;
    if active(overlay(rt, h, "VideoOverlay.open")?) {
        rt.host_mut().videos.requests.push_back(Request::Stop(h.0));
    }
    overlay(rt, h, "VideoOverlay.open")?.asset = Some(path);
    set_status(rt, h, Status::Stop)?;
    Ok(Variant::Void)
}

fn prepare(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "VideoOverlay.prepare")?;
    let asset = overlay(rt, h, "VideoOverlay.prepare")?
        .asset
        .clone()
        .ok_or_else(|| TjsError::runtime("VideoOverlay.prepare: no video is open"))?;
    // Accept only containers the shared decoders handle, so a later play
    // cannot have been reported ready for a format that will fail.
    let head = rt
        .host()
        .storage()?
        .read_prefix(&asset, 16)
        .map_err(|e| TjsError::runtime(format!("{asset}: {e}")))?;
    const ASF: [u8; 4] = [0x30, 0x26, 0xb2, 0x75];
    let mpeg = head.starts_with(&[0, 0, 1, 0xba]) || head.starts_with(&[0, 0, 1, 0xb3]);
    if !(head.starts_with(&ASF) || mpeg || head.starts_with(b"OggS")) {
        return Err(unsupported(
            "VideoOverlay.prepare",
            "a shared decoder for this video container",
        ));
    }
    // krkr perPrepare (2): the video is ready to start.
    rt.host_mut().log.push(format!("VideoOverlay.prepare {asset}"));
    post(rt, h, None, Some(2))?;
    Ok(Variant::Void)
}

fn play(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "VideoOverlay.play")?;
    if overlay(rt, h, "VideoOverlay.play")?.asset.is_none() {
        return Err(TjsError::runtime("VideoOverlay.play: no video is open"));
    }
    let request = match overlay(rt, h, "VideoOverlay.play")?.status {
        Status::Pause => Request::Resume(h.0),
        Status::Play => return Ok(Variant::Void),
        _ => Request::Play(h.0),
    };
    rt.host_mut().videos.requests.push_back(request);
    set_status(rt, h, Status::Play)?;
    Ok(Variant::Void)
}

fn active(o: &Overlay) -> bool {
    matches!(o.status, Status::Play | Status::Pause)
}

fn pause(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "VideoOverlay.pause")?;
    if overlay(rt, h, "VideoOverlay.pause")?.status == Status::Play {
        rt.host_mut().videos.requests.push_back(Request::Pause(h.0));
        set_status(rt, h, Status::Pause)?;
    }
    Ok(Variant::Void)
}

/// krkrz Rewind: back to the first picture in the current state.
fn rewind(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "VideoOverlay.rewind")?;
    seek_frame(rt, h, 0)?;
    Ok(Variant::Void)
}

fn seek_frame(rt: &mut Rt, h: ObjectHandle, frame: i64) -> Result<()> {
    if frame < 0 {
        return Err(TjsError::runtime("VideoOverlay frame must be non-negative"));
    }
    rt.set_object_member(h, "__frame", Variant::Integer(frame));
    if active(overlay(rt, h, "VideoOverlay")?) {
        rt.host_mut().videos.requests.push_back(Request::Seek(h.0, frame));
    }
    Ok(())
}

fn stop(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "VideoOverlay.stop")?;
    if active(overlay(rt, h, "VideoOverlay.stop")?) {
        rt.host_mut().log.push("VideoOverlay.stop native".into());
        rt.host_mut().videos.requests.push_back(Request::Stop(h.0));
        set_status(rt, h, Status::Stop)?;
    }
    Ok(Variant::Void)
}

fn close(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "VideoOverlay.close")?;
    if rt.host().videos.overlays.get(&h.0).is_none() {
        return Ok(Variant::Void);
    }
    if active(overlay(rt, h, "VideoOverlay.close")?) {
        rt.host_mut().log.push("VideoOverlay.close native while playing".into());
        rt.host_mut().videos.requests.push_back(Request::Stop(h.0));
    }
    overlay(rt, h, "VideoOverlay.close")?.asset = None;
    set_status(rt, h, Status::Unload)?;
    Ok(Variant::Void)
}

/// Invalidation: release the media without script events (krkrz clears
/// CanDeliverEvents before the native finalizer closes the video).
fn finalize(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "VideoOverlay.finalize")?;
    let Some(o) = rt.host_mut().videos.overlays.get_mut(&h.0) else {
        return Ok(Variant::Void);
    };
    let playing = active(o);
    o.asset = None;
    o.status = Status::Unload;
    o.statuses.clear();
    o.periods.clear();
    if playing {
        rt.host_mut().videos.requests.push_back(Request::Stop(h.0));
    }
    Ok(Variant::Void)
}

pub fn install(rt: &mut Rt) {
    let c = native_class(rt, "VideoOverlay", |rt, t, _| {
        let h = this_of(rt, t, "VideoOverlay")?;
        for (n, v) in [
            ("mode", 0),
            ("visible", 0),
            ("loop", 0),
            ("periodEventFrame", -1),
            ("left", 0),
            ("top", 0),
            ("width", 0),
            ("height", 0),
            ("volume", 100000),
            ("pan", 0),
            ("audioVolume", 100000),
            ("audioBalance", 0),
            ("segmentLoopStartFrame", -1),
            ("segmentLoopEndFrame", -1),
        ] {
            rt.set_object_member(h, n, Variant::Integer(v));
        }
        rt.set_object_member(h, "playRate", Variant::Real(1.0));
        rt.set_object_member(h, "__frame", Variant::Integer(0));
        for layer in ["layer1", "layer2"] {
            rt.set_object_member(h, layer, Variant::Null);
        }
        let videos = &mut rt.host_mut().videos;
        if !videos.overlays.contains_key(&h.0) {
            if videos.overlays.len() >= MAX_OVERLAYS {
                return Err(TjsError::runtime("VideoOverlay limit"));
            }
            let media = (0..MAX_OVERLAYS as u32)
                .map(|i| MEDIA_BASE + i)
                .find(|m| !videos.overlays.values().any(|o| o.media == *m))
                .expect("bounded above");
            videos.overlays.insert(
                h.0,
                Overlay {
                    media,
                    asset: None,
                    status: Status::Unload,
                    texture: None,
                    shown: false,
                    statuses: VecDeque::new(),
                    periods: VecDeque::new(),
                },
            );
        }
        Ok(Variant::Void)
    });
    methods(
        rt,
        c,
        &[
            ("finalize", finalize),
            ("open", open),
            ("prepare", prepare),
            ("play", play),
            ("stop", stop),
            ("close", close),
            ("pause", pause),
            ("rewind", rewind),
            // (-1, -1) is "no segment loop" (what a save without one restores).
            ("setSegmentLoop", |rt, t, a| {
                let h = this_of(rt, t, "VideoOverlay.setSegmentLoop")?;
                if (int(&a, 0, -1)?, int(&a, 1, -1)?) != (-1, -1) {
                    return Err(unsupported("VideoOverlay.setSegmentLoop", "shared media segment loops"));
                }
                rt.set_object_member(h, "segmentLoopStartFrame", Variant::Integer(-1));
                rt.set_object_member(h, "segmentLoopEndFrame", Variant::Integer(-1));
                Ok(Variant::Void)
            }),
            // The shared decoders play the container's first audio stream.
            ("selectAudioStream", |rt, t, a| {
                let h = this_of(rt, t, "VideoOverlay.selectAudioStream")?;
                let open = overlay(rt, h, "VideoOverlay.selectAudioStream")?.asset.is_some();
                if !open || int(&a, 0, 0)? != 0 {
                    return Err(unsupported(
                        "VideoOverlay.selectAudioStream",
                        "shared media audio stream selection",
                    ));
                }
                Ok(Variant::Void)
            }),
            // Deferred dispatch targets: one queued event per post.
            ("__crysirror_notify_status", |rt, t, _| {
                let h = this_of(rt, t, "VideoOverlay")?;
                let Some(status) = overlay(rt, h, "VideoOverlay")?.statuses.pop_front() else {
                    return Ok(Variant::Void);
                };
                rt.call_object_method(h, "onStatusChanged", vec![Variant::String(status.into())])
            }),
            ("__crysirror_notify_fade", |rt, t, _| {
                let h = this_of(rt, t, "VideoOverlay")?;
                let Some(period) = overlay(rt, h, "VideoOverlay")?.periods.pop_front() else {
                    return Ok(Variant::Void);
                };
                rt.call_object_method(h, "onPeriod", vec![Variant::Integer(period)])
            }),
            ("onStatusChanged", |_, _, _| Ok(Variant::Void)),
            ("onPeriod", |_, _, _| Ok(Variant::Void)),
        ],
    );
    // Geometry/mixing state is recorded and applied to the engine output.
    methods(
        rt,
        c,
        &[
            ("setPos", |rt, t, a| {
                let h = this_of(rt, t, "VideoOverlay.setPos")?;
                for (i, k) in ["left", "top"].iter().enumerate() {
                    rt.set_object_member(h, *k, Variant::Integer(int(&a, i, 0)?));
                }
                Ok(Variant::Void)
            }),
            ("setSize", |rt, t, a| {
                let h = this_of(rt, t, "VideoOverlay.setSize")?;
                for (i, k) in ["width", "height"].iter().enumerate() {
                    rt.set_object_member(h, *k, Variant::Integer(int(&a, i, 0)?));
                }
                Ok(Variant::Void)
            }),
            ("setBounds", |rt, t, a| {
                let h = this_of(rt, t, "VideoOverlay.setBounds")?;
                for (i, k) in ["left", "top", "width", "height"].iter().enumerate() {
                    rt.set_object_member(h, *k, Variant::Integer(int(&a, i, 0)?));
                }
                Ok(Variant::Void)
            }),
        ],
    );
    for (name, open_value, closed_value) in [
        ("numberOfAudioStream", 1, 0),
        ("enabledAudioStream", 0, -1),
    ] {
        rt.register_object_native_property(
            c,
            name,
            move |rt: &mut Rt, t: Option<ObjectHandle>| {
                let h = this_of(rt, t, "VideoOverlay")?;
                let open = rt.host().videos.overlays.get(&h.0).is_some_and(|o| o.asset.is_some());
                Ok(Variant::Integer(if open { open_value } else { closed_value }))
            },
            read_only,
        );
    }
    // Frame positioning is a shared media seek; reads give the presented
    // picture index (updated once per frame by `observe`).
    rt.register_object_native_property(
        c,
        "frame",
        |rt, t| {
            let h = this_of(rt, t, "VideoOverlay.frame")?;
            Ok(rt.object_member(h, "__frame"))
        },
        |rt, t, v| {
            let h = this_of(rt, t, "VideoOverlay.frame")?;
            seek_frame(rt, h, v.to_integer()?)
        },
    );
    rt.register_object_native_property(
        c,
        "status",
        |rt, t| {
            let h = this_of(rt, t, "VideoOverlay.status")?;
            Ok(Variant::String(
                rt.host()
                    .videos
                    .overlays
                    .get(&h.0)
                    .map_or("unload", |o| o.status.name())
                    .into(),
            ))
        },
        read_only,
    );
}

fn member_int(rt: &Rt, h: ObjectHandle, name: &str) -> i64 {
    rt.object_member(h, name).to_integer().unwrap_or(0)
}

/// Paint the media texture where the native mode puts it.
fn present(rt: &mut Rt, h: ObjectHandle) -> anyhow::Result<()> {
    let Some(o) = rt.host().videos.overlays.get(&h.0) else {
        return Ok(());
    };
    let (texture, media, playing) = (o.texture.clone(), o.media, active(o));
    let mode = member_int(rt, h, "mode");
    if mode == 1 {
        // vomLayer: frames go into layer1/layer2 at their image size.
        let Some(texture) = texture.filter(|_| playing) else {
            return Ok(());
        };
        for name in ["layer1", "layer2"] {
            let Some(layer) = object_arg(&rt.object_member(h, name)) else {
                continue;
            };
            let Ok(l) = rt.host().layer(layer) else {
                continue;
            };
            let (slot, w, height) = (l.slot, l.image_width.max(1), l.image_height.max(1));
            rt.host_mut().canvas_op(json!({"op":"set","id":slot,"layer":{"id":0,"rect":[0,0,w,height],"paint":{"kind":"image","asset":texture}}}));
            if let Ok(l) = rt.host_mut().layer_mut(layer) {
                l.blank = false;
                l.dirty = true;
            }
        }
        return Ok(());
    }
    let id = OVERLAY_LAYER + media;
    let visible = playing && rt.object_member(h, "visible").is_truthy();
    match texture.filter(|_| visible) {
        Some(texture) => {
            let [cw, ch] = rt.host().canvas.map(i64::from);
            let (w, height) = match (member_int(rt, h, "width"), member_int(rt, h, "height")) {
                (w, height) if w > 0 && height > 0 => (w, height),
                _ => (cw, ch),
            };
            let (x, y) = (member_int(rt, h, "left"), member_int(rt, h, "top"));
            rt.host_mut().layer_op(json!({"op":"put","layer":{"id":id,"rect":[x,y,w,height],"order":[1_000_000,0],"paint":{"kind":"image","asset":texture}}}));
            overlay(rt, h, "VideoOverlay").map_err(|e| anyhow::anyhow!("{e}"))?.shown = true;
        }
        None => {
            let o = overlay(rt, h, "VideoOverlay").map_err(|e| anyhow::anyhow!("{e}"))?;
            if o.shown {
                o.shown = false;
                rt.host_mut().layer_op(json!({"op":"remove","id":id}));
            }
        }
    }
    Ok(())
}

/// Execute queued requests against the shared media service.
pub fn drive(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<()> {
    while let Some(request) = rt.host_mut().videos.requests.pop_front() {
        match request {
            Request::Play(id) => {
                let h = ObjectHandle(id);
                let Some(o) = rt.host().videos.overlays.get(&id) else {
                    continue;
                };
                let (media, asset) = (o.media, o.asset.clone());
                let Some(asset) = asset else { continue };
                let looping = rt.object_member(h, "loop").is_truthy();
                let gain = (member_int(rt, h, "volume") as f32 / 100_000.).clamp(0., 1.);
                let mode = member_int(rt, h, "mode");
                rt.host_mut().log.push(format!("VideoOverlay play {asset} media={media} mode={mode}"));
                let texture = host.media_request(
                    client,
                    MediaRequest::Play {
                        id: media,
                        asset,
                        looping,
                        audio: true,
                        gain,
                    },
                )?;
                if let Some(o) = rt.host_mut().videos.overlays.get_mut(&id) {
                    o.texture = Some(texture.relative().to_owned());
                }
                // A frame set while stopped is where playback starts.
                let start = member_int(rt, h, "__frame");
                if start > 0 {
                    rt.host_mut().videos.requests.push_front(Request::Seek(id, start));
                }
                present(rt, h)?;
            }
            Request::Pause(id) | Request::Resume(id) => {
                let Some(media) = rt.host().videos.overlays.get(&id).map(|o| o.media) else {
                    continue;
                };
                let request = if matches!(request, Request::Pause(_)) {
                    MediaRequest::Pause { id: media }
                } else {
                    MediaRequest::Resume { id: media }
                };
                host.media_request(client, request)?;
            }
            Request::Seek(id, frame) => {
                let Some(media) = rt.host().videos.overlays.get(&id).map(|o| o.media) else {
                    continue;
                };
                // Before the decoder reports its picture duration only the
                // first picture has a known position.
                let frame_ms = host.media_info(client, media).map(|i| i.frame_ms).unwrap_or(0);
                if frame > 0 && frame_ms == 0 {
                    rt.host_mut().videos.requests.push_back(Request::Seek(id, frame));
                    break;
                }
                let position_ms = u64::try_from(frame).unwrap_or(0).saturating_mul(frame_ms);
                host.media_request(client, MediaRequest::Seek { id: media, position_ms })?;
            }
            Request::Stop(id) => {
                let Some(media) = rt.host().videos.overlays.get(&id).map(|o| o.media) else {
                    continue;
                };
                rt.host_mut().log.push(format!("VideoOverlay stop media={media}"));
                // A video that already ended may have been released by the
                // media service; stopping it is then a no-op.
                match host.media_request(client, MediaRequest::Stop { id: media }) {
                    Ok(_) | Err(crysirror_core::Error::NotFound) => {}
                    Err(e) if e.to_string().contains("media handle not found") => {}
                    Err(e) => return Err(e.into()),
                }
                if let Some(o) = rt.host_mut().videos.overlays.get_mut(&id) {
                    o.texture = None;
                }
                present(rt, ObjectHandle(id))?;
            }
        }
    }
    Ok(())
}

/// Once per frame: playback that ended or failed becomes `"stop"`, and
/// overlay geometry/visibility follows the script's members.
pub fn observe(rt: &mut Rt, host: &dyn HostServices, client: ClientId) -> anyhow::Result<()> {
    let playing: Vec<(usize, u32)> = rt
        .host()
        .videos
        .overlays
        .iter()
        .filter(|(_, o)| active(o) && o.texture.is_some())
        .map(|(id, o)| (*id, o.media))
        .collect();
    for (id, media) in playing {
        let h = ObjectHandle(id);
        let info = host.media_info(client, media).ok();
        if let Some(info) = info.filter(|i| i.frame_ms > 0 && i.published_frames > 0) {
            let frame = (info.position_ms / info.frame_ms) as i64;
            rt.set_object_member(h, "__frame", Variant::Integer(frame));
        }
        let phase = info.map(|i| i.phase);
        match phase {
            Some(MediaPhase::Loading | MediaPhase::Playing) => present(rt, h)?,
            Some(MediaPhase::Failed) => {
                rt.host_mut().log.push("VideoOverlay: shared media decoding failed".into());
                rt.host_mut().videos.requests.push_back(Request::Stop(id));
                set_status_async(rt, h, Status::Stop).map_err(|e| anyhow::anyhow!("{e}"))?;
            }
            other => {
                rt.host_mut().log.push(format!("VideoOverlay media={media} ended: {other:?}"));
                rt.host_mut().videos.requests.push_back(Request::Stop(id));
                set_status_async(rt, h, Status::Stop).map_err(|e| anyhow::anyhow!("{e}"))?;
            }
        }
    }
    Ok(())
}
