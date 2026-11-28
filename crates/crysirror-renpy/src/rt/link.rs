// SPDX-License-Identifier: MPL-2.0
//! The script-thread link: the bounded message protocol between the
//! adapter (owner thread, which holds `HostServices`) and the thread that
//! owns the Ren'Py VM.
//!
//! Ren'Py's interaction loop (`Interface.interact_core`) blocks waiting for
//! events, and event handlers start nested interactions from inside native
//! displayable code. The Python VM can only suspend Python frames, so the
//! VM runs on its own thread and blocks on this link instead: each owner
//! tick sends the host clock and input; the script thread runs until it has
//! drawn a frame or has nothing to do at that time, then returns the
//! frame's layer operations, audio commands and wait hint. Both channels
//! hold one message, so at most one tick is in flight.
use crysirror_core::service::InputFrame;
use crysirror_python::{PyErr, PyResult, Value, Vm};
use serde_json::Value as J;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc::{Receiver, SyncSender},
};

/// Host state sampled at the start of a tick.
#[derive(Clone, Default)]
pub struct Tick {
    pub now_ms: u64,
    pub input: InputFrame,
    /// Playing state reported by the host audio service, per channel name.
    pub audio: Vec<AudioStatus>,
    /// Results of the image publications requested by the previous
    /// output: (request id, layer asset path or error).
    pub published: Vec<(u32, Result<String, String>)>,
    /// Text committed to the host's text-input session since the last tick.
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct AudioStatus {
    pub channel: String,
    pub playing: Option<String>,
    pub position_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    /// Frame size of the video playing on a movie channel.
    pub video_size: Option<[u32; 2]>,
}

pub enum ToScript {
    Tick(Tick),
    /// The owner is shutting the session down; the VM unwinds with
    /// `renpy.game.QuitException`.
    Shutdown,
}

/// What the owner should wait for before the next tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// More frames are wanted (animation, pending redraw).
    NextFrame,
    /// Nothing changes before this host time unless input arrives.
    Until(u64),
    /// Only input can make progress.
    Input,
}

#[derive(Default)]
pub struct Output {
    /// Retained layer operations of the last frame drawn in this tick.
    pub frame: Option<Vec<J>>,
    /// Audio commands in issue order.
    pub audio: Vec<J>,
    pub title: Option<String>,
    /// Text-input rectangle requested by the focused input, if any.
    pub text_rect: Option<[i32; 4]>,
    pub diagnostics: Vec<String>,
    /// Self-voicing text to speak, in order (`config.tts_function`).
    pub speech: Vec<String>,
    /// Processed images to publish as client image resources, by request
    /// id; the owner answers in the next tick's `published`.
    pub publish: Vec<(
        u32,
        std::sync::Arc<dyn crysirror_core::image_processing::ImageResource>,
    )>,
}

pub enum FromScript {
    Yield {
        out: Output,
        wait: Wait,
    },
    /// The VM finished: `Ok` on a normal quit, `Err` with the report of an
    /// uncaught error.
    Exit {
        out: Output,
        result: Result<(), String>,
    },
}

pub struct Link {
    rx: Rc<Receiver<ToScript>>,
    tx: SyncSender<FromScript>,
    now_ms: u64,
    /// Text-input rectangle in effect until the game changes it.
    text_rect: Option<[i32; 4]>,
    /// Wall-clock seconds at host time 0. Ren'Py's clock is `time.time()`
    /// based, and its timeout/redraw code treats a zero time as unset.
    epoch: f64,
    out: Output,
    tick: Tick,
    /// Set once a frame was drawn for the current host time.
    pub drew_this_tick: bool,
    shutdown: bool,
    /// Ticks received; host status in tick N reflects output up to N-1.
    pub tick_no: u64,
}

pub type LinkRef = Rc<RefCell<Link>>;

impl Link {
    pub fn new(rx: Receiver<ToScript>, tx: SyncSender<FromScript>, first: Tick) -> Link {
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(1.0e9);
        let epoch = wall - first.now_ms as f64 / 1000.;
        Link {
            rx: Rc::new(rx),
            tx,
            now_ms: first.now_ms,
            text_rect: None,
            epoch,
            out: Output::default(),
            tick: first,
            drew_this_tick: false,
            shutdown: false,
            tick_no: 1,
        }
    }

    /// The link of a VM that replaces this one after an utter restart:
    /// same channels, host time and pending output.
    pub fn renew(&mut self) -> Link {
        Link {
            rx: self.rx.clone(),
            tx: self.tx.clone(),
            now_ms: self.now_ms,
            epoch: self.epoch,
            text_rect: self.text_rect,
            out: std::mem::take(&mut self.out),
            tick: self.tick.clone(),
            drew_this_tick: self.drew_this_tick,
            shutdown: self.shutdown,
            tick_no: self.tick_no,
        }
    }
}

pub fn link(vm: &Vm) -> Option<LinkRef> {
    vm.ext_get::<LinkRef>().map(|l| l.clone())
}

pub fn install(vm: &mut Vm, l: Link) {
    vm.ext_set(Rc::new(RefCell::new(l)));
}

