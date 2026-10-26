// Copyright 2017-2023 Valve Corporation.
// Copyright 2024 phonon_rs contributors.
// Copyright (c) 2026 CrySirror contributors (engine integration and corrections).
// Licensed under the Apache License, Version 2.0, http://www.apache.org/licenses/LICENSE-2.0.
// Distributed on an AS IS BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND.
//! Directivity, air absorption and volumetric visibility adapted from phonon_rs
//! 87c392a3e605f9df30abe6e291f0bbc5e86f54ad. See UPSTREAM.md for semantic corrections.
use crate::{AcousticQuery, AcousticScene, geometry::epsilon};
use anyhow::{Result, ensure};
use glam::Vec3;
use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct DirectPath {
    /// Visible fraction, one means clear (opposite to the legacy occlusion scalar).
    pub visibility: f32,
    /// Multiplicative per-crossing amplitude transmission; no paired-surface heuristic.
    pub transmission: [f32; 3],
    /// Per-band air attenuation over the direct path.
    pub air_absorption: [f32; 3],
    /// Weighted dipole source attenuation.
    pub directivity: f32,
    /// Final three-band direct gains; the mixer applies its existing distance curve separately.
    pub gains: [f32; 3],
    /// Physical travel time, reported separately; not implicitly applied to playback.
    pub delay_seconds: f32,
    /// Number of surface crossings on the direct path.
    pub crossings: u32,
}
pub fn simulate(scene: &AcousticScene, query: &AcousticQuery) -> Result<DirectPath> {
    query.settings.validate()?;
    let source = Vec3::from(query.source);
    let listener = Vec3::from(query.listener);
    let settings = &query.settings;
    ensure!(
        source.is_finite()
            && listener.is_finite()
            && source.abs().max_element() <= 1e6
            && listener.abs().max_element() <= 1e6,
        "invalid acoustic source/listener"
    );
    let delta = source - listener;
    let distance = delta.length();
    let metres = distance * settings.metres_per_unit;
    let mut visible = 0u32;
    let mut valid = 0u32;
    let count = if settings.radius == 0. {
        1
    } else {
        settings.samples
    };
    for i in 0..count {
        let sample = source + volume_sample(i) * settings.radius;
        if scene.blocked(source, sample, settings.ignore_mesh)? {
            continue;
        }
        valid += 1;
        if !scene.blocked(listener, sample, settings.ignore_mesh)? {
            visible += 1;
        }
    }
    // If every radius sample is invalid, use the source centre instead of declaring silence.
    let visibility = if valid > 0 {
        visible as f32 / valid as f32
    } else {
        if scene.blocked(listener, source, settings.ignore_mesh)? {
            0.
        } else {
            1.
        }
    };
    let mut transmission = [1.; 3];
    let mut crossings = 0u32;
    if visibility < 1. && distance > 0.002 {
        let mut min = epsilon(distance);
        let end = distance - epsilon(distance);
        loop {
            let Some(hit) = scene.closest_hit(listener, delta, min, end, settings.ignore_mesh)?
            else {
                break;
            };
            ensure!(
                crossings < settings.transmission_hits,
                "acoustic transmission crossing budget exceeded"
            );
            crossings += 1;
            for (gain, t) in transmission
                .iter_mut()
                .zip(scene.material(hit.material).transmission)
            {
                *gain *= t;
            }
            min = hit.distance + epsilon(hit.distance);
            if min >= end {
                break;
            }
        }
    }
    let air_absorption = settings.air_absorption.map(|c| (-c * metres).exp());
    let directivity = if distance <= 0.002 {
        1.
    } else {
        let cosine = Vec3::from(settings.forward)
            .normalize()
            .dot(-delta / distance);
        ((1. - settings.dipole_weight) + settings.dipole_weight * cosine)
            .abs()
            .powf(settings.dipole_power)
    };
    let gains = std::array::from_fn(|i| {
        air_absorption[i] * directivity * (visibility + (1. - visibility) * transmission[i])
    });
    Ok(DirectPath {
        visibility,
        transmission,
        air_absorption,
        directivity,
        gains,
        delay_seconds: metres / settings.sound_speed,
        crossings,
    })
}
fn radical_inverse(base: u32, mut index: u32) -> f32 {
    let inv = 1. / base as f32;
    let mut factor = inv;
    let mut value = 0.;
    while index > 0 {
        value += (index % base) as f32 * factor;
        index /= base;
        factor *= inv;
    }
    value
}
fn volume_sample(index: u32) -> Vec3 {
    let phi = std::f32::consts::TAU * radical_inverse(2, index);
    let z = 2. * radical_inverse(3, index) - 1.;
    let r = radical_inverse(5, index).cbrt();
    let xy = (1. - z * z).max(0.).sqrt();
    r * Vec3::new(xy * phi.cos(), xy * phi.sin(), z)
}
