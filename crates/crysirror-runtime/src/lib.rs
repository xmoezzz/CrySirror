// SPDX-License-Identifier: MPL-2.0
//! Shared desktop runtime. Native engine adapters are registered explicitly.
#![forbid(unsafe_code)]
pub mod assets;
pub mod model;
pub mod physics;
pub mod runtime;
pub mod script;
pub mod vfs;
pub use model::Project;
pub use runtime::{AdapterRegistry, Runtime};
