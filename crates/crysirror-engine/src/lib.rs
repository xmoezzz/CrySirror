// SPDX-License-Identifier: MPL-2.0
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
pub mod ai;
pub mod api_doc;
pub mod assets_api;
pub mod audio_api;
pub mod bitmap_api;
pub mod builtin;
pub mod canvas_api;
pub mod context;
pub mod input_api;
pub mod lifecycle;
pub mod localization;
pub mod object;
pub mod physics;
pub mod physics_api;
pub mod profiler_api;
pub mod puppet_api;
pub mod resource_api;
pub mod script;
pub mod services;
pub mod systems;
pub mod text_api;
pub mod value;
pub mod vehicle;
pub mod vfx_api;

pub use context::{EngineContext, EngineState};
pub use lifecycle::{App, AppState, Lifecycle, Plugin};
pub use object::{Class, ClassInfo, MethodInfo, ObjectId, PropertyInfo, TypeHint};
pub use script::ScriptHost;
pub use value::Value;
