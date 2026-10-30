// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `AlphaMovie.dll` facade over the shared engine media service. The movie
//! (AJPM) is decoded by `crysirror-video`'s AlphaMovie decoder (the owner's
//! amv_decoder, as used by xp3-brute) in the media workers; this class keeps
//! only the plugin's members.
//!
//! The plugin binary's source is not available. Members follow how
//! KAGEX's `AMovieControlLayer` (GFX_AMovie.tjs) drives it:
//! `open(storage)`, `play()`, `stop()`, `frame` (read: the presented
//! picture; write: seek), `numOfFrame`, `FPSRate`/`FPSScale` (frame
//! duration `FPSScale / FPSRate` s, the header's rate pair), `screenWidth`/
//! `screenHeight`, and `showNextImage(layer)`, which puts the current
//! picture into `layer` at the movie size and returns its index, or -1 when
//! none is decoded yet. `loop` is an ordinary member read at `play()`.
//! Pictures keep the movie's straight alpha, the adapter's canonical layer
//! representation for ltAlpha and ltAddAlpha.
use crate::natives::*;
use crysirror_core::{
    service::{HostServices, MediaPhase, MediaRequest},
    state::ClientId,
};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};

/// Engine media identities for AlphaMovie (VideoOverlay uses 512..576).
const MEDIA_BASE: u32 = 576;
const MAX_MOVIES: usize = 32;
/// Header and quantisation tables precede the first packet.
const HEADER_PREFIX: usize = 64 * 1024;

#[derive(Debug)]
struct Movie {
    media: u32,
    asset: Option<String>,
    info: Option<crysirror_video::amv::AmvInfo>,
    texture: Option<String>,
    playing: bool,
    /// Picture index published by the media clock, -1 before the first.
    presented: i64,
}

#[derive(Debug)]
enum Request {
    Play(usize),
    Stop(usize),
    Seek(usize, i64),
}

#[derive(Debug, Default)]
pub struct AlphaMovies {
    movies: BTreeMap<usize, Movie>,
    requests: VecDeque<Request>,
}

fn movie<'a>(rt: &'a mut Rt, h: ObjectHandle, what: &str) -> Result<&'a mut Movie> {
    rt.host_mut()
        .alpha_movies
        .movies
        .get_mut(&h.0)
        .ok_or_else(|| TjsError::runtime(format!("{what}: AlphaMovie is not constructed")))
}

fn info(rt: &mut Rt, h: ObjectHandle, what: &str) -> Result<crysirror_video::amv::AmvInfo> {
    movie(rt, h, what)?
        .info
        .ok_or_else(|| TjsError::runtime(format!("{what}: no movie is open")))
}

fn open(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    const M: &str = "AlphaMovie.open";
    let h = this_of(rt, t, M)?;
    let name = string(&a, 0, M)?;
    let storage = rt.host().storage()?;
    let path = storage
        .resolve(&name)
        .ok_or_else(|| TjsError::runtime(format!("AlphaMovie storage absent: {name}")))?;
    let head = storage
        .read_prefix(&path, HEADER_PREFIX)
        .map_err(|e| TjsError::runtime(format!("{path}: {e}")))?;
    let info = crysirror_video::amv::info(&mut std::io::Cursor::new(head))
        .map_err(|e| TjsError::runtime(format!("{path}: {e:#}")))?;
    if movie(rt, h, M)?.playing {
        rt.host_mut().alpha_movies.requests.push_back(Request::Stop(h.0));
    }
    let m = movie(rt, h, M)?;
    m.asset = Some(path);
    m.info = Some(info);
    m.playing = false;
    m.presented = -1;
    rt.set_object_member(h, "__frame", Variant::Integer(0));
    Ok(Variant::Void)
}

fn play(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "AlphaMovie.play")?;
    info(rt, h, "AlphaMovie.play")?;
    let m = movie(rt, h, "AlphaMovie.play")?;
    if !m.playing {
        m.playing = true;
        m.presented = -1;
        rt.host_mut().alpha_movies.requests.push_back(Request::Play(h.0));
    }
    Ok(Variant::Void)
}

