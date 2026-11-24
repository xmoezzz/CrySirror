//! Native font naming/wait policy over the engine's asynchronous font library.
//! Reference: wtnbgo/addFont @ 86e6ecc27844a2b4aea8ca9d171061b70bd0072b,
//! complete main.cpp/readme.txt inspected. Native registration is private to the
//! process and returns registered face count. This facade currently admits only
//! single-face sources; collection-wide native registration fails explicitly.
//! Both native extraction policies use the engine capability storage: no
//! temporary installation or global OS font registration is needed.
use crate::natives::{Rt, font_property};
use crysirror_core::service::{FontJob, FontLibrary, FontLoadSource, FontLoadSpec, FontLoadState};
use crysirror_tjs2::{
    runtime::{ObjectHandle, Variant},
    Result, TjsError,
};
use std::sync::Arc;
#[derive(Default)]
pub(crate) struct Fonts {
    pub library: Option<Arc<dyn FontLibrary>>,
    pub pending: Option<(String, FontJob)>,
    pub pending_binding: Option<String>,
    pub mapped: std::collections::BTreeMap<String, String>,
    pub loaded: std::collections::BTreeMap<String, crysirror_core::service::FontInfo>,
}
pub(crate) fn add(rt: &mut Rt, name: String) -> Result<Variant> {
    begin(rt, name, None)
}
fn signature(rt: &Rt, font: ObjectHandle) -> Result<String> {
    let mut values = Vec::new();
    for member in ["face", "height", "bold", "italic", "angle", "underline", "strikeout", "faceIsFileName"] {
        values.push(font_property(rt, font, member).to_tjs_string()?);
    }
    let key = serde_json::to_string(&values).map_err(|e| TjsError::runtime(e.to_string()))?;
    if key.len() > 4096 {
        return Err(TjsError::runtime("native font binding signature length"));
    }
    Ok(key)
}
fn mapped_alias(rt: &Rt, font: ObjectHandle) -> Result<Option<&str>> {
    let mapped = &rt.host().fonts.mapped;
    if mapped.is_empty() {
        return Ok(None);
    }
    Ok(mapped.get(&signature(rt, font)?).map(String::as_str))
}
pub(crate) fn map(rt: &mut Rt, font: ObjectHandle, name: String) -> Result<Variant> {
    let size = font_property(rt, font, "height").to_integer()?.unsigned_abs();
    if !(1..=512).contains(&size) {
        return Err(TjsError::runtime(
            "Font.mapPrerenderedFont height must have magnitude 1..512",
        ));
    }
    let binding = signature(rt, font)?;
    begin(rt, name, Some((binding, size as u16)))?;
    Ok(Variant::Void)
}
pub(crate) fn unmap(rt: &mut Rt, font: ObjectHandle) -> Result<Variant> {
    let binding = signature(rt, font)?;
    rt.host_mut().fonts.mapped.remove(&binding);
    Ok(Variant::Void)
}
fn begin(rt: &mut Rt, source_name: String, mapping: Option<(String, u16)>) -> Result<Variant> {
    begin_mode(rt,source_name,mapping,true)
}
fn begin_mode(rt: &mut Rt, source_name: String, mapping: Option<(String, u16)>,suspend:bool) -> Result<Variant> {
    let name = match &mapping {
        Some((_, size)) => format!("precomputed:{size}:{source_name}"),
        None => source_name.clone(),
    };
    let lib = rt
        .host()
        .fonts
        .library
        .clone()
        .ok_or_else(|| TjsError::runtime("engine font library absent"))?;
    // Native loading is process registration, repeated calls of an already loaded
    // immutable mounted face return success without additional font leases.
    if let Some(info) = rt.host().fonts.loaded.get(&name).cloned() {
        if let Some((binding, _)) = &mapping {
            rt.host_mut()
                .fonts
                .mapped
                .insert(binding.clone(), info.alias);
        }
        return Ok(Variant::Integer(1));
    }
    if !rt.host().can_suspend {
        return Err(TjsError::runtime(
            "System.addFont requires a resumable font worker boundary",
        ));
    }
    if rt.host().fonts.pending.is_some() {
        return Err(TjsError::runtime(
            "native font registration already pending",
        ));
    }
    let path = rt
        .host()
        .storage
        .as_ref()
        .and_then(|s| s.resolve(&source_name));
    let source = if let Some(path) = path {
        let path = crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &path)
            .map_err(|e| TjsError::runtime(e.to_string()))?;
        match mapping.as_ref() {
            Some((_, pixel_size)) => FontLoadSource::PrecomputedStorage {
                path,
                pixel_size: *pixel_size,
            },
            None => FontLoadSource::Storage(path),
        }
    } else {
        let bytes = rt
            .read_storage_binary(&source_name, "")
            .map_err(|e| TjsError::runtime(format!("font registration: {e:#}")))?
            .into();
        match mapping.as_ref() {
            Some((_, pixel_size)) => FontLoadSource::PrecomputedBytes {
                bytes,
                pixel_size: *pixel_size,
            },
            None => FontLoadSource::Bytes(bytes),
        }
    };
    let ticket = lib
        .begin(FontLoadSpec {
            name: name.clone(),
            source,
            face_index: 0,
        })
        .map_err(|e| TjsError::runtime(format!("System.addFont: {e}")))?;
    rt.host_mut().fonts.pending_binding = mapping.map(|(binding, _)| binding);
    rt.host_mut().fonts.pending = Some((name, ticket));
    if suspend {rt.request_suspend();}
    Ok(Variant::Integer(1))
}
pub(crate) fn poll(rt: &mut Rt) -> anyhow::Result<bool> {
    let Some((name, ticket)) = rt.host().fonts.pending.clone() else {
        return Ok(true);
    };
    let lib = rt
        .host()
        .fonts
        .library
        .clone()
        .ok_or_else(|| anyhow::anyhow!("engine font library absent"))?;
    match lib.status(ticket)? {
        FontLoadState::Pending => Ok(false),
        FontLoadState::Failed(error) => {
            rt.host_mut().fonts.pending = None;
            rt.host_mut().fonts.pending_binding = None;
            lib.release_job(ticket)?;
            anyhow::bail!("System.addFont({name:?}): {error}")
        }
        FontLoadState::Ready(info) => {
            lib.release_job(ticket)?;
            if info.collection_faces != 1 {
                lib.release_font(info.handle)?;
                rt.host_mut().fonts.pending = None;
                anyhow::bail!(
                    "System.addFont: collection-wide native registration unsupported ({} faces); shared font service supports explicit face selection",
                    info.collection_faces
                );
            }
            let host = rt.host_mut();
            let binding = host.fonts.pending_binding.take();
            if binding.is_none() && !lib.is_loaded(&host.font)? {
                host.font = info.alias.clone();
            }
            host.push_log(format!(
                "[addFont] {name}: {} {:?} revision {}",
                info.digest, info.families, info.revision
            ));
            if let Some(binding) = binding {
                host.fonts.mapped.insert(binding, info.alias.clone());
            }
            host.fonts.loaded.insert(name, info);
            host.fonts.pending = None;
            host.api_calls += 1;
            Ok(true)
        }
    }
}
pub(crate) struct RasterSize {
    pub measure: f32,
    pub draw: u16,
    pub offset_y: i32,
}

