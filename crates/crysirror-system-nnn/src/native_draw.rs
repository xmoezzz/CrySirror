// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Native DRAW frame waits with engine-owned frame resources. The native
//! virtual frame clock is independent of worker-poll frequency and display FPS.
use crate::{
    machine::{Machine, NativeCall, Payload},
    native_presentation::{NativeFrameEffect, NativeFrameResult, NativeFrameWork},
};
use anyhow::{bail, ensure, Result};
use crysirror_core::{service::HostServices, state::ClientId};
use std::collections::{BTreeMap, BTreeSet};
pub struct DrawResult {
    pub frame: NativeFrameResult,
    pub effects: BTreeMap<i32, NativeFrameEffect>,
    pub display_path: Option<String>,
}
pub struct NativeDraw {
    client: ClientId,
    receipt: NativeCall,
    display: u32,
    order: u16,
    size: [u32; 2],
    frame_ms: u64,
    total: u32,
    started_ms: u64,
    rendered: u32,
    work: Option<NativeFrameWork>,
    last: Option<NativeFrameResult>,
    effects: BTreeMap<i32, NativeFrameEffect>,
    visible: BTreeSet<i32>,
    underlays: BTreeMap<i32, crysirror_core::image_processing::ImageCompositeLayer>,
    display_path: Option<String>,
    pub api_calls: u64,
}
impl NativeDraw {
    pub fn begin(
        client: ClientId,
        display: u32,
        order: u16,
        size: [u32; 2],
        frame_ms: u64,
        call: &NativeCall,
        work: NativeFrameWork,
        effects: BTreeMap<i32, NativeFrameEffect>,
        visible: BTreeSet<i32>,
        underlays: BTreeMap<i32, crysirror_core::image_processing::ImageCompositeLayer>,
        display_path: Option<String>,
        now: u64,
    ) -> Result<Self> {
        let Payload::Words { count: 1, words } = &call.payload else {
            bail!("NNN native DRAW count payload")
        };
        ensure!(
            words.len() == 1 && words[0] >= 0 && words[0] <= 864000,
            "NNN native DRAW count bounds"
        );
        ensure!(
            effects
                .values()
                .all(|effect| !matches!(effect, NativeFrameEffect::Paint(_))),
            "NNN native state map cannot contain paint DTOs"
        );
        Ok(Self {
            client,
            receipt: call.clone(),
            display,
            order,
            size,
            frame_ms,
            total: words[0] as u32,
            started_ms: now,
            rendered: 0,
            work: Some(work),
            last: None,
            effects,
            visible,
            underlays,
            display_path,
            api_calls: 0,
        })
    }
    pub fn dependency_pending(&self) -> bool {
        self.work.is_some()
    }
    pub fn poll(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
    ) -> Result<Option<DrawResult>> {
        machine.validate_receipt(call)?;
        ensure!(self.receipt == *call, "NNN DRAW receipt changed");
        if let Some(work) = self.work.as_mut() {
            let before = work.api_calls;
            let result = work.poll(host);
            self.api_calls += work.api_calls - before;
            let Some(frame) = result? else {
                return Ok(None);
            };
            let published =
                host.publish_image_resource(self.client, self.display, frame.target.clone())?;
            self.api_calls += 1;
            let [w, h] = self.size;
            if let Err(error)=host.submit_engine(self.client,1,&serde_json::to_vec(&serde_json::json!({"layers":[{"op":"put","layer":{"id":self.display,"rect":[0,0,w,h],"order":[self.order,0],"paint":{"kind":"image","asset":published.path.relative()}}}]}))?){let _=host.release_image_resource(self.client,published.path.relative());return Err(error.into())}
            self.api_calls += 1;
            if let Some(previous) = self
                .display_path
                .replace(published.path.relative().to_owned())
            {
                host.release_image_resource(self.client, &previous)?;
            }
            if let Some(pattern) = &frame.shared_pattern {
                for effect in self.effects.values_mut() {
                    if let NativeFrameEffect::Suna(state) = effect {
                        state.resource = pattern.clone();
                    }
                }
            }
            for (slot, state) in &frame.lamp_updates {
                if let Some(NativeFrameEffect::Lamp(effect)) = self.effects.get_mut(slot) {
                    effect.state = state.clone();
                }
            }
            if self.total > 0 {
                // CountUp is after Print. Reaching max disables picture-less
                // FADEIN; SUNA intentionally remains active at its end count.
                self.effects.retain(|_, effect| match effect {
                    NativeFrameEffect::Paint(_) => {
                        unreachable!("paint DTO is not persistent native effect state")
                    }
                    NativeFrameEffect::Suna(state) => {
                        if state.count < state.parameters.frames {
                            state.count = (state.count + 1).min(state.parameters.frames);
                        }
                        true
                    }
                    NativeFrameEffect::Grey(state) => {
                        if state.count < state.frames {
                            state.count = state.count.saturating_add(1).min(state.frames);
                        }
                        true
                    }
                    NativeFrameEffect::Lamp(frame) => {
                        frame.state.count = frame.state.count.saturating_add(1).min(999999);
                        true
                    }
                    NativeFrameEffect::Bura(state) => {
                        if state.count < state.frames {
                            state.count = state.count.saturating_add(1).min(state.frames);
                        }
                        true
                    }
                    NativeFrameEffect::Chara(frame) => {
                        if frame.state.count < frame.state.frames {
                            frame.state.count =
                                frame.state.count.saturating_add(1).min(frame.state.frames);
                        }
                        true
                    }
                    NativeFrameEffect::Fade(state) => {
                        if state.count < state.frames {
                            state.count = (state.count + 1).min(state.frames);
                            state.fade_out || state.count != state.frames
                        } else {
                            true
                        }
                    }
                });
            }
            self.last = Some(frame);
            self.work = None;
            self.rendered += 1;
        }
        let count = self.total.max(1);
        if self.rendered >= count
            && (self.total == 0
                || host.now_ms().saturating_sub(self.started_ms)
                    >= u64::from(self.total) * self.frame_ms)
        {
            let frame = self.last.as_ref().unwrap();
            let mut vars = machine.native_variables_candidate(call)?;
            vars.replace_random_state(frame.random.clone())?;
            machine.acknowledge_with_variables(call, vars)?;
            return Ok(Some(DrawResult {
                frame: frame.clone(),
                effects: self.effects.clone(),
                display_path: self.display_path.take(),
            }));
        }
        if self.rendered < count
            && host.now_ms().saturating_sub(self.started_ms)
                >= u64::from(self.rendered) * self.frame_ms
        {
            let previous = self.last.as_ref().unwrap();
            let work = NativeFrameWork::with_base(
                self.client,
                previous.base.clone(),
                crate::native_presentation::ordered_frame_plan(
                    &self.effects,
                    &self.visible,
                    &self.underlays,
                ),
                previous.random.clone(),
                host,
            )?;
            self.api_calls += work.api_calls;
            self.work = Some(work);
        }
        Ok(None)
    }
}
