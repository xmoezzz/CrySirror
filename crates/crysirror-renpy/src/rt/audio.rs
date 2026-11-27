// SPDX-License-Identifier: MPL-2.0
//! `renpy.audio.audio`, `renpy.audio.music`, `renpy.audio.sound` and the
//! `renpy.audio.renpysound` backend, after Ren'Py 8.3 (MIT).
//!
//! The channel layer (queues, loops, music contexts, mixers, pause and
//! fades) follows upstream. The backend keeps upstream's per-channel model
//! of a playing and a queued sample on the script thread and sends each
//! change to the host as an audio command (`link::emit_audio`); the host
//! plays and crossfades the sources and reports per channel which sample
//! is playing, its position and duration. A sample's end is seen at the
//! first tick whose report no longer names it, so the queue advances one
//! host tick after the host has already started the queued sample
//! gaplessly.
//!
//! Commands: `{"op": "play" | "queue", "channel", "name", "id", "file",
//! "common", "fadein", "tight", "start", "end", "relative_volume",
//! "paused"}`, `{"op": "stop" | "pause" | "unpause", "channel"}`,
//! `{"op": "dequeue", "channel", "even_tight"}`, `{"op": "fadeout",
//! "channel", "delay"}`, `{"op": "volume", "channel", "volume"}`,
//! `{"op": "pan" | "secondary_volume", "channel", ..., "delay"}`,
//! `{"op": "global_pause", "pause"}`. Audio filters (8.2) are forwarded as
//! `{"op": "filter"}` with their repr; the host reports them unsupported.
use super::{
    link,
    py::{self, *},
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use serde_json::json;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

const AUDIO: &str = "renpy.audio.audio";
const MUSIC: &str = "renpy.audio.music";
const SOUND: &str = "renpy.audio.sound";
const RENPYSOUND: &str = "renpy.audio.renpysound";

pub const NO_VIDEO: i64 = 0;
pub const NODROP_VIDEO: i64 = 1;
pub const DROP_VIDEO: i64 = 2;

fn at_least(vm: &Vm, major: u32, minor: u32) -> bool {
    super::state::try_rt(vm).map(|r| r.profile.at_least(major, minor)).unwrap_or(true)
}

// ---------------------------------------------------------------------------
// Backend model.

#[derive(Clone)]
struct Item {
    id: u64,
    name: Value,
    tight: bool,
    issued: u64,
}

#[derive(Default)]
struct Chan {
    playing: Option<Item>,
    queued: Option<Item>,
    synced: u64,
    pos: Option<f64>,
    duration: f64,
    video: i64,
}

#[derive(Default)]
struct Sound {
    chans: HashMap<i64, Chan>,
    seq: u64,
}

type SoundRef = Rc<RefCell<Sound>>;

fn sound(vm: &mut Vm) -> SoundRef {
    if let Some(s) = vm.ext_get::<SoundRef>() {
        return s.clone();
    }
    let s: SoundRef = Rc::default();
    vm.ext_set(s.clone());
    s
}

fn item_id(channel: i64, id: u64) -> String {
    format!("{channel}:{id}")
}

/// Advances the model to the host's latest report.
fn sync(vm: &mut Vm, channel: i64) {
    let tick = link::tick_no(vm);
    let status = link::audio_status(vm);
    let s = sound(vm);
    let mut s = s.borrow_mut();
    let c = s.chans.entry(channel).or_default();
    if c.synced == tick {
        return;
    }
    c.synced = tick;
    let key = channel.to_string();
    let st = status.iter().find(|a| a.channel == key);
    let host = st.and_then(|a| a.playing.clone());
    let acked = |i: &Item| i.issued < tick;
    if let Some(q) = c.queued.clone() {
        if acked(&q) && host.as_deref() == Some(item_id(channel, q.id).as_str()) {
            c.playing = Some(q);
            c.queued = None;
        }
    }
    if let Some(p) = c.playing.clone() {
        if acked(&p) && host.as_deref() != Some(item_id(channel, p.id).as_str()) {
            c.playing = None;
            if let Some(q) = c.queued.clone() {
                if acked(&q) {
                    c.queued = None;
                } else {
                    c.playing = c.queued.take();
                }
            }
        }
    }
    c.pos = st.and_then(|a| a.position_ms).map(|p| p as f64 / 1000.0);
    c.duration = st.and_then(|a| a.duration_ms).map(|d| d as f64 / 1000.0).unwrap_or(0.0);
}

fn emit(vm: &Vm, cmd: serde_json::Value) {
    link::emit_audio(vm, cmd);
}

fn channel_arg(vm: &mut Vm, v: &Value) -> PyResult<i64> {
    py::int(vm, v)
}

/// `play`/`queue`: `file` is the file name (or a file object whose
/// `name` is one).
fn rs_enqueue(vm: &mut Vm, args: &[Value], kw: Kw, play: bool) -> PyResult<Value> {
    let channel = channel_arg(vm, &args[0])?;
    let file = &args[1];
    let name = arg_or(&args[1..], kw, 1, "name", file.clone());
    let fname = if file.as_str().is_some() { py::string(vm, file)? } else { py::get(vm, file, "name").and_then(|n| py::string(vm, &n))? };
    let g = |i: usize, n: &str, d: Value| arg_or(&args[1..], kw, i, n, d);
    let paused = g(2, "paused", Value::Bool(false));
    let fadein = g(3, "fadein", Value::Int(0));
    let tight = g(4, "tight", Value::Bool(false));
    let start = g(5, "start", Value::Int(0));
    let end = g(6, "end", Value::Int(-1));
    let rv = g(7, "relative_volume", Value::Float(1.0));
    let (paused, tight) = (py::truthy(vm, &paused)?, py::truthy(vm, &tight)?);
    let (fadein, start, end, rv) = (py::f64_of(vm, &fadein)?, py::f64_of(vm, &start)?, py::f64_of(vm, &end)?, py::f64_of(vm, &rv)?);
    let (actual, common) = match super::resloader::find(vm, &fname, Some("audio"))? {
        Some(r) => r,
        None => return Err(vm.exc_msg("IOError", format!("Couldn't find file '{fname}'."))),
    };
    let tick = link::tick_no(vm);
    sync(vm, channel);
    let s = sound(vm);
    let id = {
        let mut s = s.borrow_mut();
        s.seq += 1;
        let id = s.seq;
        let c = s.chans.entry(channel).or_default();
        let item = Item { id, name: name.clone(), tight, issued: tick };
        if play {
            c.playing = Some(item);
            c.queued = None;
            c.pos = Some(start);
        } else if c.playing.is_none() {
            c.playing = Some(item);
        } else {
            c.queued = Some(item);
        }
        id
    };
    let n = py::string(vm, &name).unwrap_or_default();
    emit(vm, json!({
        "op": if play { "play" } else { "queue" }, "channel": channel, "name": n, "id": item_id(channel, id), "file": actual, "common": common,
        "fadein": fadein, "tight": tight, "start": start, "end": end, "relative_volume": rv, "paused": paused,
    }));
    Ok(Value::None)
}

fn rs_play(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    rs_enqueue(vm, args, kw, true)
}

fn rs_queue(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    rs_enqueue(vm, args, kw, false)
}

fn with_chan<R>(vm: &mut Vm, v: &Value, f: impl FnOnce(&mut Chan) -> R) -> PyResult<(i64, R)> {
    let channel = channel_arg(vm, v)?;
    sync(vm, channel);
    let s = sound(vm);
    let r = f(s.borrow_mut().chans.entry(channel).or_default());
    Ok((channel, r))
}

fn rs_stop(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (ch, _) = with_chan(vm, &args[0], |c| {
        c.playing = None;
        c.queued = None;
    })?;
    emit(vm, json!({"op": "stop", "channel": ch}));
    Ok(Value::None)
}

fn rs_dequeue(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let even_tight = arg_or(&args[1..], kw, 0, "even_tight", Value::Bool(false));
    let even_tight = py::truthy(vm, &even_tight)?;
    let (ch, _) = with_chan(vm, &args[0], |c| {
        if c.queued.as_ref().is_some_and(|q| !q.tight || even_tight) {
            c.queued = None;
        }
    })?;
    emit(vm, json!({"op": "dequeue", "channel": ch, "even_tight": even_tight}));
    Ok(Value::None)
}

fn rs_fadeout(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let delay = py::f64_of(vm, &args[1])?;
    let (ch, _) = with_chan(vm, &args[0], |c| {
        c.queued = None;
        if delay <= 0.0 {
            c.playing = None;
        }
    })?;
    emit(vm, json!({"op": "fadeout", "channel": ch, "delay": delay}));
    Ok(Value::None)
}

fn rs_queue_depth(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (_, d) = with_chan(vm, &args[0], |c| c.playing.is_some() as i64 + c.queued.is_some() as i64)?;
    Ok(Value::Int(d))
}

fn rs_playing_name(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (_, n) = with_chan(vm, &args[0], |c| c.playing.as_ref().map(|p| p.name.clone()))?;
    Ok(n.unwrap_or(Value::None))
}

fn rs_busy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (_, b) = with_chan(vm, &args[0], |c| c.playing.is_some())?;
    Ok(Value::Bool(b))
}

fn rs_get_pos(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (_, p) = with_chan(vm, &args[0], |c| if c.playing.is_some() { c.pos.unwrap_or(0.0) } else { -1.0 })?;
    Ok(Value::Float(p))
}

fn rs_get_duration(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (_, d) = with_chan(vm, &args[0], |c| c.duration)?;
    Ok(Value::Float(d))
}

fn rs_set_volume(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ch = channel_arg(vm, &args[0])?;
    let v = py::f64_of(vm, &args[1])?;
    emit(vm, json!({"op": "volume", "channel": ch, "volume": v}));
    Ok(Value::None)
}

fn rs_set_pan(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ch = channel_arg(vm, &args[0])?;
    let (p, d) = (py::f64_of(vm, &args[1])?, py::f64_of(vm, &args[2])?);
    emit(vm, json!({"op": "pan", "channel": ch, "pan": p, "delay": d}));
    Ok(Value::None)
}

fn rs_set_secondary_volume(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ch = channel_arg(vm, &args[0])?;
    let (v, d) = (py::f64_of(vm, &args[1])?, py::f64_of(vm, &args[2])?);
    emit(vm, json!({"op": "secondary_volume", "channel": ch, "volume": v, "delay": d}));
    Ok(Value::None)
}

fn rs_pause(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ch = channel_arg(vm, &args[0])?;
    emit(vm, json!({"op": "pause", "channel": ch}));
    Ok(Value::None)
}