fn stop(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "AlphaMovie.stop")?;
    if let Some(m) = rt.host_mut().alpha_movies.movies.get_mut(&h.0)
        && m.playing
    {
        m.playing = false;
        rt.host_mut().alpha_movies.requests.push_back(Request::Stop(h.0));
    }
    Ok(Variant::Void)
}

fn finalize(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    stop(rt, t, a)?;
    let h = this_of(rt, t, "AlphaMovie.finalize")?;
    if let Some(m) = rt.host_mut().alpha_movies.movies.get_mut(&h.0) {
        m.asset = None;
        m.info = None;
    }
    Ok(Variant::Void)
}

fn show_next_image(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    const M: &str = "AlphaMovie.showNextImage";
    let h = this_of(rt, t, M)?;
    let layer = a
        .first()
        .and_then(object_arg)
        .map(|l| rt.bound_this(l).unwrap_or(l))
        .ok_or_else(|| TjsError::runtime(format!("{M}: target must be a Layer")))?;
    let info = info(rt, h, M)?;
    let m = movie(rt, h, M)?;
    let (Some(texture), true) = (m.texture.clone(), m.presented >= 0) else {
        return Ok(Variant::Integer(-1));
    };
    let presented = m.presented;
    let (w, ht) = (i64::from(info.width), i64::from(info.height));
    let l = rt.host().layer(layer)?.clone();
    if [l.image_width, l.image_height] != [w, ht] {
        rt.host_mut().reset_surface(layer, w, ht, [255, 255, 255, 0])?;
    }
    let host = rt.host_mut();
    let slot = host.layer(layer)?.slot;
    host.canvas_op(json!({"op":"set","id":slot,"layer":{"id":0,"rect":[0,0,w,ht],"paint":{"kind":"image","asset":texture}}}));
    let l = host.layer_mut(layer)?;
    (l.width, l.height) = (w, ht);
    l.blank = false;
    l.full_alpha = None;
    l.dirty = true;
    host.api_calls += 1;
    Ok(Variant::Integer(presented))
}

