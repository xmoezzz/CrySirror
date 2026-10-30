//! Native effect declarations and sequence leases. Particle pools, integration,
//! atlas animation and mesh publication belong to the shared VFX worker.
use super::*;
use crysirror_vfx::{
    falling_atlas::{FallingAtlasSpec, SequenceExchange, SequenceRng, SimulationClock},
    screen::ScreenSpriteTarget,
    BlendMode, Burst, EmitterSpec, FacingMode, RenderSpec, SimulationSpace, SystemSpec,
};

const EFFECT_ENTITY: u32 = 0x3900_0000;
const EFFECT_LAYER: u32 = 0x3900_0010;
pub(super) struct NativeEffect {
    spec: SystemSpec,
    kind: u8,
    parameter: u8,
}
impl Bridge<'_> {
    fn world(&mut self, commands: Vec<Value>) -> Result<()> {
        self.host.submit_engine(
            self.client,
            1,
            &serde_json::to_vec(&json!({"world":commands}))?,
        )?;
        Ok(())
    }
    pub(super) fn effects_ready(&mut self) -> Result<bool> {
        let Some(effect) = &self.services.effects else {
            return Ok(true);
        };
        let SimulationClock::External { tick, .. } = effect.spec.clock else {
            bail!("Circus native effect clock")
        };
        let epoch = effect
            .spec
            .sequence_exchange
            .as_ref()
            .map_or(0, |e| e.epoch);
        let bytes = self.host.query_engine(
            self.client,
            EngineQuery::VfxSequence {
                entity: EFFECT_ENTITY,
                revision: effect.spec.revision,
                completed_tick: tick,
                exchange_epoch: epoch,
            },
        )?;
        let response: Value = serde_json::from_slice(&bytes)?;
        if response["phase"] == "pending" {
            return Ok(false);
        }
        ensure!(response["phase"] == "ready", "Circus VFX completion status");
        self.services.sequence_rng =
            Some(serde_json::from_value(response["sequence_rng"].clone())?);
        Ok(true)
    }
    pub(super) fn advance_effects(&mut self, now: u64) -> Result<()> {
        let Some(effect) = &mut self.services.effects else {
            return Ok(());
        };
        let SimulationClock::External { tick, wall_time_ms } = &mut effect.spec.clock else {
            bail!("Circus native effect clock")
        };
        *tick = tick
            .checked_add(1)
            .context("Circus native effect tick overflow")?;
        *wall_time_ms = now;
        let spec = serde_json::to_value(&effect.spec)?;
        self.world(vec![json!({"op":"set_vfx","id":EFFECT_ENTITY,"vfx":spec})])
    }
    pub(super) fn falling_effect(&mut self, kind: u8, parameter: u8) -> Result<bool> {
        ensure!(
            matches!(kind, 0 | 1),
            "Circus falling effect type {kind} needs verified mapping"
        );
        if !self.effects_ready()? {
            return Ok(false);
        }
        if kind == 0 {
            if self.services.effect_stopping {
                self.services.effect_stopping = false;
                return Ok(true);
            }
            if self.services.effects.is_some() {
                self.world(vec![json!({"op":"despawn","id":EFFECT_ENTITY})])?;
                self.services.effects = None;
                // Let engine-owned screen publication release its leases
                // before a following native screenshot/fade can inspect it.
                self.services.effect_stopping = true;
                return Ok(false);
            }
            return Ok(true);
        }
        if self.services.effect_starting {
            let effect = self
                .services
                .effects
                .as_ref()
                .context("Circus pending effect declaration")?;
            ensure!(
                effect.kind == kind && effect.parameter == parameter,
                "Circus effect completion declaration changed"
            );
            self.services.effect_starting = false;
            return Ok(true);
        }
        // Five original depth layers share ONE stream, in native41F300 order.
        // Additional simultaneously active native effect families are rejected.
        ensure!(
            self.services.effects.is_none(),
            "Circus replacing active effect requires a verified pool/reset mapping"
        );
        let image = self
            .services
            .images
            .get(&28)
            .context("Circus falling texture slot28 missing")?;
        let spec = native_spec(
            parameter,
            image.path.relative(),
            image.size,
            self.services
                .sequence_rng
                .clone()
                .context("Circus process random sequence missing")?,
            self.host.now_ms(),
        )?;
        self.root()?;
        self.world(vec![json!({"op":"spawn","entity":{"id":EFFECT_ENTITY,"name":"Circus native falling layers","vfx":spec}})])?;
        self.services.effects = Some(NativeEffect {
            spec,
            kind,
            parameter,
        });
        self.services.effect_starting = true;
        Ok(false)
    }
    pub(super) fn native_random(&mut self, modulus: u16) -> Result<Option<i32>> {
        ensure!(modulus > 0, "Circus native random modulo domain");
        if !self.effects_ready()? {
            return Ok(None);
        }
        if let Some((pending, value)) = self.services.random_exchange.take() {
            ensure!(
                pending == modulus,
                "Circus random completion instruction changed"
            );
            return Ok(Some(value));
        }
        let rng = self
            .services
            .sequence_rng
            .as_mut()
            .context("Circus process random sequence missing")?;
        let value = (rng.next_u32()? % u32::from(modulus)) as i32;
        if let Some(effect) = &mut self.services.effects {
            let SimulationClock::External { tick, .. } = effect.spec.clock else {
                bail!("Circus effect random clock")
            };
            let epoch = effect.spec.sequence_exchange.as_ref().map_or(Ok(1), |e| {
                e.epoch
                    .checked_add(1)
                    .context("Circus random epoch overflow")
            })?;
            effect.spec.sequence_exchange = Some(SequenceExchange {
                epoch,
                base_tick: tick,
                state: rng.clone(),
            });
            let spec = serde_json::to_value(&effect.spec)?;
            self.world(vec![json!({"op":"set_vfx","id":EFFECT_ENTITY,"vfx":spec})])?;
            self.services.random_exchange = Some((modulus, value));
            return Ok(None);
        }
        Ok(Some(value))
    }
}
fn native_spec(
    parameter: u8,
    texture: &str,
    extent: [u32; 2],
    rng: SequenceRng,
    now: u64,
) -> Result<SystemSpec> {
    // Native42A982 splits the two decimal density/wind components.41DAC0
    // clamps density to0..6 and constructs the original five pools.
    let tens = i32::from(parameter) / 10;
    let wind = if parameter < 100 {
        tens
    } else {
        -(i32::from(parameter) % 100 / 10)
    };
    let density = (i32::from(parameter) - 10 * tens).clamp(0, 6) as usize;
    // Native storm uses a different speed distribution on the first birth
    // than on reset. Preserve an explicit boundary until both are mapped.
    ensure!(
        density != 6,
        "Circus storm initial/reset speed distinction is not mapped"
    );
    let count = [0, 1, 3, 6, 9, 12, 20][density];
    let emitters = (0..=4)
        .rev()
        .map(|depth| {
            let mut emitter = EmitterSpec::default();
            emitter.id = format!("depth-{depth}");
            emitter.capacity = count.max(1);
            emitter.space = SimulationSpace::Local;
            emitter.spawn.rate = 0.0.into();
            emitter.spawn.bursts = if count == 0 {
                vec![]
            } else {
                vec![Burst { time: 0., count }]
            };
            emitter.spawn.burst_looping = false;
            emitter.screen = Some(ScreenSpriteTarget {
                canvas: [1280, 720],
                layer: EFFECT_LAYER + depth,
                parent: Some(SCENE_ROOT),
                order: [0, 100 + 4 - depth as i32],
                anchor: [0., 0.],
            });
            emitter.falling_atlas = Some(FallingAtlasSpec {
                canvas: [1280, 720],
                tick_hz: 60,
                x_bands: vec![0, -1, 1],
                fall_range: if density == 6 { [2, 5] } else { [2, 11] },
                fall_divisor: 1.5,
                sway_range: [28, 59],
                phase_modulus: 360,
                phase_step: 0.05,
                wind_step: wind as f32,
                rows: vec![depth, depth + 1],
                row_frames: vec![7, 7, 6, 8, 5, 8],
                cell: [28, 28],
                stride: [32, 28],
                texture_extent: extent,
                scale: 1. - 0.1 * depth as f32,
                alpha_base: 0.5,
                alpha_step: 0.1,
                alpha_count: 3,
                alpha_offset: -0.02 * depth as f32,
                atlas_interval_ms: 300,
                atlas_clock_bits: 32,
            });
            emitter.render = RenderSpec::Sprite {
                texture: Some(texture.into()),
                columns: 1,
                rows: 1,
                frames: 1,
                fps: 0.,
                soft_distance: 0.,
                blend: BlendMode::Alpha,
                facing: FacingMode::Camera,
            };
            emitter
        })
        .collect();
    let spec = SystemSpec {
        emitters,
        clock: SimulationClock::External {
            tick: 0,
            wall_time_ms: now,
        },
        sequence_rng: Some(rng),
        ..Default::default()
    };
    spec.compile()?;
    Ok(spec)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_five_depth_declaration_is_one_shared_system_in_original_order() {
        let s = native_spec(5, "authored.png", [256, 256], SequenceRng::msvc(7), 42).unwrap();
        assert_eq!(s.emitters.len(), 5);
        assert_eq!(
            s.emitters
                .iter()
                .map(|e| e.spawn.bursts[0].count)
                .sum::<u32>(),
            60
        );
        assert_eq!(s.emitters[0].id, "depth-4");
        assert_eq!(s.emitters[4].id, "depth-0");
        assert_eq!(s.emitters[0].falling_atlas.as_ref().unwrap().rows, [4, 5]);
        assert_eq!(s.sequence_rng, Some(SequenceRng::msvc(7)));
        assert!(native_spec(5, "authored.png", [128, 128], SequenceRng::msvc(7), 42).is_err());
    }
}
