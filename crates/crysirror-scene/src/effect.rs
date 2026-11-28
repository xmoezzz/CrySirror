// SPDX-License-Identifier: MPL-2.0
//! Logical material effects. Custom shader source remains a separate code asset.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
fn density() -> f32 {
    1.
}
fn scale() -> f32 {
    4.
}
fn speed() -> [f32; 2] {
    [0.03, 0.01]
}
fn edge() -> f32 {
    0.15
}
fn power() -> f32 {
    3.
}
fn width() -> f32 {
    0.05
}
fn color() -> [f32; 3] {
    [1., 0.25, 0.02]
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShaderEffect {
    Fog {
        #[serde(default = "density")]
        density: f32,
        #[serde(default = "scale")]
        noise_scale: f32,
        #[serde(default = "speed")]
        speed: [f32; 2],
        #[serde(default = "edge")]
        edge_fade: f32,
    },
    Fresnel {
        #[serde(default = "power")]
        power: f32,
        #[serde(default = "density")]
        intensity: f32,
    },
    Dissolve {
        #[serde(default)]
        threshold: f32,
        #[serde(default = "width")]
        edge_width: f32,
        #[serde(default = "color")]
        edge_color: [f32; 3],
    },
    /// Unlit screen-space block noise (a censor plane): the fragment's
    /// pixel position (y up) × 0.01 (+10 on x) × `scale`, shifted by
    /// `floor(time · rate) · 3` and floored to blocks, picks
    /// `mix(color_a, color_b, simplex(block) · 0.5 + 0.5)`, then
    /// `mix(·, 0.789, white)` (linear colours).
    BlockNoise {
        scale: f32,
        #[serde(default = "rate")]
        rate: f32,
        color_a: [f32; 3],
        color_b: [f32; 3],
        #[serde(default)]
        white: f32,
    },
    /// Back-face shell over a dedicated mesh. Base texture alpha is the
    /// extrusion/coverage mask. The ILLGAMES shell's depth-limited displacement
    /// is applied in object space after skinning. The base colour tints the shell.
    NormalShell {
        width: f32,
        distance_scale: f32,
        depth_limit: f32,
        dither: f32,
        cutoff: f32,
    },
    /// Directional baked diffuse radiance. The occlusion and metallic-roughness
    /// slots carry linear RGB radiance and encoded direction/normalization
    /// respectively. UV0 is mapped into the atlas independently of base UV.
    DirectionalLightmap { scale: [f32; 2], offset: [f32; 2] },
    /// Multiply the base by an RGB lightmap in the emissive slot. Secondary
    /// UVs are stored in `_AUX.xy`; the sun's shadow blends toward `shadow_color`.
    RgbLightmap {
        scale: [f32; 2],
        offset: [f32; 2],
        shadow_color: [f32; 3],
    },
    /// Two-layer blend by the primitive's `_AUX` vertex stream: the base
    /// texture weighted by `aux.x`, the emissive-slot texture by
    /// `1 − aux.x`; the emissive slot then contributes no emission.
    /// Primitives without `_AUX` show the base layer.
    VertexLayerBlend,
    /// Baked diffuse irradiance from the primitive's `_AUX` vertex stream
    /// (linear RGB, scaled by `intensity`) replaces the environment and
    /// GI-volume irradiance; direct lights still apply. Primitives without
    /// `_AUX` receive white irradiance × `intensity`.
    BakedVertexIrradiance {
        #[serde(default = "one")]
        intensity: f32,
        /// With a tint, also the [`ShaderEffect::SlopeLayerBlend`] layering.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slope_layer: Option<[f32; 3]>,
        /// A height-mask layering instead: the emissive-slot texture ×
        /// `tint` over the base where the vertex mask (`_AUX` w) rises
        /// above the occlusion-slot texture's green channel, weight
        /// `saturate((mask − (1 + s)·m + 2s) / 2s)` for the base with `s`
        /// the softness; the occlusion slot then gives no occlusion.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mask_layer: Option<MaskLayer>,
    },
    /// Two opaque albedo layers lit by a directional Blinn lobe and a
    /// second Blinn lobe from per-vertex irradiance (`_AUX.rgb`). The
    /// occlusion texture's green channel and vertex mask (`_AUX.w`) mix
    /// the base and emissive-slot albedos, and the green specular masks in
    /// the metallic-roughness (base) and opacity (upper layer) slots.
    LayeredBlinn {
        intensity: f32,
        softness: f32,
        mask_power: f32,
        specular_scale: f32,
        specular_exponent: f32,
    },
    /// Two albedo layers by surface slope: the base texture on walls, the
    /// emissive-slot texture (white when unbound) × `tint` on upward-facing
    /// surfaces, mixed as `mix(top, base, saturate(1 − n.y))` with `n` the
    /// interpolated world normal; the emissive slot then contributes no
    /// emission.
    SlopeLayerBlend {
        #[serde(default = "white")]
        tint: [f32; 3],
    },
    Custom {
        asset: String,
        #[serde(default)]
        parameters: [[f32; 4]; 4],
    },
}
fn rate() -> f32 {
    3.
}
impl ShaderEffect {
    pub fn validate(&self) -> Result<()> {
        let range = |x: f32, a: f32, b: f32| x.is_finite() && (a..=b).contains(&x);
        match self {
            Self::Fog {
                density,
                noise_scale,
                speed,
                edge_fade,
            } => ensure!(
                range(*density, 0., 20.)
                    && range(*noise_scale, 0.01, 128.)
                    && speed.iter().all(|x| range(*x, -100., 100.))
                    && range(*edge_fade, 0., 0.5),
                "fog effect parameter range"
            ),
            Self::Fresnel { power, intensity } => ensure!(
                range(*power, 0.01, 64.) && range(*intensity, 0., 100.),
                "fresnel effect parameter range"
            ),
            Self::Dissolve {
                threshold,
                edge_width,
                edge_color,
            } => ensure!(
                range(*threshold, 0., 1.)
                    && range(*edge_width, 0.0001, 1.)
                    && edge_color.iter().all(|x| range(*x, 0., 100.)),
                "dissolve effect parameter range"
            ),
            Self::BlockNoise { scale, rate, color_a, color_b, white } => ensure!(
                range(*scale, 0.001, 1000.) && range(*rate, 0., 120.) && color_a.iter().chain(color_b).all(|x| range(*x, 0., 16.)) && range(*white, 0., 1.),
                "block noise effect parameter range"
            ),
            Self::NormalShell { width, distance_scale, depth_limit, dither, cutoff } => ensure!(
                range(*width, 0., 1.) && range(*distance_scale, 0., 1.) && range(*depth_limit, 0., 100.)
                    && range(*dither, 0., 1.) && range(*cutoff, 0., 1.),
                "normal shell parameter range"
            ),
            Self::DirectionalLightmap { scale, offset } => ensure!(
                scale.iter().chain(offset).all(|v| v.is_finite()), "non-finite lightmap transform"
            ),
            Self::RgbLightmap { scale, offset, shadow_color } => ensure!(
                scale.iter().chain(offset).all(|v| v.is_finite())
                    && shadow_color.iter().all(|v| range(*v, 0., 16.)),
                "RGB lightmap parameter range"
            ),
            Self::VertexLayerBlend => {}
            Self::BakedVertexIrradiance { intensity, slope_layer, mask_layer } => ensure!(
                intensity.is_finite()
                    && (0. ..=64.).contains(intensity)
                    && slope_layer.iter().flatten().all(|x| range(*x, 0., 16.))
                    && mask_layer.iter().all(|m| m.tint.iter().all(|x| range(*x, 0., 16.)) && range(m.softness, 1e-4, 16.))
                    && !(slope_layer.is_some() && mask_layer.is_some()),
                "baked irradiance parameter range"
            ),
            Self::LayeredBlinn { intensity, softness, mask_power, specular_scale, specular_exponent } => ensure!(
                range(*intensity, 0., 64.) && range(*softness, 1e-4, 16.) && range(*mask_power, 1e-4, 128.)
                    && range(*specular_scale, 0., 64.) && range(*specular_exponent, 1e-4, 4096.),
                "layered Blinn parameter range"
            ),
            Self::SlopeLayerBlend { tint } => ensure!(tint.iter().all(|x| range(*x, 0., 16.)), "slope layer tint range"),
            Self::Custom { asset, parameters } => {
                let p =
                    crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, asset)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                ensure!(
                    p.relative() == asset && asset.ends_with(".wgsl"),
                    "custom effect requires a canonical .wgsl asset path"
                );
                ensure!(
                    parameters
                        .iter()
                        .flatten()
                        .all(|x| range(*x, -100_000., 100_000.)),
                    "custom effect parameter range"
                );
            }
        }
        Ok(())
    }
    pub fn custom_asset(&self) -> Option<&str> {
        if let Self::Custom { asset, .. } = self {
            Some(asset)
        } else {
            None
        }
    }
    pub fn uniforms(&self) -> (f32, [[f32; 4]; 4]) {
        let mut p = [[0.; 4]; 4];
        let kind = match self {
            Self::Fog {
                density,
                noise_scale,
                speed,
                edge_fade,
            } => {
                p[0] = [*density, *noise_scale, *edge_fade, 0.];
                p[1] = [speed[0], speed[1], 0., 0.];
                1.
            }
            Self::Fresnel { power, intensity } => {
                p[0] = [*power, *intensity, 0., 0.];
                2.
            }
            Self::Dissolve {
                threshold,
                edge_width,
                edge_color,
            } => {
                p[0] = [*threshold, *edge_width, 0., 0.];
                p[1] = [edge_color[0], edge_color[1], edge_color[2], 0.];
                3.
            }
            Self::BlockNoise { scale, rate, color_a, color_b, white } => {
                p[0] = [*scale, *rate, *white, 0.];
                p[1] = [color_a[0], color_a[1], color_a[2], 0.];
                p[2] = [color_b[0], color_b[1], color_b[2], 0.];
                4.
            }
            Self::NormalShell { width, distance_scale, depth_limit, dither, cutoff } => {
                p[0] = [*width, *distance_scale, *depth_limit, *dither];
                p[1][0] = *cutoff;
                5.
            }
            Self::DirectionalLightmap { scale, offset } => {
                p[0] = [scale[0], scale[1], offset[0], offset[1]];
                6.
            }
            Self::RgbLightmap { scale, offset, shadow_color } => {
                p[0] = [scale[0], scale[1], offset[0], offset[1]];
                p[1] = [shadow_color[0], shadow_color[1], shadow_color[2], 0.];
                10.
            }
            Self::VertexLayerBlend => 7.,
            Self::BakedVertexIrradiance { intensity, slope_layer, mask_layer } => {
                p[0][0] = *intensity;
                if let Some(t) = slope_layer {
                    p[1] = [t[0], t[1], t[2], 1.];
                }
                if let Some(m) = mask_layer {
                    p[1] = [m.tint[0], m.tint[1], m.tint[2], 2.];
                    p[2][0] = m.softness;
                }
                8.
            }
            Self::LayeredBlinn { intensity, softness, mask_power, specular_scale, specular_exponent } => {
                p[0] = [*intensity, *softness, *specular_scale, *specular_exponent];
                p[1][0] = *mask_power;
                11.
            }
            Self::SlopeLayerBlend { tint } => {
                p[1] = [tint[0], tint[1], tint[2], 1.];
                9.
            }
            Self::Custom { parameters, .. } => {
                p = *parameters;
                0.
            }
        };
        (kind, p)
    }
}

