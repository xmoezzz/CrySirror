//! Signed displacement and direction/distance fields, reusable without a VM.
use crate::ImageSurface as DecodedImage;
use anyhow::{Result, ensure};
use crysirror_core::{
    image_processing::{
        DirectionDistanceSample, FieldWarpSpec, VectorFieldResource, VectorFieldSamples,
    },
    state::ClientId,
};
use std::sync::Arc;
#[derive(Clone, Debug)]
pub enum FieldPattern {
    Displacement {
        kind: u8,
        parameters: Vec<i32>,
    },
    Radial {
        mode: i32,
        center: [i32; 2],
        period: i32,
    },
    Axis {
        mode: i32,
    },
}
pub fn blank(client: ClientId, size: [u32; 2], format: i32) -> Result<Arc<VectorFieldResource>> {
    crate::rgba_len(size[0], size[1])?;
    ensure!(
        u64::from(size[0]) * u64::from(size[1]) <= crate::effects::MAX_EFFECT_PIXELS,
        "vector field pixel budget"
    );
    let n = size[0] as usize * size[1] as usize;
    let samples = match format {
        4 => VectorFieldSamples::Displacement(vec![[0; 2]; n].into()),
        6 => VectorFieldSamples::DirectionDistance(
            vec![DirectionDistanceSample::default(); n].into(),
        ),
        _ => anyhow::bail!("unsupported vector field format"),
    };
    let field = Arc::new(VectorFieldResource {
        client,
        size,
        samples,
    });
    field.validate(client)?;
    Ok(field)
}
pub fn generate(
    source: &VectorFieldResource,
    pattern: &FieldPattern,
    check: &dyn Fn() -> Result<()>,
) -> Result<Arc<VectorFieldResource>> {
    source.validate(source.client)?;
    check()?;
    let size = source.size;
    let samples = match (&source.samples, pattern) {
        (
            VectorFieldSamples::Displacement(samples),
            FieldPattern::Displacement { kind, parameters },
        ) => {
            let expected = match kind {
                0x10 | 0x13 | 0x14 | 0x15 => 4,
                0x11 => 1,
                0x12 | 0x17 => 5,
                0x16 => 6,
                _ => anyhow::bail!("unknown displacement pattern"),
            };
            ensure!(
                parameters.len() == expected,
                "displacement pattern parameter count"
            );
            let mut map = DisplacementMap {
                width: size[0],
                height: size[1],
                samples: samples.to_vec(),
            };
            match kind {
                0x10 => map.zoom(parameters, check)?,
                0x11 => map.diffuse(parameters, check)?,
                0x12 => map.radial_cosine(parameters, check)?,
                0x13 => map.perspective_bend(parameters, check)?,
                0x14 => map.curvature(parameters, check)?,
                0x15 => map.radial_lens(parameters, check)?,
                0x16 => map.wave(parameters, check)?,
                0x17 => map.radial_warp(parameters, check)?,
                _ => unreachable!(),
            }
            VectorFieldSamples::Displacement(map.samples.into())
        }
        (VectorFieldSamples::DirectionDistance(samples), pattern) => {
            let mut map = VectorMap {
                width: size[0],
                height: size[1],
                samples: samples
                    .iter()
                    .map(|s| VectorSample {
                        x: s.x,
                        y: s.y,
                        phase: s.phase,
                    })
                    .collect(),
            };
            match pattern {
                FieldPattern::Radial {
                    mode,
                    center,
                    period,
                } => {
                    ensure!((0..=1).contains(mode), "radial field mode");
                    map.generate_radial(*mode, center[0], center[1], *period, check)?;
                }
                FieldPattern::Axis { mode } => {
                    ensure!((0..=3).contains(mode), "axis field mode");
                    map.generate_axis(*mode, check)?;
                }
                _ => anyhow::bail!("field format/pattern mismatch"),
            }
            VectorFieldSamples::DirectionDistance(
                map.samples
                    .into_iter()
                    .map(|s| DirectionDistanceSample {
                        x: s.x,
                        y: s.y,
                        phase: s.phase,
                    })
                    .collect::<Vec<_>>()
                    .into(),
            )
        }
        _ => anyhow::bail!("field format/pattern mismatch"),
    };
    check()?;
    Ok(Arc::new(VectorFieldResource {
        client: source.client,
        size,
        samples,
    }))
}
pub fn copy(
    destination: &VectorFieldResource,
    source: &VectorFieldResource,
    position: [i32; 2],
    check: &dyn Fn() -> Result<()>,
) -> Result<Arc<VectorFieldResource>> {
    destination.validate(destination.client)?;
    source.validate(destination.client)?;
    let [w, h] = destination.size;
    let [sw, sh] = source.size;
    ensure!(
        position.iter().all(|v| v.unsigned_abs() <= 1_000_000),
        "field copy position"
    );
    macro_rules! copy_plane {
        ($dst:expr,$src:expr) => {{
            let mut out = $dst.to_vec();
            for y in 0..sh {
                check()?;
                let dy = i64::from(position[1]) + i64::from(y);
                if !(0..i64::from(h)).contains(&dy) {
                    continue;
                }
                for x in 0..sw {
                    let dx = i64::from(position[0]) + i64::from(x);
                    if (0..i64::from(w)).contains(&dx) {
                        out[(dy as u32 * w + dx as u32) as usize] = $src[(y * sw + x) as usize];
                    }
                }
            }
            out.into()
        }};
    }
    let samples = match (&destination.samples, &source.samples) {
        (VectorFieldSamples::Displacement(d), VectorFieldSamples::Displacement(s)) => {
            VectorFieldSamples::Displacement(copy_plane!(d, s))
        }
        (VectorFieldSamples::DirectionDistance(d), VectorFieldSamples::DirectionDistance(s)) => {
            VectorFieldSamples::DirectionDistance(copy_plane!(d, s))
        }
        _ => anyhow::bail!("field copy format mismatch"),
    };
    Ok(Arc::new(VectorFieldResource {
        client: destination.client,
        size: destination.size,
        samples,
    }))
}
#[derive(Debug, Clone)]
pub struct WaveTable {
    block_span: usize,
    samples: Vec<[i16; 2]>,
}

