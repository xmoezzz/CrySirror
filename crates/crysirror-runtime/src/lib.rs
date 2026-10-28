// SPDX-License-Identifier: MPL-2.0
//! Shared desktop runtime. Native engine adapters are registered explicitly.
#![forbid(unsafe_code)]
pub mod assets;
pub mod csv;
pub mod frame;
pub mod media;
pub mod model;
pub mod physics;
pub mod runtime;
pub mod script;
pub mod structured_text;
pub mod vfs;
pub mod vfx;
pub mod vector;
pub use model::Project;
pub use runtime::{AdapterRegistry, Runtime};

mod image_processing;

pub mod text_storage;
