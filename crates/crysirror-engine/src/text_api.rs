// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Rich text surfaces and reveal events, scoped to the current engine client.
use crate::{
    EngineContext, EngineState, Value,
    object::{Class, ClassInfo, MethodInfo, ObjectId, TypeHint as T},
};
use anyhow::{Context, Result, bail, ensure};
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
    sync::{Arc, OnceLock},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TextOptions {
    pub style: Option<TextStyle>,
    pub layout: LayoutOptions,
    pub raster: RasterOptions,
    pub graphemes_per_second: Option<f64>,
}
pub(crate) struct Block {
    pub(crate) slot: u32,
    pub(crate) markup: String,
    pub(crate) options: TextOptions,
    pub(crate) layout: Arc<Layout>,
    pub(crate) icons: BTreeMap<String, Arc<IconBitmap>>,
    pub(crate) writer: Option<Typewriter>,
    pub(crate) events: VecDeque<serde_json::Value>,
    pub(crate) render: crate::text_async::RenderState,
}
#[derive(Default)]
pub struct TextRuntime {
    pub(crate) blocks: BTreeMap<(u32, String), Block>,
    pub(crate) asynchronous: crate::text_async::TextJobs,
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
pub(crate) fn enqueue(block: &mut Block, events: Vec<RevealEvent>) -> Result<()> {
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
        .for_client(owner)
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
fn build(state: &mut EngineState, markup: &str, options: TextOptions, slot: u32) -> Result<Block> {
    let client = state.client;
    let localizer = state.localization.client(client.0);
    let fonts = localizer.fonts()?;
    let locale = localizer.locale.clone();
    let text = state.runtime.assets.text.for_client(state.client);
    build_from(&text, markup, options, slot, fonts, locale, |asset| {
        if let Some(surface) = state.runtime.assets.surface(client, asset) {
            return Ok(IconBitmap {
                width: surface.width,
                height: surface.height,
                rgba: surface.rgba.to_vec(),
            });
        }
        let bytes = state.runtime.assets.read_for(client, asset)?;
        let surface = state.runtime.assets.decode_image(client, asset, &bytes)?;
        Ok(IconBitmap {
            width: surface.width,
            height: surface.height,
            rgba: surface.rgba,
        })
    })
}
pub(crate) fn build_from(
    text: &crysirror_text::TextSystem,
    markup: &str,
    mut options: TextOptions,
    slot: u32,
    mut fonts: Vec<String>,
    locale: String,
    mut icon: impl FnMut(&str) -> Result<IconBitmap>,
) -> Result<Block> {
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
        options.layout.language = Some(locale);
    }
    options.layout.validate()?;
    options.raster.validate()?;
    let document = RichText::parse(markup, options.style.clone().unwrap())?;
    let layout = text.layout_rich(&document, &options.layout)?;
    let mut icons = BTreeMap::new();
    for node in &document.nodes {
        if let RichNode::Icon { asset, .. } = node {
            if icons.contains_key(asset) {
                continue;
            }
            let image = Arc::new(icon(asset)?);
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
        layout: Arc::new(layout),
        icons,
        writer,
        events: VecDeque::new(),
        render: Default::default(),
    })
}
pub(crate) fn summary(block: &Block) -> serde_json::Value {
    serde_json::json!({"markup":block.markup,"options":block.options,"reveal":block.writer.as_ref().map(Typewriter::state),"surface":format!("__surface/{}",block.slot),"glyphs":block.layout.glyphs.len(),"lines":block.layout.lines.len(),"graphemes":block.layout.grapheme_count,"clipped":block.layout.clipped,"visible_text":block.writer.as_ref().map(Typewriter::visible_text).unwrap_or(&block.layout.text),"render_pending":block.render.pending.is_some(),"render_error":block.render.error,"published_graphemes":if block.render.pending.is_some() || block.render.error.is_some(){block.render.published}else{block.writer.as_ref().map_or(block.layout.grapheme_count,|w|w.revealed())}})
}
/// Only changed reveal frames are rasterized. Explicit persistence is available
/// through Text.state/restore; decoder, font and surface objects are never saved.
pub fn update(cx: &mut EngineContext, delta_ms: u64) -> Result<()> {
    let state = cx.state()?;
    crate::text_async::poll(state)?;
    if state.runtime.assets.is_nonblocking() {
        return crate::text_async::reveal(state, delta_ms);
    }
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
            m("beginFont",vec![("alias",T::String),("asset",T::String),("faceIndex",T::Optional(Box::new(T::Int)))],T::Int,"Begin bounded asset read and font preparation. fontStatus publishes the immutable face to this client's measurement and renderer catalog."),
            m("fontStatus",vec![("ticket",T::Int)],T::Map,"Poll font workers. Ready contains canonical alias, digest, revision, families and resource handle; failed contains an error."),
            m("resolveFont",vec![("aliasOrFamily",T::String)],T::Any,"Resolve a client-owned logical alias or font family to immutable font metadata, or null."),
            m("releaseFontJob",vec![("ticket",T::Int)],T::Void,"Cancel/release registration ticket. Published faces remain until releaseFont."),
            m("releaseFont",vec![("handle",T::Int)],T::Void,"Release this client's dynamic face. Existing layer references must be removed or replaced before rendering."),
            m("beginDraw",vec![("name",T::String),("markup",T::String),("options",T::Optional(Box::new(T::Map)))],T::Int,"Prepare markup, shaping, icon decoding and initial raster on bounded workers. Returns a client/generation ticket; the old block remains until successful publication."),
            m("beginRestore",vec![("name",T::String),("state",T::Map)],T::Int,"Prepare saved text and reveal state on workers, publishing only after validation."),
            m("beginMeasure",vec![("font",T::String),("size",T::Number),("text",T::String)],T::Int,"Measure an arbitrarily sized bounded single line on a worker; status returns metrics after completion."),
            m("status",vec![("ticket",T::Int)],T::Map,"Poll and publish completed text work without joining. Ready drawing includes surface metadata; errors and obsolete generations fail visibly."),
            m("cancel",vec![("ticket",T::Int)],T::Void,"Release a ticket and cancel pending preparation. Already published blocks remain owned by Text until remove."),
            m("positionSimple",vec![("font",T::String),("size",T::Number),("text",T::String)],T::List(Box::new(T::Map)),"Unwrapped simple-horizontal glyph placements {character,baseline:[x,y]} for retained GlyphRun paint. Integer size 1..512; no surface or native VM. Live calls are bounded to 4096 UTF-8 bytes; complex shaping uses the rich text API."),
            m("measureSimple",vec![("font",T::String),("size",T::Number),("text",T::String)],T::Map,"Measure an unwrapped single line using simple-horizontal-v1 kerning and advances. Returns advance, line_height, ascent and descent; does not create a surface."),
            m("draw",vec![("name",T::String),("markup",T::String),("options",T::Map)],T::String,"Offline rich-text preparation; live hosts use beginDraw/drawAsync. Returns a shared surface path for a layer."),
            m("state",vec![("name",T::String)],T::Map,"Read content/options/reveal state for explicit persistence."),
            m("restore",vec![("name",T::String),("state",T::Map)],T::String,"Offline rebuild of saved markup/options/reveal; live hosts use beginRestore/restoreAsync. Pending event queues are not replayed."),
            m("pause",vec![("name",T::String),("paused",T::Bool)],T::Void,"Pause/resume this block's reveal clock."),
            m("finish",vec![("name",T::String)],T::Void,"Reveal remaining text. In live hosts, marker/finished events are delivered only after the worker raster is published."),
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
        if let Some(value)=crate::text_font_api::call(cx.state()?,method,args)? {return Ok(value);}
        if let Some(value) = crate::text_async::call(cx.state()?, method, args)? {
            return Ok(value);
        }
        if matches!(method,"measureSimple"|"positionSimple") {
            let font = args.first().context("font alias")?.as_str()?;
            let size = args.get(1).context("font size")?.as_f32()?;
            let text = args.get(2).context("text")?.as_str()?;
            ensure!(
                !cx.state()?.runtime.assets.is_nonblocking() || text.len() <= 4096,
                "large text measurement needs beginMeasure in a live host"
            );
            if method=="positionSimple" {
                ensure!(size.is_finite() && size.fract()==0. && (1.0..=512.0).contains(&size),"integer font size 1..512");
                let placements={let state=cx.state()?;state.runtime.assets.text.for_client(state.client).position_simple(font,size as u16,text)?};
                return Ok(Value::from_json(serde_json::json!(placements.into_iter().map(|g|serde_json::json!({"character":g.character.to_string(),"baseline":g.baseline})).collect::<Vec<_>>())));
            }
            let state=cx.state()?;
            let metrics=state.runtime.assets.text.for_client(state.client).measure_simple(font,size,text)?;
            return Ok(Value::from_json(serde_json::json!({
                "advance": metrics.advance, "line_height": metrics.line_height,
                "ascent": metrics.ascent, "descent": metrics.descent,
            })));
        }
        let name = args.first().context("text name")?.as_str()?.to_owned();
        valid_name(&name)?;
        let state = cx.state()?;
        let client = state.client;
        let key = (client.0, name);
        if method == "draw" || method == "restore" {
            ensure!(
                !state.runtime.assets.is_nonblocking(),
                "synchronous text preparation is offline-only; use beginDraw/beginRestore"
            );
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
            crate::text_async::cancel_name(state, &key);
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
                    if state.runtime.assets.is_nonblocking() {
                        crate::text_async::finish(state, &mut block)?;
                        return Ok(Value::Null);
                    }
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