fn rs_unpause(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ch = channel_arg(vm, &args[0])?;
    emit(vm, json!({"op": "unpause", "channel": ch}));
    Ok(Value::None)
}

fn rs_global_pause(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let p = py::truthy(vm, &args[0])?;
    emit(vm, json!({"op": "global_pause", "pause": p}));
    Ok(Value::None)
}

fn rs_set_video(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = py::int(vm, &args[1])?;
    with_chan(vm, &args[0], |c| c.video = v)?;
    Ok(Value::None)
}

fn rs_replace_audio_filter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ch = channel_arg(vm, &args[0])?;
    if !py::is_none(&args[1]) {
        let r = ops::repr(vm, &args[1])?;
        emit(vm, json!({"op": "filter", "channel": ch, "filter": r}));
    }
    Ok(Value::None)
}

fn rs_noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn rs_none(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn rs_video_ready(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(1))
}

fn rs_check_version(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn renpysound_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        RENPYSOUND,
        &[
            ("init", rs_noop),
            ("quit", rs_noop),
            ("periodic", rs_noop),
            ("advance_time", rs_noop),
            ("sample_surfaces", rs_noop),
            ("unpause_all_at_start", rs_noop),
            ("play", rs_play),
            ("queue", rs_queue),
            ("stop", rs_stop),
            ("dequeue", rs_dequeue),
            ("fadeout", rs_fadeout),
            ("queue_depth", rs_queue_depth),
            ("playing_name", rs_playing_name),
            ("busy", rs_busy),
            ("get_pos", rs_get_pos),
            ("get_duration", rs_get_duration),
            ("set_volume", rs_set_volume),
            ("set_pan", rs_set_pan),
            ("set_secondary_volume", rs_set_secondary_volume),
            ("pause", rs_pause),
            ("unpause", rs_unpause),
            ("global_pause", rs_global_pause),
            ("set_video", rs_set_video),
            ("read_video", rs_none),
            ("video_ready", rs_video_ready),
            ("replace_audio_filter", rs_replace_audio_filter),
            ("check_version", rs_check_version),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    set(vm, "NO_VIDEO", Value::Int(NO_VIDEO));
    set(vm, "NODROP_VIDEO", Value::Int(NODROP_VIDEO));
    set(vm, "DROP_VIDEO", Value::Int(DROP_VIDEO));
    set(vm, "is_webaudio", Value::Bool(false));
    Ok(m)
}

// ---------------------------------------------------------------------------
// renpy.audio.audio.

fn rs(vm: &mut Vm, name: &str, args: Vec<Value>, kw: Vec<(Rc<str>, Value)>) -> PyResult<Value> {
    let f = py::import_attr(vm, RENPYSOUND, name)?;
    vm.call(&f, args, kw)
}

fn amod(vm: &mut Vm) -> PyResult<Value> {
    py::import(vm, AUDIO)
}

fn aget(vm: &mut Vm, n: &str) -> PyResult<Value> {
    py::import_attr(vm, AUDIO, n)
}

fn pcm_ok(vm: &mut Vm) -> bool {
    aget(vm, "pcm_ok").map(|v| matches!(v, Value::Bool(true))).unwrap_or(false)
}

fn cfg(vm: &mut Vm, n: &str) -> Value {
    py::config(vm, n).unwrap_or(Value::None)
}

fn cfg_true(vm: &mut Vm, n: &str) -> bool {
    let v = cfg(vm, n);
    py::truthy(vm, &v).unwrap_or(false)
}

fn debug_sound(vm: &mut Vm) -> bool {
    cfg_true(vm, "debug_sound")
}

fn get_serial(vm: &mut Vm) -> PyResult<Value> {
    let m = amod(vm)?;
    let s = py::get(vm, &m, "serial")?;
    let n = py::int(vm, &s)? + 1;
    py::set(vm, &m, "serial", Value::Int(n))?;
    let u = py::get(vm, &m, "unique")?;
    Ok(vm.types.new_tuple(vec![u, Value::Int(n)]))
}

fn f_get_serial(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    get_serial(vm)
}

fn g(vm: &mut Vm, o: &Value, n: &str) -> PyResult<Value> {
    py::get(vm, o, n)
}

fn s(vm: &mut Vm, o: &Value, n: &str, v: Value) -> PyResult<()> {
    py::set(vm, o, n, v)
}

fn gb(vm: &mut Vm, o: &Value, n: &str) -> PyResult<bool> {
    let v = py::get(vm, o, n)?;
    py::truthy(vm, &v)
}

// MusicContext / QueueEntry ------------------------------------------------------

fn mc_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let base = py::import_attr(vm, "renpy.revertable", "RevertableObject")?;
    let init = py::get(vm, &base, "__init__")?;
    vm.call(&init, vec![this.clone()], Vec::new())?;
    s(vm, this, "pan_time", Value::None)?;
    s(vm, this, "pan", Value::Int(0))?;
    s(vm, this, "secondary_volume_time", Value::None)?;
    s(vm, this, "secondary_volume", Value::Float(1.0))?;
    s(vm, this, "last_changed", Value::Int(0))?;
    s(vm, this, "last_tight", Value::Bool(false))?;
    s(vm, this, "last_filenames", vm.types.new_list(Vec::new()))?;
    s(vm, this, "force_stop", Value::Bool(false))?;
    if at_least(vm, 7, 0) {
        s(vm, this, "pause", Value::Bool(false))?;
    }
    if at_least(vm, 7, 4) {
        s(vm, this, "last_relative_volume", Value::Float(1.0))?;
    }
    if at_least(vm, 8, 2) {
        s(vm, this, "audio_filter", Value::None)?;
        s(vm, this, "raw_audio_filter", Value::None)?;
    }
    Ok(Value::None)
}

fn mc_copy(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = aget(vm, "MusicContext")?;
    let rv = vm.call(&c, Vec::new(), Vec::new())?;
    let d = py::namespace_of(vm, &args[0])?;
    let rd = py::namespace_of(vm, &rv)?;
    vm.call_method(&rd, "update", vec![d])?;
    if at_least(vm, 8, 2) {
        let raw = g(vm, &args[0], "raw_audio_filter")?;
        s(vm, &args[0], "audio_filter", raw)?;
    }
    Ok(rv)
}

fn qe_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    for (i, n) in ["filename", "fadein", "tight", "loop", "relative_volume", "audio_filter"].iter().enumerate() {
        let d = match *n {
            "relative_volume" => Value::Float(1.0),
            "loop" => Value::Bool(false),
            _ => Value::None,
        };
        s(vm, this, n, args.get(i + 1).cloned().unwrap_or(d))?;
    }
    Ok(Value::None)
}

fn queue_entry(vm: &mut Vm, filename: Value, fadein: Value, tight: Value, loop_: bool, rv: Value, filter: Value) -> PyResult<Value> {
    let c = aget(vm, "QueueEntry")?;
    vm.call(&c, vec![filename, fadein, tight, Value::Bool(loop_), rv, filter], Vec::new())
}

// Channel ------------------------------------------------------------------------

const CH_ARGS: &[&str] = &["name", "default_loop", "stop_on_mute", "tight", "file_prefix", "file_suffix", "buffer_queue", "movie", "framedrop", "synchro_start"];

fn ch_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v: Vec<Value> = CH_ARGS.iter().enumerate().map(|(i, n)| arg_or(&args[1..], kw, i, n, Value::None)).collect();
    s(vm, this, "name", v[0].clone())?;
    s(vm, this, "_number", Value::None)?;
    s(vm, this, "mixer", Value::None)?;
    s(vm, this, "chan_volume", Value::Float(1.0))?;
    s(vm, this, "actual_volume", Value::Float(1.0))?;
    s(vm, this, "queue", vm.types.new_list(Vec::new()))?;
    s(vm, this, "loop", vm.types.new_list(Vec::new()))?;
    s(vm, this, "playing", Value::Bool(false))?;
    s(vm, this, "synchro_start", Value::Bool(false))?;
    s(vm, this, "default_synchro_start", v[9].clone())?;
    s(vm, this, "last_changed", Value::Int(0))?;
    s(vm, this, "callback", Value::None)?;
    s(vm, this, "pan_time", Value::None)?;
    s(vm, this, "secondary_volume_time", Value::None)?;
    s(vm, this, "stop_on_mute", v[2].clone())?;
    s(vm, this, "tight", v[3].clone())?;
    s(vm, this, "keep_queue", Value::Int(0))?;
    let str_or = |vm: &Vm, x: &Value| if py::is_none(x) { vm.types.new_str("") } else { x.clone() };
    s(vm, this, "file_prefix", str_or(vm, &v[4]))?;
    s(vm, this, "file_suffix", str_or(vm, &v[5]))?;
    s(vm, this, "buffer_queue", if py::is_none(&v[6]) { Value::Bool(true) } else { v[6].clone() })?;
    s(vm, this, "paused", Value::None)?;
    if py::is_none(&v[1]) {
        s(vm, this, "default_loop", Value::Bool(true))?;
        s(vm, this, "default_loop_set", Value::Bool(false))?;
    } else {
        s(vm, this, "default_loop", v[1].clone())?;
        s(vm, this, "default_loop_set", Value::Bool(true))?;
    }
    let movie = if py::truthy(vm, &v[7])? {
        if py::is_none(&v[8]) || py::truthy(vm, &v[8])? { DROP_VIDEO } else { NODROP_VIDEO }
    } else {
        NO_VIDEO
    };
    s(vm, this, "movie", Value::Int(movie))?;
    Ok(Value::None)
}

fn number(vm: &mut Vm, this: &Value) -> PyResult<i64> {
    let n = g(vm, this, "_number")?;
    if !py::is_none(&n) {
        return py::int(vm, &n);
    }
    let m = amod(vm)?;
    let next = py::get(vm, &m, "next_channel_number")?;
    let next = py::int(vm, &next)?;
    py::set(vm, &m, "next_channel_number", Value::Int(next + 1))?;
    s(vm, this, "_number", Value::Int(next))?;
    Ok(next)
}

fn ch_get_number(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(number(vm, &args[0])?))
}

fn context(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let ctx = super::exports::util::ctx(vm)?;
    let mcd = py::get(vm, &ctx, "music")?;
    let name = g(vm, this, "name")?;
    let rv = vm.call_method(&mcd, "get", vec![name.clone()])?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    let c = aget(vm, "MusicContext")?;
    let rv = vm.call(&c, Vec::new(), Vec::new())?;
    ops::setitem(vm, &mcd, name, rv.clone())?;
    Ok(rv)
}

fn ch_get_context(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    context(vm, &args[0])
}

