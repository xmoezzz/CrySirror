// SPDX-License-Identifier: MPL-2.0
//! Persistent CPU rigid-body simulation. No device, window or scene dependency.
#![forbid(unsafe_code)]
mod joints;
mod queries;
mod shapes;
mod types;
mod world;
pub use types::*;
pub use world::PhysicsWorld;

pub use shapes::decompose_transform;