/// The Windows font rasterizer requests a negative LOGFONT height (em pixels)
/// and positions ink at GetOutlineTextMetrics.otmAscent. Precomputed fonts
/// already carry the game's integer advances and bitmap origins.
pub(crate) fn raster_size(rt: &Rt, font: ObjectHandle, alias: &str, em: u64) -> Result<RasterSize> {
    let identity = RasterSize { measure: em as f32, draw: em as u16, offset_y: 0 };
    if mapped_alias(rt, font)? == Some(alias) { return Ok(identity); }
    let Some(lib) = &rt.host().fonts.library else { return Ok(identity) };
    if !lib.is_loaded(alias).map_err(|e| TjsError::runtime(e.to_string()))? { return Ok(identity); }
    let metric = match lib.outline_em_metrics(alias, em as f32) {
        Ok(m) => m,
        Err(crysirror_core::Error::Unsupported(_)) => return Ok(identity),
        Err(e) => return Err(TjsError::runtime(e.to_string())),
    };
    let draw = metric.line_height.round().max(1.) as u16;
    let draw_ascent = metric.ascent * f32::from(draw) / metric.line_height;
    Ok(RasterSize {
        measure: metric.line_height, draw,
        offset_y: (metric.typographic_ascent.round() - draw_ascent).round() as i32,
    })
}

