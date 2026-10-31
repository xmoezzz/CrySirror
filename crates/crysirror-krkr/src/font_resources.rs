//! Native font naming/wait policy over the engine's asynchronous font library.
//! Reference: wtnbgo/addFont @ 86e6ecc27844a2b4aea8ca9d171061b70bd0072b,
//! complete main.cpp/readme.txt inspected. Native registration is private to the
//! process and returns registered face count. This facade currently admits only
//! single-face sources; collection-wide native registration fails explicitly.
//! Both native extraction policies use the engine capability storage: no
//! temporary installation or global OS font registration is needed.
use crate::natives::Rt;
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
fn signature(rt: &mut Rt, font: ObjectHandle) -> Result<String> {
    let mut values = Vec::new();
    for member in ["face", "height", "bold", "italic", "angle"] {
        values.push(rt.resolve_object_member(font, member)?.to_tjs_string()?);
    }
    let key = serde_json::to_string(&values).map_err(|e| TjsError::runtime(e.to_string()))?;
    if key.len() > 4096 {
        return Err(TjsError::runtime("native font binding signature length"));
    }
    Ok(key)
}
pub(crate) fn is_mapped(rt: &mut Rt, font: ObjectHandle) -> Result<bool> {
    let key = signature(rt, font)?;
    Ok(rt.host().fonts.mapped.contains_key(&key))
}
pub(crate) fn map(rt: &mut Rt, font: ObjectHandle, name: String) -> Result<Variant> {
    let size = rt
        .resolve_object_member(font, "height")?
        .to_integer()?
        .unsigned_abs();
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
    rt.request_suspend();
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
/// KiriKiri's comma-separated family preference list. Shared registrations are
/// consulted first; configured project font is the portable host fallback.
pub(crate) fn alias(rt: &mut Rt, font: ObjectHandle) -> Result<String> {
    let binding = signature(rt, font)?;
    if let Some(alias) = rt.host().fonts.mapped.get(&binding) {
        return Ok(alias.clone());
    }
    let face = rt.resolve_object_member(font, "face")?.to_tjs_string()?;
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
