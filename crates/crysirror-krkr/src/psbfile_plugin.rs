// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `psbfile.dll` PSBFile: PSB parsed by the shared eluna_rs reader, `root`
//! exposed as plain TJS data, and `psb://<storage name>/<key>` image loads
//! served from the loaded file's resources. Embedded images are decoded by the
//! engine's shared decoder registry (`HostServices::upload_encoded_image`).
//!
//! Profile from the game scripts (psdlayer.tjs `loadPIMG`): `new PSBFile()`,
//! `load(storage)` returning success, `root`, and Layer.loadImages on
//! `psb://<lowercased extractStorageName>/<id>.tlg`.
use crate::natives::{Rt, methods, native_class, read_only, string, this_of, unsupported};
use crysirror_core::{service::HostServices, state::ClientId};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use eluna::psb::{PsbFile, PsbNormalizeOptions, PsbValue};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

pub struct Loaded {
    plain: Arc<[u8]>,
    psb: Arc<PsbFile>,
}

pub struct PendingImage {
    layer: ObjectHandle,
    name: String,
    bytes: Vec<u8>,
}

/// Plain PSB bytes retained for the `psb://` domain after their PSBFile
/// objects are gone (scripts load a local PSBFile, then its images later).
const RETAINED_BYTES: usize = 256 << 20;

#[derive(Default)]
pub struct State {
    /// Lowercased storage file name → loaded PSB; the `psb://` domain.
    files: BTreeMap<String, Arc<Loaded>>,
    /// Load order of `files`, oldest first, for bounded eviction.
    order: Vec<String>,
    /// PSBFile instance → its domain key.
    owners: BTreeMap<usize, String>,
    pending: Option<PendingImage>,
    next_slot: u32,
}

fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(format!("PSBFile: {e:#}"))
}

pub fn install(rt: &mut Rt) {
    // `new PSBFile(storage)` loads at construction (StorageData.tjs) and,
    // having no result to report, fails as an exception.
    let c = native_class(rt, "PSBFile", |rt, t, a| {
        let h = this_of(rt, t, "PSBFile")?;
        if a.first().is_some_and(|v| !matches!(v, Variant::Void)) {
            let name = string(&a, 0, "PSBFile")?;
            if !load(rt, h, &name)? {
                return Err(err(format!("{name}: storage not found")));
            }
        }
        Ok(Variant::Void)
    });
    methods(
        rt,
        c,
        &[
            ("load", |rt, t, a| {
                let h = this_of(rt, t, "PSBFile.load")?;
                let name = string(&a, 0, "PSBFile.load")?;
                Ok(Variant::Integer(load(rt, h, &name)? as i64))
            }),
            ("finalize", |rt, t, _| {
                let h = this_of(rt, t, "PSBFile.finalize")?;
                rt.host_mut().psbfile.owners.remove(&h.0);
                Ok(Variant::Void)
            }),
        ],
    );
    rt.register_object_native_property(
        c,
        "root",
        |rt, t| {
            let h = this_of(rt, t, "PSBFile.root")?;
            Ok(rt.object_member(h, "__root"))
        },
        read_only,
    );
}

fn load(rt: &mut Rt, h: ObjectHandle, name: &str) -> Result<bool> {
    let bytes = match rt.read_storage_binary(name, "") {
        Ok(b) => b,
        Err(_) => return Ok(false),
    };
    let (plain, psb) = PsbFile::parse_normalized(&bytes, &PsbNormalizeOptions::default())
        .map_err(|e| err(format!("{name}: {e}")))?;
    let key = name
        .rsplit(['/', '>'])
        .next()
        .unwrap_or(name)
        .to_lowercase();
    let root = crate::motion_plugin::value(rt, &psb.root);
    rt.set_object_member(h, "__root", root);
    let state = &mut rt.host_mut().psbfile;
    state.files.insert(
        key.clone(),
        Arc::new(Loaded {
            plain: plain.into(),
            psb: Arc::new(psb),
        }),
    );
    state.order.retain(|k| *k != key);
    state.order.push(key.clone());
    state.owners.insert(h.0, key);
    evict(state);
    Ok(true)
}

