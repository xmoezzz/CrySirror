// SPDX-License-Identifier: MPL-2.0
//! Application lifecycle, behaviours and plugins (ADR 0004).
//!
//! ```text
//! Created --init--> Initialized --start--> Running <--pause/resume--> Paused
//!    any non-terminal state --shutdown--> Stopped
//!    any error inside a hook --> Faulted (terminal; reported, never ignored)
//! ```
//!
//! One `frame(dt)` while Running executes, in order:
//! 1. per whole fixed step: built-in AI (behaviour trees), then
//!    `on_fixed_update(step)`,
//! 2. `on_update(dt)`,
//! 3. the runtime tick: retained UI input, World fixed-step simulation and
//!    every VM session (data VM, Siglus SceneVm, ...) with their own waits,
//! 4. `on_late_update(dt)`, after VM/world results are visible.
//!
//! Behaviours run in registration order; shutdown runs in reverse order.
use crate::{
    builtin,
    context::{EngineContext, EngineState},
    object::{Class, ClassRegistry, ObjectTable},
    script::ScriptHost,
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::service::InputFrame;
use crysirror_runtime::Runtime;
use std::{collections::VecDeque, sync::Arc};

/// A unit of engine-driven behaviour. Every hook has a no-op default, so a
/// behaviour implements only what it needs (like a Unity MonoBehaviour, but
/// owned by the App rather than attached to one entity).
#[allow(unused_variables)]
pub trait Lifecycle {
    fn name(&self) -> &str;
    fn on_init(&mut self, cx: &mut EngineContext) -> Result<()> {
        Ok(())
    }
    fn on_start(&mut self, cx: &mut EngineContext) -> Result<()> {
        Ok(())
    }
    fn on_fixed_update(&mut self, cx: &mut EngineContext, step_ms: u64) -> Result<()> {
        Ok(())
    }
    fn on_update(&mut self, cx: &mut EngineContext, dt_ms: u64) -> Result<()> {
        Ok(())
    }
    fn on_late_update(&mut self, cx: &mut EngineContext, dt_ms: u64) -> Result<()> {
        Ok(())
    }
    fn on_pause(&mut self, cx: &mut EngineContext) -> Result<()> {
        Ok(())
    }
    fn on_resume(&mut self, cx: &mut EngineContext) -> Result<()> {
        Ok(())
    }
    fn on_shutdown(&mut self, cx: &mut EngineContext) -> Result<()> {
        Ok(())
    }
}

/// Packages classes, behaviours, script hosts and VFS mounts. Plugins are
/// applied before `init`; a plugin name may be added once.
pub trait Plugin {
    fn name(&self) -> &'static str;
    fn build(&self, app: &mut App) -> Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppState {
    Created,
    Initialized,
    Running,
    Paused,
    Stopped,
    Faulted,
}

pub const DEFAULT_FIXED_STEP_MS: u64 = 16;
/// Upper bound of fixed steps per frame; longer frames drop the excess and
/// account it in [`App::dropped_ms`] instead of spiralling.
pub const MAX_FIXED_STEPS_PER_FRAME: u64 = 8;

pub struct App {
    cx: EngineContext,
    behaviours: Vec<Box<dyn Lifecycle>>,
    plugins: Vec<&'static str>,
    state: AppState,
    accumulator_ms: u64,
    dropped_ms: u64,
    fault: Option<String>,
}
impl App {
    /// `client` names the project client that engine code acts as; it must
    /// exist in the project (typically a `crysirror-script-v1` client).
    pub fn new(runtime: Runtime, client: &str) -> Result<Self> {
        let client = runtime.client_id(client)?;
        let mut classes = ClassRegistry::default();
        builtin::register(&mut classes)?;
        Ok(Self {
            cx: EngineContext::new(EngineState {
                runtime,
                client,
                classes,
                objects: ObjectTable::default(),
                input: InputFrame::default(),
                frame: 0,
                fixed_step_ms: DEFAULT_FIXED_STEP_MS,
                log: VecDeque::new(),
                quit_requested: false,
                actions: Default::default(),
                localization: Default::default(),
                rich_text: Default::default(),
                ai: Default::default(),
                audio: Default::default(),
                mixer_audio: Default::default(),
                resources: Default::default(),
                vehicles: Default::default(),
            }),
            behaviours: Vec::new(),
            plugins: Vec::new(),
            state: AppState::Created,
            accumulator_ms: 0,
            dropped_ms: 0,
            fault: None,
        })
    }
    pub fn state(&self) -> AppState {
        self.state
    }
    pub fn fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }
    pub fn dropped_ms(&self) -> u64 {
        self.dropped_ms
    }
    pub fn context(&mut self) -> &mut EngineContext {
        &mut self.cx
    }
    pub fn engine(&self) -> Result<&EngineState> {
        self.cx.state_ref()
    }
    /// Shut down (if needed) and hand the runtime back to the host, e.g. for
    /// rendering or snapshotting after scripted frames.
    pub fn into_runtime(mut self) -> Result<Runtime> {
        self.shutdown()?;
        Ok(self
            .cx
            .into_state()
            .context("engine context vacant")?
            .runtime)
    }
    pub fn runtime_mut(&mut self) -> Result<&mut Runtime> {
        Ok(&mut self.cx.state()?.runtime)
    }
    pub fn runtime(&self) -> Result<&Runtime> {
        Ok(&self.cx.state_ref()?.runtime)
    }
    pub fn quit_requested(&self) -> bool {
        self.cx
            .state_ref()
            .map(|s| s.quit_requested)
            .unwrap_or(true)
    }
    fn configurable(&self) -> Result<()> {
        ensure!(
            matches!(self.state, AppState::Created),
            "plugins/classes/behaviours must be added before init (state {:?})",
            self.state
        );
        Ok(())
    }
    pub fn add_plugin(&mut self, plugin: &dyn Plugin) -> Result<&mut Self> {
        self.configurable()?;
        ensure!(
            !self.plugins.contains(&plugin.name()),
            "plugin '{}' added twice",
            plugin.name()
        );
        self.plugins.push(plugin.name());
        plugin
            .build(self)
            .with_context(|| format!("plugin '{}'", plugin.name()))?;
        Ok(self)
    }
    pub fn register_class(&mut self, class: Arc<dyn Class>) -> Result<&mut Self> {
        self.configurable()?;
        self.cx.state()?.classes.register(class)?;
        Ok(self)
    }
    pub fn add_behaviour(&mut self, behaviour: Box<dyn Lifecycle>) -> Result<&mut Self> {
        self.configurable()?;
        ensure!(self.behaviours.len() < 1024, "behaviour limit");
        self.behaviours.push(behaviour);
        Ok(self)
    }
    pub fn add_script_host(&mut self, host: Box<dyn ScriptHost>) -> Result<&mut Self> {
        self.add_behaviour(host)
    }
    pub fn set_fixed_step_ms(&mut self, step: u64) -> Result<()> {
        self.configurable()?;
        ensure!((1..=1000).contains(&step), "fixed step must be 1..=1000 ms");
        self.cx.state()?.fixed_step_ms = step;
        Ok(())
    }
    pub fn typescript_declarations(&self) -> Result<String> {
        let s = self.cx.state_ref()?;
        Ok(s.classes.typescript_declarations(builtin::GLOBALS))
    }

    fn each(
        &mut self,
        hook: &str,
        reverse: bool,
        mut f: impl FnMut(&mut dyn Lifecycle, &mut EngineContext) -> Result<()>,
    ) -> Result<()> {
        let order: Vec<usize> = if reverse {
            (0..self.behaviours.len()).rev().collect()
        } else {
            (0..self.behaviours.len()).collect()
        };
        for i in order {
            let b = &mut self.behaviours[i];
            let name = b.name().to_owned();
            let result = f(b.as_mut(), &mut self.cx);
            if self.cx.is_vacant() {
                bail!("behaviour '{name}' did not return the engine context from {hook}");
            }
            result.with_context(|| format!("behaviour '{name}' {hook}"))?;
        }
        Ok(())
    }
    fn guard(&mut self, r: Result<()>) -> Result<()> {
        if let Err(e) = &r {
            self.state = AppState::Faulted;
            self.fault = Some(format!("{e:#}"));
        }
        r
    }
    fn expect(&self, allowed: &[AppState], op: &str) -> Result<()> {
        ensure!(
            allowed.contains(&self.state),
            "{op} is invalid in state {:?}{}",
            self.state,
            self.fault
                .as_deref()
                .map(|f| format!(" ({f})"))
                .unwrap_or_default()
        );
        Ok(())
    }

    pub fn init(&mut self) -> Result<()> {
        self.expect(&[AppState::Created], "init")?;
        let r = self.each("on_init", false, |b, cx| b.on_init(cx));
        self.guard(r)?;
        self.state = AppState::Initialized;
        Ok(())
    }
    pub fn start(&mut self) -> Result<()> {
        self.expect(&[AppState::Initialized], "start")?;
        let r = self.each("on_start", false, |b, cx| b.on_start(cx));
        self.guard(r)?;
        self.state = AppState::Running;
        Ok(())
    }
    /// Advance one host frame. Paused apps do not advance engine time.
    pub fn frame(&mut self, dt_ms: u64, input: InputFrame) -> Result<()> {
        self.expect(&[AppState::Running, AppState::Paused], "frame")?;
        if self.state == AppState::Paused {
            return Ok(());
        }
        let r = self.run_frame(dt_ms, input);
        self.guard(r)
    }
    fn run_frame(&mut self, dt_ms: u64, input: InputFrame) -> Result<()> {
        ensure!(dt_ms <= 60_000, "frame delta exceeds 60 s");
        let step = {
            let s = self.cx.state()?;
            s.input = input;
            s.actions.begin_frame(input);
            s.fixed_step_ms
        };
        self.accumulator_ms += dt_ms;
        let mut steps = self.accumulator_ms / step;
        self.accumulator_ms %= step;
        if steps > MAX_FIXED_STEPS_PER_FRAME {
            self.dropped_ms += (steps - MAX_FIXED_STEPS_PER_FRAME) * step;
            steps = MAX_FIXED_STEPS_PER_FRAME;
        }
        for _ in 0..steps {
            // Built-in systems run before behaviours each fixed step.
            crate::ai::step(&mut self.cx, step as f32 / 1000.).context("AI step")?;
            self.each("on_fixed_update", false, |b, cx| {
                b.on_fixed_update(cx, step)
            })?;
        }
        self.each("on_update", false, |b, cx| b.on_update(cx, dt_ms))?;
        crate::systems::streaming(&mut self.cx).context("level streaming")?;
        crate::systems::spatial_audio(&mut self.cx).context("spatial audio")?;
        {
            let s = self.cx.state()?;
            s.runtime.tick(dt_ms, input).context("runtime tick")?;
        }
        self.each("on_late_update", false, |b, cx| b.on_late_update(cx, dt_ms))?;
        crate::text_api::update(&mut self.cx, dt_ms).context("rich text reveal")?;
        crate::audio_api::update(&mut self.cx, dt_ms).context("audio mixer")?;
        let s = self.cx.state()?;
        s.frame = s.frame.checked_add(1).context("frame counter overflow")?;
        s.actions.end_frame(input);
        Ok(())
    }
    pub fn pause(&mut self) -> Result<()> {
        self.expect(&[AppState::Running], "pause")?;
        let r = self.each("on_pause", false, |b, cx| b.on_pause(cx));
        self.guard(r)?;
        self.state = AppState::Paused;
        Ok(())
    }
    pub fn resume(&mut self) -> Result<()> {
        self.expect(&[AppState::Paused], "resume")?;
        let r = self.each("on_resume", false, |b, cx| b.on_resume(cx));
        self.guard(r)?;
        self.state = AppState::Running;
        Ok(())
    }
    /// Run shutdown hooks in reverse order. Every hook runs even if an earlier
    /// one fails; the first error is returned and the app is still Stopped.
    pub fn shutdown(&mut self) -> Result<()> {
        if matches!(self.state, AppState::Stopped) {
            return Ok(());
        }
        let mut first: Option<anyhow::Error> = None;
        for i in (0..self.behaviours.len()).rev() {
            let b = &mut self.behaviours[i];
            let name = b.name().to_owned();
            if self.cx.is_vacant() {
                break;
            }
            if let Err(e) = b.on_shutdown(&mut self.cx) {
                first.get_or_insert(e.context(format!("behaviour '{name}' on_shutdown")));
            }
        }
        self.state = AppState::Stopped;
        match first {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}