fn copy_context(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let ctx = super::exports::util::ctx(vm)?;
    let mcd = py::get(vm, &ctx, "music")?;
    let d = Value::Obj(vm.types.dict.clone());
    let mcd = vm.call(&d, vec![mcd], Vec::new())?;
    py::set(vm, &ctx, "music", mcd.clone())?;
    let cur = context(vm, this)?;
    let c = vm.call_method(&cur, "copy", Vec::new())?;
    let name = g(vm, this, "name")?;
    ops::setitem(vm, &mcd, name, c.clone())?;
    Ok(c)
}

fn ch_copy_context(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    copy_context(vm, &args[0])
}

/// `split_filename(filename, looped)` -> (filename, start, end).
fn split_filename(vm: &mut Vm, this: &Value, filename: &Value, looped: bool) -> PyResult<(Value, f64, f64)> {
    let fs = py::string(vm, filename)?;
    let prefix = g(vm, this, "file_prefix")?;
    let prefix = py::string(vm, &prefix)?;
    let suffix = g(vm, this, "file_suffix")?;
    let suffix = py::string(vm, &suffix)?;
    let rest = fs.strip_prefix('<').and_then(|r| r.split_once('>'));
    let Some((spec, f)) = rest else {
        return Ok((vm.types.new_str(&format!("{prefix}{fs}{suffix}")), 0.0, -1.0));
    };
    let mut fname = format!("{prefix}{f}{suffix}");
    let err = |vm: &Vm, msg: String| py::error(vm, format!("Parsing audio spec '{fs}': {msg}."));
    let mut words: Vec<&str> = spec.split_whitespace().collect();
    words.reverse();
    let (mut start, mut end, mut loop_) = (0.0, -1.0, None);
    while let Some(clause) = words.pop() {
        let float = |vm: &Vm, w: &mut Vec<&str>| -> PyResult<f64> {
            let Some(v) = w.pop() else { return Err(err(vm, "expected float at end".into())) };
            v.parse::<f64>().map_err(|_| err(vm, format!("expected float, got '{v}'")))
        };
        match clause {
            "from" => start = float(vm, &mut words)?,
            "to" => end = float(vm, &mut words)?,
            "loop" => loop_ = Some(float(vm, &mut words)?),
            "silence" => {
                end = float(vm, &mut words)?;
                fname = "_silence.ogg".into();
            }
            "sync" => {
                let Some(ch) = words.pop() else { return Err(err(vm, "expected channel at end".into())) };
                let c = get_channel(vm, &vm.types.new_str(ch)).map_err(|_| err(vm, format!("expected channel, got '{ch}'")))?;
                let t = vm.call_method(&c, "get_pos", Vec::new())?;
                let t = py::f64_of(vm, &t).unwrap_or(-1.0);
                if t > 0.0 {
                    start = t;
                }
            }
            other => return Err(err(vm, format!("expected keyword, got '{other}'"))),
        }
    }
    if let (Some(l), true) = (loop_, looped) {
        start = l;
    }
    let ad = aget(vm, "AudioData")?;
    let fv = if ad.obj().is_some_and(|c| ops::isinstance(vm, filename, c)) {
        let data = g(vm, filename, "data")?;
        let n = vm.types.new_str(&fname);
        vm.call(&ad, vec![data, n], Vec::new())?
    } else {
        vm.types.new_str(&fname)
    };
    Ok((fv, start, end))
}

fn ch_split_filename(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let looped = py::truthy(vm, &args[2])?;
    let (f, a, b) = split_filename(vm, &args[0], &args[1], looped)?;
    Ok(vm.types.new_tuple(vec![f, Value::Float(a), py_end(b)]))
}

fn py_end(b: f64) -> Value {
    if b == -1.0 { Value::Int(-1) } else { Value::Float(b) }
}

fn prefs(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, "renpy.game", "preferences")
}

fn dict_get(vm: &mut Vm, d: &Value, k: &Value, default: Value) -> PyResult<Value> {
    vm.call_method(d, "get", vec![k.clone(), default])
}

/// `Channel.periodic()`: volume, forced stops, queue pumping, loops,
/// the empty-queue callback and pause.
fn periodic_channel(vm: &mut Vm, this: &Value) -> PyResult<()> {
    let p = prefs(vm)?;
    let vols = py::get(vm, &p, "volumes")?;
    let mute = py::get(vm, &p, "mute")?;
    let mixer = g(vm, this, "mixer")?;
    let mut mixer_volume = if py::truthy(vm, &mixer)? {
        let v = dict_get(vm, &vols, &mixer, Value::Float(1.0))?;
        py::f64_of(vm, &v)?
    } else {
        1.0
    };
    let main = if at_least(vm, 8, 2) {
        let k = py::s(vm, "main");
        let v = dict_get(vm, &vols, &k, Value::Float(1.0))?;
        py::f64_of(vm, &v)?
    } else {
        1.0
    };
    if gb(vm, &p, "self_voicing")? {
        let vm_list = cfg(vm, "voice_mixers");
        let is_voice = !py::is_none(&vm_list) && ops::contains(vm, &vm_list, &mixer)?;
        if !is_voice {
            let drop = py::get_opt(vm, &p, "self_voicing_volume_drop")?.unwrap_or(Value::Float(0.5));
            mixer_volume *= py::f64_of(vm, &drop)?;
        }
    }
    let chan_volume = g(vm, this, "chan_volume")?;
    let mut vol = py::f64_of(vm, &chan_volume)? * mixer_volume * main;
    let muted = dict_get(vm, &mute, &mixer, Value::Bool(false))?;
    let muted = py::truthy(vm, &muted)?;
    if muted {
        vol = 0.0;
    }
    let actual = g(vm, this, "actual_volume")?;
    let n = number(vm, this)?;
    if vol != py::f64_of(vm, &actual)? {
        rs(vm, "set_volume", vec![Value::Int(n), Value::Float(vol)], Vec::new())?;
        s(vm, this, "actual_volume", Value::Float(vol))?;
    }
    let ctx = context(vm, this)?;
    let force_stop = gb(vm, &ctx, "force_stop")? || (muted && gb(vm, this, "stop_on_mute")?);
    if gb(vm, this, "playing")? && force_stop {
        rs(vm, "stop", vec![Value::Int(n)], Vec::new())?;
        s(vm, this, "playing", Value::Bool(false))?;
    }
    let queue = g(vm, this, "queue")?;
    let loop_ = g(vm, this, "loop")?;
    if force_stop {
        let lq = py::items(vm, &queue)?;
        let ll = ops::len(vm, &loop_)?;
        let keep = if ll > 0 { lq[lq.len().saturating_sub(ll)..].to_vec() } else { Vec::new() };
        s(vm, this, "queue", vm.types.new_list(keep))?;
        return Ok(());
    }
    let mut topq = Value::None;
    loop {
        let depth = rs(vm, "queue_depth", vec![Value::Int(n)], Vec::new())?;
        let depth = py::int(vm, &depth)?;
        if depth == 0 {
            s(vm, this, "playing", Value::Bool(false))?;
        }
        let queue = g(vm, this, "queue")?;
        if ops::len(vm, &queue)? == 0 || depth >= 2 {
            break;
        }
        if !gb(vm, this, "buffer_queue")? && depth >= 1 {
            break;
        }
        topq = vm.call_method(&queue, "pop", vec![Value::Int(0)])?;
        let fname = g(vm, &topq, "filename")?;
        let suffix = g(vm, this, "file_suffix")?;
        let lfn = format!("{}{}", py::string(vm, &fname)?.to_lowercase(), py::string(vm, &suffix)?.to_lowercase());
        if [".mod", ".xm", ".mid", ".midi"].iter().any(|e| lfn.ends_with(e)) {
            topq = Value::None;
            continue;
        }
        let r = (|| -> PyResult<bool> {
            let looped = gb(vm, &topq, "loop")?;
            let (mut filename, start, end) = split_filename(vm, this, &fname, looped)?;
            let cb = cfg(vm, "audio_filename_callback");
            if !py::is_none(&cb) {
                filename = vm.call(&cb, vec![filename], Vec::new())?;
            }
            let q = g(vm, this, "queue")?;
            if end >= 0.0 && end - start <= 0.0 && ops::len(vm, &q)? > 0 {
                return Ok(false);
            }
            let movie = g(vm, this, "movie")?;
            let movie_i = py::int(vm, &movie)?;
            let vloop = movie_i != NO_VIDEO && ops::len(vm, &loop_)? == 1;
            rs(vm, "set_video", vec![Value::Int(n), movie], vec![("loop".into(), Value::Bool(vloop))])?;
            let kw: Vec<(Rc<str>, Value)> = vec![
                ("synchro_start".into(), g(vm, this, "synchro_start")?),
                ("fadein".into(), g(vm, &topq, "fadein")?),
                ("tight".into(), g(vm, &topq, "tight")?),
                ("start".into(), Value::Float(start)),
                ("end".into(), py_end(end)),
                ("relative_volume".into(), g(vm, &topq, "relative_volume")?),
            ];
            let op = if depth == 0 { "play" } else { "queue" };
            rs(vm, op, vec![Value::Int(n), filename, fname.clone()], kw)?;
            s(vm, this, "playing", Value::Bool(true))?;
            s(vm, this, "synchro_start", Value::Bool(false))?;
            Ok(true)
        })();
        match r {
            Ok(true) => break,
            Ok(false) => continue,
            Err(e) => {
                loop {
                    let l = g(vm, this, "loop")?;
                    if !ops::contains(vm, &l, &fname)? {
                        break;
                    }
                    vm.call_method(&l, "remove", vec![fname.clone()])?;
                }
                let ar = py::import_attr(vm, "renpy.game", "after_rollback")?;
                if debug_sound(vm) && !py::truthy(vm, &ar)? {
                    return Err(e);
                }
                return Ok(());
            }
        }
    }
    let queue = g(vm, this, "queue")?;
    if ops::len(vm, &queue)? == 0 {
        let l = g(vm, this, "loop")?;
        if ops::len(vm, &l)? > 0 {
            let filter = if at_least(vm, 8, 2) { g(vm, &ctx, "audio_filter")? } else { Value::None };
            for i in py::items(vm, &l)? {
                let e = if py::is_none(&topq) {
                    queue_entry(vm, i, Value::Int(0), Value::Bool(false), true, Value::Float(1.0), filter.clone())?
                } else {
                    let (t, rv) = (g(vm, &topq, "tight")?, g(vm, &topq, "relative_volume")?);
                    queue_entry(vm, i, Value::Int(0), t, true, rv, filter.clone())?
                };
                vm.call_method(&queue, "append", vec![e])?;
            }
        } else {
            let cb = g(vm, this, "callback")?;
            if py::truthy(vm, &cb)? {
                vm.call(&cb, Vec::new(), Vec::new())?;
            }
        }
    }
    if at_least(vm, 7, 0) {
        let want = gb(vm, &ctx, "pause")?;
        let paused = g(vm, this, "paused")?;
        if py::is_none(&paused) || py::truthy(vm, &paused)? != want {
            rs(vm, if want { "pause" } else { "unpause" }, vec![Value::Int(n)], Vec::new())?;
            s(vm, this, "paused", Value::Bool(want))?;
        }
    }
    Ok(())
}

