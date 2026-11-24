// SPDX-License-Identifier: MPL-2.0
//! State-script processes (PPU `game/state-script/ss-*.cpp`) and the
//! script natives that act on them.
//!
//! DC layout: `state-script {name, ?, initial-state, ?, options, state
//! count, states}`; a state is `{name, block count, blocks}`; a block is
//! `{kind, event, 0, track-group}` with kinds 0 `begin`, 1 `end`, 2 event
//! (name in `event`) and 3 `update` (names from `sub_7DEB70`); a track
//! group is `{track count, tracks}`; a track is `{name, lambda count,
//! [(label, script-lambda)]}`. A track runs its lambdas in order; a wait
//! native suspends the lambda mid-way and the track resumes it when the
//! wait completes (`sub_153618` status 1). `go` posts the `go` event to
//! the calling process and abandons the lambda (status 2): the old
//! state's `end` block runs, then the new state's `begin` block.
//!
//! Engine effects go through [`World`]. Natives without an implementation
//! return 0 and are counted in [`Runtime::missing`] so callers see every
//! gap (SATD-nd-retail-runtime).
use crate::dc::{Dc, TYPE_STATE_SCRIPT, sid};
use crate::native_names::NATIVES;
use crate::script::{MAX_ARGS, Natives, Outcome, Status, Thread};
use crate::task::Graph;
use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

pub const KIND_BEGIN: u32 = 0;
pub const KIND_END: u32 = 1;
pub const KIND_EVENT: u32 = 2;
pub const KIND_UPDATE: u32 = 3;
/// `wait-frames` converts frames at 1/30 s (`*(toc-0x7cb4)` = 0.0333).
const FRAME_SECONDS: f32 = 1. / 30.;
const MAX_ERRORS: usize = 256;
/// State changes applied per process and frame.
const MAX_TRANSITIONS: usize = 8;

/// A spawner schema property value.
#[derive(Clone, Debug, PartialEq)]
pub enum Property {
    Int(i32),
    Float(f32),
    Sid(u32),
    Str(String),
}

/// Engine services natives use. Names are `StringId`s.
pub trait World {
    /// Seconds since the level started.
    fn now(&self) -> f64;
    fn print(&mut self, text: &str);
    fn level_spawned(&self, level: u32) -> bool;
    fn object_spawned(&self, name: u32) -> bool;
    fn spawn(&mut self, name: u32) -> bool;
    fn kill(&mut self, name: u32) -> bool;
    /// Spawner position and rotation quaternion (x, y, z, w).
    fn locator(&self, name: u32) -> Option<([f32; 3], [f32; 4])>;
    fn property(&self, spawner: u32, prop: u32) -> Option<Property>;
    /// Fade the screen to `color` at `alpha` (1 opaque) over `seconds`.
    fn fade(&mut self, color: [f32; 3], alpha: f32, seconds: f32);
    fn camera(&mut self, name: u32, seconds: f32);
    fn player_controls(&mut self, enabled: bool);
    /// Left and right stick (x, y in −1…1) of a controller.
    fn sticks(&self, pad: u32) -> [f32; 4];
}

#[derive(Clone, Debug, PartialEq)]
enum Wait {
    None,
    Until(f64),
    Frame(u64),
    Signal(u32),
    /// Until process variable `var` of `process` equals `value`.
    Var { process: usize, var: u32, value: u32 },
}

struct Track {
    lambdas: u32,
    count: u32,
    next: u32,
    thread: Option<Thread>,
    wait: Wait,
}

struct Group {
    kind: u32,
    event: u32,
    args: Vec<u32>,
    tracks: Vec<Track>,
}

pub struct Process {
    pub name: u32,
    pub script: u32,
    /// Spawner whose properties `tag-*` read.
    pub spawner: Option<u32>,
    pub state: Option<u32>,
    pub vars: HashMap<u32, u32>,
    groups: Vec<Group>,
    events: VecDeque<(u32, Vec<u32>)>,
    pending_go: Option<u32>,
    pub dead: bool,
}

