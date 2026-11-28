// SPDX-License-Identifier: MPL-2.0
//! Per-area render settings, `dc1.psarc/render-settings.bin`.
//!
//! The module holds 238 `render-settings-2` records named
//! `*AREA-render-settings-NN*` (`*%s-render-settings-%02i*`, `sub_1271E4`).
//! Trigger regions select them with a `render-settings` entry (`StringId`
//! value, `menu-temple-ingame10`'s region names
//! `*menu-temple-render-settings-01*`); cutscenes call
//! `set-render-settings`. Field names and offsets come from the module's
//! `render-settings-menu-entries` (records `{":name", offset, type, step,
//! step, min, max}` per group) and the group bases from the EBOOT's
//! settings writer (`sub_123C1C`): lighting at 0 (`:ambient-scale` +0xcc),
//! post at 0x440 (tint, ambient, light-scale and shadow-colour RGBI), the
//! tonemap at 0x4b0, `:fog` at 0x1b0 and `:sky-fog` at 0x3d0 (the writer
//! prints both groups from these offsets). The fog writer (`sub_7587F8`)
//! turns the group into the shader terms of a Gaussian fog: opacity
//! `density·(1 − exp(−(max((w − start)/range, 0)·max(1 − (y −
//! height-start)/height-range, 0))²))` at view depth `w` and height `y`,
//! `density` applying only with a multiplier of at least 1. Other groups
//! (shadow, sky-fog, wind, sky, skydome, particles, rain, environment) are
//! not mapped.
use crate::dc::Dc;
use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tonemap {
    pub light_range: [f32; 2],
    pub manual_exposure: f32,
    pub adaption_time: f32,
    pub set_point: f32,
    pub white: f32,
    pub shoulder: f32,
    pub linear: f32,
    pub angle: f32,
    pub toe: f32,
    pub toe_numerator: f32,
    pub toe_denominator: f32,
    pub exposure_bias: f32,
}

/// The `:fog` group (record +0x1b0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    pub colour: [f32; 3],
    pub multiplier: f32,
    pub distance_start: f32,
    pub distance_range: f32,
    pub height_start: f32,
    pub height_range: f32,
    pub density: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderSettings {
    /// First light-info in the lighting group (the sun), before normalization.
    pub sun_direction: [f32; 3],
    pub sun_colour: [f32; 3],
    pub sun_intensity: f32,
    pub sun_shadow_distance: f32,
    pub ambient_scale: f32,
    /// Misc group +0, material vertex uniform g_basis3InGameScalar.
    pub baked_vertex_scale: f32,
    /// RGB × intensity.
    pub tint: [f32; 3],
    pub ambient: [f32; 3],
    pub light_scale: [f32; 3],
    pub shadow_colour: [f32; 3],
    pub fog: Fog,
    pub tonemap: Tonemap,
}

const LIGHTING_AMBIENT_SCALE: u32 = 0xcc;
const POST: u32 = 0x440;
const TONEMAP: u32 = 0x4b0;
const FOG: u32 = 0x1b0;

