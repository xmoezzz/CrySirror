// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Immutable font/layout inputs, worker raster products, ordered owner publication.
use crate::{
    EngineState, Value,
    text_api::{self, Block, TextOptions},
};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    state::ClientId,
};
use crysirror_tasks::{
    Task,
    table::{OperationState, OperationTable},
};
use crysirror_text::{
    TextSystem,
    layout::Layout,
    raster::{IconBitmap, RasterOptions},
    reveal::{RevealEvent, RevealState},
};
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, sync::Arc};

type Key = (u32, String);
pub(crate) struct PendingRaster {
    task: Task<Arc<[u8]>>,
    previous: Option<RevealState>,
    events: Vec<RevealEvent>,
    visible: usize,
}
#[derive(Default)]
pub(crate) struct RenderState {
    pub pending: Option<PendingRaster>,
    pub error: Option<String>,
    pub published: usize,
    elapsed: u64,
}
enum Product {
    Drawn {
        key: Key,
        block: Block,
        rgba: Arc<[u8]>,
    },
    Published(Json),
}
pub(crate) struct TextJobs {
    epoch: u64,
    jobs: OperationTable<Product>,
    slots: BTreeMap<Key, (u32, u64)>,
}
impl Default for TextJobs {
    fn default() -> Self {
        Self {
            epoch: 0,
            jobs: OperationTable::new(16, 64).unwrap(),
            slots: BTreeMap::new(),
        }
    }
}
fn raster(
    text: &TextSystem,
    layout: &Layout,
    options: &RasterOptions,
    icons: &BTreeMap<String, Arc<IconBitmap>>,
) -> Result<Arc<[u8]>> {
    Ok(Arc::from(text.rasterize_rich(layout, options, |name| {
        let image = icons.get(name).context("text icon not prepared")?;
        Ok(IconBitmap {
            width: image.width,
            height: image.height,
            rgba: image.rgba.clone(),
        })
    })?))
}
fn raster_options(block: &Block) -> RasterOptions {
    let mut options = block.options.raster.clone();
    if let Some(writer) = &block.writer {
        options.visible_graphemes = Some(writer.revealed());
    }
    options
}
fn visible(block: &Block) -> usize {
    block
        .writer
        .as_ref()
        .map_or(block.layout.grapheme_count, |w| w.revealed())
}
fn sync_epoch(state: &mut EngineState) {
    let epoch = state.runtime.restored;
    if state.rich_text.asynchronous.epoch != epoch {
        state.rich_text.asynchronous.jobs.clear();
        state.rich_text.asynchronous.slots.clear();
        for ((owner, _), block) in std::mem::take(&mut state.rich_text.blocks) {
            state
                .runtime
                .assets
                .remove_surface(ClientId(owner), block.slot);
        }
        state.rich_text.asynchronous.epoch = epoch;
    }
}
pub(crate) fn poll(state: &mut EngineState) -> Result<()> {
    sync_epoch(state);
    let epoch = state.runtime.restored;
    for (owner, _, ticket) in state.rich_text.asynchronous.jobs.tickets() {
        let status = state
            .rich_text
            .asynchronous
            .jobs
            .status(owner, epoch, ticket)?;
        if status.state == OperationState::Failed {
            state
                .rich_text
                .asynchronous
                .slots
                .retain(|_, (_, id)| *id != ticket);
            continue;
        }
        if status.state != OperationState::Ready {
            continue;
        }
        let assets = &mut state.runtime.assets;
        let runtime = &mut state.rich_text;
        let slots = &mut runtime.asynchronous.slots;
        let blocks = &mut runtime.blocks;
        let result = runtime
            .asynchronous
            .jobs
            .publish(owner, epoch, ticket, |product| {
                let Product::Drawn {
                    key,
                    mut block,
                    rgba,
                } = product
                else {
                    return Ok(product);
                };
                ensure!(
                    slots.get(&key).is_some_and(|(_, id)| *id == ticket),
                    "superseded text preparation"
                );
                assets.put_surface_arc(
                    ClientId(owner),
                    block.slot,
                    block.layout.canvas[0],
                    block.layout.canvas[1],
                    rgba,
                )?;
                block.render.published = visible(&block);
                let result = text_api::summary(&block);
                blocks.insert(key.clone(), block);
                slots.remove(&key);
                Ok(Product::Published(result))
            });
        if result.is_err() {
            slots.retain(|_, (_, id)| *id != ticket);
        }
    }
    Ok(())
}
pub(crate) fn cancel_name(state: &mut EngineState, key: &Key) {
    if let Some((_, ticket)) = state.rich_text.asynchronous.slots.remove(key) {
        let _ = state.rich_text.asynchronous.jobs.cancel(key.0, ticket);
    }
}
fn slot(state: &EngineState, key: &Key) -> Result<u32> {
    if let Some(block) = state.rich_text.blocks.get(key) {
        return Ok(block.slot);
    }
    if let Some((slot, _)) = state.rich_text.asynchronous.slots.get(key) {
        return Ok(*slot);
    }
    (50000..50008)
        .find(|slot| {
            !state
                .rich_text
                .blocks
                .iter()
                .any(|((owner, _), b)| *owner == key.0 && b.slot == *slot)
                && !state
                    .rich_text
                    .asynchronous
                    .slots
                    .iter()
                    .any(|((owner, _), (s, _))| *owner == key.0 && *s == *slot)
        })
        .context("text block/preparation limit")
}
fn required(args: &[Value], n: usize) -> Result<&Value> {
    args.get(n).context("text argument required")
}
pub(crate) fn call(state: &mut EngineState, method: &str, args: &[Value]) -> Result<Option<Value>> {
    if !matches!(
        method,
        "beginDraw" | "beginRestore" | "beginMeasure" | "status" | "cancel"
    ) {
        return Ok(None);
    }
    poll(state)?;
    let owner = state.client.0;
    let epoch = state.runtime.restored;
    if matches!(method, "status" | "cancel") {
        let ticket = u64::try_from(required(args, 0)?.as_i64()?)?;
        if method == "cancel" {
            state.rich_text.asynchronous.jobs.cancel(owner, ticket)?;
            state
                .rich_text
                .asynchronous
                .slots
                .retain(|_, (_, id)| *id != ticket);
            return Ok(Some(Value::Null));
        }
        let status = state
            .rich_text
            .asynchronous
            .jobs
            .status(owner, epoch, ticket)?;
        let mut result = serde_json::to_value(&status)?;
        if status.state == OperationState::Ready {
            if let Some(Product::Published(value)) = state
                .rich_text
                .asynchronous
                .jobs
                .poll(owner, epoch, ticket)?
            {
                result["result"] = value.clone();
            }
        }
        return Ok(Some(Value::from_json(result)));
    }
    state.rich_text.asynchronous.jobs.admit(owner)?;
    let pool = state.runtime.assets.cpu_tasks();
    let text = state.runtime.assets.text.clone();
    if method == "beginMeasure" {
        let font = required(args, 0)?.as_str()?.to_owned();
        let size = required(args, 1)?.as_f32()?;
        let line = required(args, 2)?.as_str()?.to_owned();
        ensure!(line.len() <= 1024 * 1024, "text measurement byte limit");
        let ticket=state.rich_text.asynchronous.jobs.submit(owner,epoch,"text_measure",&pool,move|_|{
            let m=text.measure_simple(&font,size,&line)?;
            Ok(Product::Published(json!({"advance":m.advance,"line_height":m.line_height,"ascent":m.ascent,"descent":m.descent})))
        })?;
        return Ok(Some(Value::Int(ticket.try_into()?)));
    }
    let name = required(args, 0)?.as_str()?.to_owned();
    ensure!(
        !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control),
        "text block name"
    );
    let key = (owner, name);
    let slot = slot(state, &key)?;
    let (markup, options, reveal) = if method == "beginDraw" {
        (
            required(args, 1)?.as_str()?.to_owned(),
            args.get(2)
                .filter(|v| !matches!(v, Value::Null))
                .map(|v| serde_json::from_value::<TextOptions>(v.to_json()))
                .transpose()?
                .unwrap_or_default(),
            None,
        )
    } else {
        let value = required(args, 1)?;
        (
            value
                .field("markup")
                .context("saved markup")?
                .as_str()?
                .to_owned(),
            serde_json::from_value::<TextOptions>(
                value.field("options").context("saved options")?.to_json(),
            )?,
            value
                .field("reveal")
                .filter(|v| !matches!(v, Value::Null))
                .map(|v| serde_json::from_value::<RevealState>(v.to_json()))
                .transpose()?,
        )
    };
    ensure!(markup.len() <= 1024 * 1024, "text markup byte limit");
    let localizer = state.localization.client(owner);
    let fonts = localizer.fonts()?;
    let locale = localizer.locale.clone();
    let storage = state
        .runtime
        .host
        .fs
        .storage(state.client)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let surfaces = state.runtime.assets.freeze_surfaces();
    let decoders = state.runtime.assets.decoders.clone();
    let output_key = key.clone();
    let ticket = state.rich_text.asynchronous.jobs.submit(
        owner,
        epoch,
        "text_draw",
        &pool,
        move |cancel| {
            let mut block =
                text_api::build_from(&text, &markup, options, slot, fonts, locale, |path| {
                    cancel.check()?;
                    if let Some(surface) = surfaces.surface(ClientId(owner), path) {
                        return Ok(IconBitmap {
                            width: surface.width,
                            height: surface.height,
                            rgba: surface.rgba.to_vec(),
                        });
                    }
                    let file =
                        VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let input = storage
                        .open_asset(&file)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    let bytes = crysirror_resource::read_all(
                        input.as_ref(),
                        crysirror_image::MAX_DECODED_BYTES as u64,
                    )?;
                    let dependencies = |path: &str| {
                        let result = (|| -> Result<Vec<u8>> {
                            let file = VfsPath::new(Mount::Asset, path)
                                .map_err(|e| anyhow::anyhow!("{e}"))?;
                            let input = storage
                                .open_asset(&file)
                                .map_err(|e| anyhow::anyhow!("{e}"))?;
                            crysirror_resource::read_all(
                                input.as_ref(),
                                crysirror_image::MAX_DECODED_BYTES as u64,
                            )
                        })();
                        result.map_err(|e| crysirror_core::Error::Io(format!("{e:#}")))
                    };
                    let image =
                        decoders.decode_with(path, &bytes, Some(&dependencies), None, None)?;
                    Ok(IconBitmap {
                        width: image.width,
                        height: image.height,
                        rgba: image.rgba,
                    })
                })?;
            if let Some(reveal) = reveal {
                block
                    .writer
                    .as_mut()
                    .context("saved reveal needs typewriter options")?
                    .restore(reveal)?;
            }
            cancel.check()?;
            let rgba = raster(&text, &block.layout, &raster_options(&block), &block.icons)?;
            Ok(Product::Drawn {
                key: output_key,
                block,
                rgba,
            })
        },
    )?;
    // Admission succeeds before replacing a prior preparation or pausing its reveal.
    cancel_name(state, &key);
    if let Some(block) = state.rich_text.blocks.get_mut(&key) {
        if let Some(pending) = block.render.pending.take() {
            if let Some(mut previous) = pending.previous {
                previous.paused = block.writer.as_ref().unwrap().state().paused;
                block.writer.as_mut().unwrap().restore(previous)?;
            }
        }
    }
    state
        .rich_text
        .asynchronous
        .slots
        .insert(key, (slot, ticket));
    Ok(Some(Value::Int(ticket.try_into()?)))
}
fn schedule(
    state: &EngineState,
    block: &mut Block,
    previous: Option<RevealState>,
    events: Vec<RevealEvent>,
) -> Result<()> {
    ensure!(
        block.events.len() + events.len() <= 4096,
        "text event queue full; drainEvents before advancing"
    );
    let text = state.runtime.assets.text.clone();
    let layout = block.layout.clone();
    let options = raster_options(block);
    let icons = block.icons.clone();
    let visible = visible(block);
    let result = state.runtime.assets.cpu_tasks().spawn(move |cancel| {
        cancel.check()?;
        let pixels = raster(&text, &layout, &options, &icons)?;
        cancel.check()?;
        Ok(pixels)
    });
    match result {
        Ok(task) => {
            block.render.pending = Some(PendingRaster {
                task,
                previous,
                events,
                visible,
            });
            Ok(())
        }
        Err(error) => {
            if let Some(previous) = previous {
                block.writer.as_mut().unwrap().restore(previous)?;
            }
            Err(error)
        }
    }
}
fn publish_raster(state: &mut EngineState, owner: ClientId, block: &mut Block) -> Result<bool> {
    let Some(pending) = &mut block.render.pending else {
        return Ok(true);
    };
    let result = pending.task.try_take();
    match result {
        Ok(None) => Ok(false),
        Ok(Some(pixels)) => {
            let pending = block.render.pending.take().unwrap();
            if let Err(error) = state.runtime.assets.put_surface_arc(
                owner,
                block.slot,
                block.layout.canvas[0],
                block.layout.canvas[1],
                pixels,
            ) {
                if let Some(mut previous) = pending.previous {
                    previous.paused = block.writer.as_ref().unwrap().state().paused;
                    block.writer.as_mut().unwrap().restore(previous)?;
                }
                block.render.error = Some(format!("{error:#}"));
                return Ok(false);
            }
            text_api::enqueue(block, pending.events)?;
            block.render.published = pending.visible;
            Ok(true)
        }
        Err(error) => {
            let pending = block.render.pending.take().unwrap();
            if let Some(mut previous) = pending.previous {
                previous.paused = block.writer.as_ref().unwrap().state().paused;
                block.writer.as_mut().unwrap().restore(previous)?;
            }
            block.render.error = Some(format!("{error:#}"));
            Ok(false)
        }
    }
}
pub(crate) fn reveal(state: &mut EngineState, delta: u64) -> Result<()> {
    let mut blocks = std::mem::take(&mut state.rich_text.blocks);
    let result = (|| {
        for (key, block) in &mut blocks {
            if state.rich_text.asynchronous.slots.contains_key(key) || block.render.error.is_some()
            {
                continue;
            }
            block.render.elapsed = block.render.elapsed.saturating_add(delta);
            if !publish_raster(state, ClientId(key.0), block)? {
                continue;
            }
            let elapsed = std::mem::take(&mut block.render.elapsed);
            let previous = block.writer.as_ref().map(|w| w.state());
            let before = visible(block);
            let events = if let Some(writer) = &mut block.writer {
                writer.advance(elapsed)?
            } else {
                vec![]
            };
            if before != visible(block)
                || state
                    .runtime
                    .assets
                    .surface(ClientId(key.0), &format!("__surface/{}", block.slot))
                    .is_none()
            {
                if let Err(error) = schedule(state, block, previous, events) {
                    if error.is::<crysirror_tasks::CapacityExceeded>() {
                        block.render.elapsed = elapsed;
                        continue;
                    }
                    return Err(error);
                }
            } else {
                text_api::enqueue(block, events)?;
            }
        }
        Ok(())
    })();
    state.rich_text.blocks = blocks;
    result
}
pub(crate) fn finish(state: &mut EngineState, block: &mut Block) -> Result<()> {
    let previous = block
        .render
        .pending
        .as_ref()
        .and_then(|p| p.previous.clone())
        .or_else(|| block.writer.as_ref().map(|w| w.state()));
    let mut events = block
        .render
        .pending
        .take()
        .map_or_else(Vec::new, |p| p.events);
    events.extend(
        block
            .writer
            .as_mut()
            .context("text has no typewriter")?
            .finish(false),
    );
    schedule(state, block, previous, events)
}