/// Runtime state outside the DC memory (so natives can borrow both).
pub struct Inner {
    pub procs: Vec<Process>,
    native_ids: HashMap<u32, u32>,
    signals: HashSet<u32>,
    pub frame: u64,
    pub tasks: Graph,
    /// Current `(node, subnode)`.
    pub task: Option<(usize, usize)>,
    pub missing: BTreeMap<&'static str, u32>,
    pub errors: Vec<String>,
    /// A task change requested by `task-subnode-complete` (current task)
    /// or `task-subnode-play`.
    pub task_change: Option<(usize, usize)>,
    /// Module names requested by `add-load-module`/`add-import-module`.
    pub module_requests: Vec<u32>,
    /// `pad-get-info` records by pad (one 28-byte heap record each).
    pads: HashMap<u32, u32>,
    /// Player movement types from `set-walk-type`/`set-run-type`.
    pub movement: [u32; 2],
    pub prints: Vec<String>,
}

pub struct Runtime {
    pub dc: Dc,
    pub inner: Inner,
}

/// Native call context: the calling process and the wait it requests.
struct Ctx<'a> {
    inner: &'a mut Inner,
    world: &'a mut dyn World,
    proc: Option<usize>,
    wait: Wait,
}

fn state_by_name(dc: &Dc, script: u32, name: u32) -> Result<Option<u32>> {
    let (n, states) = (dc.mem.u32(script + 0x14)?, dc.mem.u32(script + 0x18)?);
    ensure!(n < 4096, "state count");
    for i in 0..n {
        let s = states + 12 * i;
        if name == 0 || dc.mem.u32(s)? == name {
            return Ok(Some(s));
        }
    }
    Ok(None)
}

/// The track group of `state`'s block of `kind` (and `event`).
fn block(dc: &Dc, state: u32, kind: u32, event: u32) -> Result<Option<u32>> {
    let (n, blocks) = (dc.mem.u32(state + 4)?, dc.mem.u32(state + 8)?);
    ensure!(n < 4096, "block count");
    for i in 0..n {
        let b = blocks + 16 * i;
        if dc.mem.u32(b)? == kind && (kind != KIND_EVENT || dc.mem.u32(b + 4)? == event) {
            let g = dc.mem.u32(b + 12)?;
            return Ok((g != 0).then_some(g));
        }
    }
    Ok(None)
}

fn group(dc: &Dc, kind: u32, event: u32, args: Vec<u32>, g: u32) -> Result<Group> {
    let (n, tracks) = (dc.mem.u32(g)?, dc.mem.u32(g + 4)?);
    ensure!(n <= 64, "track count");
    let tracks = (0..n)
        .map(|i| {
            let t = tracks + 12 * i;
            Ok(Track { lambdas: dc.mem.u32(t + 8)?, count: dc.mem.u32(t + 4)?, next: 0, thread: None, wait: Wait::None })
        })
        .collect::<Result<_>>()?;
    Ok(Group { kind, event, args, tracks })
}

impl Runtime {
    pub fn new(dc: Dc) -> Self {
        let native_ids = NATIVES.iter().enumerate().map(|(i, (n, _))| (sid(n), i as u32)).collect();
        Self {
            dc,
            inner: Inner {
                procs: Vec::new(),
                native_ids,
                signals: HashSet::new(),
                frame: 0,
                tasks: Graph::default(),
                task: None,
                missing: BTreeMap::new(),
                errors: Vec::new(),
                task_change: None,
                module_requests: Vec::new(),
                prints: Vec::new(),
                pads: HashMap::new(),
                movement: [0; 2],
            },
        }
    }

    /// Run a top-level lambda (for example a module's `top-level`) to
    /// completion outside any process. A suspension is an error.
    pub fn run_lambda(&mut self, world: &mut dyn World, lambda: u32, args: &[u32]) -> Result<u32> {
        let mut t = Thread::new(&self.dc, lambda, args)?;
        let mut ctx = Ctx { inner: &mut self.inner, world, proc: None, wait: Wait::None };
        match t.run(&mut self.dc, &mut ctx)? {
            Outcome::Returned(v) => Ok(v),
            other => anyhow::bail!("top-level lambda ended with {other:?}"),
        }
    }

