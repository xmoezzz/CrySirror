//! Native effect declarations and sequence leases. Particle pools, integration,
//! atlas animation and mesh publication belong to the shared VFX worker.
use super::*;
use crysirror_vfx::{
    BlendMode, Burst, EmitterSpec, FacingMode, RenderSpec, SimulationSpace, SystemSpec,
    falling_atlas::{FallingAtlasSpec, SequenceExchange, SequenceRng, SimulationClock},
    screen::ScreenSpriteTarget,
};

const EFFECT_ENTITY: u32 = 0x3900_0000;
const EFFECT_LAYER: u32 = 0x3900_0010;
pub(super) struct NativeEffect {
    pub(super) spec: SystemSpec,
    kind: i16,
    parameter: i16,
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
        if !self.framebuffer_ready()? {
            return Ok(false);
        }
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
    pub(super) fn falling_effect(&mut self, kind: i16, parameter: i16) -> Result<bool> {
        if matches!(kind, 7 | 16) {
            return self.falling_scene_special(kind, parameter);
        }
        if !self.effects_ready()? {
            return Ok(false);
        }
        let dc3 = self.services.native_profile == Some(crate::Profile::Dc3WithYou);
        //DC3WY 419BC0: 4185D0 (sakura) and 4187E0 (kind 3, type 5; 419300
        //is the shared falling-glow step on the 1024x576 canvas) are
        //connected; 2/13/14/15 use 418A00/418DA0/418BA0/419040.
        //Kind 2 (418A00, type 2) is the streak step with 1 x 56*row cells
        //from slot 23 (this+1680).
        ensure!(!dc3 || matches!(kind, 0..=12) || kind >= 16, "DC3WY 419BC0 weather kind {kind} is not connected");
        //40EEC0 selects the pool wrapper even for the empty kind0 profile;
        //density and texture presence do not select the primary sprite back.
        self.mark_particle_wrappers(if dc3 {&[6,19,29,39,48]} else if kind==15 {&[59]}else{&[6,19,29,39,59]})?;
        if kind == 0 {
            if self.services.effect_stopping {
                self.services.effect_stopping = false;
                self.services.effect_control.clear_pool_controls();
                return Ok(true);
            }
            if let Some(previous) = &self.services.effects {
                if previous.kind == 0 {
                    self.services.effect_control.clear_pool_controls();
                    return Ok(true);
                }
                //41DAC0(0) clears descriptors but keeps the twenty native
                //sprite objects per pool. In particular rain inherits their
                //scale after a later restart; despawning loses that state.
                let mut spec = previous.spec.clone();
                for emitter in &mut spec.emitters {
                    emitter.revision = emitter
                        .revision
                        .checked_add(1)
                        .context("Circus cleared pool revision")?;
                    emitter.spawn.bursts.clear();
                    emitter.spawn.rate = 0.0.into();
                    emitter.screen = None;
                    emitter.falling_atlas = None;
                    emitter.orbit_atlas = None;
                    emitter.drifting_atlas = None;
                    if let RenderSpec::Sprite { texture, .. } = &mut emitter.render {
                        *texture = None;
                    }
                }
                let SimulationClock::External { tick, .. } = spec.clock else {
                    bail!("Circus cleared pool clock");
                };
                let epoch = spec.sequence_exchange.as_ref().map_or(Ok(1), |e| {
                    e.epoch.checked_add(1).context("Circus cleared pool epoch")
                })?;
                spec.sequence_exchange = Some(SequenceExchange {
                    epoch,
                    base_tick: tick,
                    state: self
                        .services
                        .sequence_rng
                        .clone()
                        .context("Circus cleared pool sequence")?,
                });
                self.world(vec![json!({"op":"set_vfx","id":EFFECT_ENTITY,"vfx":spec})])?;
                self.services.effects = Some(NativeEffect {
                    spec,
                    kind: 0,
                    parameter: 0,
                });
                self.services.effect_stopping = true;
                return Ok(false);
            }
            self.services.effect_control.clear_pool_controls();
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
        // These three profiles bind their fixed image before constructing any
        // pool (4284B0 case40); ordinary/rain pools use the existing resource.
        let built_in = match kind {
            13 => Some((28, "system\\effyousei")),
            15 => Some((30, "system\\efflens")),
            17 => Some((28, "system\\effhotaru")),
            _ => None,
        };
        //425640 reconstructs pools from the already restored resource handles;
        //in particular a saved hotaru texture must not turn into effyousei just
        //because that reconstruction invokes the common kind13 initializer.
        if let Some((slot, name)) = built_in.filter(|_| self.services.saves.restore.is_none()) {
            if !self.emit(&Event::LoadImage {
                slot,
                name: name.into(),
            })? {
                return Ok(false);
            }
        }
        let image = self.services.images.get(&if dc3 {
            //4233A0 0x27 passes this+1680 (slot 23) for kind 2, otherwise
            //this+1676 (slot 22).
            if kind == 2 { 23 } else { 22 }
        } else if kind == 15 {
            30
        } else if kind == 2 {
            29
        } else {
            28
        });
        let rng = self
            .services
            .sequence_rng
            .clone()
            .context("Circus process random sequence missing")?;
        let empty_density = if dc3 { parameter.clamp(0, 6) == 0 } else { (parameter % 10).clamp(0, 6) == 0 };
        let mut spec = if empty_density {
            //41DAC0 stores the passed handle, clears its twenty descriptors,
            //then exits before any texture bind/dimension read at density0.
            //Keep native pool objects (and retained scale), consume no RNG,
            //and permit the original null texture on this empty branch.
            let mut empty = empty_pools("", rng.clone(), self.host.now_ms());
            for emitter in &mut empty.emitters {
                emitter.screen = None;
                if let RenderSpec::Sprite { texture, .. } = &mut emitter.render {
                    *texture = None;
                }
            }
            if kind == 15 {
                let mut spec = self
                    .services
                    .effects
                    .as_ref()
                    .map(|effect| effect.spec.clone())
                    .unwrap_or_else(|| empty.clone());
                spec.emitters[0] = empty.emitters.remove(0);
                spec
            } else {
                empty
            }
        } else if kind == 15 {
            let image = image.context("Circus particle texture missing")?;
            let emitter = native_orbit_emitter(
                parameter,
                image.path.relative(),
                image.size,
                self.services.effect_control.orbit_control(),
            )?;
            let mut spec = if let Some(effect) = &self.services.effects {
                effect.spec.clone()
            } else {
                empty_pools(image.path.relative(), rng.clone(), self.host.now_ms())
            };
            spec.emitters[0] = emitter;
            spec
        } else if matches!(kind, 2 | 3 | 13 | 14 | 17) {
            let image = image.context("Circus particle texture missing")?;
            drifting_spec(
                kind,
                parameter,
                image.path.relative(),
                image.size,
                rng.clone(),
                self.host.now_ms(),
                dc3,
            )?
        } else {
            let image = image.context("Circus particle texture missing")?;
            native_spec(
                parameter,
                image.path.relative(),
                image.size,
                rng.clone(),
                self.host.now_ms(),
                dc3,
            )?
        };
        for emitter in &mut spec.emitters {
            if let Some(node) = &mut emitter.falling_atlas {
                node.spawn_window = self.services.effect_control.spawn_window();
            }
            if let Some(node) = &mut emitter.drifting_atlas {
                node.spawn_window = self.services.effect_control.drift_window();
            }
            let bypass = emitter.drifting_atlas.as_ref().is_some_and(|node| {
                matches!(
                    node.motion,
                    crysirror_vfx::drifting_atlas::DriftMotion::Buoyant
                        | crysirror_vfx::drifting_atlas::DriftMotion::FallingGlow
                )
            });
            emitter.paused = !self.services.effect_control.effects_enabled() && !bypass;
        }
        let replacing = self.services.effects.is_some();
        if let Some(previous) = &self.services.effects {
            ensure!(
                previous.spec.emitters.len() == 5 && spec.emitters.len() == 5,
                "Circus native particle pool count"
            );
            for (index, emitter) in spec.emitters.iter_mut().enumerate() {
                if kind != 15 || index == 0 {
                    emitter.revision = previous.spec.emitters[index]
                        .revision
                        .checked_add(1)
                        .context("Circus particle pool revision overflow")?;
                }
            }
            let SimulationClock::External { tick, .. } = previous.spec.clock else {
                bail!("Circus particle pool clock")
            };
            let epoch = previous
                .spec
                .sequence_exchange
                .as_ref()
                .map_or(Ok(1), |exchange| {
                    exchange
                        .epoch
                        .checked_add(1)
                        .context("Circus particle pool exchange overflow")
                })?;
            spec.clock = previous.spec.clock.clone();
            spec.revision = previous.spec.revision;
            spec.sequence_rng = previous.spec.sequence_rng.clone();
            spec.sequence_exchange = Some(SequenceExchange {
                epoch,
                base_tick: tick,
                state: rng,
            });
        }
        spec.compile()?;
        self.root()?;
        if replacing {
            self.world(vec![json!({"op":"set_vfx","id":EFFECT_ENTITY,"vfx":spec})])?;
        } else {
            self.world(vec![json!({"op":"spawn","entity":{"id":EFFECT_ENTITY,"name":"Circus native particle pools","vfx":spec}})])?;
        }
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
    parameter: i16,
    texture: &str,
    extent: [u32; 2],
    rng: SequenceRng,
    now: u64,
    dc3: bool,
) -> Result<SystemSpec> {
    // Native42A982 splits the two decimal density/wind components.41DAC0
    // clamps density to0..6 and constructs the original five pools.
    // DC3WY 419BC0 clamps the operand itself (no wind) and 4185D0 samples
    // x%1024, -(y%576), window, (r%8+1)/1.5 (r%2+1 at density 6, first birth
    // only), sway r%32+28, row depth+r%2, phase r%360, frame; scale 100%,
    // alpha 0.8; 419300 advances the atlas after 500 ms.
    let tens = if dc3 { 0 } else { i32::from(parameter) / 10 };
    let wind = if dc3 {
        0
    } else if parameter < 100 {
        tens
    } else {
        -(i32::from(parameter) % 100 / 10)
    };
    let density = (i32::from(parameter) - 10 * tens).clamp(0, 6) as usize;
    let canvas = if dc3 { [1024, 576] } else { [1280, 720] };
    let count = [0, 1, 3, 6, 9, 12, 20][density];
    let emitters = (0..=4)
        .rev()
        .map(|depth| {
            let mut emitter = EmitterSpec::default();
            emitter.id = format!("depth-{depth}");
            emitter.capacity = 20;
            emitter.space = SimulationSpace::Local;
            emitter.spawn.rate = 0.0.into();
            emitter.spawn.bursts = if count == 0 {
                vec![]
            } else {
                vec![Burst { time: 0., count }]
            };
            emitter.spawn.burst_looping = false;
            emitter.screen = Some(ScreenSpriteTarget {
                canvas,
                layer: EFFECT_LAYER + depth,
                parent: Some(SCENE_ROOT),
                order: pool_order(depth, dc3),
                anchor: [0., 0.],
            });
            emitter.falling_atlas = Some(FallingAtlasSpec {
                canvas,
                tick_hz: 60,
                x_bands: if dc3 { vec![] } else { vec![0, -1, 1] },
                fall_range: if dc3 { [1, 8] } else { [2, 11] },
                initial_fall_range: match (density == 6, dc3) {
                    (true, true) => Some([1, 2]),
                    (true, false) => Some([2, 5]),
                    _ => None,
                },
                spawn_window: None,
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
                scale: if dc3 { 1. } else { 1. - 0.1 * depth as f32 },
                alpha_base: if dc3 { 0.8 } else { 0.5 },
                alpha_step: 0.1,
                alpha_count: if dc3 { 0 } else { 3 },
                alpha_offset: if dc3 { 0. } else { -0.02 * depth as f32 },
                atlas_interval_ms: if dc3 { 500 } else { 300 },
                atlas_clock_bits: 32,
            });
            emitter.render = RenderSpec::Sprite {
                texture: Some(texture.into()),
                columns: 1,
                rows: 1,
                frames: 1,
                fps: 0.,
                soft_distance: 0.,
                triplanar_coverage: None,
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
fn pool_order(depth: u32, dc3: bool) -> [i32; 2] {
    //41F300 binds depth4/3/2/1/0 to wrappers6/19/29/39/59 (DC3WY 41A5D0:
    //objects 6/19/29/39/48). Use the same painter partitions as the
    //ordinary native object renderer.
    let object = if dc3 { [48, 39, 29, 19, 6] } else { [59, 39, 29, 19, 6] }[depth as usize];
    if object < 10 {
        [0, object]
    } else {
        [1, 50 * 64 + object]
    }
}
fn drifting_spec(
    kind: i16,
    parameter: i16,
    texture: &str,
    extent: [u32; 2],
    rng: SequenceRng,
    now: u64,
    dc3: bool,
) -> Result<SystemSpec> {
    let mut spec = empty_pools(texture, rng, now);
    let canvas = if dc3 { [1024, 576] } else { [1280, 720] };
    //DC3WY 419BC0 clamps the operand itself; DC4 splits density/wind.
    let density = if dc3 { parameter.clamp(0, 6) } else { (parameter % 10).clamp(0, 6) };
    let count = [0, 1, 3, 6, 9, 12, 20][density as usize];
    for (index, emitter) in spec.emitters.iter_mut().enumerate() {
        let depth = 4 - index as u32;
        emitter.enabled = true;
        emitter.capacity = 20;
        emitter.spawn.bursts = if count == 0 {
            vec![]
        } else {
            vec![Burst { time: 0., count }]
        };
        emitter.screen = Some(ScreenSpriteTarget {
            canvas,
            layer: EFFECT_LAYER + depth,
            parent: Some(SCENE_ROOT),
            order: pool_order(depth, dc3),
            anchor: [0., 0.],
        });
        emitter.drifting_atlas = Some(crysirror_vfx::drifting_atlas::DriftingAtlasSpec {
            canvas,
            texture_extent: extent,
            tick_hz: 60,
            depth,
            streak_cell: (dc3 && kind == 2).then_some([1, 56, 0]),
            motion: match kind {
                2 => crysirror_vfx::drifting_atlas::DriftMotion::Streak,
                3 => crysirror_vfx::drifting_atlas::DriftMotion::FallingGlow,
                14 => crysirror_vfx::drifting_atlas::DriftMotion::RisingGlow,
                _ => crysirror_vfx::drifting_atlas::DriftMotion::Buoyant,
            },
            spawn_window: None,
        });
        emitter.render = RenderSpec::Sprite {
            texture: Some(texture.into()),
            columns: 1,
            rows: 1,
            frames: 1,
            fps: 0.,
            soft_distance: 0.,
            triplanar_coverage: None,
            blend: BlendMode::Alpha,
            facing: FacingMode::Camera,
        };
    }
    spec.compile()?;
    Ok(spec)
}
fn empty_pools(texture: &str, rng: SequenceRng, now: u64) -> SystemSpec {
    let mut emitters = Vec::new();
    for depth in (0..=4).rev() {
        let mut emitter = EmitterSpec::default();
        emitter.id = format!("depth-{depth}");
        emitter.enabled = false;
        emitter.capacity = 20;
        emitter.space = SimulationSpace::Local;
        emitter.spawn.rate = 0.0.into();
        emitter.screen = Some(ScreenSpriteTarget {
            canvas: [1280, 720],
            layer: EFFECT_LAYER + depth,
            parent: Some(SCENE_ROOT),
            order: pool_order(depth, false),
            anchor: [0., 0.],
        });
        emitter.render = RenderSpec::Sprite {
            texture: Some(texture.into()),
            columns: 1,
            rows: 1,
            frames: 1,
            fps: 0.,
            soft_distance: 0.,
            triplanar_coverage: None,
            blend: BlendMode::Alpha,
            facing: FacingMode::Camera,
        };
        emitters.push(emitter);
    }
    SystemSpec {
        emitters,
        clock: SimulationClock::External {
            tick: 0,
            wall_time_ms: now,
        },
        sequence_rng: Some(rng),
        ..Default::default()
    }
}
fn native_orbit_emitter(
    parameter: i16,
    texture: &str,
    extent: [u32; 2],
    control: crysirror_vfx::orbit_atlas::OrbitControl,
) -> Result<EmitterSpec> {
    let density = (parameter % 10).clamp(0, 6) as usize;
    let count = [0, 1, 3, 6, 9, 12, 20][density];
    let mut emitter = EmitterSpec::default();
    emitter.id = "depth-4".into();
    emitter.capacity = 20;
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
        layer: EFFECT_LAYER + 4,
        parent: Some(SCENE_ROOT),
        order: pool_order(4, false),
        anchor: [0.5, 0.5],
    });
    emitter.orbit_atlas = Some(crysirror_vfx::orbit_atlas::OrbitAtlasSpec {
        tick_hz: 60,
        texture_extent: extent,
        cells: vec![[140, 0, 180, 40], [180, 0, 200, 20], [200, 0, 208, 8]],
        radius_stride: 64,
        radius_base: 8,
        radius_span: 20,
        scale_percent: [300, 699],
        alpha_steps: 10,
        alpha_divisor: 100.,
        radians_per_degree: 0.01745327777777778,
        position_anchor: [0.5, 0.5],
        signed_tolerance: true,
        control,
    });
    emitter.render = RenderSpec::Sprite {
        texture: Some(texture.into()),
        columns: 1,
        rows: 1,
        frames: 1,
        fps: 0.,
        soft_distance: 0.,
        triplanar_coverage: None,
        blend: BlendMode::Alpha,
        facing: FacingMode::Camera,
    };
    Ok(emitter)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_five_depth_declaration_is_one_shared_system_in_original_order() {
        let s = native_spec(5, "authored.png", [256, 256], SequenceRng::msvc(7), 42, false).unwrap();
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
        assert!(native_spec(5, "authored.png", [128, 128], SequenceRng::msvc(7), 42, false).is_err());
    }
}
