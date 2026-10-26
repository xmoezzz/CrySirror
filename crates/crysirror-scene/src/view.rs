// SPDX-License-Identifier: MPL-2.0
//! Camera, lighting environment and PBR scene extraction (ADR 0010).
//! Plain data only: asset paths are logical and client-scoped; GPU resources
//! (textures, prefiltered environment maps, shadow maps) are rebuilt by the
//! renderer from these references and are never saved.
use crate::{
    model::{Camera, Geometry, Scene3d},
    world::{geometry_data, EntityHandle, World},
};
use anyhow::{ensure, Result};
use glam::Vec3;
use serde::{Deserialize, Serialize};

fn finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite() && x.abs() <= 1_000_000.)
}
fn one() -> f32 {
    1.
}
fn sun_dir() -> [f32; 3] {
    [-0.4, -1.0, -0.3]
}
fn white3() -> [f32; 3] {
    [1.; 3]
}
fn sun_lux() -> f32 {
    3.
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sun {
    /// Direction the light travels (world space).
    #[serde(default = "sun_dir")]
    pub direction: [f32; 3],
    /// Linear RGB color.
    #[serde(default = "white3")]
    pub color: [f32; 3],
    /// Illuminance (lux-like renderer units).
    #[serde(default = "sun_lux")]
    pub intensity: f32,
    /// Cast shadows (cascades or virtual shadow maps).
    #[serde(default = "yes")]
    pub shadows: bool,
}
fn yes() -> bool {
    true
}
impl Default for Sun {
    fn default() -> Self {
        Self {
            direction: sun_dir(),
            color: white3(),
            intensity: sun_lux(),
            shadows: true,
        }
    }
}

/// Exponential height fog (the UE "ExponentialHeightFog" model): density
/// decays with height above `base_height`; the line integral along each view
/// ray gives the opacity, with directional inscattering toward the sun.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fog {
    /// Linear inscattered color.
    #[serde(default = "fog_color")]
    pub color: [f32; 3],
    /// Extinction per world unit at `base_height`.
    #[serde(default = "fog_density")]
    pub density: f32,
    /// Exponential decay per world unit of height (0 = uniform fog).
    #[serde(default = "fog_falloff")]
    pub height_falloff: f32,
    /// World height where `density` applies.
    #[serde(default)]
    pub base_height: f32,
    /// Distance from the camera before fog starts.
    #[serde(default)]
    pub start_distance: f32,
    /// Opacity cap (keeps distant geometry and sky partly visible).
    #[serde(default = "one")]
    pub max_opacity: f32,
    /// Extra inscattering toward the sun (color × sun color × strength).
    #[serde(default)]
    pub sun_scattering: f32,
    /// Sharpness of the inscattering lobe toward the sun (1..256).
    #[serde(default = "fog_exponent")]
    pub sun_exponent: f32,
}
fn fog_color() -> [f32; 3] {
    [0.45, 0.5, 0.6]
}
fn fog_density() -> f32 {
    0.02
}
fn fog_falloff() -> f32 {
    0.2
}
fn fog_exponent() -> f32 {
    8.
}
impl Fog {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            finite(&self.color)
                && self.color.iter().all(|c| (0.0..=1000.).contains(c))
                && (0.0..=100.).contains(&self.density)
                && (0.0..=100.).contains(&self.height_falloff)
                && self.base_height.is_finite()
                && self.base_height.abs() <= 1e6
                && (0.0..=1e6).contains(&self.start_distance)
                && (0.0..=1.).contains(&self.max_opacity)
                && (0.0..=1000.).contains(&self.sun_scattering)
                && (1.0..=256.).contains(&self.sun_exponent),
            "invalid fog"
        );
        Ok(())
    }
}

/// Eye adaptation: exposure maps the metered average luminance to middle
/// gray (0.18), clamped to a luminance range and adapted over time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutoExposure {
    /// Darkest/brightest average luminance (log2) the camera adapts to.
    #[serde(default = "ae_min")]
    pub min_log2: f32,
    /// Brightest average luminance (log2) the camera adapts to.
    #[serde(default = "ae_max")]
    pub max_log2: f32,
    /// Adaptation rates (1/s) toward brighter and darker scenes.
    #[serde(default = "ae_up")]
    pub speed_up: f32,
    /// Adaptation rate (1/s) toward darker scenes.
    #[serde(default = "ae_down")]
    pub speed_down: f32,
    /// Exposure compensation in stops, added to `Environment.exposure`.
    #[serde(default)]
    pub compensation: f32,
}
fn ae_min() -> f32 {
    -8.
}
fn ae_max() -> f32 {
    8.
}
fn ae_up() -> f32 {
    3.
}
fn ae_down() -> f32 {
    1.
}
impl AutoExposure {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [
                self.min_log2,
                self.max_log2,
                self.speed_up,
                self.speed_down,
                self.compensation
            ]
            .iter()
            .all(|x| x.is_finite())
                && -24. <= self.min_log2
                && self.min_log2 <= self.max_log2
                && self.max_log2 <= 24.
                && (0.0..=100.).contains(&self.speed_up)
                && (0.0..=100.).contains(&self.speed_down)
                && self.compensation.abs() <= 20.,
            "invalid auto exposure"
        );
        Ok(())
    }
}

