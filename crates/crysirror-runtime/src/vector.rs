// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Device-free vector authoring/bounds facade. Raster work is submitted through
//! ImageMetadata's shared image worker, never composed privately by a VM.
pub use crysirror_image::vector::{PathCommand, VectorPath};
pub use crysirror_image::vector_geometry::*;
