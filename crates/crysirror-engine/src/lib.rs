// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! CrySirror's general-purpose engine API (ADR 0004, ADR 0005).
//!
//! * [`object`]: reflective classes, objects and values — the OOP API surface
//!   that Rust code, the JS/TS host and inspection tools all use.
//! * [`lifecycle`]: [`App`], [`Lifecycle`] behaviours and [`Plugin`]s.
//! * [`script`]: the [`ScriptHost`] contract for scripting languages.
//!
//! The engine is not script-centric: a game can be pure Rust behaviours. It owns
//! no window, GPU or audio device; hosts (desktop, capture, tests) drive
//! [`App::frame`] and render the extracted runtime state.
#![forbid(unsafe_code)]
pub mod acoustics_api;
pub mod ai;
pub mod api_doc;
pub mod assets_api;
pub mod async_api;
pub mod audio_api;
mod audio_async;
mod audio_motion;
mod audio_groups_api;
mod audio_resources_api;
mod audio_sequences_api;
pub use audio_sequences_api::CueSequenceCursor;
pub use audio_sequences_api::{CueSequenceSpec, CueSequenceLine, CueSequenceMode, CueSequenceClock, CueSequenceTiming, CueSequenceRandom};
pub use audio_resources_api::{AudioSourcePrepareSpec, AudioTransformSpec};
pub mod binaural_api;
pub mod bitmap_api;
pub mod builtin;
pub mod canvas_api;
pub mod context;
pub mod input_api;
pub mod input_events_api;
pub mod lifecycle;
pub mod localization;
pub mod object;
pub mod physics;
pub mod physics_api;
pub mod profiler_api;
pub mod puppet_api;
mod reflection_tracking;
pub mod reflections_api;
pub mod resource_api;
mod resource_async;
pub mod reverb_api;
pub mod save_api;
pub mod script;
pub mod services;
pub mod systems;
pub mod text_api;
mod text_async;
pub mod value;
pub mod vehicle;
pub mod vfx_api;

pub use context::{EngineContext, EngineState};
pub use lifecycle::{App, AppState, Lifecycle, Plugin};
pub use object::{Class, ClassInfo, MethodInfo, ObjectId, PropertyInfo, TypeHint};
pub use script::ScriptHost;
pub use value::Value;

pub use acoustics_api::AcousticMeshBindingSpec;