/// Host time in wall-clock seconds (`time.time()` scale), advanced by the
/// host's ticks. Without a link (tools, tests) time stands still.
pub fn now(vm: &Vm) -> f64 {
    link(vm)
        .map(|l| {
            let l = l.borrow();
            l.epoch + l.now_ms as f64 / 1000.
        })
        .unwrap_or(0.)
}

pub fn now_ms(vm: &Vm) -> u64 {
    link(vm).map(|l| l.borrow().now_ms).unwrap_or(0)
}

/// The current tick's number (0 without a host link).
pub fn tick_no(vm: &Vm) -> u64 {
    link(vm).map(|l| l.borrow().tick_no).unwrap_or(0)
}

pub fn audio_status(vm: &Vm) -> Vec<AudioStatus> {
    link(vm)
        .map(|l| l.borrow().tick.audio.clone())
        .unwrap_or_default()
}

pub fn emit_frame(vm: &Vm, ops: Vec<J>) {
    if let Some(l) = link(vm) {
        let mut l = l.borrow_mut();
        l.out.frame = Some(ops);
        l.drew_this_tick = true;
    }
}

pub fn speak(vm: &Vm, text: String) {
    if let Some(l) = link(vm) {
        l.borrow_mut().out.speech.push(text);
    }
}

pub fn emit_audio(vm: &Vm, cmd: J) {
    if let Some(l) = link(vm) {
        l.borrow_mut().out.audio.push(cmd);
    }
}

pub fn set_title(vm: &Vm, title: String) {
    if let Some(l) = link(vm) {
        l.borrow_mut().out.title = Some(title);
    }
}

pub fn set_text_rect(vm: &Vm, rect: Option<[i32; 4]>) {
    if let Some(l) = link(vm) {
        l.borrow_mut().text_rect = rect;
    }
}

pub fn diagnostic(vm: &Vm, msg: String) {
    if let Some(l) = link(vm) {
        let mut l = l.borrow_mut();
        if l.out.diagnostics.len() < 1024 {
            l.out.diagnostics.push(msg);
        }
    }
}

pub fn drew_this_tick(vm: &Vm) -> bool {
    link(vm).is_some_and(|l| l.borrow().drew_this_tick)
}

/// Ends the current tick: hands the output to the owner and blocks until
/// the next tick, whose input is queued as pygame events. Returns the
/// new host time. A shutdown request raises `QuitException`.
pub fn end_tick(vm: &mut Vm, wait: Wait) -> PyResult<f64> {
    let Some(l) = link(vm) else {
        return Err(vm.runtime_error("the Ren'Py script thread has no host link"));
    };
    let msg = {
        let mut l = l.borrow_mut();
        if l.shutdown {
            None
        } else {
            let mut out = std::mem::take(&mut l.out);
            out.text_rect = l.text_rect;
            out.publish = super::im::take_publish();
            if l.tx.send(FromScript::Yield { out, wait }).is_err() {
                l.shutdown = true;
                None
            } else {
                l.rx.recv().ok()
            }
        }
    };
    match msg {
        Some(ToScript::Tick(mut t)) => {
            let input = t.input;
            let text = std::mem::take(&mut t.text);
            // Images still publishing have a redraw pending (`ib_render`).
            super::im::apply_published(&t.published);
            {
                let mut l = l.borrow_mut();
                l.now_ms = l.now_ms.max(t.now_ms);
                l.tick = t;
                l.drew_this_tick = false;
                l.tick_no += 1;
            }
            let scale = super::events::input_scale(vm);
            let mut evs = super::events::from_input(vm, &input, scale)?;
            if !text.is_empty() && super::events::input(vm).borrow().text_input {
                let t = vm.types.new_string(text);
                evs.push(super::events::new_event(vm, super::events::TEXTINPUT, vec![("text", t)])?);
            }
            if std::env::var_os("CRYSIRROR_RENPY_TRACE").is_some() && !evs.is_empty() {
                let r: Vec<String> = evs
                    .iter()
                    .map(|e| crysirror_python::ops::repr(vm, e).unwrap_or_default())
                    .collect();
                diagnostic(vm, format!("trace: events {}", r.join(" ")));
            }
            super::events::input(vm).borrow_mut().queue.extend(evs);
            Ok(now(vm))
        }
        _ => {
            l.borrow_mut().shutdown = true;
            Err(quit(vm))
        }
    }
}

fn quit(vm: &mut Vm) -> PyErr {
    match super::py::import_attr(vm, "renpy.game", "QuitException")
        .and_then(|c| vm.call(&c, Vec::new(), Vec::new()))
    {
        Ok(Value::Obj(o)) => PyErr::Exc(o),
        Ok(_) => vm.runtime_error("QuitException"),
        Err(e) => e,
    }
}

/// Reports the VM's end to the owner (script thread exit path).
pub fn finish(vm: &Vm, result: Result<(), String>) {
    if let Some(l) = link(vm) {
        let mut l = l.borrow_mut();
        let out = std::mem::take(&mut l.out);
        let _ = l.tx.send(FromScript::Exit { out, result });
    }
}