fn ch_periodic(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    periodic_channel(vm, &args[0])?;
    Ok(Value::None)
}

fn dequeue(vm: &mut Vm, this: &Value, even_tight: bool) -> PyResult<()> {
    let keep = g(vm, this, "keep_queue")?;
    let keep = py::int(vm, &keep)?.max(0) as usize;
    let q = g(vm, this, "queue")?;
    let items = py::items(vm, &q)?;
    s(vm, this, "queue", vm.types.new_list(items.into_iter().take(keep).collect()))?;
    s(vm, this, "loop", vm.types.new_list(Vec::new()))?;
    if !pcm_ok(vm) || py::is_none(&g(vm, this, "_number")?) {
        return Ok(());
    }
    if keep == 0 {
        let n = number(vm, this)?;
        rs(vm, "dequeue", vec![Value::Int(n), Value::Bool(even_tight)], Vec::new())?;
        s(vm, this, "synchro_start", Value::Bool(false))?;
    }
    Ok(())
}

fn ch_dequeue(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let et = arg_or(&args[1..], kw, 0, "even_tight", Value::Bool(false));
    let et = py::truthy(vm, &et)?;
    dequeue(vm, &args[0], et)?;
    Ok(Value::None)
}

fn ch_interact(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    s(vm, this, "keep_queue", Value::Int(0))?;
    if pcm_ok(vm) {
        let ctx = context(vm, this)?;
        let n = number(vm, this)?;
        let (pt, cpt) = (g(vm, this, "pan_time")?, g(vm, &ctx, "pan_time")?);
        if !ops::eq(vm, &pt, &cpt)? {
            s(vm, this, "pan_time", cpt)?;
            let pan = g(vm, &ctx, "pan")?;
            rs(vm, "set_pan", vec![Value::Int(n), pan, Value::Int(0)], Vec::new())?;
        }
        let (st, cst) = (g(vm, this, "secondary_volume_time")?, g(vm, &ctx, "secondary_volume_time")?);
        if !ops::eq(vm, &st, &cst)? {
            s(vm, this, "secondary_volume_time", cst)?;
            let v = g(vm, &ctx, "secondary_volume")?;
            rs(vm, "set_secondary_volume", vec![Value::Int(n), v, Value::Int(0)], Vec::new())?;
        }
    }
    let q = g(vm, this, "queue")?;
    let cb = g(vm, this, "callback")?;
    if ops::len(vm, &q)? == 0 && py::truthy(vm, &cb)? {
        vm.call(&cb, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn fadeout(vm: &mut Vm, this: &Value, secs: f64) -> PyResult<()> {
    s(vm, this, "keep_queue", Value::Int(0))?;
    dequeue(vm, this, false)?;
    if !pcm_ok(vm) || py::is_none(&g(vm, this, "_number")?) {
        return Ok(());
    }
    let n = number(vm, this)?;
    if secs == 0.0 {
        rs(vm, "stop", vec![Value::Int(n)], Vec::new())?;
    } else {
        rs(vm, "fadeout", vec![Value::Int(n), Value::Float(secs)], Vec::new())?;
    }
    Ok(())
}

fn ch_fadeout(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let secs = py::f64_of(vm, &args[1])?;
    fadeout(vm, &args[0], secs)?;
    Ok(Value::None)
}

fn ch_reload(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = number(vm, &args[0])?;
    rs(vm, "dequeue", vec![Value::Int(n), Value::Bool(true)], Vec::new())?;
    rs(vm, "stop", vec![Value::Int(n)], Vec::new())?;
    Ok(Value::None)
}

fn ch_set_audio_filter(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filter = args[1].clone();
    let replace = arg_or(&args[2..], kw, 0, "replace", Value::Bool(false));
    let duration = arg_or(&args[2..], kw, 1, "duration", Value::Float(0.016));
    let ctx = context(vm, this)?;
    let old = g(vm, &ctx, "raw_audio_filter")?;
    s(vm, &ctx, "raw_audio_filter", filter.clone())?;
    s(vm, this, "raw_audio_filter", filter.clone())?;
    let new = if py::is_none(&old) && py::is_none(&filter) {
        Value::None
    } else {
        let cf = py::import_attr(vm, "renpy.audio.filter", "Crossfade")?;
        let null = py::import_attr(vm, "renpy.audio.filter", "Null")?;
        let a = if py::is_none(&old) { vm.call(&null, Vec::new(), Vec::new())? } else { old };
        let b = if py::is_none(&filter) { vm.call(&null, Vec::new(), Vec::new())? } else { filter };
        vm.call(&cf, vec![a, b, duration], Vec::new())?
    };
    s(vm, &ctx, "audio_filter", new.clone())?;
    let q = g(vm, this, "queue")?;
    for e in py::items(vm, &q)? {
        s(vm, &e, "audio_filter", new.clone())?;
    }
    let n = number(vm, this)?;
    let r = Value::Int(py::truthy(vm, &replace)? as i64);
    rs(vm, "replace_audio_filter", vec![Value::Int(n), new, r], Vec::new())?;
    Ok(Value::None)
}

fn persistent_seen_audio(vm: &mut Vm, filename: &Value) -> PyResult<()> {
    let p = py::import_attr(vm, "renpy.game", "persistent")?;
    let sa = py::get(vm, &p, "_seen_audio")?;
    if py::is_none(&sa) {
        return Ok(());
    }
    let ks = py::string(vm, filename)?;
    let k = vm.types.new_str(&ks);
    ops::setitem(vm, &sa, k, Value::Bool(true))
}

const ENQ: &[&str] = &["filenames", "loop", "synchro_start", "fadein", "tight", "loop_only", "relative_volume"];

fn ch_enqueue(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let v: Vec<Value> = ENQ.iter().enumerate().map(|(i, n)| arg_or(&args[1..], kw, i, n, Value::None)).collect();
    let filenames = py::items(vm, &v[0])?;
    let loop_ = if py::is_none(&v[1]) { true } else { py::truthy(vm, &v[1])? };
    let mut synchro = v[2].clone();
    if py::is_none(&synchro) {
        synchro = g(vm, this, "default_synchro_start")?;
    }
    let mut fadein = if py::is_none(&v[3]) { Value::Int(0) } else { v[3].clone() };
    let loop_only = py::truthy(vm, &v[5])?;
    let rv = if py::is_none(&v[6]) { Value::Float(1.0) } else { v[6].clone() };
    for f in &filenames {
        let (f, _, _) = split_filename(vm, this, f, false)?;
        persistent_seen_audio(vm, &f)?;
    }
    if !loop_only {
        let tight = if py::is_none(&v[4]) { g(vm, this, "tight")? } else { v[4].clone() };
        let kq = g(vm, this, "keep_queue")?;
        let kq = py::int(vm, &kq)? + 1;
        s(vm, this, "keep_queue", Value::Int(kq))?;
        let q = g(vm, this, "queue")?;
        let filter = if at_least(vm, 8, 2) {
            let ctx = context(vm, this)?;
            g(vm, &ctx, "audio_filter")?
        } else {
            Value::None
        };
        for f in &filenames {
            let e = queue_entry(vm, f.clone(), fadein.clone(), tight.clone(), false, rv.clone(), filter.clone())?;
            vm.call_method(&q, "append", vec![e])?;
            fadein = Value::Int(0);
        }
        if py::truthy(vm, &synchro)? {
            s(vm, this, "synchro_start", Value::Bool(true))?;
        }
    }
    let l = if loop_ { vm.types.new_list(filenames) } else { vm.types.new_list(Vec::new()) };
    s(vm, this, "loop", l)?;
    Ok(Value::None)
}

fn ch_get_playing(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    if !pcm_ok(vm) {
        return Ok(Value::None);
    }
    let mut rv = Value::None;
    if !py::is_none(&g(vm, this, "_number")?) {
        let n = number(vm, this)?;
        rv = rs(vm, "playing_name", vec![Value::Int(n)], Vec::new())?;
    }
    let q = g(vm, this, "queue")?;
    if py::is_none(&rv) && ops::len(vm, &q)? > 0 {
        let first = ops::getitem(vm, &q, Value::Int(0))?;
        rv = g(vm, &first, "filename")?;
    }
    let l = g(vm, this, "loop")?;
    if py::is_none(&rv) && ops::len(vm, &l)? > 0 {
        rv = ops::getitem(vm, &l, Value::Int(0))?;
    }
    Ok(rv)
}

fn ch_set_volume(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    s(vm, &args[0], "chan_volume", args[1].clone())?;
    Ok(Value::None)
}

fn ch_get_pos(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !pcm_ok(vm) || py::is_none(&g(vm, &args[0], "_number")?) {
        return Ok(Value::Int(-1));
    }
    let n = number(vm, &args[0])?;
    rs(vm, "get_pos", vec![Value::Int(n)], Vec::new())
}

fn ch_get_duration(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !pcm_ok(vm) || py::is_none(&g(vm, &args[0], "_number")?) {
        return Ok(Value::Float(0.0));
    }
    let n = number(vm, &args[0])?;
    rs(vm, "get_duration", vec![Value::Int(n)], Vec::new())
}

fn ch_set_pan(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let now = get_serial(vm)?;
    let ctx = context(vm, this)?;
    s(vm, &ctx, "pan_time", now.clone())?;
    s(vm, &ctx, "pan", args[1].clone())?;
    if pcm_ok(vm) {
        s(vm, this, "pan_time", now)?;
        let n = number(vm, this)?;
        rs(vm, "set_pan", vec![Value::Int(n), args[1].clone(), args[2].clone()], Vec::new())?;
    }
    Ok(Value::None)
}

fn ch_set_secondary_volume(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let now = get_serial(vm)?;
    let ctx = context(vm, this)?;
    s(vm, &ctx, "secondary_volume_time", now.clone())?;
    s(vm, &ctx, "secondary_volume", args[1].clone())?;
    if pcm_ok(vm) {
        s(vm, this, "secondary_volume_time", now)?;
        let n = number(vm, this)?;
        rs(vm, "set_secondary_volume", vec![Value::Int(n), args[1].clone(), args[2].clone()], Vec::new())?;
    }
    Ok(Value::None)
}

fn ch_pause(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let n = number(vm, &args[0])?;
    rs(vm, "pause", vec![Value::Int(n)], Vec::new())
}

fn ch_unpause(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if py::is_none(&g(vm, &args[0], "_number")?) {
        return Ok(Value::None);
    }
    let n = number(vm, &args[0])?;
    rs(vm, "unpause", vec![Value::Int(n)], Vec::new())
}

fn ch_read_video(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !pcm_ok(vm) {
        return Ok(Value::None);
    }
    let n = number(vm, &args[0])?;
    rs(vm, "read_video", vec![Value::Int(n)], Vec::new())
}

fn ch_video_ready(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !pcm_ok(vm) {
        return Ok(Value::Int(1));
    }
    let n = number(vm, &args[0])?;
    rs(vm, "video_ready", vec![Value::Int(n)], Vec::new())
}

// Module functions -------------------------------------------------------------------

const REG: &[&str] = &["name", "mixer", "loop", "stop_on_mute", "tight", "file_prefix", "file_suffix", "buffer_queue", "movie", "framedrop", "force", "synchro_start"];

fn f_register_channel(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v: Vec<Value> = REG.iter().enumerate().map(|(i, n)| arg_or(args, kw, i, n, Value::None)).collect();
    let name = v[0].clone();
    let ns = py::string(vm, &name)?;
    let mut movie = py::truthy(vm, &v[8])?;
    if ns == "movie" {
        movie = true;
    }
    let mut synchro = v[11].clone();
    if py::is_none(&synchro) {
        synchro = if movie { Value::Bool(false) } else { v[2].clone() };
    }
    let force = py::truthy(vm, &v[10])?;
    let ctx = super::exports::util::ctx(vm)?;
    let ip = gb(vm, &ctx, "init_phase")?;
    if !force && !ip && !ns.contains(' ') {
        return Err(py::error(vm, "Can't register channel outside of init phase."));
    }
    let channels = aget(vm, "channels")?;
    let all = aget(vm, "all_channels")?;
    if ops::contains(vm, &channels, &name)? {
        let old = ops::getitem(vm, &channels, name.clone())?;
        vm.call_method(&all, "remove", vec![old])?;
        ops::delitem(vm, &channels, name.clone())?;
    }
    let d = |x: &Value, dv: Value| if py::is_none(x) { dv } else { x.clone() };
    let cls = aget(vm, "Channel")?;
    let c = vm.call(
        &cls,
        vec![name.clone(), v[2].clone(), d(&v[3], Value::Bool(true)), d(&v[4], Value::Bool(false)), v[5].clone(), v[6].clone(), d(&v[7], Value::Bool(true))],
        vec![("movie".into(), Value::Bool(movie)), ("framedrop".into(), d(&v[9], Value::Bool(true))), ("synchro_start".into(), synchro)],
    )?;
    s(vm, &c, "mixer", v[1].clone())?;
    vm.call_method(&all, "append", vec![c.clone()])?;
    ops::setitem(vm, &channels, name, c)?;
    Ok(Value::None)
}

fn f_alias_channel(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ctx = super::exports::util::ctx(vm)?;
    if !gb(vm, &ctx, "init_phase")? {
        return Err(py::error(vm, "Can't alias channel outside of init phase."));
    }
    let c = get_channel(vm, &args[0])?;
    let channels = aget(vm, "channels")?;
    ops::setitem(vm, &channels, args[1].clone(), c)?;
    Ok(Value::None)
}

pub fn get_channel(vm: &mut Vm, name: &Value) -> PyResult<Value> {
    let channels = aget(vm, "channels")?;
    let rv = vm.call_method(&channels, "get", vec![name.clone()])?;
    if !py::is_none(&rv) {
        return Ok(rv);
    }
    let ns = py::string(vm, name)?;
    let auto = cfg(vm, "auto_channels");
    if !py::is_none(&auto) && ops::contains(vm, &auto, name)? {
        let mut i = 0;
        loop {
            let n = vm.types.new_str(&format!("{ns} {i}"));
            let c = get_channel(vm, &n)?;
            let p = vm.call_method(&c, "get_playing", Vec::new())?;
            if !py::truthy(vm, &p)? || cfg_true(vm, "skipping") {
                return Ok(c);
            }
            i += 1;
        }
    }
    if let (Some((base, _)), false) = (ns.split_once(' '), py::is_none(&auto)) {
        let b = vm.types.new_str(base);
        let spec = ops::getitem(vm, &auto, b)?;
        let sp = py::items(vm, &spec)?;
        f_register_channel(vm, &[name.clone()], &[("loop".into(), Value::Bool(false)), ("mixer".into(), sp[0].clone()), ("file_prefix".into(), sp[1].clone()), ("file_suffix".into(), sp[2].clone())])?;
        return ops::getitem(vm, &channels, name.clone());
    }
    Err(py::error(vm, format!("Audio channel '{ns}' is unknown.")))
}

fn f_get_channel(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    get_channel(vm, &args[0])
}

fn f_set_force_stop(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = get_channel(vm, &args[0])?;
    let ctx = context(vm, &c)?;
    s(vm, &ctx, "force_stop", args[1].clone())?;
    Ok(Value::None)
}

fn f_init(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let m = amod(vm)?;
    if !cfg_true(vm, "sound") {
        py::set(vm, &m, "pcm_ok", Value::Bool(false))?;
        py::set(vm, &m, "mix_ok", Value::Bool(false))?;
        return Ok(Value::None);
    }
    py::set(vm, &m, "pcm_ok", Value::Bool(true))?;
    let p = prefs(vm)?;
    let vols = py::get(vm, &p, "volumes")?;
    let mute = py::get(vm, &p, "mute")?;
    let all = aget(vm, "all_channels")?;
    let mut mixers: Vec<Value> = Vec::new();
    for c in py::items(vm, &all)? {
        let mx = g(vm, &c, "mixer")?;
        let mut seen = false;
        for x in &mixers {
            if ops::eq(vm, x, &mx)? {
                seen = true;
            }
        }
        if !seen {
            mixers.push(mx);
        }
    }
    for mx in mixers {
        vm.call_method(&vols, "setdefault", vec![mx.clone(), Value::Float(1.0)])?;
        vm.call_method(&mute, "setdefault", vec![mx, Value::Bool(false)])?;
    }
    Ok(Value::None)
}

fn fadeout_secs(vm: &mut Vm) -> PyResult<f64> {
    let v = cfg(vm, "fadeout_audio");
    if py::is_none(&v) { Ok(0.016) } else { py::f64_of(vm, &v) }
}

fn f_fadeout_all(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let all = aget(vm, "all_channels")?;
    let secs = fadeout_secs(vm)?;
    for c in py::items(vm, &all)? {
        dequeue(vm, &c, false)?;
        fadeout(vm, &c, secs)?;
    }
    periodic(vm)?;
    Ok(Value::None)
}

fn f_quit(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if !pcm_ok(vm) {
        return Ok(Value::None);
    }
    let all = aget(vm, "all_channels")?;
    for c in py::items(vm, &all)? {
        dequeue(vm, &c, false)?;
        fadeout(vm, &c, 0.0)?;
        ch_unpause(vm, &[c.clone()], &[])?;
        s(vm, &c, "queue", vm.types.new_list(Vec::new()))?;
        s(vm, &c, "loop", vm.types.new_list(Vec::new()))?;
        s(vm, &c, "playing", Value::Bool(false))?;
        s(vm, &c, "synchro_start", Value::Bool(false))?;
    }
    let m = amod(vm)?;
    py::set(vm, &m, "pcm_ok", Value::None)?;
    Ok(Value::None)
}

/// `periodic_pass()`: emphasis volume and every channel's periodic.
fn periodic_pass(vm: &mut Vm) -> PyResult<()> {
    if !pcm_ok(vm) {
        return Ok(());
    }
    let r = (|| -> PyResult<()> {
        let m = amod(vm)?;
        let mut emph = Vec::new();
        let mut emphasized = false;
        let ec = cfg(vm, "emphasize_audio_channels");
        if !py::is_none(&ec) {
            for i in py::items(vm, &ec)? {
                let c = get_channel(vm, &i)?;
                let p = vm.call_method(&c, "get_playing", Vec::new())?;
                if py::truthy(vm, &p)? {
                    emphasized = true;
                }
                emph.push(c);
            }
        }
        let p = prefs(vm)?;
        if !py::get_opt(vm, &p, "emphasize_audio")?.map(|v| py::truthy(vm, &v)).transpose()?.unwrap_or(false) {
            emphasized = false;
        }
        let old = py::get(vm, &m, "old_emphasized")?;
        let old = py::truthy(vm, &old)?;
        let vol = if emphasized && !old {
            Some(cfg(vm, "emphasize_audio_volume"))
        } else if !emphasized && old {
            Some(Value::Float(1.0))
        } else {
            None
        };
        py::set(vm, &m, "old_emphasized", Value::Bool(emphasized))?;
        let all = aget(vm, "all_channels")?;
        if let Some(v) = vol {
            let t = cfg(vm, "emphasize_audio_time");
            for c in py::items(vm, &all)? {
                if emph.iter().any(|e| py::same(e, &c)) {
                    continue;
                }
                vm.call_method(&c, "set_secondary_volume", vec![v.clone(), t.clone()])?;
            }
        }
        for c in py::items(vm, &all)? {
            vm.call_method(&c, "periodic", Vec::new())?;
        }
        rs(vm, "periodic", Vec::new(), Vec::new())?;
        Ok(())
    })();
    match r {
        Err(e) if debug_sound(vm) => Err(e),
        _ => Ok(()),
    }
}

fn periodic(vm: &mut Vm) -> PyResult<()> {
    periodic_pass(vm)
}

fn f_periodic(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    periodic(vm)?;
    Ok(Value::None)
}

fn f_periodic_pass(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    periodic_pass(vm)?;
    Ok(Value::None)
}

/// `interact()`: per-channel interaction work and restoring the music of
/// the current context.
fn interact(vm: &mut Vm) -> PyResult<()> {
    if !pcm_ok(vm) {
        return Ok(());
    }
    let r = (|| -> PyResult<()> {
        let all = aget(vm, "all_channels")?;
        for c in py::items(vm, &all)? {
            vm.call_method(&c, "interact", Vec::new())?;
            let ctx = context(vm, &c)?;
            let (a, b) = (g(vm, &c, "last_changed")?, g(vm, &ctx, "last_changed")?);
            if ops::eq(vm, &a, &b)? {
                continue;
            }
            let filenames = g(vm, &ctx, "last_filenames")?;
            let tight = g(vm, &ctx, "last_tight")?;
            if at_least(vm, 8, 2) {
                let (cr, rr) = (g(vm, &ctx, "raw_audio_filter")?, py::get_opt(vm, &c, "raw_audio_filter")?.unwrap_or(Value::None));
                if !ops::eq(vm, &cr, &rr)? {
                    py::method_kw(vm, &c, "set_audio_filter", vec![cr], vec![("replace".into(), Value::Bool(true))])?;
                }
            }
            let loop_ = g(vm, &c, "loop")?;
            if !ops::eq(vm, &loop_, &filenames)? {
                let a = cfg(vm, "context_fadeout_music");
                let a = if py::is_none(&a) { 0.0 } else { py::f64_of(vm, &a)? };
                let b = fadeout_secs(vm)?;
                fadeout(vm, &c, a.max(if at_least(vm, 7, 4) { b } else { 0.0 }))?;
            }
            if py::truthy(vm, &filenames)? {
                let fi = cfg(vm, "context_fadein_music");
                let dss = g(vm, &c, "default_synchro_start")?;
                let mut kw: Vec<(Rc<str>, Value)> = vec![("loop".into(), Value::Bool(true)), ("synchro_start".into(), dss), ("tight".into(), tight), ("fadein".into(), fi)];
                if let Some(rv) = py::get_opt(vm, &ctx, "last_relative_volume")? {
                    kw.push(("relative_volume".into(), rv));
                }
                py::method_kw(vm, &c, "enqueue", vec![filenames], kw)?;
            }
            s(vm, &c, "last_changed", b)?;
        }
        Ok(())
    })();
    if let Err(e) = r {
        if debug_sound(vm) {
            return Err(e);
        }
    }
    periodic(vm)
}

fn f_interact(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    interact(vm)?;
    Ok(Value::None)
}

fn f_pump(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    interact(vm)?;
    periodic_pass(vm)?;
    Ok(Value::None)
}

fn f_rollback(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let all = aget(vm, "all_channels")?;
    let secs = fadeout_secs(vm)?;
    for c in py::items(vm, &all)? {
        let l = g(vm, &c, "loop")?;
        if !py::truthy(vm, &l)? {
            fadeout(vm, &c, secs)?;
        }
    }
    Ok(Value::None)
}

fn f_autoreload(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let all = aget(vm, "all_channels")?;
    for c in py::items(vm, &all)? {
        vm.call_method(&c, "reload", Vec::new())?;
    }
    super::exports::util::ex(vm, "restart_interaction", Vec::new(), Vec::new())?;
    Ok(Value::None)
}

fn f_pause_all(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if pcm_ok(vm) {
        rs(vm, "global_pause", vec![Value::Int(1)], Vec::new())?;
    }
    Ok(Value::None)
}

fn f_unpause_all(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    if pcm_ok(vm) {
        rs(vm, "global_pause", vec![Value::Int(0)], Vec::new())?;
    }
    Ok(Value::None)
}

fn f_load(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::import_attr(vm, "renpy.loader", "load")?;
    vm.call(&f, vec![args[0].clone()], vec![("directory".into(), py::s(vm, "audio"))])
}

fn ad_new(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let str_t = Value::Obj(vm.types.str_.clone());
    let new = py::get(vm, &str_t, "__new__")?;
    let rv = vm.call(&new, vec![args[0].clone(), args[2].clone()], Vec::new())?;
    py::set(vm, &rv, "data", args[1].clone())?;
    Ok(rv)
}

fn ad_init(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn ad_reduce(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = aget(vm, "AudioData")?;
    let data = g(vm, &args[0], "data")?;
    let s = ops::str(vm, &args[0])?;
    let sv = vm.types.new_str(&s);
    let a = vm.types.new_tuple(vec![data, sv]);
    Ok(vm.types.new_tuple(vec![c, a]))
}

fn lock_enter(_: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(args[0].clone())
}

fn lock_exit(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(false))
}

fn audio_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        AUDIO,
        &[
            ("get_serial", f_get_serial),
            ("load", f_load),
            ("register_channel", f_register_channel),
            ("alias_channel", f_alias_channel),
            ("get_channel", f_get_channel),
            ("set_force_stop", f_set_force_stop),
            ("init", f_init),
            ("fadeout_all", f_fadeout_all),
            ("quit", f_quit),
            ("periodic_pass", f_periodic_pass),
            ("periodic", f_periodic),
            ("interact", f_interact),
            ("pump", f_pump),
            ("rollback", f_rollback),
            ("autoreload", f_autoreload),
            ("pause_all", f_pause_all),
            ("unpause_all", f_unpause_all),
            ("sample_surfaces", rs_noop),
            ("advance_time", rs_noop),
        ],
    );
    let set = |vm: &Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let sentinel = py::import_attr(vm, "renpy.object", "Sentinel")?;
    let anr = vm.call(&sentinel, vec![py::s(vm, "AudioNotReady")], Vec::new())?;
    set(vm, "AudioNotReady", anr);
    let ns = vm.call(&sentinel, vec![py::s(vm, "NotSet")], Vec::new())?;
    set(vm, "NotSet", ns);
    let str_t = Value::Obj(vm.types.str_.clone());
    let new = vm.types.new_builtin("__new__", ad_new);
    let new = py::staticmethod(vm, new);
    let ad = py::class(vm, AUDIO, "AudioData", &[str_t], &[("__init__", ad_init), ("__reduce__", ad_reduce)], &[("__new__", new)])?;
    set(vm, "AudioData", Value::Obj(ad));
    let qe = py::class(vm, AUDIO, "QueueEntry", &[], &[("__init__", qe_init)], &[("audio_filter", Value::None)])?;
    set(vm, "QueueEntry", Value::Obj(qe));
    let ro = py::import_attr(vm, "renpy.revertable", "RevertableObject")?;
    let mc = py::class(
        vm,
        AUDIO,
        "MusicContext",
        &[ro],
        &[("__init__", mc_init), ("copy", mc_copy)],
        &[("__version__", Value::Int(0)), ("pause", Value::Bool(false)), ("last_relative_volume", Value::Float(1.0)), ("audio_filter", Value::None), ("raw_audio_filter", Value::None)],
    )?;
    set(vm, "MusicContext", Value::Obj(mc));
    let num = vm.types.new_builtin("get_number", ch_get_number);
    let num = py::property(vm, num, None)?;
    let ctxp = vm.types.new_builtin("get_context", ch_get_context);
    let ctxp = py::property(vm, ctxp, None)?;
    let ch = py::class(
        vm,
        AUDIO,
        "Channel",
        &[],
        &[
            ("__init__", ch_init),
            ("get_number", ch_get_number),
            ("get_context", ch_get_context),
            ("copy_context", ch_copy_context),
            ("split_filename", ch_split_filename),
            ("periodic", ch_periodic),
            ("dequeue", ch_dequeue),
            ("interact", ch_interact),
            ("fadeout", ch_fadeout),
            ("reload", ch_reload),
            ("set_audio_filter", ch_set_audio_filter),
            ("enqueue", ch_enqueue),
            ("get_playing", ch_get_playing),
            ("set_volume", ch_set_volume),
            ("get_pos", ch_get_pos),
            ("get_duration", ch_get_duration),
            ("set_pan", ch_set_pan),
            ("set_secondary_volume", ch_set_secondary_volume),
            ("pause", ch_pause),
            ("unpause", ch_unpause),
            ("read_video", ch_read_video),
            ("video_ready", ch_video_ready),
        ],
        &[("number", num), ("context", ctxp), ("audio_filter", Value::None), ("raw_audio_filter", Value::None)],
    )?;
    set(vm, "Channel", Value::Obj(ch));
    let lock = py::class(vm, AUDIO, "_Lock", &[], &[("__enter__", lock_enter), ("__exit__", lock_exit), ("notify", rs_noop), ("notify_all", rs_noop), ("wait", rs_noop)], &[])?;
    let l = vm.call(&Value::Obj(lock.clone()), Vec::new(), Vec::new())?;
    set(vm, "lock", l);
    let pc = vm.call(&Value::Obj(lock), Vec::new(), Vec::new())?;
    set(vm, "periodic_condition", pc);
    set(vm, "pcm_ok", Value::None);
    set(vm, "mix_ok", Value::None);
    set(vm, "unique", Value::Float(super::link::now(vm)));
    set(vm, "serial", Value::Int(0));
    set(vm, "next_channel_number", Value::Int(0));
    set(vm, "all_channels", vm.types.new_list(Vec::new()));
    set(vm, "channels", py::dict(vm));
    set(vm, "old_emphasized", Value::Bool(false));
    set(vm, "pcm_volume", Value::None);
    set(vm, "periodic_thread", Value::None);
    let rsm = py::import(vm, RENPYSOUND)?;
    set(vm, "renpysound", rsm);
    Ok(m)
}

// ---------------------------------------------------------------------------
// renpy.audio.music / renpy.audio.sound.

fn init_phase(vm: &mut Vm) -> PyResult<bool> {
    let ctx = super::exports::util::ctx(vm)?;
    gb(vm, &ctx, "init_phase")
}

fn filenames_of(vm: &mut Vm, f: &Value) -> PyResult<Option<Value>> {
    if py::is_none(f) {
        return Ok(None);
    }
    if f.as_str().is_some() {
        return Ok(Some(vm.types.new_list(vec![f.clone()])));
    }
    Ok(Some(f.clone()))
}

fn guard(vm: &mut Vm, r: PyResult<()>) -> PyResult<()> {
    match r {
        Err(e) if debug_sound(vm) => Err(e),
        _ => Ok(()),
    }
}

fn mark_changed(vm: &mut Vm, c: &Value, ctx: &Value, loop_: bool, filenames: &Value, tight: &Value, rv: &Value) -> PyResult<()> {
    let t = get_serial(vm)?;
    s(vm, ctx, "last_changed", t.clone())?;
    s(vm, c, "last_changed", t)?;
    if loop_ {
        s(vm, ctx, "last_filenames", filenames.clone())?;
        s(vm, ctx, "last_tight", tight.clone())?;
        if at_least(vm, 7, 4) {
            s(vm, ctx, "last_relative_volume", rv.clone())?;
        }
    } else {
        s(vm, ctx, "last_filenames", vm.types.new_list(Vec::new()))?;
        s(vm, ctx, "last_tight", Value::Bool(false))?;
        if at_least(vm, 7, 4) {
            s(vm, ctx, "last_relative_volume", Value::Float(1.0))?;
        }
    }
    if at_least(vm, 7, 0) {
        s(vm, ctx, "pause", Value::Bool(false))?;
    }
    Ok(())
}

fn skip_enqueue(vm: &mut Vm, loop_: bool) -> bool {
    cfg_true(vm, "skip_sounds") && cfg_true(vm, "skipping") && !loop_
}

const PLAY: &[&str] = &["filenames", "channel", "loop", "fadeout", "synchro_start", "fadein", "tight", "if_changed", "relative_volume"];

fn m_play(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v: Vec<Value> = PLAY.iter().enumerate().map(|(i, n)| arg_or(args, kw, i, n, Value::None)).collect();
    if init_phase(vm)? {
        return Err(py::error(vm, "Can't play music during init phase."));
    }
    let Some(filenames) = filenames_of(vm, &v[0])? else { return Ok(Value::None) };
    let channel = if py::is_none(&v[1]) { py::s(vm, "music") } else { v[1].clone() };
    let mut fadeout_v = v[3].clone();
    if m_get_pause_inner(vm, &channel) {
        fadeout_v = Value::Int(0);
    }
    let r = (|| -> PyResult<()> {
        let c = get_channel(vm, &channel)?;
        let ctx = copy_context(vm, &c)?;
        let loop_ = if py::is_none(&v[2]) { gb(vm, &c, "default_loop")? } else { py::truthy(vm, &v[2])? };
        let mut tight = v[6].clone();
        if py::is_none(&tight) && cfg_true(vm, "tight_loop_default") {
            tight = Value::Bool(loop_);
        }
        let cl = g(vm, &c, "loop")?;
        let loop_is_filenames = ops::eq(vm, &cl, &filenames)?;
        let fo = if py::is_none(&fadeout_v) { fadeout_secs(vm)? } else { py::f64_of(vm, &fadeout_v)? };
        let mut fadein = if py::is_none(&v[5]) { Value::Int(0) } else { v[5].clone() };
        let if_changed = py::truthy(vm, &v[7])?;
        let loop_only;
        let playing = vm.call_method(&c, "get_playing", Vec::new())?;
        if if_changed && ops::contains(vm, &filenames, &playing)? {
            fadein = Value::Int(0);
            loop_only = loop_is_filenames;
            if !loop_is_filenames {
                dequeue(vm, &c, false)?;
            }
        } else {
            dequeue(vm, &c, false)?;
            fadeout(vm, &c, fo)?;
            loop_only = false;
        }
        let rv = if py::is_none(&v[8]) { Value::Float(1.0) } else { v[8].clone() };
        if !skip_enqueue(vm, loop_) {
            py::method_kw(vm, &c, "enqueue", vec![filenames.clone()], vec![
                ("loop".into(), Value::Bool(loop_)),
                ("synchro_start".into(), v[4].clone()),
                ("fadein".into(), fadein),
                ("tight".into(), tight.clone()),
                ("loop_only".into(), Value::Bool(loop_only)),
                ("relative_volume".into(), rv.clone()),
            ])?;
        }
        mark_changed(vm, &c, &ctx, loop_, &filenames, &tight, &rv)
    })();
    guard(vm, r)?;
    Ok(Value::None)
}

const QUEUE: &[&str] = &["filenames", "channel", "loop", "clear_queue", "fadein", "tight", "relative_volume"];

fn m_queue(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v: Vec<Value> = QUEUE.iter().enumerate().map(|(i, n)| arg_or(args, kw, i, n, Value::None)).collect();
    if init_phase(vm)? {
        return Err(py::error(vm, "Can't play music during init phase."));
    }
    let mut loop_v = v[2].clone();
    let filenames = match filenames_of(vm, &v[0])? {
        Some(f) => f,
        None => {
            loop_v = Value::Bool(false);
            vm.types.new_list(Vec::new())
        }
    };
    let channel = if py::is_none(&v[1]) { py::s(vm, "music") } else { v[1].clone() };
    if cfg(vm, "skipping").as_str() == Some("fast") {
        m_stop(vm, &[channel.clone()], &[])?;
    }
    if at_least(vm, 7, 0) {
        m_set_pause(vm, &[Value::Bool(false)], &[("channel".into(), channel.clone())])?;
    }
    let r = (|| -> PyResult<()> {
        let c = get_channel(vm, &channel)?;
        let ctx = copy_context(vm, &c)?;
        let loop_ = if py::is_none(&loop_v) { gb(vm, &c, "default_loop")? } else { py::truthy(vm, &loop_v)? };
        let mut tight = v[5].clone();
        if py::is_none(&tight) && cfg_true(vm, "tight_loop_default") {
            tight = Value::Bool(loop_);
        }
        let clear = if py::is_none(&v[3]) { true } else { py::truthy(vm, &v[3])? };
        if clear {
            dequeue(vm, &c, true)?;
        }
        let rv = if py::is_none(&v[6]) { Value::Float(1.0) } else { v[6].clone() };
        if !skip_enqueue(vm, loop_) {
            let fadein = if py::is_none(&v[4]) { Value::Int(0) } else { v[4].clone() };
            py::method_kw(vm, &c, "enqueue", vec![filenames.clone()], vec![
                ("loop".into(), Value::Bool(loop_)),
                ("fadein".into(), fadein),
                ("tight".into(), tight.clone()),
                ("relative_volume".into(), rv.clone()),
            ])?;
        }
        mark_changed(vm, &c, &ctx, loop_, &filenames, &tight, &rv)
    })();
    guard(vm, r)?;
    Ok(Value::None)
}

fn m_playable(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 1, "channel", py::s(vm, "music"));
    let c = get_channel(vm, &channel)?;
    let (f, _, _) = split_filename(vm, &c, &args[0], false)?;
    let f = py::string(vm, &f)?;
    Ok(Value::Bool(super::resloader::loadable(vm, &f, true, Some("audio"))?))
}