/// Keep the domain within RETAINED_BYTES, dropping the oldest files that no
/// live PSBFile names.
fn evict(state: &mut State) {
    let mut total: usize = state.files.values().map(|l| l.plain.len()).sum();
    let mut i = 0;
    while total > RETAINED_BYTES && i < state.order.len() {
        let key = &state.order[i];
        if state.owners.values().any(|k| k == key) {
            i += 1;
            continue;
        }
        if let Some(l) = state.files.remove(key) {
            total -= l.plain.len();
        }
        state.order.remove(i);
    }
}

/// `psb://<file>/<key>` → encoded resource bytes of a loaded PSBFile.
fn resource(rt: &Rt, uri: &str) -> Result<Vec<u8>> {
    let rest = uri
        .strip_prefix("psb://")
        .ok_or_else(|| err("not a psb:// name"))?;
    let (file, key) = rest
        .split_once('/')
        .ok_or_else(|| err(format!("{uri}: missing resource key")))?;
    let loaded = rt
        .host()
        .psbfile
        .files
        .get(&file.to_lowercase())
        .ok_or_else(|| err(format!("{uri}: no loaded PSBFile `{file}`")))?;
    let value = loaded
        .psb
        .root
        .field(key)
        .ok_or_else(|| err(format!("{uri}: no root entry `{key}`")))?;
    let PsbValue::Resource(index) = value else {
        return Err(err(format!("{uri}: root entry is not a resource")));
    };
    loaded
        .psb
        .resource_bytes(&loaded.plain, *index as usize)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| err(format!("{uri}: resource {index} out of range")))
}

/// Layer.loadImages on a `psb://` name: suspend until the engine decodes the
/// resource, then attach it as the layer's image.
pub fn load_image(rt: &mut Rt, layer: ObjectHandle, uri: &str) -> Result<Variant> {
    let bytes = resource(rt, uri)?;
    load_encoded_image(rt, layer, uri, bytes)
}

/// Native encoded-resource/wait policy; actual decode/upload is engine owned.
pub(crate) fn load_encoded_image(
    rt: &mut Rt,
    layer: ObjectHandle,
    uri: &str,
    bytes: Vec<u8>,
) -> Result<Variant> {
    if !rt.host().can_suspend {
        return Err(unsupported(
            "Layer.loadImages(encoded resource) in this callback",
            "a resumable TJS boundary",
        ));
    }
    let state = &mut rt.host_mut().psbfile;
    if state.pending.is_some() {
        return Err(err("image load already pending"));
    }
    state.pending = Some(PendingImage {
        layer,
        name: uri.to_owned(),
        bytes,
    });
    rt.request_suspend();
    Ok(Variant::Void)
}

pub fn poll(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<bool> {
    let Some(p) = rt.host_mut().psbfile.pending.take() else {
        return Ok(true);
    };
    let state = &mut rt.host_mut().psbfile;
    let slot = 0x6000_0000u32
        .checked_add(state.next_slot)
        .ok_or_else(|| anyhow::anyhow!("psb image slots exhausted"))?;
    state.next_slot += 1;
    let name = p.name.rsplit('/').next().unwrap_or(&p.name).to_owned();
    let image = host.upload_encoded_image(client, slot, &name, &p.bytes)?;
    let [w, h] = image.size.map(i64::from);
    let e = |e: TjsError| anyhow::anyhow!("{e}");
    rt.host_mut()
        .reset_surface(p.layer, w, h, [255, 255, 255, 0])
        .map_err(e)?;
    let host_state = rt.host_mut();
    let l = host_state.layer_mut(p.layer).map_err(e)?;
    (l.width, l.height) = (w, h);
    l.blank = false;
    l.image_source = None;
    let slot = l.slot;
    host_state.canvas_op(json!({"op":"set","id":slot,"layer":{"id":0,"rect":[0,0,w,h],"paint":{"kind":"image","asset":image.path.relative()}}}));
    // Canvas writes invalidate retained alpha metadata. Publish the decoder's
    // verified coverage only after that write, as ordinary loadImages does.
    host_state.layer_mut(p.layer).map_err(e)?.full_alpha = image.opaque.then_some(255);
    host_state.api_calls += 1;
    Ok(true)
}

impl State {
    pub(crate) fn invalidate(&mut self, object: usize) {
        self.owners.remove(&object);
    }

    pub(crate) fn gc_roots(&self, out: &mut Vec<ObjectHandle>) {
        // PSBFile instances are owned by scripts; a collected one drops its
        // entry through `invalidate`.
        if let Some(p) = &self.pending {
            out.push(p.layer);
        }
    }
}