/// KiriKiri's comma-separated family preference list. Shared registrations are
/// consulted first; configured project font is the portable host fallback.
pub(crate) fn alias(rt: &mut Rt, font: ObjectHandle) -> Result<String> {
    if let Some(alias) = mapped_alias(rt, font)? {
        return Ok(alias.to_owned());
    }
    outline_alias(rt, font)
}

fn outline_alias(rt: &mut Rt, font: ObjectHandle) -> Result<String> {
    let face = font_property(rt, font, "face").to_tjs_string()?;
    if font_property(rt, font, "faceIsFileName").is_truthy() {
        if face.is_empty(){return Err(TjsError::runtime("Font.faceIsFileName requires a resource filename"));}
        if let Some(info)=rt.host().fonts.loaded.get(&face){return Ok(info.alias.clone());}
        if rt.host().fonts.pending.is_some(){return Err(TjsError::resource_pending("font file preparation"));}
        begin_mode(rt,face,None,false)?;
        return Err(TjsError::resource_pending("font file preparation"));
    }
    if let Some(lib) = &rt.host().fonts.library {
        for name in face.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if name.starts_with('@') {
                return Err(TjsError::runtime(
                    "vertical native Font.face requires a mapped vertical layout profile",
                ));
            }
            if let Some(info) = lib
                .resolve(name)
                .map_err(|e| TjsError::runtime(e.to_string()))?
            {
                return Ok(info.alias);
            }
            if lib
                .is_loaded(name)
                .map_err(|e| TjsError::runtime(e.to_string()))?
            {
                return Ok(name.into());
            }
        }
    }
    Ok(rt.host().font.clone())
}

pub(crate) struct FontRun<'a> {
    pub text: &'a str,
    pub alias: String,
    pub raster: RasterSize,
    pub mapped: bool,
}

/// A bitmap font can cover only part of the text. KiriKiri uses its mapped
/// glyphs where available and measures/draws the remaining glyphs with the
/// underlying outline face (krkrz LayerBitmapImpl.cpp, GetTextSize).
pub(crate) fn text_runs<'a>(rt: &mut Rt, font: ObjectHandle, em: u64, text: &'a str) -> Result<Vec<FontRun<'a>>> {
    let Some(mapped) = mapped_alias(rt, font)?.map(str::to_owned) else {
        let alias = outline_alias(rt, font)?;
        let raster = raster_size(rt, font, &alias, em)?;
        return Ok(vec![FontRun { text, alias, raster, mapped: false }]);
    };
    let library = rt.host().fonts.library.clone()
        .ok_or_else(|| TjsError::runtime("engine font library absent"))?;
    let mut segments: Vec<(usize, bool)> = Vec::new();
    for (start, ch) in text.char_indices() {
        let bitmap = library.glyph_id(&mapped, if ch == '\t' { ' ' } else { ch }).is_ok();
        if segments.last().is_none_or(|(_, prev)| *prev != bitmap) {
            segments.push((start, bitmap));
        }
    }
    let outline = if segments.iter().any(|(_, bitmap)| !*bitmap) {
        Some(outline_alias(rt, font)?)
    } else {
        None
    };
    segments.iter().enumerate().map(|(i, &(start, bitmap))| {
        let end = segments.get(i + 1).map_or(text.len(), |(next, _)| *next);
        let alias = if bitmap { &mapped } else { outline.as_ref().expect("outline fallback") };
        Ok(FontRun {
            text: &text[start..end], alias: alias.clone(),
            raster: raster_size(rt, font, alias, em)?, mapped: bitmap,
        })
    }).collect()
}