fn m_stop(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    if init_phase(vm)? {
        return Ok(Value::None);
    }
    let channel = arg_or(args, kw, 0, "channel", py::s(vm, "music"));
    let mut fo = arg_or(args, kw, 1, "fadeout", Value::None);
    if m_get_pause_inner(vm, &channel) {
        fo = Value::Float(0.0);
    }
    let r = (|| -> PyResult<()> {
        let c = get_channel(vm, &channel)?;
        let ctx = copy_context(vm, &c)?;
        let secs = if py::is_none(&fo) { fadeout_secs(vm)? } else { py::f64_of(vm, &fo)? };
        fadeout(vm, &c, secs)?;
        let t = get_serial(vm)?;
        s(vm, &ctx, "last_changed", t.clone())?;
        s(vm, &c, "last_changed", t)?;
        s(vm, &ctx, "last_filenames", vm.types.new_list(Vec::new()))?;
        s(vm, &ctx, "last_tight", Value::Bool(false))?;
        Ok(())
    })();
    guard(vm, r)?;
    if at_least(vm, 7, 0) {
        m_set_pause(vm, &[Value::Bool(false)], &[("channel".into(), channel)])?;
    }
    Ok(Value::None)
}

fn m_set_music(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let c = get_channel(vm, &args[0])?;
    let default = arg_or(&args[2..], kw, 0, "default", Value::Bool(false));
    if py::truthy(vm, &default)? && gb(vm, &c, "default_loop_set")? {
        return Ok(Value::None);
    }
    s(vm, &c, "default_loop", args[1].clone())?;
    s(vm, &c, "default_loop_set", Value::Bool(true))?;
    Ok(Value::None)
}

