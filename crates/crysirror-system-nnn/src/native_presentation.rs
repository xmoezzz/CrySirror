// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Native Print/overlap dependency ordering. Rasterisation, dynamic patterns,
//! weighted mixing and publication resources are all engine services.
use crate::native_effects::{FadeInState, LayerGreyState, SunaState};
use anyhow::{bail, ensure, Result};
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState, ImageResource},
    service::{
        HostServices, PatternJob, PatternOutput, PatternProcessState, PatternRequest,
        PatternResource,
    },
    state::ClientId,
};
use crysirror_image::temporal_pattern::PatternRng;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
#[derive(Clone)]
pub enum NativeFrameEffect {
    /// A native picture/fill at its original painter position. All pixel work is shared.
    Paint(crysirror_core::image_processing::ImageCompositeLayer),
    Suna(SunaState),
    Fade(FadeInState),
    Grey(LayerGreyState),
    Chara(CharaTransFrame),
    Lamp(OldLampFrame),
    Bura(crate::native_bura::BuraState),
}
/// The native state selects a shared immutable source and rectangle. Image
/// graph workers perform the complete raster operation, including clipping.
#[derive(Clone)]
pub struct CharaTransFrame {
    pub state: crate::native_chara_trans::CharaTransState,
    pub source: Option<Arc<dyn ImageResource>>,
    pub rectangles: crate::native_geometry::NativeRects,
}
#[derive(Clone)]
pub struct OldLampFrame {
    pub slot: i32,
    pub state: crate::native_old_lamp::OldLampState,
    pub source: Option<Arc<dyn ImageResource>>,
    pub destination: [i32; 2],
}
/// Build the original ascending-slot Print sequence. Picture/fill pixels are
/// prepared image leases or Solid DTOs; native policy does not inspect pixels.
/// Calcu still runs over the entire resulting plan before the first Print.
pub fn ordered_frame_plan(
    effects: &BTreeMap<i32, NativeFrameEffect>,
    visible: &std::collections::BTreeSet<i32>,
    underlays: &BTreeMap<i32, crysirror_core::image_processing::ImageCompositeLayer>,
) -> Vec<NativeFrameEffect> {
    let slots: std::collections::BTreeSet<_> =
        effects.keys().chain(underlays.keys()).copied().collect();
    let mut plan = Vec::new();
    for slot in slots {
        let terminal_fill = matches!(effects.get(&slot),Some(NativeFrameEffect::Fade(state)) if state.command()==54);
        if let Some(layer) = underlays.get(&slot).filter(|_| !terminal_fill) {
            plan.push(NativeFrameEffect::Paint(layer.clone()));
        }
        if let Some(effect) = effects.get(&slot) {
            if visible.contains(&slot) || matches!(effect, NativeFrameEffect::Lamp(_)) {
                plan.push(effect.clone());
            }
        }
    }
    plan
}
fn prepare_calcu(
    effects: &mut [NativeFrameEffect],
    random: &mut PatternRng,
    calcu: bool,
) -> Result<BTreeMap<i32, crate::native_old_lamp::OldLampState>> {
    let mut updates = BTreeMap::new();
    for effect in effects {
        if let NativeFrameEffect::Lamp(frame) = effect {
            if calcu {
                frame.state.calcu(random)?;
            }
            updates.insert(frame.slot, frame.state.clone());
        }
    }
    Ok(updates)
}
enum Work {
    Old(ImageJob),
    Base(ImageJob),
    Effect(ImageJob),
    Advance(PatternJob, SunaState),
    Pattern(PatternJob),
}
#[derive(Clone)]
pub struct NativeFrameResult {
    pub old: Arc<dyn ImageResource>,
    pub target: Arc<dyn ImageResource>,
    pub random: PatternRng,
    pub shared_pattern: Option<Arc<dyn PatternResource>>,
    pub base: Arc<dyn ImageResource>,
    pub lamp_updates: BTreeMap<i32, crate::native_old_lamp::OldLampState>,
}
pub struct NativeFrameWork {
    client: ClientId,
    base_canvas: u32,
    old: Option<Arc<dyn ImageResource>>,
    current: Option<Arc<dyn ImageResource>>,
    effects: VecDeque<NativeFrameEffect>,
    random: PatternRng,
    pattern: Option<Arc<dyn PatternResource>>,
    work: Option<Work>,
    ready: Option<NativeFrameResult>,
    pub api_calls: u64,
    base: Option<Arc<dyn ImageResource>>,
    lamp_updates: BTreeMap<i32, crate::native_old_lamp::OldLampState>,
}
impl NativeFrameWork {
    pub fn begin(
        client: ClientId,
        old_canvas: u32,
        base_canvas: u32,
        old: Option<Arc<dyn ImageResource>>,
        mut effects: Vec<NativeFrameEffect>,
        mut random: PatternRng,
        host: &mut dyn HostServices,
    ) -> Result<Self> {
        random.validate()?;
        let lamp_updates = prepare_calcu(&mut effects, &mut random, false)?;
        let work = if old.is_some() {
            Work::Base(host.begin_canvas_image(client, base_canvas)?)
        } else {
            Work::Old(host.begin_canvas_image(client, old_canvas)?)
        };
        Ok(Self {
            client,
            base_canvas,
            old,
            current: None,
            effects: effects.into(),
            random,
            pattern: None,
            work: Some(work),
            ready: None,
            api_calls: 1,
            base: None,
            lamp_updates,
        })
    }
    pub fn begin_draw(
        client: ClientId,
        base_canvas: u32,
        mut effects: Vec<NativeFrameEffect>,
        mut random: PatternRng,
        calcu: bool,
        host: &mut dyn HostServices,
    ) -> Result<Self> {
        random.validate()?;
        let lamp_updates = prepare_calcu(&mut effects, &mut random, calcu)?;
        Ok(Self {
            client,
            base_canvas,
            old: None,
            current: None,
            effects: effects.into(),
            random,
            pattern: None,
            work: Some(Work::Base(host.begin_canvas_image(client, base_canvas)?)),
            ready: None,
            api_calls: 1,
            base: None,
            lamp_updates,
        })
    }
    pub fn with_base(
        client: ClientId,
        base: Arc<dyn ImageResource>,
        mut effects: Vec<NativeFrameEffect>,
        mut random: PatternRng,
        host: &mut dyn HostServices,
    ) -> Result<Self> {
        ensure!(base.client() == client, "NNN cached base resource scope");
        random.validate()?;
        let lamp_updates = prepare_calcu(&mut effects, &mut random, true)?;
        let mut job = Self {
            client,
            base_canvas: 0,
            old: Some(base.clone()),
            current: Some(base.clone()),
            base: Some(base),
            effects: effects.into(),
            random,
            pattern: None,
            work: None,
            lamp_updates,
            ready: None,
            api_calls: 0,
        };
        job.next(host)?;
        Ok(job)
    }
    fn next(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let Some(effect) = self.effects.pop_front() else {
            self.ready = Some(NativeFrameResult {
                old: self.old.clone().unwrap(),
                target: self.current.clone().unwrap(),
                random: self.random.clone(),
                shared_pattern: self.pattern.clone(),
                base: self.base.clone().unwrap(),
                lamp_updates: self.lamp_updates.clone(),
            });
            return Ok(());
        };
        self.work = Some(match effect {
            NativeFrameEffect::Paint(layer) => {
                let input = ImageInput::Composite {
                    base: Box::new(ImageInput::Resource(self.current.clone().unwrap())),
                    layers: vec![layer],
                };
                Work::Effect(
                    host.image_metadata(self.client)?
                        .begin_process(input, b"[]".as_slice().into())?,
                )
            }
            NativeFrameEffect::Suna(mut state) => {
                if let Some(shared) = &self.pattern {
                    state.resource = shared.clone();
                }
                let job = host.begin_pattern(
                    self.client,
                    PatternRequest::AdvancePrefix {
                        source: state.resource.clone(),
                        rng: serde_json::to_vec(&self.random)?.into(),
                        words: ((state.parameters.rect[2] / 2) * (state.parameters.rect[3] / 2) / 4)
                            as usize,
                    },
                )?;
                Work::Advance(job, state)
            }
            NativeFrameEffect::Fade(state) => {
                let effects = serde_json::to_vec(&[state.linear_image_effect()?])?;
                Work::Effect(host.image_metadata(self.client)?.begin_process(
                    ImageInput::Resource(self.current.clone().unwrap()),
                    effects.into(),
                )?)
            }
            NativeFrameEffect::Bura(state) => {
                let effect = state.image_effect(self.current.as_ref().unwrap().size())?;
                Work::Effect(host.image_metadata(self.client)?.begin_process(
                    ImageInput::Resource(self.current.clone().unwrap()),
                    serde_json::to_vec(&[effect])?.into(),
                )?)
            }
            NativeFrameEffect::Grey(state) => {
                let effects = serde_json::to_vec(&[state.linear_image_effect()?])?;
                Work::Effect(host.image_metadata(self.client)?.begin_process(
                    ImageInput::Resource(self.current.clone().unwrap()),
                    effects.into(),
                )?)
            }
            NativeFrameEffect::Lamp(frame) => {
                if !frame
                    .state
                    .visible(frame.source.is_some(), &mut self.random)?
                {
                    return self.next(host);
                }
                let source = frame.source.unwrap();
                let size = source.size();
                let input = ImageInput::Composite {
                    base: Box::new(ImageInput::Resource(self.current.clone().unwrap())),
                    layers: vec![crysirror_core::image_processing::ImageCompositeLayer {
                        input: Box::new(ImageInput::Resource(source)),
                        source: [0, 0, size[0] as i32, size[1] as i32],
                        destination: frame.destination,
                        clip: None,
                        opacity: 255,
                        math:
                            crysirror_core::image_processing::ImageCompositeMath::MaskPiecewise256 {
                                weight: 256,
                            },
                    }],
                };
                Work::Effect(
                    host.image_metadata(self.client)?
                        .begin_process(input, b"[]".as_slice().into())?,
                )
            }
            NativeFrameEffect::Chara(frame) => {
                let Some(source) = frame.source else {
                    return self.next(host);
                };
                let [x, y, w, h] = frame.rectangles.destination;
                let [sx, sy, _, _] = frame.rectangles.source;
                let clip = frame.state.clip_rect();
                // Original Print returns without drawing for empty clipped
                // regions. The effect/count remains active at its endpoint.
                if w <= 0 || h <= 0 || clip.is_some_and(|r| r[2] <= 0 || r[3] <= 0) {
                    return self.next(host);
                }
                let input = ImageInput::Composite {
                    base: Box::new(ImageInput::Resource(self.current.clone().unwrap())),
                    layers: vec![crysirror_core::image_processing::ImageCompositeLayer {
                        input: Box::new(ImageInput::Resource(source)),
                        source: [sx, sy, w, h],
                        destination: [x, y],
                        clip,
                        opacity: 255,
                        math:
                            crysirror_core::image_processing::ImageCompositeMath::MaskPiecewise256 {
                                weight: frame.state.weight256(),
                            },
                    }],
                };
                Work::Effect(
                    host.image_metadata(self.client)?
                        .begin_process(input, b"[]".as_slice().into())?,
                )
            }
        });
        self.api_calls += 1;
        Ok(())
    }
    /// Bounded nonblocking dependency pump. Success retains immutable results;
    /// native random/effect state is still only a candidate until owner commit.
    pub fn poll(&mut self, host: &mut dyn HostServices) -> Result<Option<NativeFrameResult>> {
        for _ in 0..4 {
            if let Some(result) = &self.ready {
                return Ok(Some(result.clone()));
            }
            let work = self
                .work
                .take()
                .ok_or_else(|| anyhow::anyhow!("NNN frame dependency absent"))?;
            match work {
                Work::Old(mut job) => match job.poll()? {
                    ImageProcessState::Pending => {
                        self.work = Some(Work::Old(job));
                        return Ok(None);
                    }
                    ImageProcessState::Failed(error) => {
                        bail!("NNN previous-frame engine capture: {error}")
                    }
                    ImageProcessState::Ready(image) => {
                        ensure!(
                            image.client() == self.client,
                            "NNN foreign previous-frame resource"
                        );
                        self.old = Some(image);
                        self.work = Some(Work::Base(
                            host.begin_canvas_image(self.client, self.base_canvas)?,
                        ));
                        self.api_calls += 1;
                    }
                },
                Work::Base(mut job) => match job.poll()? {
                    ImageProcessState::Pending => {
                        self.work = Some(Work::Base(job));
                        return Ok(None);
                    }
                    ImageProcessState::Failed(error) => {
                        bail!("NNN base-frame engine capture: {error}")
                    }
                    ImageProcessState::Ready(image) => {
                        ensure!(
                            image.client() == self.client,
                            "NNN foreign base-frame resource"
                        );
                        if self.old.is_none() {
                            self.old = Some(image.clone());
                        }
                        self.base = Some(image.clone());
                        self.current = Some(image);
                        self.next(host)?;
                    }
                },
                Work::Effect(mut job) => match job.poll()? {
                    ImageProcessState::Pending => {
                        self.work = Some(Work::Effect(job));
                        return Ok(None);
                    }
                    ImageProcessState::Failed(error) => {
                        bail!("NNN native effect engine operation: {error}")
                    }
                    ImageProcessState::Ready(image) => {
                        self.current = Some(image);
                        self.next(host)?;
                    }
                },
                Work::Advance(mut job, state) => match job.poll()? {
                    PatternProcessState::Pending => {
                        self.work = Some(Work::Advance(job, state));
                        return Ok(None);
                    }
                    PatternProcessState::Failed(error) => {
                        bail!("NNN native pattern advance: {error}")
                    }
                    PatternProcessState::Ready(PatternOutput::Resource {
                        resource,
                        rng_after,
                    }) => {
                        ensure!(
                            resource.client() == self.client,
                            "NNN foreign advanced pattern"
                        );
                        self.random = serde_json::from_slice(&rng_after)?;
                        self.random.validate()?;
                        self.pattern = Some(resource.clone());
                        if state.parameters.rect[2] == 0 || state.parameters.rect[3] == 0 {
                            self.next(host)?;
                            continue;
                        }
                        let opacity = state.opacity256()?;
                        self.work = Some(Work::Pattern(host.begin_pattern(
                            self.client,
                            PatternRequest::Render {
                                source: resource,
                                destination: ImageInput::Resource(self.current.clone().unwrap()),
                                rect: state.parameters.rect,
                                opacity256: opacity,
                            },
                        )?));
                        self.api_calls += 1;
                    }
                    _ => bail!("NNN native advance returned image instead of pattern"),
                },
                Work::Pattern(mut job) => match job.poll()? {
                    PatternProcessState::Pending => {
                        self.work = Some(Work::Pattern(job));
                        return Ok(None);
                    }
                    PatternProcessState::Failed(error) => {
                        bail!("NNN native pattern render: {error}")
                    }
                    PatternProcessState::Ready(PatternOutput::Image(image)) => {
                        self.current = Some(image);
                        self.next(host)?;
                    }
                    _ => bail!("NNN native render returned pattern instead of image"),
                },
            }
        }
        Ok(self.ready.clone())
    }
}