pub fn install(rt: &mut Rt) {
    let c = native_class(rt, "AlphaMovie", |rt, t, _| {
        let h = this_of(rt, t, "AlphaMovie")?;
        rt.set_object_member(h, "__frame", Variant::Integer(0));
        rt.set_object_member(h, "loop", Variant::Integer(0));
        let movies = &mut rt.host_mut().alpha_movies.movies;
        if !movies.contains_key(&h.0) {
            if movies.len() >= MAX_MOVIES {
                return Err(TjsError::runtime("AlphaMovie limit"));
            }
            let media = (0..MAX_MOVIES as u32)
                .map(|i| MEDIA_BASE + i)
                .find(|m| !movies.values().any(|o| o.media == *m))
                .expect("bounded above");
            movies.insert(
                h.0,
                Movie { media, asset: None, info: None, texture: None, playing: false, presented: -1 },
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
            ("play", play),
            ("stop", stop),
            ("showNextImage", show_next_image),
        ],
    );
    rt.register_object_native_property(
        c,
        "frame",
        |rt, t| {
            let h = this_of(rt, t, "AlphaMovie.frame")?;
            Ok(rt.object_member(h, "__frame"))
        },
        |rt, t, v| {
            let h = this_of(rt, t, "AlphaMovie.frame")?;
            let frame = v.to_integer()?;
            let frames = i64::from(info(rt, h, "AlphaMovie.frame")?.frames);
            if !(0..frames.max(1)).contains(&frame) {
                return Err(TjsError::runtime(format!("AlphaMovie.frame {frame} out of 0..{frames}")));
            }
            rt.set_object_member(h, "__frame", Variant::Integer(frame));
            if movie(rt, h, "AlphaMovie.frame")?.playing {
                rt.host_mut().alpha_movies.requests.push_back(Request::Seek(h.0, frame));
            }
            Ok(())
        },
    );
    for (name, get) in [
        ("numOfFrame", (|i| i64::from(i.frames)) as fn(&crysirror_video::amv::AmvInfo) -> i64),
        ("FPSRate", |i| i64::from(i.rate)),
        ("FPSScale", |i| i64::from(i.scale)),
        ("screenWidth", |i| i64::from(i.width)),
        ("screenHeight", |i| i64::from(i.height)),
    ] {
        rt.register_object_native_property(
            c,
            name,
            move |rt, t| {
                let h = this_of(rt, t, "AlphaMovie")?;
                Ok(Variant::Integer(get(&info(rt, h, "AlphaMovie")?)))
            },
            read_only,
        );
    }
}

/// Execute queued requests against the shared media service.
pub fn drive(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<()> {
    while let Some(request) = rt.host_mut().alpha_movies.requests.pop_front() {
        match request {
            Request::Play(id) => {
                let h = ObjectHandle(id);
                let Some(m) = rt.host().alpha_movies.movies.get(&id) else { continue };
                let (media, Some(asset)) = (m.media, m.asset.clone()) else { continue };
                let looping = rt.object_member(h, "loop").is_truthy();
                let texture = host.media_request(
                    client,
                    MediaRequest::Play { id: media, asset, looping, audio: false, gain: 1. },
                )?;
                if let Some(m) = rt.host_mut().alpha_movies.movies.get_mut(&id) {
                    m.texture = Some(texture.relative().to_owned());
                }
                let start = rt.object_member(h, "__frame").to_integer().unwrap_or(0);
                if start > 0 {
                    rt.host_mut().alpha_movies.requests.push_front(Request::Seek(id, start));
                }
            }
            Request::Stop(id) => {
                let Some(media) = rt.host().alpha_movies.movies.get(&id).map(|m| m.media) else {
                    continue;
                };
                match host.media_request(client, MediaRequest::Stop { id: media }) {
                    Ok(_) | Err(crysirror_core::Error::NotFound) => {}
                    Err(e) if e.to_string().contains("media handle not found") => {}
                    Err(e) => return Err(e.into()),
                }
                if let Some(m) = rt.host_mut().alpha_movies.movies.get_mut(&id) {
                    m.texture = None;
                    m.presented = -1;
                }
            }
            Request::Seek(id, frame) => {
                let Some(m) = rt.host().alpha_movies.movies.get(&id) else { continue };
                let (media, Some(info)) = (m.media, m.info) else { continue };
                let position_ms = (frame.max(0) as f64 * info.frame_ms).round() as u64;
                host.media_request(client, MediaRequest::Seek { id: media, position_ms })?;
            }
        }
    }
    Ok(())
}

/// Once per frame: the presented picture index follows the media clock.
pub fn observe(rt: &mut Rt, host: &dyn HostServices, client: ClientId) -> anyhow::Result<()> {
    let active: Vec<(usize, u32, f64)> = rt
        .host()
        .alpha_movies
        .movies
        .iter()
        .filter(|(_, m)| m.playing && m.texture.is_some())
        .filter_map(|(id, m)| Some((*id, m.media, m.info?.frame_ms)))
        .collect();
    for (id, media, frame_ms) in active {
        let Ok(info) = host.media_info(client, media) else { continue };
        if info.phase == MediaPhase::Failed {
            anyhow::bail!("AlphaMovie: shared media decoding failed");
        }
        if info.published_frames == 0 {
            continue;
        }
        let frame = (info.position_ms as f64 / frame_ms) as i64;
        let frames = rt.host().alpha_movies.movies.get(&id).and_then(|m| m.info).map_or(1, |i| i64::from(i.frames).max(1));
        // The clock can pass the last picture before end-of-stream is seen;
        // a loop restarts the media clock itself.
        let frame = if info.phase == MediaPhase::Ended { frames - 1 } else { frame.min(frames - 1) };
        if let Some(m) = rt.host_mut().alpha_movies.movies.get_mut(&id) {
            m.presented = frame;
        }
        rt.set_object_member(ObjectHandle(id), "__frame", Variant::Integer(frame));
    }
    Ok(())
}