    /// Start a process running the state script named `script`.
    pub fn spawn(&mut self, name: u32, script: u32, spawner: Option<u32>) -> Result<usize> {
        let addr = *self.dc.symbols.get(&script).with_context(|| format!("state script {script:#010x} is not loaded"))?;
        ensure!(self.dc.mem.type_of(addr)? == TYPE_STATE_SCRIPT, "{script:#010x} is not a state-script");
        let initial = self.dc.mem.u32(addr + 8)?;
        let state = state_by_name(&self.dc, addr, initial)?.or(state_by_name(&self.dc, addr, 0)?);
        let mut p = Process { name, script: addr, spawner, state: None, vars: HashMap::new(), groups: Vec::new(), events: VecDeque::new(), pending_go: None, dead: false };
        if let Some(s) = state {
            p.state = Some(s);
            if let Some(g) = block(&self.dc, s, KIND_BEGIN, 0)? {
                p.groups.push(group(&self.dc, KIND_BEGIN, 0, Vec::new(), g)?);
            }
        }
        self.inner.procs.push(p);
        Ok(self.inner.procs.len() - 1)
    }

    /// Advance every process by one frame.
    pub fn tick(&mut self, world: &mut dyn World) -> Result<()> {
        self.inner.frame += 1;
        let mut i = 0;
        while i < self.inner.procs.len() {
            self.step_process(world, i)?;
            i += 1;
        }
        Ok(())
    }

    fn step_process(&mut self, world: &mut dyn World, i: usize) -> Result<()> {
        if self.inner.procs[i].dead {
            return Ok(());
        }
        // Events queued for this process start their handler groups.
        while let Some((event, args)) = self.inner.procs[i].events.pop_front() {
            if let Some(state) = self.inner.procs[i].state {
                if let Some(g) = block(&self.dc, state, KIND_EVENT, event)? {
                    let g = group(&self.dc, KIND_EVENT, event, args, g)?;
                    self.inner.procs[i].groups.retain(|x| !(x.kind == KIND_EVENT && x.event == event));
                    self.inner.procs[i].groups.push(g);
                }
            }
        }
        // `update` runs every frame from its first lambda once done.
        if let Some(state) = self.inner.procs[i].state {
            if !self.inner.procs[i].groups.iter().any(|g| g.kind == KIND_UPDATE) {
                if let Some(g) = block(&self.dc, state, KIND_UPDATE, 0)? {
                    let g = group(&self.dc, KIND_UPDATE, 0, Vec::new(), g)?;
                    self.inner.procs[i].groups.push(g);
                }
            }
        }
        // A state change runs the new state's blocks in the same frame;
        // states that keep changing continue on the next frame.
        for _ in 0..MAX_TRANSITIONS {
            self.run_groups(world, i)?;
            let Some(next) = self.inner.procs[i].pending_go.take() else { return Ok(()) };
            self.transition(world, i, next)?;
        }
        Ok(())
    }

    fn run_groups(&mut self, world: &mut dyn World, i: usize) -> Result<()> {
        let now = world.now();
        let mut gi = 0;
        while gi < self.inner.procs[i].groups.len() {
            let mut ti = 0;
            while ti < self.inner.procs[i].groups[gi].tracks.len() {
                if self.inner.procs[i].pending_go.is_some() || self.inner.procs[i].dead {
                    return Ok(());
                }
                self.run_track(world, i, gi, ti, now)?;
                ti += 1;
            }
            let g = &self.inner.procs[i].groups[gi];
            if g.tracks.iter().all(|t| t.next >= t.count && t.thread.is_none()) {
                self.inner.procs[i].groups.remove(gi);
            } else {
                gi += 1;
            }
        }
        Ok(())
    }

    fn wait_done(&self, wait: &Wait, now: f64) -> bool {
        match wait {
            Wait::None => true,
            Wait::Until(t) => now >= *t,
            Wait::Frame(f) => self.inner.frame >= *f,
            Wait::Signal(s) => self.inner.signals.contains(s),
            Wait::Var { process, var, value } => self.inner.procs.get(*process).is_none_or(|p| p.vars.get(var).copied().unwrap_or(0) == *value),
        }
    }

