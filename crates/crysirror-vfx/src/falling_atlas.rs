//! Engine-owned per-particle falling/periodic-drift/atlas node. It operates on
//! the existing VFX Particle and explicit clocks/random state; no VM, renderer
//! or second particle-world scheduler is embedded here.
use crate::Particle;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

/// Serializable integer sequence. One owner hands the stream to a worker and
/// receives the resulting state at completion; concurrent consumers must fence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "algorithm", rename_all = "snake_case", deny_unknown_fields)]
pub enum SequenceRng {
    /// Wrapping32-bit linear congruential stream with explicit output mapping.
    Lcg32 {
        /// Current32-bit sequence accumulator.
        state: u32,
        /// Wrapping multiplier applied before the additive increment.
        multiplier: u32,
        /// Wrapping additive increment for each consumed value.
        increment: u32,
        /// Right shift applied to the updated accumulator.
        output_shift: u8,
        /// Mask applied after the output shift.
        output_mask: u32,
    },
}
/// Simulation integration and wall-time sources are independent. External
/// ticks are counted since the current system revision started.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum SimulationClock {
    /// Existing engine-delta fixed-step behavior.
    #[default]
    Engine,
    /// Explicit caller-owned render ticks and independent monotonic wall time.
    External {
        /// Monotonic fixed-step count since the current system revision.
        tick: u64,
        /// Independent caller clock used by time-based presentation operators.
        wall_time_ms: u64,
    },
}
/// Fenced replacement of an engine-owned shared random stream. The emitter
/// pools are preserved; a completed receipt acknowledges this epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceExchange {
    /// Strictly increasing exchange identity within a system revision.
    pub epoch: u64,
    /// Completed external tick at which the stream was borrowed.
    pub base_tick: u64,
    /// State after the external consumer's own semantic random operations.
    pub state: SequenceRng,
}
/// Worker-completed state, never a copy of the last submitted declaration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemSequenceState {
    /// Capability owner of the completed worker state.
    pub client: u32,
    /// Stable owner-local system identity supplied to the particle world.
    pub system_key: u64,
    /// Authored system restart identity.
    pub revision: u64,
    /// Number of fixed simulation updates actually completed by the worker.
    pub completed_tick: u64,
    /// Latest fenced sequence exchange actually consumed by the worker.
    pub exchange_epoch: u64,
    /// Stream accumulator after completed initialization and update work.
    pub sequence_rng: SequenceRng,
}
impl SequenceRng {
    /// The published MSVC CRT integer-sequence contract, also usable by scripts.
    pub fn msvc(state: u32) -> Self {
        Self::Lcg32 {
            state,
            multiplier: 214013,
            increment: 2531011,
            output_shift: 16,
            output_mask: 0x7fff,
        }
    }
    /// Reject invalid shift/output domains before worker admission.
    pub fn validate(&self) -> Result<()> {
        let Self::Lcg32 {
            output_shift,
            output_mask,
            ..
        } = self;
        ensure!(
            *output_shift < 32 && *output_mask != 0,
            "VFX integer sequence output domain"
        );
        Ok(())
    }
    /// Advance exactly once; modulo sampling never substitutes a hash oracle.
    pub fn next_u32(&mut self) -> Result<u32> {
        self.validate()?;
        let Self::Lcg32 {
            state,
            multiplier,
            increment,
            output_shift,
            output_mask,
        } = self;
        *state = state.wrapping_mul(*multiplier).wrapping_add(*increment);
        Ok((*state >> *output_shift) & *output_mask)
    }
    fn modulo(&mut self, count: u32) -> Result<u32> {
        ensure!(count > 0, "VFX random modulo domain");
        Ok(self.next_u32()? % count)
    }
}
/// Data-driven pixel-motion and nonuniform-atlas operator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallingAtlasSpec {
    /// Spawn width and bottom reset plane in logical pixels.
    pub canvas: [u32; 2],
    /// Fixed ticks per second, defining published velocity/age units.
    pub tick_hz: u32,
    /// Chosen x-band offsets in multiples of canvas width, in sequence order.
    pub x_bands: Vec<i32>,
    /// Inclusive integer speed numerator sampled once per birth/reset.
    pub fall_range: [u32; 2],
    /// Converts the speed numerator into displacement per fixed update tick.
    pub fall_divisor: f32,
    /// Inclusive integer horizontal oscillation amplitude range.
    pub sway_range: [u32; 2],
    /// Initial phase is an integer in0..phase_modulus, interpreted as radians.
    pub phase_modulus: u32,
    /// Radians advanced once per explicit fixed tick.
    pub phase_step: f32,
    /// Center displacement per fixed tick, separate from the sinusoidal offset.
    pub wind_step: f32,
    /// Atlas row choices, sampled in this declared order.
    pub rows: Vec<u32>,
    /// Number of horizontal frames for every declared atlas row.
    pub row_frames: Vec<u32>,
    /// Unscaled source cell width/height in texture pixels.
    pub cell: [u32; 2],
    /// Horizontal/vertical distance between cell origins in texture pixels.
    pub stride: [u32; 2],
    /// Actual texture backing extent, including explicit engine padding.
    pub texture_extent: [u32; 2],
    /// Uniform destination scale; bottom cropping is applied before scaling.
    pub scale: f32,
    /// Alpha is base + step*(random % count) + offset.
    pub alpha_base: f32,
    /// Discrete alpha step.
    pub alpha_step: f32,
    /// Number of discrete alpha choices.
    pub alpha_count: u32,
    /// Final alpha offset, useful for authored depth attenuation.
    pub alpha_offset: f32,
    /// Atlas advances by at most one frame only when elapsed exceeds this.
    pub atlas_interval_ms: u64,
    /// Timer arithmetic:32 for wrapping millisecond clocks,64 for monotonic.
    pub atlas_clock_bits: u8,
}
impl FallingAtlasSpec {
    /// Validate the bounded node before allocating a particle pool.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=240).contains(&self.tick_hz),
            "VFX falling fixed tick rate"
        );
        ensure!(
            self.canvas.iter().all(|&n| (1..=16384).contains(&n))
                && !self.x_bands.is_empty()
                && self.x_bands.len() <= 32
                && self.x_bands.iter().all(|n| n.unsigned_abs() <= 16),
            "VFX falling spawn domain"
        );
        for range in [self.fall_range, self.sway_range] {
            ensure!(
                range[0] <= range[1] && range[1] <= 65535,
                "VFX falling integer range"
            );
        }
        ensure!(
            self.fall_range[0] > 0
                && self.fall_divisor.is_finite()
                && self.fall_divisor > 0.
                && self.fall_range[1] as f32 / self.fall_divisor <= self.canvas[1] as f32 * 2.,
            "VFX falling speed"
        );
        ensure!(
            (1..=65536).contains(&self.phase_modulus)
                && self.phase_step.is_finite()
                && self.phase_step.abs() <= 10.
                && self.wind_step.is_finite()
                && self.wind_step.abs() <= 16384.,
            "VFX falling phase/wind"
        );
        ensure!(
            !self.rows.is_empty()
                && self.rows.len() <= 256
                && !self.row_frames.is_empty()
                && self.row_frames.len() <= 256
                && self.row_frames.iter().all(|n| (1..=256).contains(n)),
            "VFX falling atlas row counts"
        );
        ensure!(
            self.cell
                .iter()
                .chain(&self.stride)
                .chain(&self.texture_extent)
                .all(|&n| (1..=16384).contains(&n))
                && self.scale.is_finite()
                && (0.0001..=16.).contains(&self.scale),
            "VFX falling atlas geometry"
        );
        for &row in &self.rows {
            let &frames = self
                .row_frames
                .get(row as usize)
                .context("VFX falling row index")?;
            ensure!(
                u64::from(frames - 1) * u64::from(self.stride[0]) + u64::from(self.cell[0])
                    <= u64::from(self.texture_extent[0])
                    && u64::from(row) * u64::from(self.stride[1]) + u64::from(self.cell[1])
                        <= u64::from(self.texture_extent[1]),
                "VFX falling atlas backing extent"
            );
        }
        ensure!(
            (1..=256).contains(&self.alpha_count)
                && [self.alpha_base, self.alpha_step, self.alpha_offset]
                    .iter()
                    .all(|n| n.is_finite()),
            "VFX falling alpha domain"
        );
        for choice in [0, self.alpha_count - 1] {
            ensure!(
                (0.0..=1.).contains(
                    &(self.alpha_base + self.alpha_step * choice as f32 + self.alpha_offset)
                ),
                "VFX falling alpha range"
            );
        }
        ensure!(
            (1..=600000).contains(&self.atlas_interval_ms)
                && matches!(self.atlas_clock_bits, 32 | 64),
            "VFX falling atlas clock"
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> FallingAtlasSpec {
        FallingAtlasSpec {
            canvas: [32, 12],
            tick_hz: 60,
            x_bands: vec![0, -1, 1],
            fall_range: [2, 4],
            fall_divisor: 2.,
            sway_range: [0, 3],
            phase_modulus: 5,
            phase_step: 0.05,
            wind_step: 0.25,
            rows: vec![0, 1],
            row_frames: vec![2, 3],
            cell: [4, 4],
            stride: [5, 4],
            texture_extent: [16, 8],
            scale: 0.5,
            alpha_base: 0.5,
            alpha_step: 0.1,
            alpha_count: 3,
            alpha_offset: 0.,
            atlas_interval_ms: 300,
            atlas_clock_bits: 32,
        }
    }
    fn particle() -> Particle {
        crate::spawn_particle(
            &crate::EmitterSpec::default()
                .compile(&Default::default())
                .unwrap(),
            1,
            glam::Mat4::IDENTITY.to_cols_array_2d(),
        )
        .unwrap()
    }
    #[test]
    fn msvc_single_stream_matches_crt_and_roundtrips_without_particle_resimulation() {
        let mut rng = SequenceRng::msvc(1);
        let values = (0..8).map(|_| rng.next_u32().unwrap()).collect::<Vec<_>>();
        assert_eq!(values, [41, 18467, 6334, 26500, 19169, 15724, 11478, 29358]);
        let encoded = serde_json::to_vec(&rng).unwrap();
        let mut resumed: SequenceRng = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(rng.next_u32().unwrap(), resumed.next_u32().unwrap());
        let mut bad = SequenceRng::Lcg32 {
            state: 0,
            multiplier: 1,
            increment: 1,
            output_shift: 32,
            output_mask: 1,
        };
        assert!(bad.next_u32().is_err());
    }
    #[test]
    fn atlas_wall_clock_does_not_create_motion_or_animation_catchup_frames() {
        let node = CompiledFallingAtlas::compile(spec()).unwrap();
        let mut rng = SequenceRng::msvc(1);
        let mut particle = particle();
        let mut state = node.initialize(&mut particle, &mut rng, 0).unwrap();
        let first = state.frame;
        let row = state.row;
        let y = particle.position[1];
        let speed = state.fall_step;
        node.step(&mut particle, &mut state, &mut rng, 300, true)
            .unwrap();
        assert_eq!(state.frame, first);
        node.step(&mut particle, &mut state, &mut rng, 301, true)
            .unwrap();
        assert_eq!(state.frame, (first + 1) % spec().row_frames[row as usize]);
        node.step(&mut particle, &mut state, &mut rng, 100000, true)
            .unwrap();
        assert_eq!(state.frame, (first + 2) % spec().row_frames[row as usize]);
        assert!((particle.position[1] - y - speed * 3.).abs() < 0.0001);
    }
    #[test]
    fn crop_precedes_scale_reset_retains_clock_and_stopping_drains() {
        let node = CompiledFallingAtlas::compile(spec()).unwrap();
        let mut rng = SequenceRng::msvc(9);
        let mut particle = particle();
        let mut state = node.initialize(&mut particle, &mut rng, 77).unwrap();
        state.fall_step = 1.;
        particle.position[1] = 10.;
        let frame = node
            .step(&mut particle, &mut state, &mut rng, 200, true)
            .unwrap();
        assert_eq!(particle.position[1], 11.);
        assert_eq!(particle.size[1], 0.5);
        assert_eq!(frame.uv[3] - frame.uv[1], 0.125);
        particle.position[1] = 12.;
        let frame = node
            .step(&mut particle, &mut state, &mut rng, 200, true)
            .unwrap();
        assert!(frame.recycled && particle.position[1] <= 0.);
        assert_eq!(frame.uv[0], 0.);
        assert_eq!(state.atlas_at_ms, 77);
        particle.position[1] = 13.;
        let before = rng.clone();
        let frame = node
            .step(&mut particle, &mut state, &mut rng, 200, false)
            .unwrap();
        assert!(!frame.alive);
        assert_eq!(rng, before);
    }
    #[test]
    fn wrapping_timer_and_saved_auxiliary_state_continue_identically() {
        let node = CompiledFallingAtlas::compile(spec()).unwrap();
        let mut rng = SequenceRng::msvc(99);
        let mut particle = particle();
        let mut state = node
            .initialize(&mut particle, &mut rng, u64::from(u32::MAX) - 100)
            .unwrap();
        let frame = state.frame;
        node.step(&mut particle, &mut state, &mut rng, 250, true)
            .unwrap();
        assert_eq!(
            state.frame,
            (frame + 1) % spec().row_frames[state.row as usize]
        );
        let bytes = serde_json::to_vec(&(particle.clone(), state.clone(), rng.clone())).unwrap();
        let (mut p2, mut s2, mut r2): (Particle, FallingAtlasState, SequenceRng) =
            serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            node.step(&mut particle, &mut state, &mut rng, 600, true)
                .unwrap(),
            node.step(&mut p2, &mut s2, &mut r2, 600, true).unwrap()
        );
        assert_eq!(particle, p2);
        assert_eq!(state, s2);
        assert_eq!(rng, r2);
    }
    #[test]
    fn birth_draws_first_cell_but_retains_the_sampled_animation_counter() {
        let node = CompiledFallingAtlas::compile(spec()).unwrap();
        let mut rng = SequenceRng::msvc(1);
        let mut p = particle();
        let mut state = node.initialize(&mut p, &mut rng, 0).unwrap();
        state.frame = 1;
        assert_eq!(node.initial_uv(&state)[0], 0.);
        let result = node.step(&mut p, &mut state, &mut rng, 0, true).unwrap();
        assert_eq!(result.uv[0], 5. / 16.);
        assert_eq!(state.frame, 1);
    }
}
/// Auxiliary typed attributes beside one existing VFX particle slot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallingAtlasState {
    pub center_x: f32,
    pub phase: f32,
    pub amplitude: u32,
    pub fall_step: f32,
    pub row: u32,
    pub frame: u32,
    pub atlas_at_ms: u64,
}
/// Result of one fixed update, including its immutable source UV rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct FallingStep {
    pub alive: bool,
    pub recycled: bool,
    pub uv: [f32; 4],
}
/// Validated node; compilation is separate from the per-particle hot path.
#[derive(Clone, Debug)]
pub struct CompiledFallingAtlas {
    spec: FallingAtlasSpec,
}
impl CompiledFallingAtlas {
    pub fn compile(spec: FallingAtlasSpec) -> Result<Self> {
        spec.validate()?;
        Ok(Self { spec })
    }
    fn range(rng: &mut SequenceRng, range: [u32; 2]) -> Result<u32> {
        Ok(range[0] + rng.modulo(range[1] - range[0] + 1)?)
    }
    fn clock(&self, ms: u64) -> u64 {
        if self.spec.atlas_clock_bits == 32 {
            u64::from(ms as u32)
        } else {
            ms
        }
    }
    /// Birth/reset consumes one shared integer stream in a stable documented
    /// order: center,band,y,speed,amplitude,row,phase,frame,alpha. The atlas
    /// clock is retained on reset and initially zero, independent of birth time.
    pub fn initialize(
        &self,
        particle: &mut Particle,
        rng: &mut SequenceRng,
        previous_clock: u64,
    ) -> Result<FallingAtlasState> {
        let s = &self.spec;
        let x = rng.modulo(s.canvas[0])? as f32;
        let band = s.x_bands[rng.modulo(s.x_bands.len() as u32)? as usize];
        let x = x + band as f32 * s.canvas[0] as f32;
        let y = -(rng.modulo(s.canvas[1])? as f32);
        let speed = Self::range(rng, s.fall_range)? as f32 / s.fall_divisor;
        let amplitude = Self::range(rng, s.sway_range)?;
        let row = s.rows[rng.modulo(s.rows.len() as u32)? as usize];
        let phase = rng.modulo(s.phase_modulus)? as f32;
        let frame = rng.modulo(s.row_frames[row as usize])?;
        let alpha = ((f64::from(s.alpha_base)
            + f64::from(s.alpha_step) * f64::from(rng.modulo(s.alpha_count)?))
            as f32)
            + s.alpha_offset;
        particle.position = [x, y, 0.];
        particle.previous_position = particle.position;
        particle.velocity = [s.wind_step * s.tick_hz as f32, speed * s.tick_hz as f32, 0.];
        particle.size = [s.cell[0] as f32 * s.scale, s.cell[1] as f32 * s.scale];
        particle.color[3] = alpha;
        particle.age = 0.;
        particle.lifetime = f32::MAX;
        Ok(FallingAtlasState {
            center_x: x,
            phase,
            amplitude,
            fall_step: speed,
            row,
            frame,
            atlas_at_ms: self.clock(previous_clock),
        })
    }
    fn uv(&self, state: &FallingAtlasState, height: u32) -> [f32; 4] {
        let s = &self.spec;
        let x = state.frame * s.stride[0];
        let y = state.row * s.stride[1];
        [
            x as f32 / s.texture_extent[0] as f32,
            y as f32 / s.texture_extent[1] as f32,
            (x + s.cell[0]) as f32 / s.texture_extent[0] as f32,
            (y + height) as f32 / s.texture_extent[1] as f32,
        ]
    }
    /// One native/fixed tick. The supplied wall clock drives only the atlas;
    /// skipped wall time never creates extra position integration or atlas
    /// catch-up frames. `emitting=false` drains particles at the bottom plane.
    pub fn step(
        &self,
        particle: &mut Particle,
        state: &mut FallingAtlasState,
        rng: &mut SequenceRng,
        wall_ms: u64,
        emitting: bool,
    ) -> Result<FallingStep> {
        let s = &self.spec;
        ensure!(
            state.row < (s.row_frames.len() as u32)
                && state.frame < s.row_frames[state.row as usize]
                && [state.center_x, state.phase, state.fall_step]
                    .iter()
                    .all(|n| n.is_finite()),
            "VFX falling particle state"
        );
        let now = self.clock(wall_ms);
        let elapsed = if s.atlas_clock_bits == 32 {
            u64::from((now as u32).wrapping_sub(state.atlas_at_ms as u32))
        } else {
            now.checked_sub(state.atlas_at_ms)
                .context("VFX falling wall clock moved backwards")?
        };
        let sine = f64::from(state.phase).sin() as f32;
        particle.previous_position = particle.position;
        particle.position[0] =
            (f64::from(sine) * f64::from(state.amplitude) + f64::from(state.center_x)) as f32;
        state.center_x += s.wind_step;
        state.phase += s.phase_step;
        if elapsed > s.atlas_interval_ms {
            state.frame = (state.frame + 1) % s.row_frames[state.row as usize];
            state.atlas_at_ms = now;
        }
        particle.position[1] += state.fall_step;
        particle.velocity = [
            s.wind_step * s.tick_hz as f32,
            state.fall_step * s.tick_hz as f32,
            0.,
        ];
        particle.age += 1. / s.tick_hz as f32;
        if particle.position[1] > s.canvas[1] as f32 {
            if !emitting {
                return Ok(FallingStep {
                    alive: false,
                    recycled: false,
                    uv: self.uv(state, 0),
                });
            }
            *state = self.initialize(particle, rng, state.atlas_at_ms)?;
            return Ok(FallingStep {
                alive: true,
                recycled: true,
                uv: self.initial_uv(state),
            });
        }
        let height = if particle.position[1] + s.cell[1] as f32 > s.canvas[1] as f32 {
            (s.canvas[1] as f32 - particle.position[1]).max(0.) as u32
        } else {
            s.cell[1]
        };
        particle.size = [s.cell[0] as f32 * s.scale, height as f32 * s.scale];
        Ok(FallingStep {
            alive: true,
            recycled: false,
            uv: self.uv(state, height),
        })
    }
    /// UV immediately after birth, before a fixed update. The sampled animation
    /// counter remains independent; a newly initialized sprite displays cell0.
    pub fn initial_uv(&self, state: &FallingAtlasState) -> [f32; 4] {
        let mut initial = state.clone();
        initial.frame = 0;
        self.uv(&initial, self.spec.cell[1])
    }
}
