// SPDX-License-Identifier: MPL-2.0
//! Engine-owned, device-free material data. `String` textures are client asset
//! paths; importers use image indices with the same shading/alpha contract.
use anyhow::{Result, ensure};
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
    /// Screen blend using premultiplied source colour, `D + (1 - D) × S × alpha`.
    SoftAdditive,
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
        matches!(self, Self::Blend | Self::Additive | Self::Premultiplied | Self::SoftAdditive)
    }
    pub fn code(self) -> f32 {
        match self {
            Self::Opaque => 0.,
            Self::Mask => 1.,
            Self::Blend => 2.,
            Self::Additive => 3.,
            Self::Premultiplied => 4.,
            Self::SoftAdditive => 5.,
        }
    }
}
/// Base shade of the `ToonShading::shadow_light` model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToonShadeKind {
    /// HSV of the linear base: hue kept, saturation `√S · min(10√V, 1)`,
    /// value `0.5√V + 0.5`.
    #[default]
    BaseHsv,
    /// `mix((0.4508, 0.6061, 1), base, max(√S · min(10V, 1), 1 − V))`:
    /// pale colours shade toward blue, saturated ones keep their colour.
    Cloth,
    /// `shade_color`.
    Fixed,
}

/// Surface-line composition from raw mask channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToonLineModel {
    /// R gates, G covers; colour range capped to 0.95.
    #[default]
    Gated,
    /// R tints the lit surface with nose_color, max(G,B) covers the face
    /// line, and A attenuates its view-dependent shadow tint.
    Face { nose_color: [f32; 4] },
    /// R modulates edge width, G is the painted detail line. The base
    /// alpha and its eight neighbours determine distance-dependent coverage.
    Cloth { width: f32, edge_alpha: f32, mip_bias: f32, alpha_dither: f32, opacity: f32, clip: f32, clip_force: f32 },
}
impl ToonLineModel {
    pub fn uniforms(self) -> [[f32; 4]; 2] {
        match self {
            Self::Gated => [[0.; 4]; 2],
            Self::Face { nose_color } => [[1., 0., 0., 0.], nose_color],
            Self::Cloth { width, edge_alpha, mip_bias, alpha_dither, opacity, clip, clip_force } =>
                [[2., width, edge_alpha, mip_bias], [alpha_dither, opacity, clip, clip_force]],
        }
    }
    fn validate(self) -> Result<()> {
        let unit = |v: f32| (0. ..=1.).contains(&v);
        let valid = match self {
            Self::Gated => true,
            Self::Face { nose_color } => nose_color.into_iter().all(unit),
            Self::Cloth { width, edge_alpha, mip_bias, alpha_dither, opacity, clip, clip_force } =>
                (0. ..=8.).contains(&width) && (-16. ..=16.).contains(&mip_bias)
                    && [edge_alpha, alpha_dither, opacity, clip, clip_force].into_iter().all(unit),
        };
        ensure!(valid, "invalid toon line parameter");
        Ok(())
    }
}

