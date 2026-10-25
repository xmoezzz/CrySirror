// SPDX-License-Identifier: MPL-2.0
//! Engine-owned, device-free material data. `String` textures are client asset
//! paths; importers use image indices with the same shading/alpha contract.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    /// Alpha ignored.
    #[default]
    Opaque,
    /// Cutout at `alpha_cutoff`, drawn opaque.
    Mask,
    /// Sorted alpha blending.
    Blend,
    /// Sorted additive blending.
    Additive,
    /// Sorted premultiplied-alpha blending.
    Premultiplied,
}
/// How a tangent-space normal texture stores its vector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormalEncoding {
    /// XYZ in RGB (glTF).
    #[default]
    Rgb,
    /// XY in RG, Z reconstructed (BC5 / two-channel maps).
    Rg,
    /// X in alpha, Y in green, Z reconstructed (DXT5nm).
    Ag,
}
impl NormalEncoding {
    pub fn code(self) -> f32 {
        match self {
            Self::Rgb => 0.,
            Self::Rg => 1.,
            Self::Ag => 2.,
        }
    }
}
impl AlphaMode {
    pub fn transparent(self) -> bool {
        matches!(self, Self::Blend | Self::Additive | Self::Premultiplied)
    }
    pub fn code(self) -> f32 {
        match self {
            Self::Opaque => 0.,
            Self::Mask => 1.,
            Self::Blend => 2.,
            Self::Additive => 3.,
            Self::Premultiplied => 4.,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Material<T = String> {
    /// Linear RGBA multiplier of the base color.
    pub base_color_factor: [f32; 4],
    /// Metalness 0..1.
    pub metallic_factor: f32,
    /// Perceptual roughness 0..1.
    pub roughness_factor: f32,
    /// Linear emissive color.
    pub emissive_factor: [f32; 3],
    /// Emissive multiplier (KHR_materials_emissive_strength).
    pub emissive_strength: f32,
    /// Tangent-space normal XY scale.
    pub normal_scale: f32,
    /// How the normal texture stores its vector.
    pub normal_encoding: NormalEncoding,
    /// Ambient occlusion texture strength 0..1.
    pub occlusion_strength: f32,
    /// Dielectric specular strength (KHR_materials_specular `specularFactor`).
    pub specular_factor: f32,
    /// Opaque, mask (cutout) or one of the transparent blend modes.
    pub alpha_mode: AlphaMode,
    /// Alpha threshold for `mask`.
    pub alpha_cutoff: f32,
    /// Disable back-face culling.
    pub double_sided: bool,
    /// Skip lighting; output base color.
    pub unlit: bool,
    /// sRGB base color (+ alpha) texture asset.
    pub base_color_texture: Option<T>,
    /// glTF packed texture: roughness in G, metalness in B.
    pub metallic_roughness_texture: Option<T>,
    /// Tangent-space normal texture asset.
    pub normal_texture: Option<T>,
    /// Ambient occlusion texture (R).
    pub occlusion_texture: Option<T>,
    /// sRGB emissive texture asset.
    pub emissive_texture: Option<T>,
    /// Optional independent opacity texture, multiplied into base-color alpha.
    pub opacity_texture: Option<T>,
    /// Select alpha rather than red from the independent opacity texture.
    pub opacity_from_alpha: bool,
    /// Linear-light specular color multiplier of dielectric F0, sRGB encoded
    /// (KHR_materials_specular `specularColorTexture`); grayscale maps work.
    pub specular_texture: Option<T>,
    /// Shared UV0 scale/offset, before all texture lookups (not a shader program).
    pub uv_scale: [f32; 2],
    /// UV0 offset after `uv_scale`.
    pub uv_offset: [f32; 2],
    /// Soft intersection distance in world units against opaque scene depth.
    pub depth_fade: f32,
    /// Built-in shader effect or custom WGSL surface hook (ADR 0016).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<crate::effect::ShaderEffect>,
}
impl<T> Default for Material<T> {
    fn default() -> Self {
        Self {
            base_color_factor: [1.; 4],
            metallic_factor: 0.,
            roughness_factor: 0.6,
            emissive_factor: [0.; 3],
            emissive_strength: 1.,
            normal_scale: 1.,
            normal_encoding: NormalEncoding::Rgb,
            occlusion_strength: 1.,
            specular_factor: 1.,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            unlit: false,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            opacity_texture: None,
            opacity_from_alpha: false,
            specular_texture: None,
            uv_scale: [1.; 2],
            uv_offset: [0.; 2],
            depth_fade: 0.,
            effect: None,
        }
    }
}
impl<T> Material<T> {
    pub fn textures(&self) -> [Option<&T>; 7] {
        [
            self.base_color_texture.as_ref(),
            self.metallic_roughness_texture.as_ref(),
            self.normal_texture.as_ref(),
            self.occlusion_texture.as_ref(),
            self.emissive_texture.as_ref(),
            self.opacity_texture.as_ref(),
            self.specular_texture.as_ref(),
        ]
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(e) = &self.effect {
            e.validate()?;
            match e {
                crate::effect::ShaderEffect::Fog { .. } => ensure!(
                    self.alpha_mode.transparent(),
                    "fog requires a transparent alpha mode"
                ),
                crate::effect::ShaderEffect::Dissolve { .. } => ensure!(
                    self.alpha_mode != AlphaMode::Opaque,
                    "dissolve requires mask or transparency"
                ),
                _ => {}
            }
        }
        ensure!(
            self.base_color_factor
                .iter()
                .all(|x| x.is_finite() && (0.0..=1.).contains(x)),
            "material base color must be finite 0..1"
        );
        ensure!(
            self.emissive_factor
                .iter()
                .all(|x| x.is_finite() && (0.0..=65504.).contains(x))
                && self.emissive_strength.is_finite()
                && (0.0..=65504.).contains(&self.emissive_strength),
            "material emissive range"
        );
        ensure!(
            [
                self.metallic_factor,
                self.roughness_factor,
                self.occlusion_strength,
                self.alpha_cutoff,
                self.specular_factor
            ]
            .iter()
            .all(|x| x.is_finite() && (0.0..=1.).contains(x)),
            "material scalar must be finite 0..1"
        );
        ensure!(
            self.normal_scale.is_finite() && self.normal_scale.abs() <= 100.,
            "material normal scale"
        );
        ensure!(
            self.depth_fade.is_finite() && (0.0..=100_000.).contains(&self.depth_fade),
            "material depth fade"
        );
        ensure!(
            self.uv_scale
                .iter()
                .chain(&self.uv_offset)
                .all(|x| x.is_finite() && x.abs() <= 100_000.),
            "material UV transform"
        );
        Ok(())
    }
}
pub type MaterialSpec = Material<String>;
impl MaterialSpec {
    pub fn validate_paths(&self) -> Result<()> {
        self.validate()?;
        for p in self.textures().into_iter().flatten() {
            let v = crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, p)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(v.relative() == p, "non-canonical material asset path");
        }
        Ok(())
    }
}