impl WaveTable {
    fn compact_unchecked(
        quarter_period: i32,
        amplitude: i32,
        block_height: i32,
        block_count: i32,
    ) -> Self {
        let cycle_len = quarter_period.max(1) as usize * 4;
        let block_height = block_height.max(1) as usize;
        let block_count = block_count.max(1) as usize;
        let mut samples = Vec::with_capacity(
            cycle_len
                .saturating_mul(block_height)
                .saturating_mul(block_count),
        );
        for _ in 0..block_count {
            append_sine_cycle(&mut samples, cycle_len, amplitude, 0);
            for _ in 1..block_height {
                samples.resize(samples.len().saturating_add(cycle_len), [0, 0]);
            }
        }
        Self {
            samples,
            block_span: cycle_len * block_height,
        }
    }

    fn extended_unchecked(
        quarter_period: i32,
        amplitude: i32,
        lead_levels: i32,
        trail_levels: i32,
        block_height: i32,
        block_count: i32,
    ) -> Self {
        let cycle_len = quarter_period.max(1) as usize * 4;
        let lead_levels = lead_levels.max(0) as usize;
        let trail_levels = trail_levels.max(0) as usize;
        let block_height = block_height.max(1) as usize;
        let block_count = block_count.max(1) as usize;
        let cycles_per_row = lead_levels.saturating_add(trail_levels).saturating_add(1);
        let row_len = cycle_len.saturating_mul(cycles_per_row);
        let mut samples = Vec::with_capacity(
            row_len
                .saturating_mul(block_height)
                .saturating_mul(block_count),
        );
        for _ in 0..block_count {
            for _ in 1..block_height {
                samples.resize(samples.len().saturating_add(row_len), [0, 0]);
            }
            for shift in (1..=lead_levels).rev() {
                append_sine_cycle(&mut samples, cycle_len, amplitude, shift);
            }
            append_sine_cycle(&mut samples, cycle_len, amplitude, 0);
            for shift in 1..=trail_levels {
                append_sine_cycle(&mut samples, cycle_len, amplitude, shift);
            }
        }
        Self {
            samples,
            block_span: row_len * block_height,
        }
    }
}

