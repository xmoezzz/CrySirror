// Copyright (c) 2026 CrySirror contributors.
// Licensed under the Apache License, Version 2.0; see LICENSE.
//! First-order image-source paths; primitive intersection/BVH come from Parry/obvhs.
use crate::{AcousticScene, AcousticSource, geometry::epsilon};
use anyhow::{Context, Result, ensure};
use glam::Vec3;
use parry3d::{
    na::{Point3, Vector3},
    query::{Ray, RayCast},
    shape::Triangle,
};
use serde::{Deserialize, Serialize};

/// Bounded point-source, single-specular-bounce profile. Diffuse/multiple
/// reflections and diffraction require separate simulation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReflectionOptions {
    /// Physical units, air/directivity and ignored mesh; radius must be zero.
    pub propagation: AcousticSource,
    /// Maximum excess delay relative to the undelayed direct clock, 0..2 seconds.
    pub max_delay_seconds: f32,
    /// Maximum unique audible paths, 1..64. Overflow fails, rather than truncates.
    pub max_paths: usize,
    /// Complete-scene triangle budget, 1..262144. A larger scene fails visibly.
    pub triangle_budget: usize,
    /// Pressure reference distance, 0.01..100 metres; attenuation is min(1,ref/r).
    pub reference_distance_metres: f32,
    /// Additional wet amplitude, 0..1; dry playback is independent.
    pub wet_gain: f32,
}
impl Default for ReflectionOptions {
    fn default() -> Self {
        Self {
            propagation: Default::default(),
            max_delay_seconds: 0.5,
            max_paths: 64,
            triangle_budget: 262144,
            reference_distance_metres: 1.,
            wet_gain: 1.,
        }
    }
}
impl ReflectionOptions {
    pub fn validate(&self) -> Result<()> {
        self.propagation.validate()?;
        ensure!(
            self.propagation.radius == 0.,
            "reflection profile requires a point source"
        );
        ensure!(
            self.max_delay_seconds.is_finite() && (0. ..=2.).contains(&self.max_delay_seconds),
            "reflection delay limit"
        );
        ensure!(
            (1..=64).contains(&self.max_paths) && (1..=262144).contains(&self.triangle_budget),
            "reflection path/triangle budget"
        );
        ensure!(
            self.reference_distance_metres.is_finite()
                && (0.01..=100.).contains(&self.reference_distance_metres)
                && self.wet_gain.is_finite()
                && (0. ..=1.).contains(&self.wet_gain),
            "reflection distance/wet gain"
        );
        Ok(())
    }
}
/// Immutable emission/listener snapshot captured by a worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflectionQuery {
    /// Point emitter world position.
    pub source: [f32; 3],
    /// Listener world position.
    pub listener: [f32; 3],
    /// Listener facing direction; defaults to -Z.
    #[serde(default = "forward")]
    pub listener_forward: [f32; 3],
    /// Listener up; defaults to +Y and must be independent of forward.
    #[serde(default = "up")]
    pub listener_up: [f32; 3],
    /// Captured simulation and wet controls.
    #[serde(default)]
    pub options: ReflectionOptions,
}
fn forward() -> [f32; 3] {
    [0., 0., -1.]
}
fn up() -> [f32; 3] {
    [0., 1., 0.]
}
impl ReflectionQuery {
    pub fn validate(&self) -> Result<()> {
        self.options.validate()?;
        for position in [self.source, self.listener] {
            let v = Vec3::from(position);
            ensure!(
                v.is_finite() && v.abs().max_element() <= 1e6,
                "invalid reflection position"
            );
        }
        let f = Vec3::from(self.listener_forward);
        let u = Vec3::from(self.listener_up);
        ensure!(
            f.is_finite()
                && u.is_finite()
                && f.length_squared().is_finite()
                && u.length_squared().is_finite()
                && f.length_squared() > 1e-12
                && f.cross(u).length_squared().is_finite()
                && f.cross(u).length_squared() > 1e-12,
            "invalid reflection listener basis"
        );
        Ok(())
    }
}
/// One physically visible mirror path; gain is pressure, not energy.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflectionPath {
    /// Stable acoustic mesh ID.
    pub mesh: u32,
    /// Scene insertion-order triangle ID; coplanar edge ties use the first.
    pub triangle: u32,
    /// Acoustic material table index.
    pub material: u32,
    /// World-space specular reflection point.
    pub bounce: [f32; 3],
    /// Source-to-bounce plus bounce-to-listener distance in metres.
    pub length_metres: f32,
    /// Absolute physical travel time; direct playback is not delayed by this API.
    pub delay_seconds: f32,
    /// Actual DSP delay relative to a zero-latency direct clock.
    pub excess_delay_seconds: f32,
    /// Low/mid/high pressure gains including reflection, range, air and directivity.
    pub gains: [f32; 3],
    /// Multiplicative amplitude transmission across both legs, per band.
    pub leg_transmission: [f32; 3],
    /// Total crossed surfaces on both legs, excluding the reflector.
    pub crossings: u32,
    /// Unit listener-to-bounce arrival direction, in world coordinates.
    pub direction: [f32; 3],
}
/// Complete-scene first-order result; all bounded output paths are retained.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflectionPaths {
    /// Input snapshot; changing a pose requires a new simulation.
    pub query: ReflectionQuery,
    /// Visible single-bounce paths ordered by excess delay then triangle identity.
    pub paths: Vec<ReflectionPath>,
    /// All scene triangles considered, including rejected/ignored surfaces.
    pub tested_triangles: usize,
}
pub fn simulate(scene: &AcousticScene, query: &ReflectionQuery) -> Result<ReflectionPaths> {
    simulate_cancellable(scene, query, || Ok(()))
}
pub fn simulate_cancellable(
    scene: &AcousticScene,
    query: &ReflectionQuery,
    mut check: impl FnMut() -> Result<()>,
) -> Result<ReflectionPaths> {
    query.validate()?;
    ensure!(
        scene.triangle_count() <= query.options.triangle_budget,
        "reflection complete-scene triangle budget exceeded"
    );
    let source = Vec3::from(query.source);
    let listener = Vec3::from(query.listener);
    let p = &query.options.propagation;
    let direct = source.distance(listener);
    let mut result = ReflectionPaths {
        query: query.clone(),
        paths: vec![],
        tested_triangles: scene.triangle_count(),
    };
    let mut planes: Vec<(Vec3, Vec3)> = vec![];
    for (id, (vertices, mesh, material)) in scene.surfaces().enumerate() {
        if id % 64 == 0 {
            check()?;
        }
        if p.ignore_mesh == Some(*mesh) {
            continue;
        }
        let normal = (vertices[1] - vertices[0])
            .cross(vertices[2] - vertices[0])
            .normalize();
        let sd = (source - vertices[0]).dot(normal);
        let ld = (listener - vertices[0]).dot(normal);
        if sd.abs() <= epsilon(direct) || ld.abs() <= epsilon(direct) || sd.signum() != ld.signum()
        {
            continue;
        }
        let image = source - 2. * sd * normal;
        let delta = image - listener;
        let distance = delta.length();
        if distance <= 0.002 {
            continue;
        }
        let primitive = Triangle::new(
            Point3::from(vertices[0].to_array()),
            Point3::from(vertices[1].to_array()),
            Point3::from(vertices[2].to_array()),
        );
        let ray = Ray::new(
            Point3::from(listener.to_array()),
            Vector3::from((delta / distance).to_array()),
        );
        let Some(t) = primitive.cast_local_ray(&ray, distance, false) else {
            continue;
        };
        if t <= epsilon(distance) || t >= distance - epsilon(distance) {
            continue;
        }
        let bounce = listener + delta / distance * t;
        let total = source.distance(bounce) + listener.distance(bounce);
        let length = total * p.metres_per_unit;
        let excess = ((total - direct).max(0.) * p.metres_per_unit / p.sound_speed).max(0.);
        if excess > query.options.max_delay_seconds {
            continue;
        }
        let tolerance = epsilon(total) * 4.;
        if planes.iter().any(|(point, n)| {
            point.distance_squared(bounce) <= tolerance * tolerance && n.dot(normal).abs() > 0.99999
        }) {
            continue;
        }
        let (source_leg, source_hits) = crate::direct::segment_transmission(
            scene,
            source,
            bounce,
            p.ignore_mesh,
            p.transmission_hits,
        )
        .context("source reflection leg")?;
        let (listener_leg, listener_hits) = crate::direct::segment_transmission(
            scene,
            listener,
            bounce,
            p.ignore_mesh,
            p.transmission_hits,
        )
        .context("listener reflection leg")?;
        let leg_transmission = std::array::from_fn::<_, 3, _>(|i| source_leg[i] * listener_leg[i]);
        let mat = scene.material(*material);
        let range = (query.options.reference_distance_metres / length.max(1e-6)).min(1.);
        let directivity = crate::direct::source_directivity(p, (bounce - source).normalize());
        let gains = std::array::from_fn(|i| {
            let reflected = (1. - mat.absorption[i] - mat.transmission[i] * mat.transmission[i])
                .max(0.)
                * (1. - mat.scattering);
            reflected.sqrt()
                * (-p.air_absorption[i] * length).exp()
                * range
                * directivity
                * leg_transmission[i]
        });
        if gains.iter().all(|x| *x == 0.) {
            continue;
        }
        ensure!(
            result.paths.len() < query.options.max_paths,
            "reflection audible path budget exceeded"
        );
        planes.push((bounce, normal));
        result.paths.push(ReflectionPath {
            mesh: *mesh,
            triangle: id as u32,
            material: *material as u32,
            bounce: bounce.to_array(),
            length_metres: length,
            delay_seconds: length / p.sound_speed,
            excess_delay_seconds: excess,
            gains,
            leg_transmission,
            crossings: source_hits + listener_hits,
            direction: (bounce - listener).normalize().to_array(),
        });
    }
    check()?;
    result.paths.sort_by(|a, b| {
        a.excess_delay_seconds
            .total_cmp(&b.excess_delay_seconds)
            .then(a.triangle.cmp(&b.triangle))
    });
    Ok(result)
}
