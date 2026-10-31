// SPDX-License-Identifier: AGPL-3.0-or-later
//! layerExMovie native policy over engine Media; no DirectShow or pixel loop.
//! Reference: krkrz 49c4d53506edecb824cd7b2cff8d32959b1f1b70,
//! src/plugins/win32/layerExMovie. Original W.Dee/Watanabe notices retained.
use crate::natives::*;
use anyhow::Context;
use crysirror_core::{
    deferred_dispatch::DispatchPhase,
    service::{HostServices, MediaPhase, MediaRequest},
    state::ClientId,
};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{Closure, ObjectHandle, Variant},
};
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
const MEDIA_BASE: u32 = 640;
const LIMIT: usize = 32;
struct Movie {
    media: u32,
    asset: Option<String>,
    texture: Option<String>,
    size: Option<[u32; 2]>,
    opening: bool,
    playing: bool,
    revision: u64,
    callbacks: [Option<Variant>; 3],
    events: VecDeque<Variant>,
}
#[derive(Default)]
pub struct LayerMovies {
    movies: BTreeMap<usize, Movie>,
    requests: VecDeque<Request>,
}
enum Request {
    Open(usize),
    Start(usize, bool),
    Stop(u32),
    Remove(u32),
}
impl LayerMovies {
    pub fn retire(&mut self, id: usize) -> bool {
        if let Some(movie) = self.movies.remove(&id) {
            self.requests.push_back(Request::Remove(movie.media));
            true
        } else {
            false
        }
    }
    pub fn gc_roots(&self, out: &mut Vec<ObjectHandle>) {
        for movie in self.movies.values() {
            for v in movie.callbacks.iter().flatten().chain(movie.events.iter()) {
                match v {
                    Variant::Object(h) => out.push(*h),
                    Variant::Closure(c) => out.extend(std::iter::once(c.object).chain(c.this_obj)),
                    _ => {}
                }
            }
        }
    }
}
fn callback(rt: &mut Rt, h: ObjectHandle, name: &str) -> Result<Option<Variant>> {
    let v = match rt.resolve_object_member(h, name) {
        Ok(v) => v,
        Err(e) if e.is_member_not_found() => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(match v {
        Variant::Void | Variant::Null => None,
        Variant::Closure(mut c) => {
            if c.this_obj.is_none() {
                c.this_obj = Some(h);
            }
            Some(Variant::Closure(c))
        }
        Variant::Object(o) => Some(Variant::Closure(Closure::new(o, Some(h)))),
        _ => {
            return Err(TjsError::runtime(format!(
                "{name}: callback is not an object"
            )));
        }
    })
}
fn ensure_movie(rt: &mut Rt, h: ObjectHandle) -> Result<()> {
    rt.host().layer(h)?;
    if rt.host().layer_movies.movies.contains_key(&h.0) {
        return Ok(());
    }
    if rt.host().layer_movies.movies.len() >= LIMIT {
        return Err(TjsError::runtime("layerExMovie resource limit"));
    }
    let media = (MEDIA_BASE..MEDIA_BASE + LIMIT as u32)
        .find(|id| {
            !rt.host()
                .layer_movies
                .movies
                .values()
                .any(|m| m.media == *id)
        })
        .unwrap();
    let callbacks = [
        callback(rt, h, "onStartMovie")?,
        callback(rt, h, "onUpdateMovie")?,
        callback(rt, h, "onStopMovie")?,
    ];
    rt.host_mut().layer_movies.movies.insert(
        h.0,
        Movie {
            media,
            asset: None,
            texture: None,
            size: None,
            opening: false,
            playing: false,
            revision: 0,
            callbacks,
            events: VecDeque::new(),
        },
    );
    Ok(())
}
fn open(rt: &mut Rt, t: Option<ObjectHandle>, args: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.openMovie")?;
    if args.get(1).is_some_and(Variant::is_truthy) {
        return Err(unsupported(
            "Layer.openMovie alpha=true",
            "shared side-by-side movie alpha extraction",
        ));
    }
    if !rt.host().can_suspend {
        return Err(unsupported(
            "Layer.openMovie in this callback",
            "resumable media metadata preparation",
        ));
    }
    let name = string(&args, 0, "Layer.openMovie")?;
    let asset = rt
        .host()
        .storage()?
        .resolve(&name)
        .ok_or_else(|| TjsError::runtime(format!("movie storage absent: {name}")))?;
    ensure_movie(rt, h)?;
    let m = rt.host_mut().layer_movies.movies.get_mut(&h.0).unwrap();
    if m.playing {
        return Err(unsupported(
            "Layer.openMovie while playing",
            "an explicit stop before reopening",
        ));
    }
    m.asset = Some(asset);
    m.texture = None;
    m.size = None;
    m.opening = true;
    m.revision = 0;
    rt.host_mut()
        .layer_movies
        .requests
        .push_back(Request::Open(h.0));
    rt.request_suspend();
    Ok(Variant::Void)
}
fn start(rt: &mut Rt, t: Option<ObjectHandle>, args: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.startMovie")?;
    ensure_movie(rt, h)?;
    let m = rt.host_mut().layer_movies.movies.get_mut(&h.0).unwrap();
    if m.asset.is_none() {
        return Ok(Variant::Void);
    }
    if m.opening || m.size.is_none() {
        return Err(TjsError::runtime("movie metadata has not completed"));
    }
    if m.playing {
        return Err(unsupported(
            "Layer.startMovie while playing",
            "an explicit stop/open cycle before restarting",
        ));
    }
    m.playing = true;
    m.revision = 0;
    let cb = m.callbacks[0].clone();
    rt.host_mut()
        .layer_movies
        .requests
        .push_back(Request::Start(
            h.0,
            args.first().is_some_and(Variant::is_truthy),
        ));
    if let Some(cb) = cb {
        rt.call_function(cb, vec![])?;
    }
    Ok(Variant::Void)
}
fn stop(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.stopMovie")?;
    ensure_movie(rt, h)?;
    let m = rt.host_mut().layer_movies.movies.get_mut(&h.0).unwrap();
    let playing = std::mem::replace(&mut m.playing, false);
    m.opening = false;
    m.asset = None;
    let media = m.media;
    let cb = m.callbacks[2].clone();
    m.events.clear();
    let client = rt.host().client;
    rt.host_mut()
        .deferred
        .cancel(client, crate::async_trigger::target(h.0, 3));
    rt.host_mut()
        .layer_movies
        .requests
        .push_back(Request::Stop(media));
    if playing {
        if let Some(cb) = cb {
            rt.call_function(cb, vec![])?;
        }
    }
    Ok(Variant::Void)
}
pub fn install(rt: &mut Rt) {
    let class = rt.global_member("Layer").object_handle().unwrap();
    methods(
        rt,
        class,
        &[
            ("openMovie", open),
            ("startMovie", start),
            ("stopMovie", stop),
            ("isPlayingMovie", |rt, t, _| {
                let h = this_of(rt, t, "Layer.isPlayingMovie")?;
                ensure_movie(rt, h)?;
                Ok(Variant::Integer(i64::from(
                    rt.host().layer_movies.movies[&h.0].playing,
                )))
            }),
        ],
    );
}
/// Real metadata completion fences the original TJS call before geometry reads.
pub fn drive(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<bool> {
    while let Some(request) = rt.host_mut().layer_movies.requests.pop_front() {
        match request {
            Request::Open(id) => {
                let m = &rt.host().layer_movies.movies[&id];
                let (media, asset) = (m.media, m.asset.clone().unwrap());
                let texture = host.media_request(
                    client,
                    MediaRequest::Play {
                        id: media,
                        asset,
                        looping: false,
                        audio: false,
                        gain: 1.,
                    },
                )?;
                host.media_request(client, MediaRequest::Pause { id: media })?;
                rt.host_mut()
                    .layer_movies
                    .movies
                    .get_mut(&id)
                    .unwrap()
                    .texture = Some(texture.relative().into());
            }
            Request::Start(id, looping) => {
                let Some(m) = rt.host().layer_movies.movies.get(&id) else {
                    continue;
                };
                let (media, Some(asset)) = (m.media, m.asset.clone()) else {
                    continue;
                };
                let texture = host.media_request(
                    client,
                    MediaRequest::Play {
                        id: media,
                        asset,
                        looping,
                        audio: false,
                        gain: 1.,
                    },
                )?;
                rt.host_mut()
                    .layer_movies
                    .movies
                    .get_mut(&id)
                    .unwrap()
                    .texture = Some(texture.relative().into());
            }
            Request::Stop(id) => {
                if host.media_info(client, id).is_ok() {
                    host.media_request(client, MediaRequest::Stop { id })?;
                }
            }
            Request::Remove(id) => {
                if host.media_info(client, id).is_ok() {
                    host.media_request(client, MediaRequest::Remove { id })?;
                }
            }
        }
    }
    let pending: Vec<_> = rt
        .host()
        .layer_movies
        .movies
        .iter()
        .filter(|(_, m)| m.opening)
        .map(|(id, m)| (*id, m.media))
        .collect();
    for (id, media) in pending {
        let info = host.media_info(client, media)?;
        anyhow::ensure!(
            info.phase != MediaPhase::Failed,
            "layerExMovie metadata decoder failed"
        );
        let Some([w, h]) = info.size else {
            return Ok(false);
        };
        let target = ObjectHandle(id);
        rt.host_mut()
            .reset_surface(target, i64::from(w), i64::from(h), [0; 4])?;
        let layer = rt.host_mut().layer_mut(target)?;
        layer.kind = 1;
        layer.dirty = true;
        layer.full_alpha = Some(255);
        let m = rt.host_mut().layer_movies.movies.get_mut(&id).unwrap();
        m.size = Some([w, h]);
        m.opening = false;
    }
    Ok(true)
}
pub fn take_event(rt: &mut Rt, id: usize) -> Option<Variant> {
    rt.host_mut()
        .layer_movies
        .movies
        .get_mut(&id)?
        .events
        .pop_front()
}
/// Only actual owner-published media pictures trigger native update callbacks.
pub fn observe(rt: &mut Rt, host: &dyn HostServices, client: ClientId) -> anyhow::Result<()> {
    let movies: Vec<_> = rt
        .host()
        .layer_movies
        .movies
        .iter()
        .filter(|(_, m)| m.playing)
        .map(|(id, m)| (*id, m.media))
        .collect();
    for (id, media) in movies {
        let info = host.media_info(client, media)?;
        anyhow::ensure!(
            info.phase != MediaPhase::Failed,
            "layerExMovie playback decoder failed"
        );
        let mut updated = false;
        if let Some(frame) = host.media_frame(client, media)? {
            let m = &rt.host().layer_movies.movies[&id];
            if frame.revision != m.revision {
                updated = true;
                let target = ObjectHandle(id);
                let layer = rt.host().layer(target)?;
                let (slot, w, h) = (layer.slot, layer.image_width, layer.image_height);
                anyhow::ensure!(
                    m.size == Some([w as u32, h as u32]),
                    "layerExMovie resized image profile is unsupported"
                );
                let texture = m.texture.clone().context("movie texture absent")?;
                rt.host_mut().canvas_op(json!({"op":"set","id":slot,"layer":{"id":0,"rect":[0,0,w,h],"paint":{"kind":"image","asset":texture}}}));
                let l = rt.host_mut().layer_mut(target)?;
                l.image_source = None;
                l.full_alpha = Some(255);
                l.blank = false;
                l.dirty = true;
                let m = rt.host_mut().layer_movies.movies.get_mut(&id).unwrap();
                m.revision = frame.revision;
                if let Some(cb) = m.callbacks[1].clone() {
                    m.events.push_back(cb);
                    rt.host_mut().deferred.post(
                        client.0,
                        crate::async_trigger::target(id, 3),
                        DispatchPhase::Normal,
                        true,
                    )?;
                }
            }
        }
        // Original sample completion is a separate continuous update after
        // the final successful picture, so its callback still sees playing.
        if !updated && matches!(info.phase, MediaPhase::Ended | MediaPhase::Stopped) {
            let m = rt.host_mut().layer_movies.movies.get_mut(&id).unwrap();
            m.playing = false;
            m.asset = None;
            rt.host_mut()
                .layer_movies
                .requests
                .push_back(Request::Stop(media));
            let m = rt.host_mut().layer_movies.movies.get_mut(&id).unwrap();
            if let Some(cb) = m.callbacks[2].clone() {
                m.events.push_back(cb);
                rt.host_mut().deferred.post(
                    client.0,
                    crate::async_trigger::target(id, 3),
                    DispatchPhase::Normal,
                    false,
                )?;
            }
        }
    }
    Ok(())
}