/// sub_409AE0 / sub_409C50: the amplitude is converted as unsigned,
/// cvttsd2si gives 0x80000000 out of range, and the sample keeps its low
/// 16 bits.
fn append_sine_cycle(samples: &mut Vec<[i16; 2]>, cycle_len: usize, amplitude: i32, shift: usize) {
    let divisor = 2_f64.powi(shift.min(30) as i32);
    for index in 0..cycle_len {
        let phase = index as f64 * std::f64::consts::TAU / cycle_len as f64;
        let scaled = phase.sin() * f64::from(amplitude as u32) / divisor;
        let truncated = if scaled.is_finite() && scaled > -2147483649.0 && scaled < 2147483648.0 {
            scaled as i32
        } else {
            i32::MIN
        };
        let value = truncated as i16;
        samples.push([value, value]);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct VectorSample {
    x: i16,
    y: i16,
    phase: u16,
}

#[derive(Debug, Clone)]
struct VectorMap {
    width: u32,
    height: u32,
    samples: Vec<VectorSample>,
}

#[derive(Debug, Clone)]
struct DisplacementMap {
    width: u32,
    height: u32,
    samples: Vec<[i16; 2]>,
}

impl DisplacementMap {
    fn zoom(&mut self, parameters: &[i32], check: &dyn Fn() -> Result<()>) -> Result<()> {
        let [center_x, center_y, width_scale, height_scale, ..] = parameters else {
            return Ok(());
        };
        let width = self.width.max(1) as i64;
        let height = self.height.max(1) as i64;
        for y in 0..self.height as i64 {
            check()?;
            for x in 0..self.width as i64 {
                let dx = 16 * (i64::from(*center_x) - x) + 16 * x * i64::from(*width_scale) / width;
                let dy =
                    16 * (i64::from(*center_y) - y) + 16 * y * i64::from(*height_scale) / height;
                self.set(x as u32, y as u32, dx, dy);
            }
        }
        Ok(())
    }

    fn diffuse(&mut self, parameters: &[i32], check: &dyn Fn() -> Result<()>) -> Result<()> {
        let Some(radius) = parameters.first().copied() else {
            return Ok(());
        };
        let radius = radius.unsigned_abs().min(i16::MAX as u32) as i32;
        let span = radius.saturating_mul(2).saturating_add(1).max(1) as u32;
        for y in 0..self.height {
            check()?;
            for x in 0..self.width {
                let seed = x
                    .wrapping_mul(0x9E37_79B9)
                    .wrapping_add(y.wrapping_mul(0x85EB_CA6B));
                let dx = radius - (mix32(seed) % span) as i32;
                let dy = radius - (mix32(seed ^ 0xA5A5_5A5A) % span) as i32;
                self.set(x, y, i64::from(dx), i64::from(dy));
            }
        }
        Ok(())
    }

    fn radial_cosine(&mut self, parameters: &[i32], check: &dyn Fn() -> Result<()>) -> Result<()> {
        let [center_x, center_y, wavelength, phase, amplitude, ..] = parameters else {
            return Ok(());
        };
        if *wavelength == 0 {
            return Ok(());
        }
        let mut scalar = vec![0_i64; (self.width as usize + 1) * (self.height as usize + 1)];
        let stride = self.width as usize + 1;
        for y in 0..=self.height {
            check()?;
            for x in 0..=self.width {
                let dx = f64::from(*center_x) - f64::from(x);
                let dy = f64::from(*center_y) - f64::from(y);
                let distance = (dx * dx + dy * dy).sqrt();
                scalar[y as usize * stride + x as usize] = ((1.0
                    - ((distance - f64::from(*phase)) * std::f64::consts::TAU
                        / f64::from(*wavelength))
                    .cos())
                    * f64::from(*amplitude))
                    as i64;
            }
        }
        self.gradient_from_scalar(&scalar, 65_536, check)?;
        Ok(())
    }

    fn perspective_bend(
        &mut self,
        parameters: &[i32],
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        let [center_x, center_y, angle_fixed, perspective, ..] = parameters else {
            return Ok(());
        };
        let angle = f64::from(*angle_fixed) * std::f64::consts::PI / (180.0 * 65_536.0);
        let sin_angle = angle.sin();
        let cos_angle = angle.cos();
        if cos_angle <= 0.0 {
            self.samples.fill([0, 0]);
            return Ok(());
        }
        let projected_height = f64::from(self.height) / cos_angle;
        for y in 0..self.height {
            check()?;
            let local_y = f64::from(y as i32 - *center_y);
            for x in 0..self.width {
                let local_x = f64::from(x as i32 - *center_x);
                let distance = (local_x * local_x + local_y * local_y).sqrt();
                let polar = local_y.atan2(local_x);
                let source_x = (1.0 - (180.0 - polar.to_degrees()) / 360.0)
                    * f64::from(16 * self.width.saturating_sub(1))
                    / 16.0;
                let denominator = f64::from(*perspective) + distance * sin_angle;
                let source_y = if denominator.abs() < f64::EPSILON {
                    f64::from(y)
                } else {
                    (projected_height + f64::from(*perspective)) * distance / denominator
                };
                self.set(
                    x,
                    y,
                    ((source_x - f64::from(x)) * 16.0) as i64,
                    ((source_y - f64::from(y)) * 16.0) as i64,
                );
            }
        }
        Ok(())
    }

    fn curvature(&mut self, parameters: &[i32], check: &dyn Fn() -> Result<()>) -> Result<()> {
        let [center_x, center_y, bend_angle_fixed, radius, ..] = parameters else {
            return Ok(());
        };
        if *bend_angle_fixed == 0 || *radius <= 0 {
            return Ok(());
        }
        let angle = f64::from(*bend_angle_fixed) * std::f64::consts::PI / (180.0 * 65_536.0);
        let sin_angle = angle.sin();
        let cos_angle = angle.cos();
        let radius = f64::from(*radius);
        for y in 0..self.height {
            check()?;
            let local_y = f64::from(y as i32 - *center_y);
            for x in 0..self.width {
                let local_x = f64::from(x as i32 - *center_x);
                let distance = (local_x * local_x + local_y * local_y).sqrt();
                if distance <= 0.0 || distance >= radius {
                    self.set(x, y, 0, 0);
                    continue;
                }
                let q = distance * sin_angle / radius;
                if q.abs() >= 1.0 {
                    self.set(x, y, 0, 0);
                    continue;
                }
                let root = (1.0 - q * q).sqrt();
                if root <= f64::EPSILON {
                    self.set(x, y, 0, 0);
                    continue;
                }
                let factor = (root - cos_angle) * q / root;
                self.set(
                    x,
                    y,
                    (-local_x * 16.0 * factor) as i64,
                    (-local_y * 16.0 * factor) as i64,
                );
            }
        }
        Ok(())
    }

    fn radial_lens(&mut self, parameters: &[i32], check: &dyn Fn() -> Result<()>) -> Result<()> {
        let [center_x, center_y, strength, radius, ..] = parameters else {
            return Ok(());
        };
        if *strength == 0 || *radius <= 0 {
            return Ok(());
        }
        let mut scalar = vec![0_i64; (self.width as usize + 1) * (self.height as usize + 1)];
        let stride = self.width as usize + 1;
        for y in 0..=self.height {
            check()?;
            for x in 0..=self.width {
                let dx = f64::from(*center_x) - f64::from(x);
                let dy = f64::from(*center_y) - f64::from(y);
                let distance = (dx * dx + dy * dy).sqrt();
                scalar[y as usize * stride + x as usize] = if distance >= f64::from(*radius) {
                    0
                } else {
                    (4_194_304.0
                        - (distance * std::f64::consts::PI / (2.0 * f64::from(*radius))).sin()
                            * 4_194_304.0) as i64
                };
            }
        }
        self.gradient_from_scalar(
            &scalar,
            (0x4000_0000_i64 / i64::from(*strength)).max(1),
            check,
        )?;
        Ok(())
    }

    fn wave(&mut self, parameters: &[i32], check: &dyn Fn() -> Result<()>) -> Result<()> {
        let [
            x_period,
            x_phase,
            x_amplitude,
            y_period,
            y_phase,
            y_amplitude,
            ..,
        ] = parameters
        else {
            return Ok(());
        };
        let (x_period, x_amplitude) = if *x_period == 0 {
            (1, 0)
        } else {
            (*x_period, *x_amplitude)
        };
        let (y_period, y_amplitude) = if *y_period == 0 {
            (1, 0)
        } else {
            (*y_period, *y_amplitude)
        };
        for y in 0..self.height {
            check()?;
            let dx = ((f64::from(y as i32 + *x_phase) * std::f64::consts::TAU
                / f64::from(x_period))
            .sin()
                * f64::from(x_amplitude)
                * 16.0) as i64;
            for x in 0..self.width {
                let dy = ((f64::from(x as i32 + *y_phase) * std::f64::consts::TAU
                    / f64::from(y_period))
                .sin()
                    * f64::from(y_amplitude)
                    * 16.0) as i64;
                self.set(x, y, dx, dy);
            }
        }
        Ok(())
    }

    fn radial_warp(&mut self, parameters: &[i32], check: &dyn Fn() -> Result<()>) -> Result<()> {
        let [
            source_origin_x,
            source_origin_y,
            warp_center_x,
            warp_center_y,
            radius_bias,
            ..,
        ] = parameters
        else {
            return Ok(());
        };
        let radius = (((*warp_center_x - *source_origin_x) as f64).powi(2)
            + ((*warp_center_y - *source_origin_y) as f64).powi(2))
        .sqrt()
            + f64::from(*radius_bias);
        if radius <= 0.0 {
            return Ok(());
        }
        for y in 0..self.height {
            check()?;
            let local_y = f64::from(y as i32 - *warp_center_y);
            for x in 0..self.width {
                let local_x = f64::from(x as i32 - *warp_center_x);
                let distance = (local_x * local_x + local_y * local_y).sqrt();
                let radial_scale = (distance * std::f64::consts::FRAC_PI_2 / radius).sin();
                let source_x = radial_scale * local_x + f64::from(*source_origin_x);
                let source_y = local_y + f64::from(*source_origin_y);
                self.set(
                    x,
                    y,
                    ((source_x - f64::from(x)) * 16.0) as i64,
                    ((source_y - f64::from(y)) * 16.0) as i64,
                );
            }
        }
        Ok(())
    }

    fn gradient_from_scalar(
        &mut self,
        scalar: &[i64],
        divisor: i64,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        let stride = self.width as usize + 1;
        for y in 0..self.height {
            check()?;
            for x in 0..self.width {
                let index = y as usize * stride + x as usize;
                let current = scalar[index];
                let right = scalar[index + 1];
                let below = scalar[index + stride];
                let dx =
                    (right.saturating_mul(right) - current.saturating_mul(current)) / (2 * divisor);
                let dy =
                    (below.saturating_mul(below) - current.saturating_mul(current)) / (2 * divisor);
                self.set(
                    x,
                    y,
                    dx.clamp(-i64::from(i16::MAX), i64::from(i16::MAX)),
                    dy.clamp(-i64::from(i16::MAX), i64::from(i16::MAX)),
                );
            }
        }
        Ok(())
    }

    fn set(&mut self, x: u32, y: u32, dx: i64, dy: i64) {
        self.samples[y as usize * self.width as usize + x as usize] = [
            dx.clamp(i16::MIN as i64, i16::MAX as i64) as i16,
            dy.clamp(i16::MIN as i64, i16::MAX as i64) as i16,
        ];
    }
}

fn mix32(mut value: u32) -> u32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68B);
    value ^ (value >> 16)
}

