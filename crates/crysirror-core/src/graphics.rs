// SPDX-License-Identifier: MPL-2.0
//! Backend-independent capability selection; no device pointers or SDK types.
use crate::{Error, Result};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderingPath {
    #[default]
    Raster,
    ComputeRaytrace,
    HardwareRaytrace,
}
impl RenderingPath {
    pub fn name(self) -> &'static str {
        match self {
            Self::Raster => "raster",
            Self::ComputeRaytrace => "compute-raytrace",
            Self::HardwareRaytrace => "hardware-raytrace",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicsCapabilities {
    pub raster: bool,
    pub compute_raytrace: bool,
    pub hardware_raytrace: bool,
}
impl GraphicsCapabilities {
    pub fn require(self, path: RenderingPath) -> Result<()> {
        let supported = match path {
            RenderingPath::Raster => self.raster,
            RenderingPath::ComputeRaytrace => self.compute_raytrace,
            RenderingPath::HardwareRaytrace => self.hardware_raytrace,
        };
        if supported {
            Ok(())
        } else {
            Err(Error::Unsupported(path.name()))
        }
    }
}
/// Temporal/spatial upscalers (ADR 0007). Vendor SDKs (NVIDIA DLSS, AMD FSR,
/// Intel XeSS, Apple MetalFX) are integrated only by optional backend plugins
/// with their own licences; CrySirror never distributes those SDKs. A required
/// upscaler that is not implemented on the active device is an error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Upscaler {
    #[default]
    Native,
    Dlss,
    Fsr,
    Xess,
    MetalFx,
}
impl Upscaler {
    pub fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Dlss => "dlss",
            Self::Fsr => "fsr",
            Self::Xess => "xess",
            Self::MetalFx => "metalfx",
        }
    }
    /// Inputs a temporal upscaler needs from the renderer; used to reject
    /// selection before frame-graph work when the renderer cannot supply them.
    pub fn requires_motion_vectors(self) -> bool {
        !matches!(self, Self::Native)
    }
}
/// Upscalers actually implemented by the active backend (bitset by variant).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UpscalerSupport {
    pub dlss: bool,
    pub fsr: bool,
    pub xess: bool,
    pub metalfx: bool,
}
impl UpscalerSupport {
    pub fn require(self, u: Upscaler) -> Result<()> {
        let ok = match u {
            Upscaler::Native => true,
            Upscaler::Dlss => self.dlss,
            Upscaler::Fsr => self.fsr,
            Upscaler::Xess => self.xess,
            Upscaler::MetalFx => self.metalfx,
        };
        if ok {
            Ok(())
        } else {
            Err(Error::Unsupported(u.name()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unimplemented_upscalers_fail_instead_of_falling_back() {
        let none = UpscalerSupport::default();
        assert!(none.require(Upscaler::Native).is_ok());
        assert_eq!(
            none.require(Upscaler::Dlss),
            Err(Error::Unsupported("dlss"))
        );
        assert_eq!(
            none.require(Upscaler::MetalFx),
            Err(Error::Unsupported("metalfx"))
        );
        assert!(Upscaler::Dlss.requires_motion_vectors());
    }
    #[test]
    fn required_raytracing_never_silently_falls_back() {
        let caps = GraphicsCapabilities {
            raster: true,
            compute_raytrace: true,
            hardware_raytrace: false,
        };
        assert!(caps.require(RenderingPath::ComputeRaytrace).is_ok());
        assert_eq!(
            caps.require(RenderingPath::HardwareRaytrace),
            Err(Error::Unsupported("hardware-raytrace"))
        );
    }
}
