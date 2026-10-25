// SPDX-License-Identifier: MPL-2.0
//! Rich text surfaces and reveal events, scoped to the current engine client.
use crate::{
    object::{Class, ClassInfo, MethodInfo, ObjectId, TypeHint as T},
    EngineContext, EngineState, Value,
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::state::ClientId;
use crysirror_text::{
    layout::{Layout, LayoutOptions},
    raster::{IconBitmap, RasterOptions},
    reveal::{RevealEvent, RevealState, Typewriter},
    rich::{RichNode, RichText, TextStyle},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::OnceLock,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextOptions {
    pub style: Option<TextStyle>,
    pub layout: LayoutOptions,
    pub raster: RasterOptions,
    pub graphemes_per_second: Option<f64>,
}
struct Block {
    slot: u32,
    markup: String,
    options: TextOptions,
    layout: Layout,
    icons: BTreeMap<String, IconBitmap>,
    writer: Option<Typewriter>,
    events: VecDeque<serde_json::Value>,
}
#[derive(Default)]
pub struct TextRuntime {
    blocks: BTreeMap<(u32, String), Block>,
}
const MAX_BLOCKS: usize = 8;
const MAX_EVENTS: usize = 4096;
fn valid_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control),
        "text block name"
    );
    Ok(())
}
fn enqueue(block: &mut Block, events: Vec<RevealEvent>) -> Result<()> {
    ensure!(
        block.events.len() + events.len() <= MAX_EVENTS,
        "text event queue full; drainEvents before advancing"
    );
    for event in events {
        block.events.push_back(match event {
            RevealEvent::Grapheme{index,bytes,text}=>serde_json::json!({"kind":"grapheme","index":index,"bytes":[bytes.start,bytes.end],"text":text}),
            RevealEvent::Marker(name)=>serde_json::json!({"kind":"marker","name":name}),
            RevealEvent::Finished=>serde_json::json!({"kind":"finished"}),
        });
    }
    Ok(())
}
fn publish(state: &mut EngineState, owner: ClientId, block: &Block) -> Result<String> {
    let mut raster = block.options.raster.clone();
    if let Some(writer) = &block.writer {
        raster.visible_graphemes = Some(writer.revealed());
    }
    let rgba = state
        .runtime
        .assets
        .text
        .rasterize_rich(&block.layout, &raster, |asset| {
            let icon = block.icons.get(asset).context("text icon not loaded")?;
            Ok(IconBitmap {
                width: icon.width,
                height: icon.height,
                rgba: icon.rgba.clone(),
            })
        })?;
    state.runtime.assets.put_surface(
        owner,
        block.slot,
        block.layout.canvas[0],
        block.layout.canvas[1],
        rgba,
    )
}
fn build(
    state: &mut EngineState,
    markup: &str,
    mut options: TextOptions,
    slot: u32,
) -> Result<Block> {
    let client = state.client;
    let localizer = state.localization.client(client.0);
    let mut fonts = localizer.fonts()?;
    if fonts.is_empty() {
        fonts.push("default".into());
    }
    if options.style.is_none() {
        options.style = Some(TextStyle {
            fonts,
            size: 24.,
            color: [255; 4],
            bold: false,
        });
    }
    if options.layout.language.is_none() {
        options.layout.language = Some(localizer.locale.clone());
    }
    options.layout.validate()?;
    options.raster.validate()?;
    let document = RichText::parse(markup, options.style.clone().unwrap())?;
    let layout = state
        .runtime
        .assets
        .text
        .layout_rich(&document, &options.layout)?;
    let mut icons = BTreeMap::new();
    for node in &document.nodes {
        if let RichNode::Icon { asset, .. } = node {
            if icons.contains_key(asset) {
                continue;
            }
            let image = if let Some(surface) = state.runtime.assets.surface(client, asset) {
                IconBitmap {
                    width: surface.width,
                    height: surface.height,
                    rgba: surface.rgba.to_vec(),
                }
            } else {
                let bytes = state.runtime.assets.read_for(client, asset)?;
                let surface = state.runtime.assets.decode_image(client,asset,bytes.as_ref())?;
                IconBitmap {
                    width: surface.width,
                    height: surface.height,
                    rgba: surface.rgba,
                }
            };
            icons.insert(asset.clone(), image);
        }
    }
    let writer = options
        .graphemes_per_second
        .map(|rate| Typewriter::new(&document, rate))
        .transpose()?;
    Ok(Block {
        slot,
        markup: markup.into(),
        options,
        layout,
        icons,
        writer,
        events: VecDeque::new(),
    })
}
fn summary(block: &Block) -> serde_json::Value {
    serde_json::json!({"markup":block.markup,"options":block.options,"reveal":block.writer.as_ref().map(Typewriter::state),"surface":format!("__surface/{}",block.slot),"glyphs":block.layout.glyphs.len(),"lines":block.layout.lines.len(),"graphemes":block.layout.grapheme_count,"clipped":block.layout.clipped,"visible_text":block.writer.as_ref().map(Typewriter::visible_text).unwrap_or(&block.layout.text)})
}
/// Only changed reveal frames are rasterized. Explicit persistence is available
/// through Text.state/restore; decoder, font and surface objects are never saved.
pub fn update(cx: &mut EngineContext, delta_ms: u64) -> Result<()> {
    let state = cx.state()?;
    let mut blocks = std::mem::take(&mut state.rich_text.blocks);
    let result = (|| {
        for ((owner, _), block) in &mut blocks {
            let before = block.writer.as_ref().map(Typewriter::revealed);
            let previous = block.writer.as_ref().map(Typewriter::state);
            let pending = block.events.len();
            let update = (|| {
                if let Some(writer) = &mut block.writer {
                    let events = writer.advance(delta_ms)?;
                    enqueue(block, events)?;
                }
                let path = format!("__surface/{}", block.slot);
                if before != block.writer.as_ref().map(Typewriter::revealed)
                    || state
                        .runtime
                        .assets
                        .surface(ClientId(*owner), &path)
                        .is_none()
                {
                    publish(state, ClientId(*owner), block)?;
                }
                Ok(())
            })();
            if let Err(error) = update {
                if let Some(previous) = previous {
                    block.writer.as_mut().unwrap().restore(previous)?;
                }
                block.events.truncate(pending);
                return Err(error);
            }
        }
        Ok(())
    })();
    state.rich_text.blocks = blocks;
    result
}
pub struct TextClass;
fn info() -> &'static ClassInfo {
    static INFO: OnceLock<ClassInfo> = OnceLock::new();
    INFO.get_or_init(||{
        fn m(name:&'static str,params:Vec<(&'static str,T)>,returns:T,doc:&'static str)->MethodInfo {MethodInfo{name,params,returns,doc}}
        ClassInfo{name:"Text",doc:"CPU-shaped rich text rendered to client-owned RGBA surfaces. Fonts must be explicitly installed.",properties:vec![],methods:vec![
            m("draw",vec![("name",T::String),("markup",T::String),("options",T::Map)],T::String,"Shape and rasterize rich text; returns __surface path for a layer. Optional typewriter rate advances with App frames."),
            m("state",vec![("name",T::String)],T::Map,"Read content/options/reveal state for explicit persistence."),
            m("restore",vec![("name",T::String),("state",T::Map)],T::String,"Rebuild saved markup/options and restore reveal clock. Pending event queues are not replayed."),
            m("pause",vec![("name",T::String),("paused",T::Bool)],T::Void,"Pause/resume this block's reveal clock."),
            m("finish",vec![("name",T::String)],T::Void,"Reveal remaining text and emit marker/finished events."),
            m("drainEvents",vec![("name",T::String)],T::List(Box::new(T::Map)),"Drain grapheme, authored marker and finished events."),
            m("remove",vec![("name",T::String)],T::Void,"Remove block and release its dynamic surface."),
        ]}
    })
}
impl Class for TextClass {
    fn info(&self) -> &ClassInfo {
        info()
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, _: &str) -> Result<Value> {
        bail!("Text has no properties")
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        method: &str,
        args: &[Value],
    ) -> Result<Value> {
        let name = args.first().context("text name")?.as_str()?.to_owned();
        valid_name(&name)?;
        let state = cx.state()?;
        let client = state.client;
        let key = (client.0, name);
        if method == "draw" || method == "restore" {
            let existing = state.rich_text.blocks.get(&key).map(|b| b.slot);
            ensure!(
                existing.is_some()
                    || state
                        .rich_text
                        .blocks
                        .keys()
                        .filter(|(owner, _)| *owner == client.0)
                        .count()
                        < MAX_BLOCKS,
                "text block limit"
            );
            let slot = existing.unwrap_or_else(|| {
                (50000..50000 + MAX_BLOCKS as u32)
                    .find(|s| {
                        !state
                            .rich_text
                            .blocks
                            .iter()
                            .any(|((owner, _), b)| *owner == client.0 && b.slot == *s)
                    })
                    .unwrap()
            });
            let (markup, options, reveal) = if method == "draw" {
                (
                    args.get(1).context("text markup")?.as_str()?.to_owned(),
                    args.get(2)
                        .map(|v| serde_json::from_value::<TextOptions>(v.to_json()))
                        .transpose()?
                        .unwrap_or_default(),
                    None,
                )
            } else {
                let value = args.get(1).context("text state")?;
                let markup = value
                    .field("markup")
                    .context("saved markup")?
                    .as_str()?
                    .to_owned();
                let options = serde_json::from_value::<TextOptions>(
                    value.field("options").context("saved options")?.to_json(),
                )?;
                let reveal = value
                    .field("reveal")
                    .filter(|v| !matches!(v, Value::Null))
                    .map(|v| serde_json::from_value::<RevealState>(v.to_json()))
                    .transpose()?;
                (markup, options, reveal)
            };
            let mut block = build(state, &markup, options, slot)?;
            if let Some(reveal) = reveal {
                block
                    .writer
                    .as_mut()
                    .context("saved reveal needs a typewriter rate")?
                    .restore(reveal)?;
            }
            let path = publish(state, client, &block)?;
            state.rich_text.blocks.insert(key, block);
            return Ok(Value::Str(path));
        }
        if method == "remove" {
            let block = state
                .rich_text
                .blocks
                .remove(&key)
                .context("text block missing")?;
            state.runtime.assets.remove_surface(client, block.slot);
            return Ok(Value::Null);
        }
        let mut block = state
            .rich_text
            .blocks
            .remove(&key)
            .context("text block missing")?;
        let result = (|| {
            Ok(match method {
                "state" => Value::from_json(summary(&block)),
                "pause" => {
                    block
                        .writer
                        .as_mut()
                        .context("text has no typewriter")?
                        .pause(args.get(1).context("paused")?.as_bool()?);
                    Value::Null
                }
                "finish" => {
                    let previous = block
                        .writer
                        .as_ref()
                        .context("text has no typewriter")?
                        .state();
                    let pending = block.events.len();
                    let finished = (|| {
                        let events = block.writer.as_mut().unwrap().finish(false);
                        enqueue(&mut block, events)?;
                        publish(state, client, &block)?;
                        Ok(())
                    })();
                    if let Err(error) = finished {
                        block.writer.as_mut().unwrap().restore(previous)?;
                        block.events.truncate(pending);
                        return Err(error);
                    }
                    Value::Null
                }
                "drainEvents" => {
                    Value::List(block.events.drain(..).map(Value::from_json).collect())
                }
                _ => bail!("unknown Text method"),
            })
        })();
        state.rich_text.blocks.insert(key, block);
        result
    }
}