impl VectorMap {
    fn generate_radial(
        &mut self,
        mode: i32,
        center_x: i32,
        center_y: i32,
        period: i32,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        // sub_404CA0: 4 * period as unsigned, 0xFFFFFFFF for zero.
        let divisor = match (period as u32).wrapping_mul(4) {
            0 => u32::MAX,
            divisor => divisor,
        };
        for y in 0..self.height as i32 {
            check()?;
            for x in 0..self.width as i32 {
                let dx = x - center_x;
                let dy = y - center_y;
                let distance_sq = i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy);
                let distance = (distance_sq as f64).sqrt();
                let (base_x, base_y) = if distance == 0.0 {
                    (0_i16, 0_i16)
                } else {
                    (
                        (f64::from(dx) * 32767.0 / distance) as i16,
                        (f64::from(dy) * 32767.0 / distance) as i16,
                    )
                };
                let (vector_x, vector_y) = if mode == 0 {
                    (base_x, base_y)
                } else {
                    (base_y, base_x)
                };
                let raw_phase = (distance * 4.0) as u32;
                let phase = (raw_phase % divisor) as u16;
                self.samples[y as usize * self.width as usize + x as usize] = VectorSample {
                    x: vector_x,
                    y: vector_y,
                    phase,
                };
            }
        }
        Ok(())
    }

    fn generate_axis(&mut self, mode: i32, check: &dyn Fn() -> Result<()>) -> Result<()> {
        for y in 0..self.height {
            check()?;
            for x in 0..self.width {
                let (vector_x, vector_y, phase_axis) = match mode {
                    0 => (i16::MAX, 0, y),
                    1 => (0, i16::MAX, x),
                    2 => (i16::MAX, 0, x),
                    3 => (0, i16::MAX, y),
                    _ => unreachable!("validated System92:11 mode"),
                };
                self.samples[y as usize * self.width as usize + x as usize] = VectorSample {
                    x: vector_x,
                    y: vector_y,
                    phase: phase_axis.wrapping_mul(4) as u16,
                };
            }
        }
        Ok(())
    }
}