/// Cel ("toon") shading, evaluated on display-referred (gamma) values like
/// the forward toon shaders of Unity-era games:
///
/// - `c = base · base_color` (+ rim `pow(1 − n·v + rim_shift, rim_power) · rim_color`);
/// - `u = n·l · 0.4951 + 0.5`; `a = shadow.a · (1 − rate(u))`;
/// - `m = mix(c, shadow.rgb, a)` (+ highlight `hi.rgb · hi.a · max(n·h, 0)^highlight_power · highlight_rate`),
///   blended toward `shadow_color` by the cast-shadow term (with
///   `cast_shadow_ramp`, the ramp moves to its shadow end `ramp(0)` instead);
/// - colour = `m · ramp(u) · sun + max(n·h, 0)^48 · shininess + c · ambient · ambient_scale`,
///   with `sun` limited per channel to `light_limit` when it is positive.
///
/// `lookup_texture` (linear, 256×3) holds the diffuse ramp in row 0, the
/// shadow-layer rate in row 1 (red) and, with `color_table`, a colour table
/// in row 2: base, shadow, ramp and highlight colours are replaced by
/// `table(red · 0.9951)`. `shadow_texture` and `highlight_texture` are
/// sampled raw (no sRGB decode). Toon materials bind these three textures in
/// place of the metallic-roughness, occlusion and specular slots, which must
/// be empty, and take no shader effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ToonShading<T = String> {
    pub lookup_texture: Option<T>,
    /// Raw hull colour texture. Lookup row 2 supplies its independent lighting ramp.
    pub outline_texture: Option<T>,
    pub shadow_texture: Option<T>,
    pub highlight_texture: Option<T>,
    /// Second base texture the colour moves toward by the node's
    /// `AnimationState::base_blend` weight (sampled like the base texture).
    pub blend_texture: Option<T>,
    /// Liquid layer (with `liquid_regions`): blob masks sampled at
    /// `uv · liquid_tiling.xy + liquid_tiling.zw` — red the level-2 blobs
    /// `A`, green the level-1 blobs `B` (linear).
    pub liquid_blobs: Option<T>,
    /// Liquid layer region masks (RGBA, linear, uv0): region `c` at level
    /// `l` (`AnimationState::liquid_levels`, 0–2) covers
    /// `min(max(sat(l − 1)·A, l·B), region_c)`; the summed coverage
    /// (saturated) moves the base toward `liquid_color` (linear).
    pub liquid_regions: Option<T>,
    pub liquid_tiling: [f32; 4],
    pub liquid_color: [f32; 3],
    pub color_table: bool,
    pub rim_color: [f32; 3],
    pub rim_power: f32,
    pub rim_shift: f32,
    /// Display-referred colour under cast shadows.
    pub shadow_color: [f32; 3],
    /// Cast shadows shade with the ramp's shadow tone (`ramp(0)`, relative
    /// to the surface colour) instead of the absolute `shadow_color`, and
    /// the outline is shaded like the surface
    /// (`outline · (ramp · sun + ambient · ambient_scale)`) instead of
    /// `ramp · outline · sun · 2 + ambient · 2`.
    pub cast_shadow_ramp: bool,
    /// Scale of the environment ambient term.
    pub ambient_scale: f32,
    /// Per-channel upper bound of the display-referred key light colour
    /// (× intensity) the ramp is multiplied by; 0 leaves it unbounded.
    pub light_limit: f32,
    /// Row 0's red channel is the lit amount and the shadow tone comes from
    /// the base texture colour in linear space: its hue, saturation
    /// `√S · min(10√V, 1)` and value `0.5√V + 0.5` (HSV), i.e. the shaded
    /// colour is `base · mix(shade(base), 1, lit)` (linear).
    pub shade_from_base: bool,
    /// With `shade_from_base`: the shadow tone moves toward this linear
    /// colour (rgb) at grazing angles, `min((1 − n'·v)^1.8, 1) ·
    /// (0.7 − 0.2 · (clamp(lum(sun), 1, 3) − 1)) · a` with `n'` the normal
    /// bent 0.25 down.
    pub shadow_rim: [f32; 4],
    /// With `shade_from_base` and `a > 0`: the ILLGAMES AL skin light model.
    /// The shadow tone becomes `lum + k·(f·M − lum)` with `M` the mix of the
    /// ambient tone (`HSV(SH · 0.7 + 0.3)`, saturation × 1.5) toward the
    /// base shade by `w = (0.7 − 0.2·(clamp(lum(L), 1, 3) − 1)) · 0.375 ·
    /// (1 + sat(n·l))`, `f`/`k` from the key light's per-channel
    /// `clamp(log10 L, 0, 3) + sat(L)` sum `s` (`f = 1 − (min(s, 4) − 1)/6`,
    /// `k = 1.1 + 0.9·sat((s − 1)/3)`); the result is lit by
    /// `mix(rgb, L', max(shadow, 0.5)·sat((n·l + 0.8)·0.625))`, `rgb` (linear)
    /// the light on the unlit side (toward the SH ambient as `avg(L)`
    /// exceeds 1), `L' = max(min(L, 1), 0.8·min(L, 1) + 0.2)`.
    pub shadow_light: [f32; 4],
    /// The base shade `M` mixes toward under `shadow_light` (the shadow
    /// rim tint still applies to `base_hsv`).
    pub shade_kind: ToonShadeKind,
    /// Linear shade colour of `ToonShadeKind::Fixed`.
    pub shade_color: [f32; 3],
    /// Linear override RGB and blend weight for the completed shadow tone,
    /// after ambient/key grading and before the light ramp. Zero weight keeps the grade.
    pub shadow_tone_override: [f32; 4],
    /// Linear RGBA mask interpreted according to `line_model`. The default
    /// uses R as a gate and G as coverage over the lit surface.
    pub surface_line_texture: Option<T>,
    /// Selects the mask channels and whether the line affects alpha coverage.
    pub line_model: ToonLineModel,
    pub line_color: [f32; 4],
    pub shininess: f32,
    pub highlight_rate: f32,
    pub highlight_power: f32,
    /// Inverted-hull outline: clip-space width (0 disables) and colour,
    /// lit by the ramp at `n·l`.
    pub outline_width: f32,
    pub outline_color: [f32; 4],
}
impl<T> Default for ToonShading<T> {
    fn default() -> Self {
        Self {
            lookup_texture: None,
            outline_texture: None,
            shadow_texture: None,
            highlight_texture: None,
            blend_texture: None,
            liquid_blobs: None,
            liquid_regions: None,
            liquid_tiling: [1., 1., 0., 0.],
            liquid_color: [1.; 3],
            color_table: false,
            rim_color: [0.; 3],
            rim_power: 3.,
            rim_shift: 0.,
            shadow_color: [0.5; 3],
            cast_shadow_ramp: false,
            ambient_scale: 1.,
            light_limit: 0.,
            shade_from_base: false,
            shadow_rim: [0.; 4],
            shadow_light: [0.; 4],
            shade_kind: ToonShadeKind::BaseHsv,
            shade_color: [0.5; 3],
            shadow_tone_override: [0.; 4],
            surface_line_texture: None,
            line_model: ToonLineModel::default(),
            line_color: [0.; 4],
            shininess: 0.,
            highlight_rate: 0.,
            highlight_power: 1.,
            outline_width: 0.,
            outline_color: [0., 0., 0., 1.],
        }
    }
}
impl<T> ToonShading<T> {
    pub fn textures(&self) -> [Option<&T>; 7] {
        [self.lookup_texture.as_ref(), self.shadow_texture.as_ref(), self.highlight_texture.as_ref(), self.blend_texture.as_ref(), self.liquid_blobs.as_ref(), self.liquid_regions.as_ref(), self.surface_line_texture.as_ref()]
    }
    pub fn validate(&self) -> Result<()> {
        let range = |x: f32, a: f32, b: f32| x.is_finite() && (a..=b).contains(&x);
        ensure!(
            self.rim_color.iter().chain(&self.shadow_color).chain(&self.outline_color).all(|x| range(*x, 0., 16.))
                && range(self.rim_power, 0., 64.)
                && range(self.rim_shift, -1., 1.)
                && range(self.shininess, 0., 16.)
                && range(self.highlight_rate, 0., 16.)
                && range(self.highlight_power, 0., 256.)
                && range(self.outline_width, 0., 0.1)
                && range(self.ambient_scale, 0., 16.)
                && range(self.light_limit, 0., 16.)
                && self.shadow_rim.iter().all(|x| range(*x, 0., 1.))
                && self.shadow_light.iter().all(|x| range(*x, 0., 1.))
                && self.shadow_tone_override.iter().all(|x| range(*x, 0., 1.))
                && self.line_color.iter().all(|x| range(*x, 0., 1.))
                && self.shade_color.iter().all(|x| range(*x, 0., 1.))
                && self.liquid_tiling.iter().all(|x| range(*x, -1000., 1000.))
                && self.liquid_color.iter().all(|x| range(*x, 0., 16.)),
            "toon shading parameter range"
        );
        ensure!(self.outline_texture.is_none() || (self.lookup_texture.is_some() && !self.color_table && self.surface_line_texture.is_none()), "textured outline requires an independent lookup row and mask slot");
        self.line_model.validate()?;
        ensure!(matches!(self.line_model, ToonLineModel::Gated) || self.surface_line_texture.is_some(), "toon line model needs a texture");
        ensure!(!self.color_table || self.lookup_texture.is_some(), "toon colour table needs the lookup texture");
        Ok(())
    }
}

