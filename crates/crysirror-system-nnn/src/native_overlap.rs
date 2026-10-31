// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Legacy overlap wait/presentation policy over shared engine jobs.
use crate::{
    machine::{Machine, NativeCall, Payload},
    native_presentation::{NativeFrameResult, NativeFrameWork},
};
use anyhow::{bail, ensure, Result};
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState},
    service::HostServices,
    state::ClientId,
};
use crysirror_image::{
    effects::ImageEffect,
    fixed_mix::{FixedMixAlpha, FixedMixRounding, FixedMixSpec},
    transition::ImageTransition,
};
pub struct OverlapResult {
    pub frame: NativeFrameResult,
    pub display_path: Option<String>,
}
pub struct NativeOverlap {
    receipt: NativeCall,
    client: ClientId,
    display: u32,
    order: u16,
    size: [u32; 2],
    frame_ms: u64,
    frames: u32,
    started_ms: u64,
    work: NativeFrameWork,
    frame: Option<NativeFrameResult>,
    blend: Option<ImageJob>,
    rendering_count: u32,
    presented_count: Option<u32>,
    display_path: Option<String>,
    pub api_calls: u64,
}
impl NativeOverlap {
    pub fn begin(
        client: ClientId,
        display: u32,
        order: u16,
        size: [u32; 2],
        frame_ms: u64,
        call: &NativeCall,
        work: NativeFrameWork,
        display_path: Option<String>,
        now_ms: u64,
    ) -> Result<Self> {
        let Payload::Words { count, words } = &call.payload else {
            bail!("NNN overlap payload")
        };
        ensure!(
            *count >= 2 && words.len() >= 2,
            "NNN overlap parameter count"
        );
        ensure!(
            words[0] == 0,
            "NNN native wipe type {} requires shared exact profile",
            words[0]
        );
        let frames =
            u32::try_from(words[1]).map_err(|_| anyhow::anyhow!("NNN negative overlap length"))?;
        ensure!(
            frames > 0 && frames <= 864000,
            "NNN overlap frame length bounds"
        );
        Ok(Self {
            receipt: call.clone(),
            client,
            display,
            order,
            size,
            frame_ms,
            frames,
            started_ms: now_ms,
            work,
            frame: None,
            blend: None,
            rendering_count: 0,
            presented_count: None,
            display_path,
            api_calls: 0,
        })
    }
    pub fn dependency_pending(&self)->bool {self.frame.is_none()||self.blend.is_some()}
    pub fn poll(
        &mut self,
        machine: &mut Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
    ) -> Result<Option<OverlapResult>> {
        ensure!(self.receipt == *call, "NNN overlap receipt changed");
        machine.validate_receipt(call)?;
        if self.frame.is_none() {
            let before = self.work.api_calls;
            let result = self.work.poll(host);
            self.api_calls += self.work.api_calls - before;
            self.frame = result?;
            if self.frame.is_none() {
                return Ok(None);
            }
        }
        if let Some(job) = self.blend.as_mut() {
            match job.poll()? {
                ImageProcessState::Pending => return Ok(None),
                ImageProcessState::Failed(error) => {
                    bail!("NNN shared integer overlap failed: {error}")
                }
                ImageProcessState::Ready(image) => {
                    ensure!(image.client() == self.client, "NNN foreign overlap frame");
                    let published =
                        host.publish_image_resource(self.client, self.display, image)?;
                    self.api_calls += 1;
                    let [w, h] = self.size;
                    // A sibling of the source hierarchy: captures cannot feed a
                    // previously composed display image back into the next frame.
                    if let Err(error)=host.submit_engine(self.client,1,&serde_json::to_vec(&serde_json::json!({"layers":[{"op":"put","layer":{"id":self.display,"rect":[0,0,w,h],"order":[self.order,0],"paint":{"kind":"image","asset":published.path.relative()}}}]}))?){
                        let _=host.release_image_resource(self.client,published.path.relative());return Err(error.into())
                    }
                    self.api_calls += 1;
                    if let Some(previous) = self
                        .display_path
                        .replace(published.path.relative().to_owned())
                    {
                        host.release_image_resource(self.client, &previous)?;
                    }
                    self.presented_count = Some(self.rendering_count);
                    self.blend = None;
                }
            }
        }
        if self.presented_count == Some(self.frames) {
            let frame = self.frame.as_ref().unwrap();
            let mut candidate = machine.native_variables_candidate(call)?;
            candidate.replace_random_state(frame.random.clone())?;
            machine.acknowledge_with_variables(call, candidate)?;
            return Ok(Some(OverlapResult {
                frame: frame.clone(),
                display_path: self.display_path.take(),
            }));
        }
        let count = (host.now_ms().saturating_sub(self.started_ms) / self.frame_ms)
            .min(u64::from(self.frames)) as u32;
        if self.presented_count == Some(count) {
            return Ok(None);
        }
        let percent = u64::from(count) * 100 / u64::from(self.frames);
        let frame = self.frame.as_ref().unwrap();
        let effect = ImageEffect::Transition {
            spec: ImageTransition {
                progress: count as f32 / self.frames as f32,
                softness: 0.,
                blur: 0.,
                fixed_weighted: Some(FixedMixSpec {
                    left_weight: ((100 - percent) * 256 / 100) as u32,
                    right_weight: (percent * 256 / 100) as u32,
                    fractional_bits: 8,
                    rounding: FixedMixRounding::SumThenShift,
                    alpha: FixedMixAlpha::Opaque,
                }),
            },
        };
        self.blend = Some(host.image_metadata(self.client)?.begin_process(
            ImageInput::Transition {
                from: frame.old.clone(),
                to: frame.target.clone(),
                mask: None,
            },
            serde_json::to_vec(&[effect])?.into(),
        )?);
        self.api_calls += 1;
        self.rendering_count = count;
        Ok(None)
    }
}
