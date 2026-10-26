// SPDX-License-Identifier: MPL-2.0
//! Device-independent, bounded VFX authoring, compilation and reference simulation.
//! No UE implementation, renderer, window, mixer or executable code is embedded.
#![forbid(unsafe_code)]
mod schema;
mod simulation;
pub use schema::*;
pub use simulation::*;