impl WaveTable {
    /// Construct bounded duplicated signed sine samples. Zero and negative
    /// dimensions retain the native minimum-one convention.
    pub fn compact(
        quarter_period: i32,
        amplitude: i32,
        block_height: i32,
        block_count: i32,
    ) -> Result<Self> {
        wave_budget(quarter_period, 0, 0, block_height, block_count)?;
        Ok(Self::compact_unchecked(
            quarter_period,
            amplitude,
            block_height,
            block_count,
        ))
    }
    pub fn extended(
        quarter_period: i32,
        amplitude: i32,
        lead: i32,
        trail: i32,
        block_height: i32,
        block_count: i32,
    ) -> Result<Self> {
        wave_budget(quarter_period, lead, trail, block_height, block_count)?;
        Ok(Self::extended_unchecked(
            quarter_period,
            amplitude,
            lead,
            trail,
            block_height,
            block_count,
        ))
    }
    pub fn signed_samples(&self, count: i32, offset: i32, alpha: i32) -> Result<Arc<[i16]>> {
        ensure!(count > 0, "wave distance count");
        let mut start = -4i64 * i64::from(offset);
        if start < 0 {
            start = self.block_span as i64 - ((-start) % self.block_span as i64);
        }
        let start = usize::try_from(start)?;
        let end = start
            .checked_add(
                (count as usize)
                    .checked_mul(4)
                    .ok_or_else(|| anyhow::anyhow!("wave sample count"))?,
            )
            .ok_or_else(|| anyhow::anyhow!("wave sample range"))?;
        let samples = self.samples.get(start..end).ok_or_else(|| {
            anyhow::anyhow!("wave table does not contain requested distance/offset")
        })?;
        Ok(samples
            .iter()
            .map(|s| (i32::from(s[0]).wrapping_mul(alpha) >> 8) as i16)
            .collect::<Vec<_>>()
            .into())
    }
}
fn wave_budget(period: i32, lead: i32, trail: i32, height: i32, count: i32) -> Result<()> {
    let samples = (period.max(1) as u64)
        .checked_mul(4)
        .and_then(|n| n.checked_mul(lead.max(0) as u64 + trail.max(0) as u64 + 1))
        .and_then(|n| n.checked_mul(height.max(1) as u64))
        .and_then(|n| n.checked_mul(count.max(1) as u64));
    ensure!(
        samples.is_some_and(|n| n <= 4_194_304),
        "wave table allocation budget"
    );
    Ok(())
}