    fn run_track(&mut self, world: &mut dyn World, i: usize, gi: usize, ti: usize, now: f64) -> Result<()> {
        let mut guard = 0;
        loop {
            let t = &self.inner.procs[i].groups[gi].tracks[ti];
            if !self.wait_done(&t.wait, now) {
                return Ok(());
            }
            let mut thread = match self.inner.procs[i].groups[gi].tracks[ti].thread.take() {
                Some(th) => th,
                None => {
                    let t = &self.inner.procs[i].groups[gi].tracks[ti];
                    if t.next >= t.count {
                        return Ok(());
                    }
                    guard += 1;
                    ensure!(guard <= 4096, "track lambda loop");
                    let lambda = self.dc.mem.u32(t.lambdas + 8 * t.next + 4)?;
                    match Thread::new(&self.dc, lambda, &[]) {
                        Ok(th) => th,
                        Err(e) => {
                            // A track entry without a lambda (label only)
                            // has nothing to run.
                            if lambda != 0 {
                                self.inner.error(format!("process {:#010x}: {e:#}", self.inner.procs[i].name));
                            }
                            self.inner.procs[i].groups[gi].tracks[ti].next += 1;
                            continue;
                        }
                    }
                }
            };
            let mut ctx = Ctx { inner: &mut self.inner, world, proc: Some(i), wait: Wait::None };
            let outcome = thread.run(&mut self.dc, &mut ctx);
            let wait = std::mem::replace(&mut ctx.wait, Wait::None);
            let outcome = match outcome {
                Ok(o) => o,
                Err(e) => {
                    self.inner.error(format!("process {:#010x}: {e:#}", self.inner.procs[i].name));
                    Outcome::Aborted
                }
            };
            let Some(track) = self.inner.procs.get_mut(i).and_then(|p| p.groups.get_mut(gi)).and_then(|g| g.tracks.get_mut(ti)) else { return Ok(()) };
            match outcome {
                Outcome::Returned(_) => {
                    track.next += 1;
                    track.wait = Wait::None;
                }
                Outcome::Suspended => {
                    track.thread = Some(thread);
                    track.wait = wait;
                    return Ok(());
                }
                Outcome::Aborted => {
                    track.next = track.count;
                    return Ok(());
                }
            }
        }
    }

    fn transition(&mut self, world: &mut dyn World, i: usize, next: u32) -> Result<()> {
        let script = self.inner.procs[i].script;
        let Some(target) = state_by_name(&self.dc, script, next)? else {
            self.inner.error(format!("process {:#010x}: go to unknown state {next:#010x}", self.inner.procs[i].name));
            return Ok(());
        };
        // The old state's `end` block runs until it waits; then the new
        // state begins.
        if let Some(old) = self.inner.procs[i].state {
            self.inner.procs[i].groups.clear();
            if let Some(g) = block(&self.dc, old, KIND_END, 0)? {
                self.inner.procs[i].groups.push(group(&self.dc, KIND_END, 0, Vec::new(), g)?);
                self.run_groups(world, i)?;
            }
        }
        let p = &mut self.inner.procs[i];
        p.groups.clear();
        p.state = Some(target);
        if let Some(g) = block(&self.dc, target, KIND_BEGIN, 0)? {
            self.inner.procs[i].groups.push(group(&self.dc, KIND_BEGIN, 0, Vec::new(), g)?);
        }
        Ok(())
    }

    /// Make `at` the current task subnode. Completion is native runtime
    /// state in each subnode's +0x1c record: word 0 is 3 once complete
    /// (`task-subnode-complete?` in `task-defines.bin`). Every subnode
    /// ordered before `at` is marked complete, the others not.
    pub fn select_task(&mut self, at: (usize, usize)) -> Result<()> {
        self.inner.task = Some(at);
        for i in 0..self.inner.tasks.nodes.len() {
            for j in 0..self.inner.tasks.nodes[i].subnodes.len() {
                let record = self.dc.mem.u32(self.inner.tasks.nodes[i].subnodes[j].address + 0x1c)?;
                if record != 0 {
                    let done = self.inner.tasks.before((i, j), at);
                    self.dc.mem.write_u32(record, if done { 3 } else { 0 })?;
                }
            }
        }
        Ok(())
    }

    /// Native names called but not implemented, with call counts.
    pub fn missing(&self) -> &BTreeMap<&'static str, u32> {
        &self.inner.missing
    }

    pub fn process_by_name(&self, name: u32) -> Option<usize> {
        self.inner.procs.iter().position(|p| p.name == name && !p.dead)
    }

    /// The current state name of a process.
    pub fn state_name(&self, i: usize) -> Option<u32> {
        self.inner.procs[i].state.and_then(|s| self.dc.mem.u32(s).ok())
    }
}

