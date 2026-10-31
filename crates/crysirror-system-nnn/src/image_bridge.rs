// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Native picture-slot loading through scoped resource workers and ImageJob.
use crate::{
    machine::{Machine, NativeCall, Payload},
    native_resources::ResourceNames,
    resource_bridge::NativeResources,
};
use anyhow::{bail, ensure, Result};
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState, ImageResource},
    service::HostServices,
    state::ClientId,
};
use crysirror_tasks::{Task, TaskPool};
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone)]
pub struct PreparedPicture {
    pub has_mask: bool,
    pub slot: i32,
    pub preload: bool,
    pub name: Vec<u8>,
    pub raw: Arc<dyn ImageResource>,
    pub opaque: Arc<dyn ImageResource>,
    pub display: Arc<dyn ImageResource>,
}
struct Pending {
    call: NativeCall,
    task: Option<Task<crate::resource_bridge::NativeImageSource>>,
    job: Option<ImageJob>,
    raw: Option<Arc<dyn ImageResource>>,
    stage: u8,
    opaque: Option<Arc<dyn ImageResource>>,
    has_mask: Option<bool>,
    ready: Option<PreparedPicture>,
}
pub struct ImageBridge {
    client: ClientId,
    names: ResourceNames,
    levels: u32,
    resources: NativeResources,
    pool: TaskPool,
    pending: Option<Pending>,
    /// Original CPicture slots retain raw alpha even when their current
    /// background display ignores it. Engine resources own every pixel.
    pub pictures: BTreeMap<i32, PreparedPicture>,
    pub api_calls: u64,
}
impl ImageBridge {
    pub fn new(
        client: ClientId,
        names: ResourceNames,
        levels: u32,
        resources: NativeResources,
    ) -> Result<Self> {
        ensure!(
            client.0 != 0 && levels <= 1024,
            "NNN image capability/pack-level bounds"
        );
        Ok(Self {
            client,
            names,
            levels,
            resources,
            pool: TaskPool::new("nnn-native-picture", 1, 1)?,
            pending: None,
            pictures: BTreeMap::new(),
            api_calls: 0,
        })
    }
    pub fn dependency_pending(&self)->bool {self.pending.as_ref().is_some_and(|p|p.ready.is_none())}
    pub fn client(&self) -> ClientId {
        self.client
    }
    pub fn prepare(
        &mut self,
        machine: &Machine,
        call: &NativeCall,
        host: &mut dyn HostServices,
    ) -> Result<Option<PreparedPicture>> {
        machine.validate_receipt(call)?;
        let Payload::Image {
            preload,
            slot,
            name,
        } = &call.payload
        else {
            bail!("NNN image slot payload")
        };
        ensure!((0..16).contains(slot), "NNN legacy picture slot bounds");
        if self.pending.is_none() {
            let lookup = self.names.script_image(name, self.levels)?;
            let resources = self.resources.clone();
            let task = self.pool.spawn(move |cancel| {
                cancel.check()?;
                let input = resources.image_source(&lookup, &cancel)?;
                cancel.check()?;
                Ok(input)
            })?;
            self.pending = Some(Pending {
                call: call.clone(),
                task: Some(task),
                job: None,
                raw: None,
                stage: 0,
                opaque: None,
                has_mask: None,
                ready: None,
            });
        }
        let pending = self.pending.as_mut().unwrap();
        ensure!(pending.call == *call, "NNN image slot receipt changed");
        if let Some(ready) = &pending.ready {
            return Ok(Some(ready.clone()));
        }
        if let Some(task) = pending.task.as_mut() {
            let Some(source) = task.try_take()? else {
                return Ok(None);
            };
            pending.has_mask = Some(source.has_mask);
            pending.job = Some(
                host.image_metadata(self.client)?
                    .begin_process(source.input, b"[]".as_slice().into())?,
            );
            self.api_calls += 1;
            pending.task = None;
        }
        let Some(job) = pending.job.as_mut() else {
            bail!("NNN image dependency missing")
        };
        match job.poll()? {
            ImageProcessState::Pending => Ok(None),
            ImageProcessState::Failed(error) => bail!("NNN shared image decoder: {error}"),
            ImageProcessState::Ready(image) => {
                ensure!(image.client() == self.client, "NNN foreign decoded image");
                if pending.stage == 0 {
                    pending.raw = Some(image.clone());
                    // Retain a separate engine-owned opaque view for native
                    // alpha-ignore operations, including CTRANS without mask.
                    // Do not destroy the original bytes or infer mask presence.
                    pending.stage=1;
                    if !image.opaque() {
                        pending.job=Some(host.image_metadata(self.client)?.begin_process(
                            ImageInput::Resource(image),serde_json::to_vec(&[crysirror_image::effects::ImageEffect::OverrideAlpha{alpha:255}])?.into())?);
                        self.api_calls+=1;return Ok(None);
                    }
                }
                if pending.stage==1 {
                    pending.opaque=Some(image.clone());
                    pending.stage=2;
                    if !preload && !(*slot<4||*slot==14) && !pending.has_mask.unwrap() {
                        pending.job=Some(host.image_metadata(self.client)?.begin_process(
                            ImageInput::Resource(pending.raw.clone().unwrap()),serde_json::to_vec(&[crysirror_image::effects::ImageEffect::SpriteAlpha{spec:crysirror_image::sprite_alpha::SpriteAlpha::CornerKey{right:false}}])?.into())?);
                        self.api_calls+=1;return Ok(None);
                    }
                }
                let display=if *preload || pending.has_mask.unwrap() && !(*slot<4||*slot==14) {
                    pending.raw.clone().unwrap()
                } else {image};
                let prepared = PreparedPicture {
                    has_mask: pending.has_mask.unwrap(),
                    slot: *slot,
                    preload: *preload,
                    name: name.clone(),
                    raw: pending.raw.clone().unwrap(),
                    opaque:pending.opaque.clone().unwrap(),
                    display,
                };
                pending.ready = Some(prepared.clone());
                Ok(Some(prepared))
            }
        }
    }
    /// Owner presentation and exact native acknowledgment must succeed first.
    pub fn accept(&mut self, call: &NativeCall) -> Result<()> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("NNN picture acceptance absent"))?;
        ensure!(
            pending.call == *call,
            "NNN picture acceptance receipt changed"
        );
        let prepared = pending
            .ready
            .clone()
            .ok_or_else(|| anyhow::anyhow!("NNN picture is not ready"))?;
        self.pictures.insert(prepared.slot, prepared);
        self.pending = None;
        Ok(())
    }
}