fn m_is_music(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = get_channel(vm, &args[0])?;
    g(vm, &c, "default_loop")
}

fn pos_of(vm: &mut Vm, channel: &Value) -> PyResult<Option<f64>> {
    let c = get_channel(vm, channel)?;
    let t = vm.call_method(&c, "get_pos", Vec::new())?;
    let t = py::f64_of(vm, &t).unwrap_or(-1.0);
    Ok((t > 0.0).then_some(t))
}

fn m_get_delay(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 1, "channel", py::s(vm, "music"));
    let time = py::f64_of(vm, &args[0])?;
    match pos_of(vm, &channel) {
        Ok(Some(t)) => Ok(if t > time { Value::Int(0) } else { Value::Float(time - t) }),
        Ok(None) => Ok(Value::None),
        Err(e) if debug_sound(vm) => Err(e),
        Err(_) => Ok(Value::None),
    }
}

fn m_get_pos(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 0, "channel", py::s(vm, "music"));
    match pos_of(vm, &channel) {
        Ok(Some(t)) => Ok(Value::Float(t)),
        Ok(None) => Ok(Value::None),
        Err(e) if debug_sound(vm) => Err(e),
        Err(_) => Ok(Value::None),
    }
}

fn m_get_duration(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 0, "channel", py::s(vm, "music"));
    match get_channel(vm, &channel).and_then(|c| vm.call_method(&c, "get_duration", Vec::new())) {
        Ok(v) => Ok(v),
        Err(e) if debug_sound(vm) => Err(e),
        Err(_) => Ok(Value::Float(0.0)),
    }
}

