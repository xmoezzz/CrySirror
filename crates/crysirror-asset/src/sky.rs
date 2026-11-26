// SPDX-License-Identifier: MPL-2.0
//! Gradient skies as equirectangular HDR environments: a zenith–horizon
//! gradient above the horizon, a dim ground below and a glow around the
//! sun, for engines whose sky is a parameterised time-of-day model rather
//! than an image. Rows run from +Y (top) to −Y; column 0 is −X, matching
//! the renderer's `equirect_uv`.
use crate::pbr::HdrImage;
use anyhow::{ensure, Context, Result};
use glam::Vec3;

#[derive(Clone, Debug, PartialEq)]
pub struct GradientSky {
    /// Direction toward the sun (world, normalised by `render`).
    pub sun: [f32; 3],
    /// Linear radiance at the zenith and at the horizon.
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    /// Radiance of the lower hemisphere.
    pub ground: [f32; 3],
    /// Sun glow colour (radiance at the sun) and its cosine exponent.
    pub glow: [f32; 3],
    pub glow_power: f32,
}

impl GradientSky {
    /// Radiance seen along `dir`.
    pub fn radiance(&self, dir: Vec3) -> Vec3 {
        let d = dir.normalize_or_zero();
        let sun = Vec3::from_array(self.sun).normalize_or_zero();
        let up = d.y;
        let base = if up >= 0. {
            // Horizon colour dominates near the horizon (√ falloff).
            Vec3::from_array(self.horizon).lerp(Vec3::from_array(self.zenith), up.max(0.).sqrt())
        } else {
            Vec3::from_array(self.horizon).lerp(Vec3::from_array(self.ground), (-up * 4.).min(1.))
        };
        let c = d.dot(sun).max(0.);
        base + Vec3::from_array(self.glow) * c.powf(self.glow_power.max(1.)) * if up >= -0.05 { 1. } else { 0. }
    }

    /// The sky as a `w`×`h` equirectangular image (`w = 2h`).
    pub fn render(&self, w: u32, h: u32) -> Result<HdrImage> {
        ensure!(w == h * 2 && (2..=4096).contains(&w), "sky size");
        let mut rgb = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            let theta = (y as f32 + 0.5) / h as f32 * std::f32::consts::PI;
            for x in 0..w {
                let phi = ((x as f32 + 0.5) / w as f32 - 0.5) * std::f32::consts::TAU;
                let dir = Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
                rgb.push(self.radiance(dir).max(Vec3::ZERO).to_array());
            }
        }
        Ok(HdrImage { width: w, height: h, rgb })
    }
}

/// A sky painted as a hemispherical latitude–longitude image: column
/// `u = atan2(z, x) / 2π + u_offset` (wrapping) and row
/// `v = zenith angle / zenith_span` (0 at the zenith; `zenith_span` π/2 puts
/// the horizon at the bottom row). Directions past the last row take the
/// mean colour of that row (a uniform horizon haze rather than streaks of
/// its texels). Texels are sRGB and become linear radiance × `scale`.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageSky<'a> {
    pub width: u32,
    pub height: u32,
    /// RGBA8 rows from the top.
    pub rgba: &'a [u8],
    pub u_offset: f32,
    pub zenith_span: f32,
    pub scale: f32,
}

fn srgb_to_linear(c: u8) -> f32 {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| {
            let c = i as f32 / 255.;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        })
    })[usize::from(c)]
}