/// Virtual shadow maps for the sun (ADR 0027): camera-centred clipmap of
/// 8192² virtual levels; level 0 has `texel_size` meters per shadow texel.
/// Replaces the cascades when set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VirtualShadows {
    /// Meters per shadow texel at level 0.
    #[serde(default = "vsm_texel")]
    pub texel_size: f32,
    /// Clipmap levels (1..16); level l has texel `texel_size·2^l`.
    #[serde(default = "vsm_levels")]
    pub levels: u32,
}
fn vsm_texel() -> f32 {
    0.02
}
fn vsm_levels() -> u32 {
    10
}
impl VirtualShadows {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.texel_size.is_finite()
                && (0.0001..=10.).contains(&self.texel_size)
                && (1..=16).contains(&self.levels),
            "invalid virtual shadows"
        );
        Ok(())
    }
}

/// Cluster-LOD virtualized geometry (ADR 0026): static model primitives get
/// a crack-free cluster DAG at load and draw the cut whose geometric error
/// projects to at most `pixel_error` pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeometryLod {
    /// Allowed projected geometric error in pixels (0 = full detail).
    #[serde(default = "one")]
    pub pixel_error: f32,
    /// Select and frustum-cull clusters on the GPU (compute + indirect
    /// draws); false keeps the CPU reference path.
    #[serde(default = "lod_gpu")]
    pub gpu: bool,
}
fn lod_gpu() -> bool {
    true
}
impl GeometryLod {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.pixel_error.is_finite() && (0.0..=1000.).contains(&self.pixel_error),
            "invalid geometry LOD"
        );
        Ok(())
    }
}

