// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! Device-free acoustic geometry and propagation. No VM, renderer or output device.
#![forbid(unsafe_code)]
pub mod binaural;
pub mod direct;
pub mod geometry;
pub mod schema;
pub mod reverb;
pub mod spectrum;
pub use direct::{DirectPath, simulate};
pub use geometry::{AcousticScene, SurfaceHit};
pub use schema::*;
