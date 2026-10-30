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
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use std::sync::Arc;
#[derive(Default)]
pub(crate) struct Fonts {
    pub library: Option<Arc<dyn FontLibrary>>,
    pub pending: Option<(String, FontJob)>,
    pub loaded: std::collections::BTreeMap<String, crysirror_core::service::FontInfo>,
}
pub(crate) fn add(rt: &mut Rt, name: String) -> Result<Variant> {
    let lib = rt
        .host()
        .fonts
        .library
        .clone()
        .ok_or_else(|| TjsError::runtime("engine font library absent"))?;
    // Native loading is process registration, repeated calls of an already loaded
    // immutable mounted face return success without additional font leases.
    if rt.host().fonts.loaded.contains_key(&name) {
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
    let path = rt.host().storage.as_ref().and_then(|s| s.resolve(&name));
    let source = if let Some(path) = path {
        FontLoadSource::Storage(
            crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &path)
                .map_err(|e| TjsError::runtime(e.to_string()))?,
        )
    } else {
        FontLoadSource::Bytes(
            rt.read_storage_binary(&name, "")
                .map_err(|e| TjsError::runtime(format!("System.addFont: {e:#}")))?
                .into(),
        )
    };
    let ticket = lib
        .begin(FontLoadSpec {
            name: name.clone(),
            source,
            face_index: 0,
        })
        .map_err(|e| TjsError::runtime(format!("System.addFont: {e}")))?;
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
            if !lib.is_loaded(&host.font)? {
                host.font = info.alias.clone();
            }
            host.push_log(format!(
                "[addFont] {name}: {} {:?} revision {}",
                info.digest, info.families, info.revision
            ));
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
