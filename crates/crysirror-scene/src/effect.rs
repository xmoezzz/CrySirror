// SPDX-License-Identifier: MPL-2.0
//! Logical material effects. Custom shader source remains a separate code asset.
use anyhow::{ensure, Result};
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
    Custom {
        asset: String,
        #[serde(default)]
        parameters: [[f32; 4]; 4],
    },
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
