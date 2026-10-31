// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Client/epoch/revision-checked publication of shared worker image operations.
use crate::{EngineState, Value};
use anyhow::{ensure, Context, Result};
use crysirror_core::{image_processing::*, service::HostServices, state::ClientId};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
#[derive(Debug)]
struct Snapshot {
    owner: ClientId,
    size: [u32; 2],
    pixels: Arc<[u8]>,
}
impl ImageResource for Snapshot {
    fn client(&self) -> ClientId {
        self.owner
    }
    fn size(&self) -> [u32; 2] {
        self.size
    }
    fn rgba(&self) -> Arc<[u8]> {
        self.pixels.clone()
    }
    fn opaque(&self) -> bool {
        false
    } // conservative; not used for processing admission
}
struct Entry {
    owner: u32,
    epoch: u64,
    slot: u32,
    revision: u64,
    job: Option<ImageJob>,
    state: &'static str,
    result: serde_json::Value,
    error: Option<String>,
}
#[derive(Default)]
pub struct BitmapJobs {
    entries: BTreeMap<u64, Entry>,
}
pub(crate) fn call(state: &mut EngineState, name: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        name,
        "beginEffect" | "beginTransition" | "beginRowCopy" | "beginAlphaClip" | "effectStatus" | "cancelEffect"
    ) {
        return Ok(None);
    }
    let owner = state.client.0;
    let epoch = state.runtime.restored;
    state.bitmap_jobs.entries.retain(|_, e| e.epoch == epoch);
    let id = args
        .first()
        .context("bitmap operation argument")?
        .as_i64()?;
    if matches!(name, "beginEffect" | "beginTransition" | "beginRowCopy" | "beginAlphaClip") {
        ensure!(
            state.bitmap_jobs.entries.len() < 32
                && state
                    .bitmap_jobs
                    .entries
                    .values()
                    .filter(|e| e.owner == owner)
                    .count()
                    < 8,
            "bitmap ticket capacity"
        );
        let slot = u32::try_from(id)?;
        let path = format!("{}{}", crysirror_runtime::assets::SURFACE_PREFIX, slot);
        let surface = state
            .runtime
            .assets
            .surface(state.client, &path)
            .context("bitmap slot absent")?;
        let input = Arc::new(Snapshot {
            owner: state.client,
            size: [surface.width, surface.height],
            pixels: surface.rgba.clone(),
        });
        let revision = surface.revision;
        let (input, effects) = if matches!(name,"beginRowCopy"|"beginAlphaClip") {
            let source_slot = u32::try_from(args.get(1).context("pair source slot")?.as_i64()?)?;
            let path = format!("{}{}", crysirror_runtime::assets::SURFACE_PREFIX, source_slot);
            let source = state
                .runtime
                .assets
                .surface(state.client, &path)
                .context("row-copy source absent")?;
            let source = Arc::new(Snapshot {
                owner: state.client,
                size: [source.width, source.height],
                pixels: source.rgba.clone(),
            });
            let options=args.get(2).context("pair image parameters")?.to_json();
            let effect=if name=="beginRowCopy" {
                crysirror_image::effects::ImageEffect::SineRowCopy{spec:serde_json::from_value(options)?}
            }else{
                let spec:crysirror_image::alpha_clip::AlphaClipRect=serde_json::from_value(options)?;
                ensure!(!spec.self_source||source_slot==slot,"self-source alpha clip requires the same bitmap slot");
                crysirror_image::effects::ImageEffect::AlphaClipRect{spec}
            };
            (
                ImageInput::Raster {
                    destination: Box::new(ImageInput::Resource(input)),
                    source: Box::new(ImageInput::Resource(source)),
                },
                vec![effect],
            )
        } else if name == "beginTransition" {
            let other = u32::try_from(args.get(1).context("transition second slot")?.as_i64()?)?;
            let snapshot = |slot: u32| -> Result<Arc<dyn ImageResource>> {
                let p = format!("{}{}", crysirror_runtime::assets::SURFACE_PREFIX, slot);
                let s = state
                    .runtime
                    .assets
                    .surface(state.client, &p)
                    .context("transition image slot absent")?;
                Ok(Arc::new(Snapshot {
                    owner: state.client,
                    size: [s.width, s.height],
                    pixels: s.rgba.clone(),
                }))
            };
            let mask = args
                .get(3)
                .filter(|v| !matches!(v, Value::Null))
                .map(|v| snapshot(u32::try_from(v.as_i64()?)?))
                .transpose()?;
            let spec =
                serde_json::from_value(args.get(2).context("transition options")?.to_json())?;
            (
                ImageInput::Transition {
                    from: input,
                    to: snapshot(other)?,
                    mask,
                },
                vec![crysirror_image::effects::ImageEffect::Transition { spec }],
            )
        } else {
            let effects: Vec<crysirror_image::effects::ImageEffect> =
                serde_json::from_value(args.get(1).context("image effects array")?.to_json())?;
            (ImageInput::Resource(input), effects)
        };
        let bytes = serde_json::to_vec(&effects)?;
        let provider = state
            .runtime
            .host
            .image_metadata(state.client)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let job = provider
            .begin_process(input, bytes.into())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let id = crysirror_tasks::table::next_operation_id()?;
        state.bitmap_jobs.entries.insert(
            id,
            Entry {
                owner,
                epoch,
                slot,
                revision,
                job: Some(job),
                state: "pending",
                result: json!(null),
                error: None,
            },
        );
        return Ok(Some(Value::Int(i64::try_from(id)?)));
    }
    let id = u64::try_from(id)?;
    let entry = state
        .bitmap_jobs
        .entries
        .get_mut(&id)
        .filter(|e| e.owner == owner)
        .context("unknown or foreign bitmap ticket")?;
    if name == "cancelEffect" {
        state.bitmap_jobs.entries.remove(&id);
        return Ok(Some(Value::Null));
    }
    if let Some(job) = &mut entry.job {
        let progress = job.poll().map_err(|e| anyhow::anyhow!("{e}"))?;
        match progress {
            ImageProcessState::Pending => (),
            ImageProcessState::Failed(e) => {
                entry.job = None;
                entry.state = "failed";
                entry.error = Some(e);
            }
            ImageProcessState::Ready(image) => {
                entry.job = None;
                let result = (|| -> Result<serde_json::Value> {
                    ensure!(image.client() == state.client, "foreign bitmap result");
                    let path = format!(
                        "{}{}",
                        crysirror_runtime::assets::SURFACE_PREFIX,
                        entry.slot
                    );
                    let current = state
                        .runtime
                        .assets
                        .surface(state.client, &path)
                        .context("bitmap removed before completion")?;
                    ensure!(
                        current.revision == entry.revision,
                        "bitmap changed before completion; stale result rejected"
                    );
                    let [w, h] = image.size();
                    let path = state.runtime.assets.put_surface_arc(
                        state.client,
                        entry.slot,
                        w,
                        h,
                        image.rgba(),
                    )?;
                    Ok(json!({"surface":path,"width":w,"height":h}))
                })();
                match result {
                    Ok(v) => {
                        entry.state = "ready";
                        entry.result = v;
                    }
                    Err(e) => {
                        entry.state = "failed";
                        entry.error = Some(e.to_string());
                    }
                }
            }
        }
    }
    Ok(Some(Value::from_json(
        json!({"ticket":id,"kind":"bitmap_effect","state":entry.state,"result":entry.result,"error":entry.error}),
    )))
}