impl Inner {
    fn error(&mut self, message: String) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.push(message);
        }
    }
    fn process(&self, name: u32) -> Option<usize> {
        self.procs.iter().position(|p| p.name == name && !p.dead)
    }
}

impl Ctx<'_> {
    fn arg_f(args: &[u32; MAX_ARGS], i: usize) -> f32 {
        f32::from_bits(args[i])
    }
    /// The process `args[i]` names (`self` and absent arguments are the
    /// caller).
    fn target(&self, args: &[u32; MAX_ARGS], argc: u32, i: usize) -> Option<usize> {
        if (argc as usize) > i && args[i] != sid("self") {
            self.inner.process(args[i])
        } else {
            self.proc
        }
    }
    fn set_var(&mut self, args: &[u32; MAX_ARGS], argc: u32) -> u32 {
        if argc < 2 {
            return 0;
        }
        match self.target(args, argc, 2) {
            Some(p) => {
                self.inner.procs[p].vars.insert(args[0], args[1]);
                1
            }
            None => 0,
        }
    }
    fn get_var(&mut self, args: &[u32; MAX_ARGS], argc: u32) -> u32 {
        if argc < 1 {
            return 0;
        }
        match self.target(args, argc, 1) {
            Some(p) => self.inner.procs[p].vars.get(&args[0]).copied().unwrap_or(0),
            None => 0,
        }
    }
    fn tag(&mut self, prop: u32) -> Option<Property> {
        let spawner = self.inner.procs.get(self.proc?)?.spawner?;
        self.world.property(spawner, prop)
    }
    fn suspend(&mut self, wait: Wait) -> (u32, Status) {
        if self.proc.is_none() {
            self.inner.error("wait outside a state-script track".into());
            return (0, Status::Continue);
        }
        self.wait = wait;
        (1, Status::Suspend)
    }
    fn send(&mut self, event: u32, target: u32, args: Vec<u32>) -> u32 {
        let Some(p) = self.inner.process(target) else { return 0 };
        if event == sid("go") {
            self.inner.procs[p].pending_go = args.first().copied();
        } else {
            self.inner.procs[p].events.push_back((event, args));
        }
        1
    }
}