pub fn warp(
    source: &DecodedImage,
    size: [u32; 2],
    spec: &FieldWarpSpec,
    check: &dyn Fn() -> Result<()>,
) -> Result<DecodedImage> {
    source.validate()?;
    crate::rgba_len(size[0], size[1])?;
    check()?;
    ensure!(
        u64::from(size[0]) * u64::from(size[1]) <= crate::effects::MAX_EFFECT_PIXELS,
        "field warp pixel budget"
    );
    let mut output = DecodedImage {
        width: size[0],
        height: size[1],
        rgba: vec![0; crate::rgba_len(size[0], size[1])?],
    };
    match spec {
        FieldWarpSpec::Displacement {
            first,
            second,
            amount,
            bilinear,
        } => {
            spec.validate(first.client, size)?;
            let VectorFieldSamples::Displacement(a_samples) = &first.samples else {
                anyhow::bail!("displacement field format")
            };
            let b_samples = second.as_ref().map(|s| {
                let VectorFieldSamples::Displacement(samples) = &s.samples else {
                    unreachable!("validated field format")
                };
                (s.size[0], samples)
            });
            let alpha = i32::from(*amount);
            for y in 0..size[1] {
                check()?;
                for x in 0..size[0] {
                    let a = a_samples[(y * first.size[0] + x) as usize];
                    let b = b_samples.map(|(w, samples)| samples[(y * w + x) as usize]);
                    let offset = std::array::from_fn::<_, 2, _>(|c| {
                        let a = a[c];
                        if *bilinear {
                            if let Some(b) = b {
                                if alpha == 256 {
                                    return i32::from(b[c]) >> 1;
                                }
                                if alpha == 0 {
                                    return i32::from(a) >> 1;
                                }
                                let base = a & !1;
                                ((i32::from(b[c].wrapping_sub(base)) * (alpha << 7)) >> 16)
                                    + (i32::from(base) >> 1)
                            } else if alpha == 256 {
                                i32::from(a) >> 1
                            } else {
                                (i32::from(a) * (alpha << 7)) >> 16
                            }
                        } else if let Some(b) = b {
                            let base = a & !3;
                            let delta = (i32::from(b[c].wrapping_sub(base)) * (alpha << 6)) >> 16;
                            ((delta + (i32::from(base) >> 2) + 2) as i16 as i32) >> 2
                        } else {
                            ((((i32::from(a) * (alpha << 5)) >> 16) + 1) as i16 as i32) >> 1
                        }
                    });
                    let color = if *bilinear {
                        crate::integer_raster::display::sample_background_pixel(
                            source,
                            x as i32 + (offset[0] >> 3),
                            y as i32 + (offset[1] >> 3),
                            offset[0] & 7,
                            offset[1] & 7,
                            8,
                        )
                    } else {
                        crate::integer_raster::display::sample_background_pixel(
                            source,
                            x as i32 + offset[0],
                            y as i32 + offset[1],
                            0,
                            0,
                            8,
                        )
                    };
                    output.rgba
                        [((y * size[0] + x) * 4) as usize..((y * size[0] + x) * 4 + 4) as usize]
                        .copy_from_slice(&color);
                }
            }
        }
        FieldWarpSpec::Wave {
            field,
            samples: waves,
            origin,
            wrap,
            bilinear,
        } => {
            spec.validate(field.client, size)?;
            let VectorFieldSamples::DirectionDistance(samples) = &field.samples else {
                anyhow::bail!("direction/distance field format")
            };
            let count = i64::from(source.width) * i64::from(source.height);
            let pixel = |index: i64| -> [u8; 4] {
                let index = if *wrap {
                    index.rem_euclid(count)
                } else if !(0..count).contains(&index) {
                    return [0; 4];
                } else {
                    index
                };
                source.rgba[index as usize * 4..index as usize * 4 + 4]
                    .try_into()
                    .unwrap()
            };
            for y in 0..size[1] {
                check()?;
                for x in 0..size[0] {
                    let vector = samples[(y * field.size[0] + x) as usize];
                    let wave = i32::from(*waves.get(vector.phase as usize).unwrap_or(&0));
                    let px = i32::from(vector.x) * wave;
                    let py = i32::from(vector.y) * wave;
                    let address = i64::from(y as i32 + origin[1] + (py >> 16))
                        * i64::from(source.width)
                        + i64::from(x as i32 + origin[0] + (px >> 16));
                    let mut color = pixel(address);
                    if *bilinear {
                        let fx = (px as u32 >> 12) & 15;
                        let fy = (py as u32 >> 12) & 15;
                        let right = pixel(address + 1);
                        let below = pixel(address + i64::from(source.width));
                        let diagonal = pixel(address + i64::from(source.width) + 1);
                        for c in 0..4 {
                            let mix = |a: u8, b: u8, t: u32| -> i32 {
                                i32::from(b)
                                    + ((i32::from(a) - i32::from(b)) * (16 - t) as i32 >> 4)
                            };
                            let top = mix(color[c], right[c], fx);
                            let bottom = mix(below[c], diagonal[c], fx);
                            color[c] = (bottom + ((top - bottom) * (16 - fy) as i32 >> 4))
                                .clamp(0, 255) as u8;
                        }
                    }
                    output.rgba
                        [((y * size[0] + x) * 4) as usize..((y * size[0] + x) * 4 + 4) as usize]
                        .copy_from_slice(&color);
                }
            }
        }
    }
    check()?;
    Ok(output)
}
pub fn pack(field: &VectorFieldResource, check: &dyn Fn() -> Result<()>) -> Result<Arc<[u8]>> {
    field.validate(field.client)?;
    let mut bytes = Vec::with_capacity(field.byte_len());
    match &field.samples {
        VectorFieldSamples::Displacement(samples) => {
            for row in samples.chunks(field.size[0] as usize) {
                check()?;
                for sample in row {
                    for value in sample {
                        bytes.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        VectorFieldSamples::DirectionDistance(samples) => {
            for row in samples.chunks(field.size[0] as usize) {
                check()?;
                for sample in row {
                    bytes.extend_from_slice(&sample.x.to_le_bytes());
                    bytes.extend_from_slice(&sample.y.to_le_bytes());
                    bytes.extend_from_slice(&sample.phase.to_le_bytes());
                }
            }
        }
    }
    Ok(bytes.into())
}
pub fn unpack(
    client: ClientId,
    size: [u32; 2],
    format: i32,
    bytes: &[u8],
    check: &dyn Fn() -> Result<()>,
) -> Result<Arc<VectorFieldResource>> {
    let template = blank(client, size, format)?;
    ensure!(
        bytes.len() == template.byte_len(),
        "packed field byte length"
    );
    let samples = match format {
        4 => {
            let mut samples = Vec::with_capacity(bytes.len() / 4);
            for row in bytes.chunks(size[0] as usize * 4) {
                check()?;
                for pixel in row.chunks_exact(4) {
                    samples.push([
                        i16::from_le_bytes(pixel[..2].try_into().unwrap()),
                        i16::from_le_bytes(pixel[2..4].try_into().unwrap()),
                    ]);
                }
            }
            VectorFieldSamples::Displacement(samples.into())
        }
        6 => {
            let mut samples = Vec::with_capacity(bytes.len() / 6);
            for row in bytes.chunks(size[0] as usize * 6) {
                check()?;
                for pixel in row.chunks_exact(6) {
                    samples.push(DirectionDistanceSample {
                        x: i16::from_le_bytes(pixel[..2].try_into().unwrap()),
                        y: i16::from_le_bytes(pixel[2..4].try_into().unwrap()),
                        phase: u16::from_le_bytes(pixel[4..6].try_into().unwrap()),
                    });
                }
            }
            VectorFieldSamples::DirectionDistance(samples.into())
        }
        _ => anyhow::bail!("packed field format"),
    };
    Ok(Arc::new(VectorFieldResource {
        client,
        size,
        samples,
    }))
}