/// Authored draw order and Forward depth state. Queues through 2500 use
/// the geometry stage and depth/normal prepass. Higher values draw in order
/// in the transparent stage. Both submit to raster shadow maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RasterQueue {
    pub order: u16,
    pub depth_write: bool,
    /// Cull front-facing fragments, including in raster shadow draws.
    #[serde(default)]
    pub cull_front: bool,
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
    /// Explicit raster queue and forward depth state. Omission keeps the alpha mode's default passes.
    pub raster_queue: Option<RasterQueue>,
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
    /// Cel shading model instead of PBR lighting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toon: Option<ToonShading<T>>,
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
            raster_queue: None,
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
            toon: None,
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
    /// Every texture, the toon textures after the seven material slots.
    pub fn all_textures(&self) -> Vec<&T> {
        let toon = self.toon.iter().flat_map(|t| t.textures().into_iter().chain([t.outline_texture.as_ref()]));
        self.textures().into_iter().chain(toon).flatten().collect()
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(queue) = self.raster_queue {
            ensure!(queue.order <= 5000 && (self.alpha_mode == AlphaMode::Blend || (queue.order <= 2500 && !self.alpha_mode.transparent())), "invalid material raster queue");
        }
        if let Some(t) = &self.toon {
            t.validate()?;
            ensure!((t.surface_line_texture.is_none() && t.outline_texture.is_none()) || self.normal_texture.is_none(), "toon surface lines require the free normal texture slot");
            ensure!(
                self.effect.is_none()
                    && !self.unlit
                    && self.metallic_roughness_texture.is_none()
                    && self.occlusion_texture.is_none()
                    && self.specular_texture.is_none(),
                "toon materials take no effect, unlit flag or metallic-roughness/occlusion/specular texture"
            );
        }
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
                crate::effect::ShaderEffect::DirectionalLightmap { .. } => ensure!(
                    !self.unlit && self.occlusion_texture.is_some() && self.metallic_roughness_texture.is_some(),
                    "directional lightmaps require a lit material with radiance and direction textures"
                ),
                crate::effect::ShaderEffect::RgbLightmap { .. } => ensure!(
                    self.emissive_texture.is_some() && !self.alpha_mode.transparent(),
                    "RGB lightmaps require a lightmap texture and an opaque or masked material"
                ),
                crate::effect::ShaderEffect::LayeredBlinn { .. } => ensure!(
                    !self.unlit && self.alpha_mode == AlphaMode::Opaque,
                    "layered Blinn requires opaque lit surfaces"
                ),
                crate::effect::ShaderEffect::NormalShell { .. } => ensure!(
                    self.unlit && self.double_sided && self.alpha_mode == AlphaMode::Opaque,
                    "normal shells require opaque double-sided drawing with the shell lighting model"
                ),
                crate::effect::ShaderEffect::BlockNoise { .. } => ensure!(self.unlit, "block noise requires an unlit material"),
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
        for p in self.all_textures() {
            let v = crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, p)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            ensure!(v.relative() == p, "non-canonical material asset path");
        }
        Ok(())
    }
}