impl Natives for Ctx<'_> {
    fn lookup(&self, s: u32) -> Option<u32> {
        self.inner.native_ids.get(&s).copied()
    }
    fn error(&mut self, message: String) {
        self.inner.error(message);
    }
    fn call(&mut self, dc: &mut Dc, id: u32, a: &[u32; MAX_ARGS], argc: u32) -> Result<(u32, Status)> {
        let name = NATIVES.get(id as usize).context("native id")?.0;
        let n = argc as usize;
        let ok = |v: u32| Ok((v, Status::Continue));
        let b = |v: bool| Ok((u32::from(v), Status::Continue));
        let now = self.world.now();
        match name {
            // (add-load-module 'name 'script): the host loads the module
            // after the calling lambda (`sub_1545A8` queues it).
            "add-load-module" | "add-import-module" => {
                if n == 0 || a[0] == 0 {
                    return ok(0);
                }
                self.inner.module_requests.push(a[0]);
                ok(1)
            }
            "go" => {
                let Some(p) = self.proc else { return ok(0) };
                self.inner.procs[p].pending_go = Some(a[0]);
                Ok((1, Status::Abort))
            }
            "wait" => {
                let s = Self::arg_f(a, 0);
                if n == 0 || s <= 0. { ok(0) } else { Ok(self.suspend(Wait::Until(now + f64::from(s)))) }
            }
            "wait-frames" => {
                let s = a[0] as i32 as f32 * FRAME_SECONDS;
                if n == 0 || s <= 0. { ok(0) } else { Ok(self.suspend(Wait::Until(now + f64::from(s)))) }
            }
            "wait-one-frame" => Ok(self.suspend(Wait::Frame(self.inner.frame + 1))),
            "wait-for" => {
                if n == 0 || a[0] == 0 {
                    return ok(0);
                }
                if self.inner.signals.contains(&a[0]) { ok(0) } else { Ok(self.suspend(Wait::Signal(a[0]))) }
            }
            "signal" => {
                if n == 0 || a[0] == 0 {
                    return ok(0);
                }
                self.inner.signals.insert(a[0]);
                ok(1)
            }
            "wait-boolean" | "wait-int32" => {
                let Some(p) = self.target(a, argc, 2) else { return ok(0) };
                if n < 2 || self.inner.procs[p].vars.get(&a[0]).copied().unwrap_or(0) == a[1] {
                    return ok(0);
                }
                Ok(self.suspend(Wait::Var { process: p, var: a[0], value: a[1] }))
            }
            "set-boolean" | "set-int32" | "set-float" | "set-symbol" => ok(self.set_var(a, argc)),
            "get-boolean" | "get-int32" | "get-float" | "get-symbol" => ok(self.get_var(a, argc)),
            "tag-symbol" | "tag-int32" | "tag-boolean" | "tag-float" => {
                let v = match self.tag(a[0]) {
                    Some(Property::Sid(s)) => s,
                    Some(Property::Int(i)) => i as u32,
                    Some(Property::Float(f)) => if name == "tag-float" { f.to_bits() } else { f as i32 as u32 },
                    Some(Property::Str(s)) => sid(&s),
                    None => {
                        self.inner.error(format!("{name}: no tag {:#010x}", a[0]));
                        0
                    }
                };
                ok(if name == "tag-float" && matches!(self.tag(a[0]), Some(Property::Int(_))) { (v as i32 as f32).to_bits() } else { v })
            }
            "tag-specified?" => b(self.tag(a[0]).is_some()),
            "send-event" => {
                if n < 2 {
                    return ok(0);
                }
                let args = a[2..n.min(MAX_ARGS)].to_vec();
                ok(self.send(a[0], a[1], args))
            }
            "event-arg-int32" | "event-arg-float" | "event-arg-symbol" | "event-arg-boolean" => {
                let i = a[0] as usize;
                let v = self.proc.and_then(|p| self.inner.procs[p].groups.iter().rev().find(|g| g.kind == KIND_EVENT)).and_then(|g| g.args.get(i).copied());
                ok(v.unwrap_or(0))
            }
            "spawn-script" => {
                // The process starts with the next frame's tick.
                let Some(&addr) = dc.symbols.get(&a[0]) else {
                    self.inner.error(format!("spawn-script: {:#010x} not loaded", a[0]));
                    return ok(0);
                };
                if dc.mem.type_of(addr)? != TYPE_STATE_SCRIPT {
                    return ok(0);
                }
                let initial = dc.mem.u32(addr + 8)?;
                let state = state_by_name(dc, addr, initial)?.or(state_by_name(dc, addr, 0)?);
                let mut p = Process { name: a[0], script: addr, spawner: None, state, vars: HashMap::new(), groups: Vec::new(), events: VecDeque::new(), pending_go: None, dead: false };
                if let Some(s) = state {
                    if let Some(g) = block(dc, s, KIND_BEGIN, 0)? {
                        p.groups.push(group(dc, KIND_BEGIN, 0, Vec::new(), g)?);
                    }
                }
                self.inner.procs.push(p);
                ok(1)
            }
            "end-script" => {
                if let Some(p) = self.proc {
                    self.inner.procs[p].dead = true;
                }
                Ok((0, Status::Abort))
            }
            "get-script-state" => ok(self.target(a, argc, 0).and_then(|p| self.inner.procs[p].state).map(|s| dc.mem.u32(s)).transpose()?.unwrap_or(0)),
            "get-current-time" => ok((now as f32).to_bits()),
            // (pad-get-info pad) → `{held, pressed, released, left x, y,
            // right x, y}` (`sub_1459E8`). Button words go through
            // `sub_137B28`, whose bit mapping is not recovered: they are 0.
            "pad-get-info" => {
                let pad = if n > 0 { a[0] } else { 0 };
                let at = match self.inner.pads.get(&pad) {
                    Some(at) => *at,
                    None => {
                        let at = dc.mem.alloc(28)?;
                        self.inner.pads.insert(pad, at);
                        at
                    }
                };
                for (k, v) in self.world.sticks(pad).iter().enumerate() {
                    dc.mem.write_u32(at + 12 + 4 * k as u32, v.to_bits())?;
                }
                ok(at)
            }
            // The movement-type table entry (`sub_678348`) is recorded; the
            // player movement does not use it yet.
            "set-walk-type" | "set-run-type" => {
                self.inner.movement[usize::from(name == "set-run-type")] = a[0];
                ok(0)
            }
            "print-string" => {
                let text = if a[0] == 0 { String::new() } else { dc.mem.cstr(a[0]).unwrap_or_default() };
                self.world.print(&text);
                self.inner.prints.push(text);
                ok(0)
            }
            "levels-fully-spawned?" | "levels-fully-spawned-ignore-player?" => b((0..n.max(1)).all(|i| self.world.level_spawned(a[i]))),
            "object-spawned?" => b(n > 0 && (self.world.object_spawned(a[0]) || self.inner.process(a[0]).is_some())),
            "spawn-entity" => b(n > 0 && self.world.spawn(a[0])),
            "kill-entity" => {
                let killed = n > 0 && self.world.kill(a[0]);
                if let Some(p) = self.inner.process(a[0]) {
                    self.inner.procs[p].dead = true;
                }
                b(killed)
            }
            "get-spawner-locator" | "get-spawner-position" => {
                let Some((pos, rot)) = self.world.locator(a[0]) else {
                    self.inner.error(format!("{name}: no spawner {:#010x}", a[0]));
                    return ok(0);
                };
                // A heap `{x, y, z, 1, qx, qy, qz, qw}` record.
                let at = dc.mem.alloc(32)?;
                for (k, v) in pos.iter().chain(&[1.]).chain(&rot).enumerate() {
                    dc.mem.write_u32(at + 4 * k as u32, v.to_bits())?;
                }
                ok(at)
            }
            // (fade-to-black alpha seconds), `sub_87D20(160, alpha, seconds)`.
            "fade-to-black" | "fade-to-white" => {
                let white = name == "fade-to-white";
                self.world.fade(if white { [1.; 3] } else { [0.; 3] }, Self::arg_f(a, 0), if n > 1 { Self::arg_f(a, 1) } else { 0. });
                ok(0)
            }
            // (set-black-fade seconds on?): fade fully in or out.
            "set-black-fade" | "set-white-fade" => {
                let white = name == "set-white-fade";
                let on = n > 1 && a[1] != 0;
                self.world.fade(if white { [1.; 3] } else { [0.; 3] }, if on { 1. } else { 0. }, Self::arg_f(a, 0));
                ok(0)
            }
            "camera-enable" => {
                self.world.camera(a[0], if n > 2 { Self::arg_f(a, 2) } else { 0. });
                ok(0)
            }
            // The argument is a control mask (`player-controls-all` returns
            // 0xffff, `player-controls-none` 0): no mask leaves controls on.
            "player-disable-controls" => {
                self.world.player_controls(n > 0 && a[0] == 0);
                ok(0)
            }
            "task-find" => {
                // (task-find node subnode) → subnode record.
                let found = self.inner.tasks.find(if n > 1 { a[1] } else { a[0] });
                ok(found.map(|(i, j)| self.inner.tasks.nodes[i].subnodes[j].address).unwrap_or(0))
            }
            "task-subnode-complete" | "task-subnode-play" => {
                let Some(at) = self.inner.tasks.nodes.iter().enumerate().find_map(|(i, nd)| nd.subnodes.iter().position(|s| s.address == a[0]).map(|j| (i, j))) else { return b(false) };
                if name == "task-subnode-play" {
                    self.inner.task_change = Some(at);
                    return b(true);
                }
                let record = dc.mem.u32(a[0] + 0x1c)?;
                if record != 0 {
                    dc.mem.write_u32(record, 3)?;
                }
                if self.inner.task == Some(at) {
                    self.inner.task_change = self.inner.tasks.next(at);
                }
                b(true)
            }
            "task-node-active?" => b(self.inner.task.is_some_and(|(i, _)| self.inner.tasks.nodes[i].id == a[0])),
            "task-subnode-active?" => {
                let (Some(current), Some(at)) = (self.inner.task, self.inner.tasks.nodes.iter().enumerate().find_map(|(i, nd)| nd.subnodes.iter().position(|s| s.address == a[0]).map(|j| (i, j)))) else { return b(false) };
                b(at == current)
            }
            _ => {
                *self.inner.missing.entry(name).or_default() += 1;
                ok(0)
            }
        }
    }
}
