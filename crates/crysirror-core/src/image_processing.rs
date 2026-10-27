// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Device-free immutable image leases and owner-polled processing completion.
use crate::{state::ClientId, Result};
use alloc::{boxed::Box, string::String, sync::Arc};
/// A CPU resource owned by the engine; no decoder or mutation logic lives in a VM.
pub trait ImageResource: Send + Sync + core::fmt::Debug {
    fn client(&self) -> ClientId;
    fn size(&self) -> [u32; 2];
    fn rgba(&self) -> Arc<[u8]>;
    fn opaque(&self) -> bool;
}
#[derive(Clone, Debug)]
pub enum ImageInput {
    Solid {
        size: [u32; 2],
        color: [u8; 4],
    },
    Raster {
        destination: Box<ImageInput>,
        source: Box<ImageInput>,
    },
    Transition {
        from: Arc<dyn ImageResource>,
        to: Arc<dyn ImageResource>,
        mask: Option<Arc<dyn ImageResource>>,
    },
    /// Expected dimensions bound admission; the worker verifies the decoded size.
    Asset {
        path: String,
        size: [u32; 2],
        fingerprint: Option<[u8; 32]>,
    },
    Resource(Arc<dyn ImageResource>),
}
#[derive(Clone, Debug)]
pub enum ImageProcessState {
    Pending,
    Ready(Arc<dyn ImageResource>),
    Failed(String),
}
/// Poll never joins a worker. Ready/error are retained; dropping requests cancel.
pub trait ImageProcess: Send {
    fn poll(&mut self) -> Result<ImageProcessState>;
}
pub type ImageJob = Box<dyn ImageProcess>;