fn m_get_playing(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 0, "channel", py::s(vm, "music"));
    match get_channel(vm, &channel).and_then(|c| vm.call_method(&c, "get_playing", Vec::new())) {
        Ok(v) => Ok(v),
        Err(e) if debug_sound(vm) => Err(e),
        Err(_) => Ok(Value::None),
    }
}

fn m_is_playing(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let p = m_get_playing(vm, args, kw)?;
    Ok(Value::Bool(!py::is_none(&p)))
}

fn m_get_loop(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 0, "channel", py::s(vm, "music"));
    let c = get_channel(vm, &channel)?;
    let ctx = context(vm, &c)?;
    let l = g(vm, &ctx, "last_filenames")?;
    Ok(if py::truthy(vm, &l)? { l } else { Value::None })
}

fn channel_call(vm: &mut Vm, channel: &Value, method: &str, args: Vec<Value>) -> PyResult<Value> {
    match get_channel(vm, channel).and_then(|c| vm.call_method(&c, method, args)) {
        Ok(_) => Ok(Value::None),
        Err(e) if debug_sound(vm) => Err(e),
        Err(_) => Ok(Value::None),
    }
}

fn m_set_volume(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let delay = arg_or(args, kw, 1, "delay", Value::Int(0));
    let channel = arg_or(args, kw, 2, "channel", py::s(vm, "music"));
    channel_call(vm, &channel, "set_secondary_volume", vec![args[0].clone(), delay])
}

fn m_set_pan(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let delay = arg_or(args, kw, 1, "delay", Value::Int(0));
    let channel = arg_or(args, kw, 2, "channel", py::s(vm, "music"));
    channel_call(vm, &channel, "set_pan", vec![args[0].clone(), delay])
}