/// Screen-space reflections (ADR 0023): rays marched against the depth
/// prepass, radiance reprojected from the previous HDR frame, blended over
/// the prefiltered environment specular by hit confidence and roughness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenSpaceReflections {
    /// Strength of the screen-space contribution (0..4).
    #[serde(default = "one")]
    pub intensity: f32,
    /// World-space ray length (m).
    #[serde(default = "ssr_distance")]
    pub max_distance: f32,
    /// Depth tolerance (m) for counting a ray as behind a surface.
    #[serde(default = "ssr_thickness")]
    pub thickness: f32,
    /// Roughness at which reflections have fully faded to the environment.
    #[serde(default = "ssr_roughness")]
    pub max_roughness: f32,
    /// Linear march steps (refined by a binary search).
    #[serde(default = "ssr_steps")]
    pub steps: u32,
}
fn ssr_distance() -> f32 {
    30.
}
fn ssr_thickness() -> f32 {
    0.3
}
fn ssr_roughness() -> f32 {
    0.6
}
fn ssr_steps() -> u32 {
    48
}
impl ScreenSpaceReflections {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [
                self.intensity,
                self.max_distance,
                self.thickness,
                self.max_roughness
            ]
            .iter()
            .all(|x| x.is_finite())
                && (0.0..=4.).contains(&self.intensity)
                && (0.01..=10_000.).contains(&self.max_distance)
                && (0.001..=100.).contains(&self.thickness)
                && (0.01..=1.).contains(&self.max_roughness)
                && (4..=256).contains(&self.steps),
            "invalid screen-space reflections"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tonemap {
    /// Khronos PBR Neutral: hue-preserving, matches glTF sample viewers.
    #[default]
    PbrNeutral,
    /// ACES filmic approximation (Narkowicz).
    Aces,
    /// Clamp only, for debugging linear output.
    None,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// Equirectangular Radiance `.hdr` asset for image-based lighting and the
    /// sky. `None` uses a procedural gradient sky.
    #[serde(default)]
    pub hdr: Option<String>,
    /// Multiplier of the environment light (sky, IBL, GI fallback).
    #[serde(default = "one")]
    pub intensity: f32,
    /// Rotation of the environment around +Y, degrees.
    #[serde(default)]
    pub rotation_degrees: f32,
    /// Exposure in EV stops added before tonemapping.
    #[serde(default)]
    pub exposure: f32,
    /// HDR to display mapping.
    #[serde(default)]
    pub tonemap: Tonemap,
    /// Draw the environment as the background (false: black).
    #[serde(default = "yes")]
    pub show_sky: bool,
    /// Directional sun light; `null` for none.
    #[serde(default)]
    pub sun: Option<Sun>,
    /// Bloom mix strength (0 disables).
    #[serde(default = "bloom")]
    pub bloom: f32,
    /// Screen-space ambient occlusion strength (0 disables).
    #[serde(default = "ao")]
    pub ambient_occlusion: f32,
    /// Temporal anti-aliasing (jittered projection + reprojection).
    #[serde(default = "yes")]
    pub temporal_aa: bool,
    /// Maximum distance covered by the cascaded sun shadows.
    #[serde(default = "shadow_distance")]
    pub shadow_distance: f32,
    /// Exponential height fog (ADR 0019).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fog: Option<Fog>,
    /// Eye adaptation (ADR 0019); `null` keeps a fixed exposure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_exposure: Option<AutoExposure>,
    /// Screen-space reflections (ADR 0023).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reflections: Option<ScreenSpaceReflections>,
    /// Cluster-LOD virtualized geometry (ADR 0026).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry_lod: Option<GeometryLod>,
    /// Virtual shadow maps for the sun, replacing cascades (ADR 0027).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_shadows: Option<VirtualShadows>,
}
fn ao() -> f32 {
    1.
}
fn shadow_distance() -> f32 {
    60.
}
fn bloom() -> f32 {
    0.04
}
impl Default for Environment {
    fn default() -> Self {
        Self {
            hdr: None,
            intensity: 1.,
            rotation_degrees: 0.,
            exposure: 0.,
            tonemap: Tonemap::default(),
            show_sky: true,
            sun: Some(Sun::default()),
            bloom: bloom(),
            ambient_occlusion: ao(),
            temporal_aa: true,
            shadow_distance: shadow_distance(),
            fog: None,
            auto_exposure: None,
            reflections: None,
            geometry_lod: None,
            virtual_shadows: None,
        }
    }
}
impl Environment {
    pub fn validate(&self) -> Result<()> {
        if let Some(h) = &self.hdr {
            ensure!(
                !h.is_empty() && !h.starts_with('/') && !h.contains("..") && !h.contains('\\'),
                "invalid environment asset path"
            );
        }
        ensure!(
            self.intensity.is_finite()
                && (0.0..=1000.).contains(&self.intensity)
                && self.exposure.is_finite()
                && (-20.0..=20.).contains(&self.exposure)
                && self.rotation_degrees.is_finite()
                && (0.0..=1.).contains(&self.bloom)
                && (0.0..=4.).contains(&self.ambient_occlusion)
                && self.shadow_distance.is_finite()
                && (0.1..=100_000.).contains(&self.shadow_distance),
            "invalid environment parameters"
        );
        if let Some(f) = &self.fog {
            f.validate()?;
        }
        if let Some(a) = &self.auto_exposure {
            a.validate()?;
        }
        if let Some(r) = &self.reflections {
            r.validate()?;
        }
        if let Some(g) = &self.geometry_lod {
            g.validate()?;
        }
        if let Some(v) = &self.virtual_shadows {
            v.validate()?;
        }
        if let Some(s) = &self.sun {
            ensure!(
                finite(&s.direction)
                    && Vec3::from_array(s.direction).length() > 1e-4
                    && finite(&s.color)
                    && s.color.iter().all(|c| *c >= 0.)
                    && s.intensity.is_finite()
                    && (0.0..=100_000.).contains(&s.intensity),
                "invalid sun"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    #[serde(default)]
    pub camera: Option<Camera>,
    /// (owning client, environment). Its presence selects the PBR renderer.
    #[serde(default)]
    pub environment: Option<(u32, Environment)>,
}
impl View {
    pub fn validate(&self) -> Result<()> {
        if let Some(c) = &self.camera {
            Scene3d {
                camera: c.clone(),
                meshes: vec![],
            }
            .validate()?;
        }
        if let Some((_, e)) = &self.environment {
            e.validate()?;
        }
        Ok(())
    }
}

/// One drawable for the PBR path, with its world matrix (column-major).
#[derive(Clone, Debug, PartialEq)]
pub enum PbrDrawable {
    /// glTF mesh by client-scoped asset path.
    Model {
        client: u32,
        asset: String,
        mesh: u32,
    },
    /// Heightfield terrain (mesh and color map derived by the renderer).
    Terrain {
        client: u32,
        spec: crate::terrain::TerrainSpec,
    },
    /// Whole glTF scene with optional skeletal animation layers.
    Scene {
        client: u32,
        asset: String,
        animation: Vec<crate::components::AnimationLayer>,
        /// Non-zero morph target weights by target name.
        morphs: Vec<(String, f32)>,
    },
    /// Untextured triangles (cubes, legacy meshes) with a base color.
    Triangles {
        vertices: Vec<[f32; 3]>,
        indices: Vec<u32>,
        color: [u8; 4],
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct PbrInstance {
    pub material: Option<(u32, crate::material::MaterialSpec)>,
    /// Stable identity across frames (client << 32 | entity id; static
    /// project meshes use the high bit), used for motion vectors.
    pub key: u64,
    pub drawable: PbrDrawable,
    pub matrix: [[f32; 4]; 4],
}
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleEmitter {
    pub key: u64,
    pub matrix: [[f32; 4]; 4],
    pub spec: crate::components::EmitterSpec,
}
/// A local light resolved to world space for the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct PbrLight {
    pub key: u64,
    pub position: [f32; 3],
    /// Unit emission axis (spot/rect), world space.
    pub direction: [f32; 3],
    /// Rect light half-extent axes (world space, scaled), zero otherwise.
    pub right: [f32; 3],
    pub up: [f32; 3],
    /// Linear color × candela.
    pub intensity: [f32; 3],
    pub range: f32,
    pub shape: crate::components::LightShape,
    pub falloff: crate::components::Falloff,
    pub shadows: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PbrScene {
    /// World simulation time (seconds) for visual simulations.
    pub time: f32,
    pub emitters: Vec<ParticleEmitter>,
    pub camera: Camera,
    pub environment: Environment,
    pub environment_client: u32,
    pub instances: Vec<PbrInstance>,
    pub lights: Vec<PbrLight>,
    pub probes: Vec<PbrProbe>,
    pub volumes: Vec<PbrVolume>,
}
/// Irradiance volume in world space (ADR 0028).
#[derive(Clone, Debug, PartialEq)]
pub struct PbrVolume {
    pub key: u64,
    pub center: [f32; 3],
    pub spec: crate::components::IrradianceVolumeSpec,
}
/// Reflection capture at a world position with an axis-aligned box (ADR 0024).
#[derive(Clone, Debug, PartialEq)]
pub struct PbrProbe {
    pub key: u64,
    pub position: [f32; 3],
    pub extent: [f32; 3],
    pub intensity: f32,
    pub blend_distance: f32,
    pub revision: u32,
    /// (owning client, equirect .hdr asset) of an authored probe.
    pub source: Option<(u32, String)>,
    pub parallax: bool,
}

impl World {
    /// PBR extraction; `None` when no environment selects the PBR renderer.
    /// The project's static `Scene3d` meshes are included as untextured
    /// drawables so legacy content stays visible under PBR lighting.
    pub fn pbr_scene(&self, base: Option<&Scene3d>) -> Result<Option<PbrScene>> {
        self.validate()?;
        let selected = self.view.environment.clone().or_else(|| {
            self.client_ids()
                .find(|client| {
                    self.entities(crysirror_core::state::ClientId(*client))
                        .into_iter()
                        .any(|(_, _, spec)| spec.material.is_some())
                })
                .map(|client| (client, Environment::default()))
        });
        let Some((env_client, environment)) = selected else {
            return Ok(None);
        };
        let camera = self
            .view
            .camera
            .clone()
            .or_else(|| base.map(|b| b.camera.clone()))
            .unwrap_or_else(crate::world::default_camera);
        let mut instances = Vec::new();
        let mut emitters = Vec::new();
        let mut lights = Vec::new();
        let mut probes = Vec::new();
        let mut volumes = Vec::new();
        if let Some(base) = base {
            for (mi, m) in base.meshes.iter().enumerate() {
                let (vertices, indices) = geometry_data(&m.geometry);
                let t = crate::world::Transform {
                    translation: m.translation,
                    rotation_degrees: m.rotation_degrees,
                    scale: m.scale,
                };
                instances.push(PbrInstance {
                    material: None,
                    key: (1u64 << 63) | mi as u64,
                    drawable: PbrDrawable::Triangles {
                        vertices,
                        indices,
                        color: m.color,
                    },
                    matrix: t.matrix().to_cols_array_2d(),
                });
            }
        }
        for client in self.client_ids().collect::<Vec<_>>() {
            for (id, generation, spec) in self.entities(crysirror_core::state::ClientId(client)) {
                if !self.effective_visible(client, id) {
                    continue;
                }
                if let Some(p) = spec.particles.as_ref().filter(|p| p.enabled) {
                    let matrix = self
                        .global_matrix(EntityHandle {
                            client,
                            id,
                            generation,
                        })?
                        .to_cols_array_2d();
                    emitters.push(ParticleEmitter {
                        key: (u64::from(client) << 32) | u64::from(id),
                        matrix,
                        spec: p.clone(),
                    });
                }
                if let Some(v) = &spec.irradiance_volume {
                    let m = self.global_matrix(EntityHandle {
                        client,
                        id,
                        generation,
                    })?;
                    volumes.push(PbrVolume {
                        key: (u64::from(client) << 32) | u64::from(id),
                        center: m.transform_point3(Vec3::ZERO).to_array(),
                        spec: v.clone(),
                    });
                }
                if let Some(p) = &spec.reflection_probe {
                    let m = self.global_matrix(EntityHandle {
                        client,
                        id,
                        generation,
                    })?;
                    probes.push(PbrProbe {
                        key: (u64::from(client) << 32) | u64::from(id),
                        position: m.transform_point3(Vec3::ZERO).to_array(),
                        extent: p.extent,
                        intensity: p.intensity,
                        blend_distance: p.blend_distance,
                        revision: p.revision,
                        source: p.source.clone().map(|s| (client, s)),
                        parallax: p.parallax,
                    });
                }
                if let Some(l) = spec
                    .light
                    .as_ref()
                    .filter(|l| l.enabled && l.intensity > 0.)
                {
                    let m = self.global_matrix(EntityHandle {
                        client,
                        id,
                        generation,
                    })?;
                    let axis = |v: Vec3| m.transform_vector3(v);
                    let half = match l.shape {
                        crate::components::LightShape::Rect { size } => {
                            [size[0] * 0.5, size[1] * 0.5]
                        }
                        _ => [0.; 2],
                    };
                    let direction = axis(-Vec3::Z).normalize_or(-Vec3::Y);
                    let cd = l.candela();
                    lights.push(PbrLight {
                        key: (u64::from(client) << 32) | u64::from(id),
                        position: m.transform_point3(Vec3::ZERO).to_array(),
                        direction: direction.to_array(),
                        right: (axis(Vec3::X).normalize_or_zero() * half[0]).to_array(),
                        up: (axis(Vec3::Y).normalize_or_zero() * half[1]).to_array(),
                        intensity: l.color.map(|c| c * cd),
                        range: l.range,
                        shape: l.shape,
                        falloff: l.falloff,
                        shadows: l.shadows,
                    });
                }
                let Some(geometry) = &spec.geometry else {
                    continue;
                };
                let matrix = self
                    .global_matrix(EntityHandle {
                        client,
                        id,
                        generation,
                    })?
                    .to_cols_array_2d();
                let drawable = match geometry {
                    Geometry::Model { asset, mesh } => PbrDrawable::Model {
                        client,
                        asset: asset.clone(),
                        mesh: *mesh,
                    },
                    Geometry::Terrain(t) => PbrDrawable::Terrain {
                        client,
                        spec: t.clone(),
                    },
                    Geometry::Scene { asset } => PbrDrawable::Scene {
                        client,
                        asset: asset.clone(),
                        animation: spec
                            .animation
                            .as_ref()
                            .map(|a| a.layers.clone())
                            .unwrap_or_default(),
                        morphs: spec
                            .animation
                            .iter()
                            .flat_map(|a| &a.morphs)
                            .filter(|(_, w)| **w != 0.)
                            .map(|(n, w)| (n.clone(), *w))
                            .collect(),
                    },
                    g => {
                        let (vertices, indices) = geometry_data(g);
                        PbrDrawable::Triangles {
                            vertices,
                            indices,
                            color: spec.color,
                        }
                    }
                };
                instances.push(PbrInstance {
                    material: spec.material.clone().map(|m| (client, m)),
                    key: (u64::from(client) << 32) | u64::from(id),
                    drawable,
                    matrix,
                });
            }
        }
        Ok(Some(PbrScene {
            time: self.elapsed_ms as f32 / 1000.,
            emitters,
            camera,
            environment,
            environment_client: env_client,
            instances,
            lights,
            probes,
            volumes,
        }))
    }
}
