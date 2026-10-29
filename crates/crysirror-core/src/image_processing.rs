// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Device-free immutable image leases and owner-polled processing completion.
use crate::{state::ClientId, Result};
use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};
/// Explicit byte arithmetic for immutable bitmap composition. These profiles
/// operate on stored RGBA, independently of the eventual display alpha policy.
#[derive(Clone, Copy, Debug)]
pub enum ImageCompositeMath {
    Copy,
    /// Encoded RGB d + floor((s-d)*opacity/256), destination alpha zero.
    /// Full opacity copies all four channels exactly.
    OpaqueScalar256,
    /// Encoded RGB interpolation with source alpha (times opacity/256 when
    /// opacity < 255). Writes alpha zero, including transparent source pixels.
    MaskScalar256,
    /// Opaque input over a straight-alpha destination with scalar /256 tables.
    OpaqueOnAlphaScalar256,
    /// Straight-alpha input over straight-alpha destination, scalar /256 tables.
    AlphaOnAlphaScalar256,
}
#[derive(Clone, Debug)]
pub struct ImageCompositeLayer {
    pub input: Box<ImageInput>,
    pub source: [i32; 4],
    pub destination: [i32; 2],
    pub opacity: u8,
    pub math: ImageCompositeMath,
}
/// A CPU resource owned by the engine; no decoder or mutation logic lives in a VM.
pub trait ImageResource: Send + Sync + core::fmt::Debug {
    fn client(&self) -> ClientId;
    fn size(&self) -> [u32; 2];
    fn rgba(&self) -> Arc<[u8]>;
    fn opaque(&self) -> bool;
    /// Read one stored coverage byte. This does not apply display opacity,
    /// filters or a VM's hit policy. Out-of-bounds/malformed resources return None.
    fn alpha_at(&self, point: [u32; 2]) -> Option<u8> {
        let [w,h]=self.size();
        if point[0]>=w || point[1]>=h { return None; }
        let index=u64::from(point[1]).checked_mul(u64::from(w))?
            .checked_add(u64::from(point[0]))?.checked_mul(4)?.checked_add(3)?;
        self.rgba().get(usize::try_from(index).ok()?).copied()
    }
}
#[derive(Clone, Debug)]
pub enum ImageInput {
    /// Immutable ordered bitmap operation graph, evaluated by shared CPU workers.
    /// Base and descendants retain original channel bytes; no screen is captured.
    Composite {
        base: Box<ImageInput>,
        layers: Vec<ImageCompositeLayer>,
    },
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