impl ImageSky<'_> {
    /// Bilinear linear radiance along `dir` (repeat in u, clamp in v).
    pub fn radiance(&self, dir: Vec3) -> Vec3 {
        self.radiance_with(dir, || self.horizon())
    }

    /// [`Self::radiance`] with the horizon colour supplied (computed once
    /// per render instead of per direction).
    fn radiance_with(&self, dir: Vec3, horizon: impl FnOnce() -> Vec3) -> Vec3 {
        let d = dir.normalize_or_zero();
        let u = d.z.atan2(d.x) / std::f32::consts::TAU + self.u_offset;
        let v = d.y.clamp(-1., 1.).acos() / self.zenith_span;
        if v > 1. {
            return horizon() * self.scale;
        }
        let (w, h) = (self.width as i64, self.height as i64);
        let (x, y) = (u.rem_euclid(1.) * w as f32 - 0.5, (v.clamp(0., 1.) * h as f32 - 0.5).clamp(0., (h - 1) as f32));
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let texel = |tx: i64, ty: i64| {
            let at = ((ty.clamp(0, h - 1) * w + tx.rem_euclid(w)) * 4) as usize;
            Vec3::new(srgb_to_linear(self.rgba[at]), srgb_to_linear(self.rgba[at + 1]), srgb_to_linear(self.rgba[at + 2]))
        };
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = texel(x0, y0).lerp(texel(x0 + 1, y0), fx);
        let bottom = texel(x0, y0 + 1).lerp(texel(x0 + 1, y0 + 1), fx);
        top.lerp(bottom, fy) * self.scale
    }

    /// Mean linear colour of the bottom row.
    fn horizon(&self) -> Vec3 {
        let row = ((self.height - 1) * self.width * 4) as usize;
        let sum = (0..self.width as usize).fold(Vec3::ZERO, |a, x| {
            let at = row + 4 * x;
            a + Vec3::new(srgb_to_linear(self.rgba[at]), srgb_to_linear(self.rgba[at + 1]), srgb_to_linear(self.rgba[at + 2]))
        });
        sum / self.width as f32
    }

    /// The sky as a `w`×`h` equirectangular image (`w = 2h`).
    pub fn render(&self, w: u32, h: u32) -> Result<HdrImage> {
        ensure!(w == h * 2 && (2..=4096).contains(&w), "sky size");
        ensure!(self.width > 0 && self.height > 0 && self.rgba.len() == (self.width * self.height * 4) as usize, "sky image size");
        ensure!(self.zenith_span > 0. && self.scale.is_finite() && self.scale >= 0., "sky mapping");
        let horizon = self.horizon();
        let mut rgb = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            let theta = (y as f32 + 0.5) / h as f32 * std::f32::consts::PI;
            for x in 0..w {
                let phi = ((x as f32 + 0.5) / w as f32 - 0.5) * std::f32::consts::TAU;
                let dir = Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
                rgb.push(self.radiance_with(dir, || horizon).to_array());
            }
        }
        Ok(HdrImage { width: w, height: h, rgb })
    }
}

/// Encode linear RGB pixels as a Radiance `.hdr` file.
pub fn encode_hdr(img: &HdrImage) -> Result<Vec<u8>> {
    ensure!(img.rgb.len() == (img.width as usize) * (img.height as usize), "HDR pixel count");
    let mut out = Vec::new();
    let px: Vec<image::Rgb<f32>> = img.rgb.iter().map(|p| image::Rgb(*p)).collect();
    image::codecs::hdr::HdrEncoder::new(&mut out).encode(&px, img.width as usize, img.height as usize).context("encode HDR")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_round_trips_through_hdr_and_glows_toward_the_sun() {
        let sky = GradientSky { sun: [0., 0.2, 1.], zenith: [0.1, 0.2, 0.6], horizon: [0.6, 0.6, 0.7], ground: [0.1, 0.1, 0.1], glow: [2., 1., 0.5], glow_power: 16. };
        let img = sky.render(64, 32).unwrap();
        let back = crate::pbr::load_hdr(&encode_hdr(&img).unwrap()).unwrap();
        assert_eq!((back.width, back.height), (64, 32));
        let toward = sky.radiance(Vec3::new(0., 0.2, 1.));
        let away = sky.radiance(Vec3::new(0., 0.2, -1.));
        assert!(toward.x > away.x + 1.);
        assert!(sky.radiance(Vec3::Y).z > sky.radiance(Vec3::Y).x);
        assert!(sky.render(10, 10).is_err());
    }

    #[test]
    fn image_sky_maps_zenith_to_the_top_row_and_repeats_the_horizon() {
        // 4×2 image: top row white, bottom row a red/blue split in u.
        let mut rgba = vec![255u8; 4 * 2 * 4];
        for x in 0..4 {
            let at = (4 + x) * 4;
            rgba[at..at + 3].copy_from_slice(if x < 2 { &[255, 0, 0] } else { &[0, 0, 255] });
        }
        let sky = ImageSky { width: 4, height: 2, rgba: &rgba, u_offset: 0., zenith_span: std::f32::consts::FRAC_PI_2, scale: 2. };
        assert!((sky.radiance(Vec3::Y) - Vec3::splat(2.)).length() < 1e-3);
        // Just above the horizon u = atan2(z, x) / 2π picks the bottom row's
        // texels; below it the row's mean applies.
        let a = sky.radiance(Vec3::new(1., 0.001, 1.));
        assert!(a.x > 1.5 && a.z < 0.5, "{a}");
        let b = sky.radiance(Vec3::new(-1., 0.001, -1.));
        assert!(b.z > 1.5 && b.x < 0.5, "{b}");
        let below = sky.radiance(Vec3::new(1., -0.5, 1.));
        assert!((below - Vec3::new(1., 0., 1.)).length() < 1e-3, "{below}");
        let img = sky.render(16, 8).unwrap();
        assert_eq!(img.rgb.len(), 128);
    }
}
