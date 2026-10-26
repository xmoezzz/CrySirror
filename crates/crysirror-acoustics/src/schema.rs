// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
use anyhow::{Result, ensure};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcousticBackend {
    #[default]
    Parry,
    Obvhs,
}
/// Frequency-band energy absorption and amplitude transmission at each crossing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AcousticMaterial {
    /// Absorbed fractions in low/mid/high frequency bands; reserved for reflection.
    pub absorption: [f32; 3],
    /// Amplitude transmitted at each surface crossing, in low/mid/high bands.
    pub transmission: [f32; 3],
    /// Diffuse scattering fraction, reserved for reflection simulation.
    pub scattering: f32,
}
impl Default for AcousticMaterial {
    fn default() -> Self {
        Self {
            absorption: [0.5, 0.3, 0.1],
            transmission: [0.1, 0.05, 0.01],
            scattering: 0.05,
        }
    }
}
fn identity() -> [f32; 16] {
    Mat4::IDENTITY.to_cols_array()
}
/// Logical geometry; each triangle can have a distinct acoustic material.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcousticMesh {
    /// Client-local stable mesh identity.
    pub id: u32,
    /// Local vertex positions, at most one million across a scene.
    pub vertices: Vec<[f32; 3]>,
    /// Indexed triangles; degenerate triangles fail instead of entering the BVH.
    pub triangles: Vec<[u32; 3]>,
    /// One material index per triangle, or empty to use material zero.
    #[serde(default)]
    pub materials: Vec<u32>,
    /// Column-major local-to-world matrix; nonuniform scale is baked on workers.
    #[serde(default = "identity")]
    pub transform: [f32; 16],
}
/// A client-owned acoustic scene, separate from physics collider selection.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AcousticSceneSpec {
    /// CPU query backend. Unavailable optional backends fail visibly.
    pub backend: AcousticBackend,
    /// Nonempty material table when geometry exists.
    pub materials: Vec<AcousticMaterial>,
    /// Meshes with unique IDs; maximum 4096 meshes and 262144 triangles.
    pub meshes: Vec<AcousticMesh>,
}
/// Direct sound propagation settings, reusable by any client or native adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AcousticSource {
    /// Source volume radius in world units; zero traces the source center.
    pub radius: f32,
    /// Deterministic volume visibility samples, 1..64.
    pub samples: u32,
    /// Maximum surface crossings before reporting budget exhaustion, 1..64.
    pub transmission_hits: u32,
    /// Per-metre amplitude absorption coefficients for three frequency bands.
    pub air_absorption: [f32; 3],
    /// Source forward direction for weighted dipole directivity.
    pub forward: [f32; 3],
    /// Omnidirectional at zero, cardioid at 0.5, dipole at one.
    pub dipole_weight: f32,
    /// Directivity exponent, 0..16.
    pub dipole_power: f32,
    /// Explicit mesh to exclude (for example the emitter's own geometry).
    pub ignore_mesh: Option<u32>,
    /// Conversion from world units to metres.
    pub metres_per_unit: f32,
    /// Sound speed in metres/second; delay is reported, not applied by this profile.
    pub sound_speed: f32,
}
impl Default for AcousticSource {
    fn default() -> Self {
        Self {
            radius: 0.,
            samples: 16,
            transmission_hits: 32,
            air_absorption: [0.0002, 0.0017, 0.182],
            forward: [0., 0., -1.],
            dipole_weight: 0.,
            dipole_power: 1.,
            ignore_mesh: None,
            metres_per_unit: 1.,
            sound_speed: 343.,
        }
    }
}
impl AcousticSource {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.radius.is_finite()
                && (0.0..=1000.).contains(&self.radius)
                && (1..=64).contains(&self.samples)
                && (1..=64).contains(&self.transmission_hits),
            "acoustic sampling limits"
        );
        ensure!(
            self.air_absorption
                .iter()
                .all(|x| x.is_finite() && (0.0..=10.).contains(x)),
            "acoustic air absorption"
        );
        let forward = Vec3::from(self.forward);
        ensure!(
            forward.is_finite()
                && forward.length_squared().is_finite()
                && forward.length_squared() > 1e-12,
            "acoustic forward vector"
        );
        ensure!(
            self.dipole_weight.is_finite()
                && (0.0..=1.).contains(&self.dipole_weight)
                && self.dipole_power.is_finite()
                && (0.0..=16.).contains(&self.dipole_power),
            "acoustic directivity"
        );
        ensure!(
            self.metres_per_unit.is_finite()
                && (0.0001..=1000.).contains(&self.metres_per_unit)
                && self.sound_speed.is_finite()
                && (1.0..=10000.).contains(&self.sound_speed),
            "acoustic units/speed"
        );
        Ok(())
    }
}
/// Immutable source/listener positions captured by an acoustic job.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcousticQuery {
    /// Emitter position in world units.
    pub source: [f32; 3],
    /// Listener position in the same coordinate system.
    pub listener: [f32; 3],
    /// Sampling, materials and physical unit settings captured by this job.
    #[serde(default)]
    pub settings: AcousticSource,
}