#[cfg(test)]
mod tests {
    use crate::material::MaterialSpec;
    #[test]
    fn effect_data_rejects_unknown_code_and_bad_parameters() {
        for bad in [
            serde_json::json!({"alpha_mode":"blend","effect":{"kind":"frog"}}),
            serde_json::json!({"effect":{"kind":"fog"}}),
            serde_json::json!({"effect":{"kind":"dissolve"}}),
            serde_json::json!({"effect":{"kind":"custom","asset":"../shader.wgsl"}}),
            serde_json::json!({"effect":{"kind":"custom","asset":"code.js"}}),
            serde_json::json!({"effect":{"kind":"custom","asset":"a.wgsl","source":"inline code"}}),
            serde_json::json!({"effect":{"kind":"fresnel","power":-1}}),
        ] {
            let parsed = serde_json::from_value::<MaterialSpec>(bad);
            assert!(parsed.is_err() || parsed.unwrap().validate_paths().is_err());
        }
        for good in [
            serde_json::json!({"alpha_mode":"blend","effect":{"kind":"fog"}}),
            serde_json::json!({"alpha_mode":"mask","effect":{"kind":"dissolve"}}),
            serde_json::json!({"effect":{"kind":"fresnel"}}),
            serde_json::json!({"effect":{"kind":"custom","asset":"fx/a.wgsl"}}),
        ] {
            serde_json::from_value::<MaterialSpec>(good)
                .unwrap()
                .validate_paths()
                .unwrap();
        }
    }
}

/// The height-mask layer of [`ShaderEffect::BakedVertexIrradiance`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaskLayer {
    #[serde(default = "white")]
    pub tint: [f32; 3],
    pub softness: f32,
}

fn white() -> [f32; 3] {
    [1.; 3]
}
fn one() -> f32 {
    1.
}
