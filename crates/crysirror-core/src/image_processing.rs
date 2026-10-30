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
    /// Copy the colour channels, keep the destination alpha.
    CopyRgb,
    /// Encoded RGB d + floor((s-d)*opacity/256), destination alpha zero.
    /// Full opacity copies all four channels exactly.
    OpaqueScalar256,
    /// Encoded RGB interpolation with source alpha (times opacity/256 when
    /// opacity < 255). Writes alpha zero, including transparent source pixels.
    MaskScalar256,
    /// MaskScalar256's RGB interpolation, keeping the destination alpha
    /// (krkr `_HDA` hold-destination-alpha blend).
    MaskHoldAlphaScalar256,
    /// Opaque input over a straight-alpha destination with scalar /256 tables.
    OpaqueOnAlphaScalar256,
    /// Straight-alpha input over straight-alpha destination, scalar /256 tables.
    AlphaOnAlphaScalar256,
    /// Photoshop-like colour blend (krkr `tvpps.c` tables); `hold_alpha`
    /// keeps the destination alpha, otherwise alpha is written as zero.
    Ps { blend: PsBlend, hold_alpha: bool },
}
/// Photoshop-like colour operations with exact byte-table forms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsBlend {
    Normal,
    Overlay,
    HardLight,
    SoftLight,
    ColorDodge,
    ColorBurn,
    Lighten,
    Darken,
}
#[derive(Clone, Debug)]
pub struct ImageCompositeLayer {
    pub input: Box<ImageInput>,
    pub source: [i32; 4],
    pub destination: [i32; 2],
    /// Destination-space rectangle limiting this operation. Clipping retains
    /// the original source-to-destination mapping; `None` uses the full image.
    pub clip: Option<[i32; 4]>,
    pub opacity: u8,
    pub math: ImageCompositeMath,
}
/// A CPU resource owned by the engine; no decoder or mutation logic lives in a VM.
pub trait ImageResource: Send + Sync + core::fmt::Debug {
    fn client(&self) -> ClientId;
    fn size(&self) -> [u32; 2];
    fn rgba(&self) -> Arc<[u8]>;
    fn opaque(&self) -> bool;
    /// Extract immutable row-major coverage for a checked [x,y,width,height]
    /// source region. This copies only alpha once, without applying display
    /// filters or caller-specific hit policy. Work is bounded to 16 Mi pixels.
    fn alpha_region(&self, rect: [u32; 4]) -> Result<Arc<[u8]>> {
        let [x,y,w,h]=rect;
        let [iw,ih]=self.size();
        if w==0 || h==0 || x.checked_add(w).is_none_or(|v|v>iw)
            || y.checked_add(h).is_none_or(|v|v>ih) {
            return Err(crate::Error::Invalid("image alpha region bounds"));
        }
        let count=u64::from(w)*u64::from(h);
        if count>16*1024*1024 {return Err(crate::Error::Limit("image alpha region"));}
        let rgba=self.rgba();
        let expected=u64::from(iw).checked_mul(u64::from(ih)).and_then(|v|v.checked_mul(4));
        if expected!=Some(rgba.len() as u64){return Err(crate::Error::Invalid("image alpha resource layout"));}
        let mut coverage=Vec::with_capacity(count as usize);
        for row in y..y+h {
            let start=((u64::from(row)*u64::from(iw)+u64::from(x))*4) as usize;
            for pixel in rgba[start..start+w as usize*4].chunks_exact(4){coverage.push(pixel[3]);}
        }
        Ok(coverage.into())
    }
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