/// The record named `name` (a `StringId`) of the loaded `render-settings`
/// module.
pub fn read(dc: &Dc, name: u32) -> Result<Option<RenderSettings>> {
    let Some(module) = dc.module(crate::dc::sid("render-settings")) else { return Ok(None) };
    let Some(at) = module.entries.iter().find(|(n, _)| *n == name).map(|(_, a)| *a) else { return Ok(None) };
    let m = &dc.mem;
    if m.type_of(at).ok() != Some(crate::dc::sid("render-settings-2")) {
        return Ok(None);
    }
    let f = |o: u32| m.f32(at + o);
    let rgbi = |o: u32| -> Result<[f32; 3]> {
        let i = f(o + 12)?;
        Ok([f(o)? * i, f(o + 4)? * i, f(o + 8)? * i])
    };
    let t = |o: u32| f(TONEMAP + o);
    Ok(Some(RenderSettings {
        sun_direction: [f(0x20)?, f(0x24)?, f(0x28)?],
        sun_colour: [f(0)?, f(4)?, f(8)?],
        sun_intensity: f(0xc)?,
        sun_shadow_distance: f(0xc0)?,
        ambient_scale: f(LIGHTING_AMBIENT_SCALE)?,
        baked_vertex_scale: f(0x7e0)?,
        tint: rgbi(POST)?,
        ambient: rgbi(POST + 0x20)?,
        light_scale: rgbi(POST + 0x30)?,
        shadow_colour: rgbi(POST + 0x40)?,
        fog: Fog {
            colour: [f(FOG)?, f(FOG + 4)?, f(FOG + 8)?],
            multiplier: f(FOG + 0xc)?,
            distance_start: f(FOG + 0x10)?,
            distance_range: f(FOG + 0x14)?,
            height_start: f(FOG + 0x18)?,
            height_range: f(FOG + 0x1c)?,
            density: f(FOG + 0x20)?,
        },
        tonemap: Tonemap {
            light_range: [t(4)?, t(8)?],
            manual_exposure: t(0xc)?,
            adaption_time: t(0x14)?,
            set_point: t(0x18)?,
            white: t(0x20)?,
            shoulder: t(0x24)?,
            linear: t(0x28)?,
            angle: t(0x2c)?,
            toe: t(0x30)?,
            toe_numerator: t(0x34)?,
            toe_denominator: t(0x38)?,
            exposure_bias: t(0x3c)?,
        },
    }))
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

impl RenderSettings {
    /// Field-wise blend toward `other` (`set-render-settings a b t`,
    /// `sub_127BA4`).
    pub fn blend(&self, other: &Self, t: f32) -> Self {
        let m = |a: f32, b: f32| a + (b - a) * t;
        let (a, b) = (&self.tonemap, &other.tonemap);
        let (fa, fb) = (&self.fog, &other.fog);
        Self {
            sun_direction: mix3(self.sun_direction, other.sun_direction, t),
            sun_colour: mix3(self.sun_colour, other.sun_colour, t),
            sun_intensity: m(self.sun_intensity, other.sun_intensity),
            sun_shadow_distance: m(self.sun_shadow_distance, other.sun_shadow_distance),
            ambient_scale: m(self.ambient_scale, other.ambient_scale),
            baked_vertex_scale: m(self.baked_vertex_scale, other.baked_vertex_scale),
            tint: mix3(self.tint, other.tint, t),
            ambient: mix3(self.ambient, other.ambient, t),
            light_scale: mix3(self.light_scale, other.light_scale, t),
            shadow_colour: mix3(self.shadow_colour, other.shadow_colour, t),
            fog: Fog {
                colour: mix3(fa.colour, fb.colour, t),
                multiplier: m(fa.multiplier, fb.multiplier),
                distance_start: m(fa.distance_start, fb.distance_start),
                distance_range: m(fa.distance_range, fb.distance_range),
                height_start: m(fa.height_start, fb.height_start),
                height_range: m(fa.height_range, fb.height_range),
                density: m(fa.density, fb.density),
            },
            tonemap: Tonemap {
                light_range: [m(a.light_range[0], b.light_range[0]), m(a.light_range[1], b.light_range[1])],
                manual_exposure: m(a.manual_exposure, b.manual_exposure),
                adaption_time: m(a.adaption_time, b.adaption_time),
                set_point: m(a.set_point, b.set_point),
                white: m(a.white, b.white),
                shoulder: m(a.shoulder, b.shoulder),
                linear: m(a.linear, b.linear),
                angle: m(a.angle, b.angle),
                toe: m(a.toe, b.toe),
                toe_numerator: m(a.toe_numerator, b.toe_numerator),
                toe_denominator: m(a.toe_denominator, b.toe_denominator),
                exposure_bias: m(a.exposure_bias, b.exposure_bias),
            },
        }
    }
}
