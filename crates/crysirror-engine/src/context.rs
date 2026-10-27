// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! The engine context handed to classes, behaviours and script hosts.
use crate::{
    object::{ClassRegistry, ObjectId, ObjectTable},
    value::Value,
};
use anyhow::{Context, Result};
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::Runtime;
use std::collections::VecDeque;

/// Everything the OOP API can reach. Device handles never live here.
pub struct EngineState {
    pub bitmap_jobs: crate::bitmap_async::BitmapJobs,
    pub streaming: crate::systems::StreamingRuntime,
    /// Project/VM scheduler: shared World/Layer/UI, VFS and native VM sessions.
    pub runtime: Runtime,
    /// The project client that engine-level code (Rust behaviours, scripts)
    /// acts as. All world mutations are scoped to it.
    pub client: ClientId,
    pub classes: ClassRegistry,
    pub objects: ObjectTable,
    pub input: InputFrame,
    pub frame: u64,
    pub fixed_step_ms: u64,
    pub log: VecDeque<String>,
    pub quit_requested: bool,
    pub actions: crate::input_api::ActionMap,
    pub input_events: crate::input_events_api::InputEvents,
    pub localization: crate::localization::LocalizationRuntime,
    pub rich_text: crate::text_api::TextRuntime,
    /// Derived AI data: navigation grid, path caches (never saved).
    pub ai: crate::ai::AiRuntime,
    /// Positional voices (not saved; voices restart after restore).
    pub audio: crate::systems::SpatialAudio,
    /// Derived PCM mixer and decoders; native voice declarations remain logical.
    pub mixer_audio: crate::audio_api::AudioRuntime,
    /// Resource formats and client-scoped decode profiles (no game keys saved).
    pub resources: crate::resource_api::ResourceRuntime,
    pub saves: crate::save_api::SaveRuntime,
    /// Arcade vehicles by entity (ADR 0043); not saved yet.
    pub vehicles: std::collections::HashMap<
        (u32, u32),
        (crate::vehicle::VehicleSpec, crate::vehicle::VehicleState),
    >,
}
impl EngineState {
    /// Sync derived physics with the current world and return it.
    pub fn physics(&mut self) -> anyhow::Result<&crate::physics::Physics> {
        self.runtime.physics.set_query_client(self.client.0);
        self.runtime
            .physics
            .sync(&self.runtime.host.engine.world, &mut self.runtime.host.fs)?;
        Ok(&self.runtime.physics)
    }
}
pub const MAX_LOG: usize = 1024;

/// A context is either occupied or vacant. Script hosts temporarily move the
/// occupied state into a thread-local slot while foreign code runs, so native
/// callbacks can reach it without raw pointers; a vacant context errors.
pub struct EngineContext {
    state: Option<Box<EngineState>>,
}
impl EngineContext {
    pub fn new(state: EngineState) -> Self {
        Self {
            state: Some(Box::new(state)),
        }
    }
    pub fn vacant() -> Self {
        Self { state: None }
    }
    pub fn is_vacant(&self) -> bool {
        self.state.is_none()
    }
    pub fn state(&mut self) -> Result<&mut EngineState> {
        self.state
            .as_deref_mut()
            .context("engine context is not available here")
    }
    pub fn state_ref(&self) -> Result<&EngineState> {
        self.state
            .as_deref()
            .context("engine context is not available here")
    }
    pub fn into_state(self) -> Option<EngineState> {
        self.state.map(|b| *b)
    }

    // ---- OOP dispatch: the single path for Rust and script callers ----
    pub fn get(&mut self, this: ObjectId, property: &str) -> Result<Value> {
        let class = self.class_of(this)?;
        class.info().property(property)?;
        class.get(self, this, property)
    }
    pub fn set(&mut self, this: ObjectId, property: &str, value: Value) -> Result<()> {
        let class = self.class_of(this)?;
        let info = class.info().property(property)?;
        anyhow::ensure!(
            info.writable,
            "{}.{property} is read-only",
            class.info().name
        );
        class.set(self, this, property, value)
    }
    pub fn call(&mut self, this: ObjectId, method: &str, args: &[Value]) -> Result<Value> {
        let class = self.class_of(this)?;
        let info = class.info().method(method)?;
        anyhow::ensure!(
            args.len() <= info.params.len(),
            "{}.{method}() takes at most {} argument(s)",
            class.info().name,
            info.params.len()
        );
        class.call(self, this, method, args)
    }
    pub fn singleton(&mut self, class: &'static str) -> Result<ObjectId> {
        let s = self.state()?;
        s.classes.get(class)?;
        s.objects.singleton(class)
    }
    pub fn log(&mut self, line: impl Into<String>) -> Result<()> {
        let s = self.state()?;
        if s.log.len() == MAX_LOG {
            s.log.pop_front();
        }
        s.log.push_back(line.into());
        Ok(())
    }
    fn class_of(&mut self, this: ObjectId) -> Result<std::sync::Arc<dyn crate::object::Class>> {
        let s = self.state()?;
        let name = s.objects.get(this)?.class;
        s.classes.get(name)
    }
}
