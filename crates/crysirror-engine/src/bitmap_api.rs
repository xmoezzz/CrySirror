// SPDX-License-Identifier: MPL-2.0
//! Mutable CPU bitmaps backed by the shared, client-scoped surface store.
//!
//! Rust adapters use crysirror_image::bitmap directly; scripts reach the same
//! operations here. A returned path can be used by Layers.put or text icons.
//! Slots belong to the calling client and share its existing surface budget.
use crate::{
    EngineContext, EngineState, Value,
    object::{Class, ClassInfo, MethodInfo, ObjectId, TypeHint as T},
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::FileSystem,
};
use crysirror_image::{
    ImageSurface,
    associated::{AssociatedBlitOptions, CoveragePlane, RgbFilter},
    bitmap::{BitmapRect, BlitOptions, StretchOptions},
};
use crysirror_runtime::assets::SURFACE_PREFIX;
use serde::de::DeserializeOwned;
use std::sync::OnceLock;

fn arg<'a>(args: &'a [Value], index: usize, name: &str) -> Result<&'a Value> {
    args.get(index).with_context(|| format!("missing {name}"))
}
fn number(args: &[Value], index: usize, name: &str) -> Result<u32> {
    u32::try_from(arg(args, index, name)?.as_i64()?).with_context(|| format!("invalid {name}"))
}
fn decode<T: DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.to_json()).context("invalid bitmap operation parameters")
}
fn path(slot: u32) -> String {
    format!("{SURFACE_PREFIX}{slot}")
}
fn current(state: &EngineState, slot: u32) -> Result<ImageSurface> {
    let surface = state
        .runtime
        .assets
        .surface(state.client, &path(slot))
        .context("bitmap slot does not exist for this client")?;
    ensure!(
        crysirror_image::rgba_len(surface.width, surface.height)? == surface.rgba.len(),
        "bitmap RGBA length"
    );
    Ok(ImageSurface {
        width: surface.width,
        height: surface.height,
        rgba: surface.rgba.to_vec(),
    })
}
fn source(state: &mut EngineState, asset: &str) -> Result<ImageSurface> {
    if asset.starts_with(SURFACE_PREFIX) {
        let slot = asset[SURFACE_PREFIX.len()..]
            .parse::<u32>()
            .context("invalid bitmap surface path")?;
        ensure!(asset == path(slot), "noncanonical bitmap surface path");
        return current(state, slot);
    }
    let asset_path = VfsPath::new(Mount::Asset, asset).map_err(|e| anyhow::anyhow!("{e}"))?;
    // Read through provider mounts and the client's capability, not a raw path.
    // Native decoders may publish immutable engine resources rather than files.
    if asset_path.relative().starts_with("__crysirror/") {
        if let Some(bytes) = state
            .runtime
            .assets
            .generated_for(state.client, asset_path.relative())?
        {
            return state
                .runtime
                .assets
                .decode_image(state.client, asset, &bytes);
        }
    }
    let bytes = state
        .runtime
        .host
        .fs
        .read(state.client, &asset_path)
        .map_err(|e| anyhow::anyhow!("bitmap asset: {e}"))?;
    state
        .runtime
        .assets
        .decode_image(state.client, asset, &bytes)
}
fn publish(state: &mut EngineState, slot: u32, image: ImageSurface) -> Result<Value> {
    image.validate()?;
    // put_surface validates all budgets before replacing the old revision.
    Ok(state
        .runtime
        .assets
        .put_surface(state.client, slot, image.width, image.height, image.rgba)?
        .into())
}
fn method(
    name: &'static str,
    params: Vec<(&'static str, T)>,
    returns: T,
    doc: &'static str,
) -> MethodInfo {
    MethodInfo {
        name,
        params,
        returns,
        doc,
    }
}

pub struct BitmapClass;
impl Class for BitmapClass {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(|| ClassInfo {
            name: "Bitmap",
            doc: "Client-scoped CPU RGBA bitmaps; paths feed Layers and share its surface limits. Transient pixels are not runtime snapshots.",
            properties: vec![],
            methods: vec![
                method("beginRowCopy",vec![("slot",T::Int),("source",T::Int),("options",T::Map)],T::Int,"Prepare a SineRowCopy from a shared bitmap snapshot. Full-image clip; uncovered destination pixels are preserved. Uses effectStatus/cancelEffect."),
                method("beginTransition",vec![("slot",T::Int),("other",T::Int),("options",T::Map),("mask",T::Optional(Box::new(T::Int)))],T::Int,"Prepare a snapshot crossfade/rule-mask/blur transition into slot at explicit ImageTransition progress. Inputs must be client-owned equal-size bitmaps. Uses effectStatus/cancelEffect and rejects a stale target revision."),
                method("beginEffect",vec![("slot",T::Int),("effects",T::List(Box::new(T::Map)))],T::Int,"Prepare an ImageEffect chain on shared workers. Captures immutable pixels and the target revision; effectStatus publishes only if the client/epoch/revision still match. Max 4M pixels, 16 effects."),
                method("effectStatus",vec![("ticket",T::Int)],T::Map,"Poll a bitmap effect without waiting. Ready includes result.surface; stale or failed work is visible and does not overwrite newer pixels."),
                method("cancelEffect",vec![("ticket",T::Int)],T::Void,"Release a bitmap effect ticket and cancel unfinished work. Completed published pixels remain engine-owned."),
                method("create", vec![("slot",T::Int),("width",T::Int),("height",T::Int),("rgba",T::Optional(Box::new(T::List(Box::new(T::Int)))))], T::String,
                    "Create/replace a slot with uniform RGBA (default transparent). Returns __surface/<slot>."),
                method("load", vec![("slot",T::Int),("source",T::String)],T::String,
                    "Decode a client asset or copy an existing client surface into a mutable slot."),
                method("fill",vec![("slot",T::Int),("rect",T::Map),("rgba",T::List(Box::new(T::Int)))],T::String,
                    "Replace pixels inside clipped {x,y,width,height}; signed positions are allowed."),
                method("blit",vec![("slot",T::Int),("source",T::String),("options",T::Map)],T::String,
                    "Copy/blend source_rect into destination:[x,y]. Explicit mode and opacity; self-copy reads the original pixels."),
                method("stretch",vec![("slot",T::Int),("source",T::String),("options",T::Map)],T::String,
                    "Nearest-neighbor source_rect to destination rectangle with original mapping preserved after clipping. Explicit pixel math."),
                method("composite",vec![("slot",T::Int),("source",T::String),("options",T::Map),("coverage",T::Optional(Box::new(T::String)))],T::String,
                    "Composite associated RGB with copy/over/under, explicit 0..256 weight and preserve/source coverage_write. Optional coverage asset supplies alpha independently of source RGB. Stored channels remain associated; self-copies use snapshots."),
                method("filterRgb",vec![("slot",T::Int),("rect",T::Map),("filter",T::Map)],T::String,
                    "Filter RGB bytes, retaining alpha: {kind:'fade',rgb:[r,g,b],weight:0..256} or {kind:'multiply',factors:[0..256,0..256,0..256]}. Fade floors signed deltas."),
                method("info",vec![("slot",T::Int)],T::Map,
                    "Return client-local width, height, revision, bytes and surface path."),
                method("remove",vec![("slot",T::Int)],T::Bool,
                    "Release this client's surface slot; return whether it existed."),
            ],
        })
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, property: &str) -> Result<Value> {
        bail!("unknown Bitmap property {property}")
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        if let Some(v)=crate::bitmap_async::call(cx.state()?,name,args)?{return Ok(v)}
        let slot = number(args, 0, "slot")?;
        let state = cx.state()?;
        match name {
            "create" => {
                let width = number(args, 1, "width")?;
                let height = number(args, 2, "height")?;
                let color = match args.get(3) {
                    None | Some(Value::Null) => [0; 4],
                    Some(value) => decode(value)?,
                };
                publish(state, slot, ImageSurface::filled(width, height, color)?)
            }
            "load" => {
                let image = source(state, arg(args, 1, "source")?.as_str()?)?;
                publish(state, slot, image)
            }
            "fill" | "blit" | "stretch" => {
                // Staging keeps both parse/operation and publication failures
                // from partially changing the retained render product.
                let mut image = current(state, slot)?;
                if name == "fill" {
                    let rect: BitmapRect = decode(arg(args, 1, "rect")?)?;
                    let color = decode(arg(args, 2, "rgba")?)?;
                    image.fill_rect(rect, color)?;
                } else {
                    let src = source(state, arg(args, 1, "source")?.as_str()?)?;
                    let options = arg(args, 2, "options")?;
                    if name == "blit" {
                        image.blit(&src, decode::<BlitOptions>(options)?)?;
                    } else {
                        image.stretch_blit(&src, decode::<StretchOptions>(options)?)?;
                    }
                }
                publish(state, slot, image)
            }
            "composite" => {
                let mut image = current(state, slot)?;
                let src = source(state, arg(args, 1, "source")?.as_str()?)?;
                let options: AssociatedBlitOptions = decode(arg(args, 2, "options")?)?;
                let coverage = match args.get(3) {
                    None | Some(Value::Null) => None,
                    Some(value) => {
                        Some(CoveragePlane::from_alpha(&source(state, value.as_str()?)?)?)
                    }
                };
                image.composite_associated(&src, coverage.as_ref(), options)?;
                publish(state, slot, image)
            }
            "filterRgb" => {
                let mut image = current(state, slot)?;
                image.filter_rgb(
                    decode(arg(args, 1, "rect")?)?,
                    decode::<RgbFilter>(arg(args, 2, "filter")?)?,
                )?;
                publish(state, slot, image)
            }
            "info" => {
                let asset = path(slot);
                let surface = state
                    .runtime
                    .assets
                    .surface(state.client, &asset)
                    .context("bitmap slot does not exist for this client")?;
                Ok(Value::from_json(
                    serde_json::json!({"asset":asset,"width":surface.width,
                    "height":surface.height,"revision":surface.revision,"bytes":surface.rgba.len()}),
                ))
            }
            "remove" => Ok(Value::Bool(
                state.runtime.assets.remove_surface(state.client, slot),
            )),
            _ => bail!("unknown Bitmap method {name}"),
        }
    }
}