fn m_set_queue_empty_callback(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 1, "channel", py::s(vm, "music"));
    match get_channel(vm, &channel) {
        Ok(c) => s(vm, &c, "callback", args[0].clone())?,
        Err(e) if debug_sound(vm) => return Err(e),
        Err(_) => {}
    }
    Ok(Value::None)
}

fn m_set_pause(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 1, "channel", py::s(vm, "music"));
    let value = arg_or(args, kw, 0, "value", Value::Bool(false));
    match get_channel(vm, &channel).and_then(|c| copy_context(vm, &c)) {
        Ok(ctx) => s(vm, &ctx, "pause", value)?,
        Err(e) if debug_sound(vm) => return Err(e),
        Err(_) => {}
    }
    Ok(Value::None)
}

fn m_get_pause_inner(vm: &mut Vm, channel: &Value) -> bool {
    let r = (|| -> PyResult<bool> {
        let c = get_channel(vm, channel)?;
        let ctx = context(vm, &c)?;
        let p = py::get_opt(vm, &ctx, "pause")?.unwrap_or(Value::Bool(false));
        py::truthy(vm, &p)
    })();
    r.unwrap_or(false)
}

fn m_get_pause(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 0, "channel", py::s(vm, "music"));
    Ok(Value::Bool(m_get_pause_inner(vm, &channel)))
}

fn m_set_mixer(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let default = arg_or(&args[2..], kw, 0, "default", Value::Bool(false));
    match get_channel(vm, &args[0]) {
        Ok(c) => {
            let cur = g(vm, &c, "mixer")?;
            if !py::truthy(vm, &default)? || py::is_none(&cur) {
                s(vm, &c, "mixer", args[1].clone())?;
            }
        }
        Err(e) if debug_sound(vm) => return Err(e),
        Err(_) => {}
    }
    Ok(Value::None)
}

fn m_get_all_mixers(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    let set_ = vm.types.new_set(Default::default());
    let all = aget(vm, "all_channels")?;
    for c in py::items(vm, &all)? {
        let mx = g(vm, &c, "mixer")?;
        ops::set_add(vm, &set_, mx)?;
    }
    let auto = cfg(vm, "auto_channels");
    if !py::is_none(&auto) {
        let vals = vm.call_method(&auto, "values", Vec::new())?;
        for i in py::items(vm, &vals)? {
            let first = ops::getitem(vm, &i, Value::Int(0))?;
            ops::set_add(vm, &set_, first)?;
        }
    }
    let items = py::items(vm, &set_)?;
    Ok(vm.types.new_list(items))
}

fn m_channel_defined(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(get_channel(vm, &args[0]).is_ok()))
}

fn m_pump(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    f_pump(vm, &[], &[])
}

fn m_set_audio_filter(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = args[0].clone();
    let mut filter = args.get(1).cloned().or_else(|| py::kw(kw, "audio_filter").cloned()).unwrap_or(Value::None);
    let replace = arg_or(&args[2..], kw, 0, "replace", Value::Bool(false));
    let duration = arg_or(&args[2..], kw, 1, "duration", Value::Float(0.016));
    let ar = py::import_attr(vm, "renpy.game", "after_rollback")?;
    let replace = py::truthy(vm, &replace)? || py::truthy(vm, &ar)?;
    if !py::is_none(&filter) {
        let f = py::import_attr(vm, "renpy.audio.filter", "to_audio_filter")?;
        filter = vm.call(&f, vec![filter], Vec::new())?;
    }
    let r = (|| -> PyResult<()> {
        let c = get_channel(vm, &channel)?;
        let ctx = copy_context(vm, &c)?;
        let t = get_serial(vm)?;
        s(vm, &ctx, "last_changed", t)?;
        py::method_kw(vm, &c, "set_audio_filter", vec![filter], vec![("replace".into(), Value::Bool(replace)), ("duration".into(), duration)])?;
        Ok(())
    })();
    guard(vm, r)?;
    Ok(Value::None)
}

fn music_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        MUSIC,
        &[
            ("play", m_play),
            ("queue", m_queue),
            ("playable", m_playable),
            ("stop", m_stop),
            ("set_music", m_set_music),
            ("is_music", m_is_music),
            ("get_delay", m_get_delay),
            ("get_pos", m_get_pos),
            ("get_duration", m_get_duration),
            ("get_playing", m_get_playing),
            ("is_playing", m_is_playing),
            ("get_loop", m_get_loop),
            ("set_volume", m_set_volume),
            ("set_pan", m_set_pan),
            ("set_queue_empty_callback", m_set_queue_empty_callback),
            ("set_pause", m_set_pause),
            ("get_pause", m_get_pause),
            ("pump", m_pump),
            ("set_mixer", m_set_mixer),
            ("get_all_mixers", m_get_all_mixers),
            ("channel_defined", m_channel_defined),
            ("set_audio_filter", m_set_audio_filter),
        ],
    );
    for n in ["get_channel", "get_serial", "register_channel", "alias_channel"] {
        let v = py::import_attr(vm, AUDIO, n)?;
        crysirror_python::modules::set(vm, &m, n, v);
    }
    Ok(m)
}

fn with_default_channel(args: &[Value], kw: Kw, pos: usize, names: &[&str]) -> (Vec<Value>, Vec<(Rc<str>, Value)>) {
    let mut k: Vec<(Rc<str>, Value)> = kw.to_vec();
    let mut a = args.to_vec();
    for (i, v) in args.iter().enumerate().skip(1) {
        if let Some(n) = names.get(i) {
            k.retain(|(kk, _)| **kk != **n);
            k.push(((*n).into(), v.clone()));
        }
    }
    a.truncate(pos.min(a.len()).max(1));
    (a, k)
}

fn sound_defaults(vm: &mut Vm, kw: &mut Vec<(Rc<str>, Value)>, defaults: &[(&str, Value)]) {
    for (n, d) in defaults {
        if !kw.iter().any(|(k, _)| &**k == *n) {
            kw.push(((*n).into(), d.clone()));
        }
    }
    let _ = vm;
}

fn s_play(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (a, mut k) = with_default_channel(args, kw, 1, &["filename", "channel", "fadeout", "fadein", "tight", "loop", "relative_volume"]);
    sound_defaults(vm, &mut k, &[("channel", py::s(vm, "sound")), ("fadeout", Value::Int(0)), ("fadein", Value::Int(0)), ("tight", Value::Bool(false)), ("loop", Value::Bool(false))]);
    m_play(vm, &a, &k)
}

fn s_queue(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let (a, mut k) = with_default_channel(args, kw, 1, &["filename", "channel", "clear_queue", "fadein", "tight", "loop", "relative_volume"]);
    sound_defaults(vm, &mut k, &[("channel", py::s(vm, "sound")), ("clear_queue", Value::Bool(true)), ("fadein", Value::Int(0)), ("tight", Value::Bool(false)), ("loop", Value::Bool(false))]);
    m_queue(vm, &a, &k)
}

fn s_stop(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 0, "channel", py::s(vm, "sound"));
    let fo = arg_or(args, kw, 1, "fadeout", Value::Int(0));
    m_stop(vm, &[], &[("channel".into(), channel), ("fadeout".into(), fo)])
}

fn s_set_volume(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let delay = arg_or(args, kw, 1, "delay", Value::Int(0));
    let channel = arg_or(args, kw, 2, "channel", py::s(vm, "sound"));
    m_set_volume(vm, &[args[0].clone(), delay], &[("channel".into(), channel)])
}

fn s_set_pan(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let delay = arg_or(args, kw, 1, "delay", Value::Int(0));
    let channel = arg_or(args, kw, 2, "channel", py::s(vm, "sound"));
    m_set_pan(vm, &[args[0].clone(), delay], &[("channel".into(), channel)])
}

fn s_is_playing(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 0, "channel", py::s(vm, "sound"));
    m_is_playing(vm, &[], &[("channel".into(), channel)])
}

fn s_get_playing(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let channel = arg_or(args, kw, 0, "channel", py::s(vm, "sound"));
    m_get_playing(vm, &[], &[("channel".into(), channel)])
}

fn sound_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(
        vm,
        SOUND,
        &[("play", s_play), ("queue", s_queue), ("stop", s_stop), ("set_volume", s_set_volume), ("set_pan", s_set_pan), ("is_playing", s_is_playing), ("get_playing", s_get_playing)],
    );
    for n in ["set_mixer", "set_queue_empty_callback"] {
        let v = py::import_attr(vm, MUSIC, n)?;
        crysirror_python::modules::set(vm, &m, n, v);
    }
    Ok(m)
}

// renpy.audio.filter (8.2): filter descriptions; DSP is not provided.

fn filter_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = vm.types.new_tuple(args[1..].to_vec());
    py::set(vm, &args[0], "args", a)?;
    let k = py::kwargs_dict(vm, kw)?;
    py::set(vm, &args[0], "kwargs", k)?;
    Ok(Value::None)
}

fn filter_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let t = ops::type_of(vm, &args[0]);
    let a = py::get(vm, &args[0], "args")?;
    let r = ops::repr(vm, &a)?;
    let n = py::get(vm, &Value::Obj(t.clone()), "__name__")?;
    let n = py::string(vm, &n)?;
    Ok(vm.types.new_str(&format!("<{n} {r}>")))
}

fn to_audio_filter(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = &args[0];
    if matches!(f.payload(), Some(crysirror_python::Payload::List(_) | crysirror_python::Payload::Tuple(_))) {
        let seq = py::import_attr(vm, "renpy.audio.filter", "Sequence")?;
        let items = py::items(vm, f)?;
        return vm.call(&seq, items, Vec::new());
    }
    Ok(f.clone())
}

fn filter_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, "renpy.audio.filter", &[("to_audio_filter", to_audio_filter)]);
    let base = py::class(vm, "renpy.audio.filter", "AudioFilter", &[], &[("__init__", filter_init), ("__repr__", filter_repr)], &[])?;
    crysirror_python::modules::set(vm, &m, "AudioFilter", Value::Obj(base.clone()));
    for n in ["Null", "Sequence", "Crossfade", "Biquad", "Lowpass", "Highpass", "Bandpass", "Lowshelf", "Highshelf", "Peaking", "Allpass", "Notch", "Mix", "Multiply", "Delay", "Comb", "WetDry", "Reverb", "SimpleReverb"] {
        let c = py::class(vm, "renpy.audio.filter", n, &[Value::Obj(base.clone())], &[], &[])?;
        crysirror_python::modules::set(vm, &m, n, Value::Obj(c));
    }
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(RENPYSOUND, renpysound_module);
    vm.register_native_module(AUDIO, audio_module);
    vm.register_native_module(MUSIC, music_module);
    vm.register_native_module(SOUND, sound_module);
    vm.register_native_module("renpy.audio.filter", filter_module);
}
