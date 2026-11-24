// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! `psbfile.dll` PSBFile: PSB parsed by the shared eluna_rs reader, `root`
//! exposed as plain TJS data, and `psb://<storage name>/<key>` image loads
//! served from the loaded file's resources. Embedded images are decoded by the
//! shared worker image service. Canvas commits retire the temporary publication
//! while native layers and frozen frames keep their own image references.
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
use std::{collections::{BTreeMap, VecDeque}, sync::{Arc, Mutex, Weak}};
use crysirror_core::image_processing::{ImageJob, ImageResource};

pub struct Loaded {
    mounted_path: Option<String>,
    plain: Arc<[u8]>,
    psb: Arc<PsbFile>,
    decoded: Mutex<BTreeMap<usize, Weak<dyn ImageResource>>>,
}

impl Loaded {
    fn parse(name: &str, bytes: Vec<u8>, mounted_path: Option<String>) -> Result<Arc<Self>> {
        let (plain, psb) = PsbFile::parse_normalized_owned(bytes, &PsbNormalizeOptions::default())
            .map_err(|e| err(format!("{name}: {e}")))?;
        Ok(Arc::new(Self { mounted_path, plain: plain.into(), psb: Arc::new(psb), decoded: Default::default() }))
    }
}

pub struct PendingImage {
    layer: ObjectHandle,
    name: String,
    bytes: Vec<u8>,
    member: Option<(Arc<Loaded>, usize)>,
}

/// Retained `psb://` package bytes and the most recent large PNG's CPU pixels.
/// Scripts can load a local PSBFile, then use its images after it is gone.
const RETAINED_BYTES: usize = 256 << 20;

// Small UI members can reuse a Canvas publication. PNG overlays may retain
// just their pixels under the same per-session byte and entry limits.
const UI_MEMBER_BYTES: u64 = 256 << 10;
const MEMBER_CACHE_BYTES: u64 = 8 << 20;
const MEMBER_CACHE_COUNT: usize = 256;
// A single PNG this large can crowd out its own overlays and the gallery UI.
const LARGE_PNG_BYTES: u64 = MEMBER_CACHE_BYTES / 2;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

struct CachedMember {
    file: Weak<Loaded>,
    index: usize,
    path: Option<String>,
    image: Arc<dyn ImageResource>,
    bytes: u64,
}

#[derive(Default)]
pub struct State {
    registered: bool,
    /// Lowercased storage file name → loaded PSB; the `psb://` domain.
    files: BTreeMap<String, Arc<Loaded>>,
    /// Load order of `files`, oldest first, for bounded eviction.
    order: Vec<String>,
    /// PSBFile instance → its domain key.
    owners: BTreeMap<usize, String>,
    pending: Option<PendingImage>,
    decoding: Option<(ObjectHandle, ImageJob, Option<(Arc<Loaded>, usize)>)>,
    ready: Option<(ObjectHandle, Arc<dyn ImageResource>)>,
    /// Images loaded inside a non-suspending callback, publishing next boundary.
    deferred: Vec<(ObjectHandle, crysirror_core::image_processing::ImageInput)>,
    next_slot: u32,
    members: VecDeque<CachedMember>,
    member_bytes: u64,
    recent_large_png: Option<CachedMember>,
}

fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(format!("PSBFile: {e:#}"))
}

pub fn install(rt: &mut Rt) {
    rt.host_mut().psbfile.registered = true;
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
    let key = name
        .rsplit(['/', '>'])
        .next()
        .unwrap_or(name)
        .to_lowercase();
    let path = mounted_path(rt, name);
    let prior = rt.host().psbfile.files.get(&key).cloned();
    let loaded = if let Some(file) = prior.as_ref().filter(|f| path.is_some() && f.mounted_path == path) {
        file.clone()
    } else {
        let Ok(bytes) = rt.read_storage_binary(name, "") else { return Ok(false) };
        Loaded::parse(name, bytes, path)?
    };
    let root = crate::motion_plugin::value(rt, &loaded.psb.root);
    rt.set_object_member(h, "__root", root);
    let state = &mut rt.host_mut().psbfile;
    state.owners.insert(h.0, key.clone());
    remember(state, key, loaded);
    Ok(true)
}

fn mounted_path(rt: &Rt, name: &str) -> Option<String> {
    let storage = rt.host().storage().ok()?;
    let path = storage.resolve(name)?;
    let checked = crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, &path).ok()?;
    storage.is_mounted_asset(&checked).then_some(path)
}

fn remember(state: &mut State, key: String, loaded: Arc<Loaded>) {
    state.files.insert(key.clone(), loaded);
    state.order.retain(|k| *k != key);
    state.order.push(key);
    evict(state);
}

/// Reclaim the optional PNG lease first and then the oldest files that no
/// live PSBFile names.
fn evict(state: &mut State) {
    let mut total = state.package_bytes();
    if state.recent_large_png.as_ref().is_some_and(|image| {
        !state.contains_package(&image.file) || total as u64 + image.bytes > RETAINED_BYTES as u64
    }) {
        state.recent_large_png = None;
    }
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

/// The media can open the named storage itself. A live script PSBFile with
/// that name still takes precedence over a storage search.
fn find_resource(rt: &mut Rt, uri: &str) -> Result<Option<(Arc<Loaded>, usize)>> {
    if !rt.host().psbfile.registered {
        return Ok(None);
    }
    let (_, rest) = uri
        .split_once("://")
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("psb"))
        .ok_or_else(|| err("not a psb:// name"))?;
    let (file, key) = rest
        .split_once('/')
        .ok_or_else(|| err(format!("{uri}: missing resource key")))?;
    if file.is_empty() || matches!(file, "." | "..") || file.contains(['\\', ':', '>']) {
        return Err(err(format!("{uri}: invalid PSB storage name")));
    }
    let domain = file.to_lowercase();
    let loaded = match rt.host().psbfile.files.get(&domain) {
        Some(loaded) => Arc::clone(loaded),
        None => {
            if rt.host().storage()?.resolve(file).is_none() {
                return Ok(None);
            }
            let bytes = rt.read_storage_binary(file, "")?;
            let loaded = Loaded::parse(file, bytes, mounted_path(rt, file))?;
            remember(&mut rt.host_mut().psbfile, domain, Arc::clone(&loaded));
            loaded
        }
    };
    let Some(PsbValue::Resource(index)) = loaded.psb.root.field(key) else { return Ok(None) };
    let index = *index as usize;
    if loaded.psb.resource_bytes(&loaded.plain, index).is_none() {
        return Err(err(format!("{uri}: resource {index} out of range")));
    }
    Ok(Some((loaded, index)))
}

pub(crate) fn is_uri(name: &str) -> bool {
    name.split_once("://").is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("psb"))
}

pub(crate) fn exists(rt: &mut Rt, uri: &str) -> Result<bool> {
    if rt.host().storage()?.launcher_override(uri).is_some() {
        return Ok(true);
    }
    Ok(find_resource(rt, uri)?.is_some())
}

/// Layer.loadImages on a `psb://` name: suspend until the engine decodes the
/// resource, then attach it as the layer's image.
pub fn load_image(rt: &mut Rt, layer: ObjectHandle, uri: &str) -> Result<Variant> {
    if let Some(path) = rt.host().storage()?.launcher_override(uri) {
        let bytes = rt.host().storage()?.read_limited(&path, 64 * 1024 * 1024)
            .map_err(|e| err(format!("KiriKiri launcher override `{path}`: {e}")))?;
        let offset = png_offset(&bytes);
        queue_encoded_image(rt, layer, &path, bytes, None)?;
        return Ok(offset_metadata(rt, offset));
    }
    let (loaded, index) = find_resource(rt, uri)?
        .ok_or_else(|| err(format!("{uri}: resource not found")))?;
    let bytes = loaded.psb.resource_bytes(&loaded.plain, index)
        .ok_or_else(|| err(format!("{uri}: invalid resource range")))?;
    let offset = png_offset(bytes);
    let can_suspend = rt.host().can_suspend;
    let cached = rt.host_mut().psbfile.cached_member(&loaded, index, can_suspend);
    if let Some((Some(path), image)) = cached {
        assign_member(rt, layer, image, &path)?;
        return Ok(offset_metadata(rt, offset));
    }
    let resident = if can_suspend {
        cached.map(|(_, image)| image).or_else(|| {
            loaded.decoded.lock().unwrap_or_else(|e| e.into_inner())
                .get(&index).and_then(Weak::upgrade)
        })
    } else {
        None
    };
    if let Some(image) = resident {
        let state = &mut rt.host_mut().psbfile;
        if state.pending.is_some() || state.decoding.is_some() || state.ready.is_some() {
            return Err(err("image load already pending"));
        }
        state.ready = Some((layer, image));
        rt.request_suspend();
    } else {
        queue_encoded_image(rt, layer, uri, bytes.to_vec(), Some((loaded, index)))?;
    }
    Ok(offset_metadata(rt, offset))
}

fn offset_metadata(rt: &mut Rt, offset: Option<(i32, i32, &'static str)>) -> Variant {
    let Some((x, y, unit)) = offset else { return Variant::Void };
    let tags = rt.alloc_dictionary_object();
    // KiriKiri's PNG metadata callback publishes the offset values as strings.
    rt.set_object_member(tags, "offs_x", Variant::String(x.to_string()));
    rt.set_object_member(tags, "offs_y", Variant::String(y.to_string()));
    rt.set_object_member(tags, "offs_unit", Variant::String(unit.into()));
    Variant::Object(tags)
}

fn assign_member(rt: &mut Rt, layer: ObjectHandle, image: Arc<dyn ImageResource>, path: &str) -> Result<()> {
    let [w, h] = image.size().map(i64::from);
    let host = rt.host_mut();
    host.reset_surface(layer, w, h, [255, 255, 255, 0])?;
    let slot = host.layer(layer)?.slot;
    host.canvas_op(json!({"op":"set","id":slot,"layer":{"id":0,"rect":[0,0,w,h],"paint":{"kind":"image","asset":path}}}));
    let target = host.layer_mut(layer)?;
    (target.width, target.height) = (w, h);
    target.full_alpha = image.opaque().then_some(255);
    target.image_source = Some(crysirror_core::image_processing::ImageInput::Resource(image));
    target.blank = false;
    target.image_modified = true;
    host.api_calls += 1;
    Ok(())
}

/// KiriKiri LoadPNG reads oFFs from the information preceding IDAT.
fn png_offset(bytes: &[u8]) -> Option<(i32, i32, &'static str)> {
    let mut chunks = bytes.strip_prefix(PNG_SIGNATURE)?;
    while chunks.len() >= 12 {
        let length = u32::from_be_bytes(chunks[..4].try_into().ok()?) as usize;
        let chunk = chunks.get(..length.checked_add(12)?)?;
        match &chunk[4..8] {
            b"oFFs" if length == 9 => {
                let x = i32::from_be_bytes(chunk[8..12].try_into().ok()?);
                let y = i32::from_be_bytes(chunk[12..16].try_into().ok()?);
                let unit = match chunk[16] { 0 => "pixel", 1 => "micrometer", _ => "unknown" };
                return Some((x, y, unit));
            }
            b"IDAT" | b"IEND" => return None,
            _ => chunks = &chunks[chunk.len()..],
        }
    }
    None
}

/// Native encoded-resource/wait policy; actual decode/upload is engine owned.
pub(crate) fn load_encoded_image(
    rt: &mut Rt,
    layer: ObjectHandle,
    uri: &str,
    bytes: Vec<u8>,
) -> Result<Variant> {
    queue_encoded_image(rt, layer, uri, bytes, None)
}

fn queue_encoded_image(
    rt: &mut Rt, layer: ObjectHandle, uri: &str, bytes: Vec<u8>, member: Option<(Arc<Loaded>, usize)>,
) -> Result<Variant> {
    if !rt.host().can_suspend {
        return load_encoded_deferred(rt, layer, uri, bytes);
    }
    let state = &rt.host().psbfile;
    if state.pending.is_some() || state.decoding.is_some() || state.ready.is_some() {
        return Err(err("image load already pending"));
    }
    // Launch the worker before the caller yields and its Canvas batches are
    // flushed. A small button can then be ready at the first poll, without
    // forcing a complete extra presentation tick for each distinct image.
    if state.deferred.is_empty() && rt.host().image_job.is_none() {
        if let Some(size) = crysirror_image::encoded_dimensions_hint(&bytes) {
            let input = crysirror_core::image_processing::ImageInput::Encoded {
                name: uri.rsplit('/').next().unwrap_or(uri).to_owned(), bytes: bytes.into(), size,
            };
            let job = rt.host().images.as_ref().ok_or_else(|| err("image service absent"))?
                .begin_process(input, Arc::from(&b"[]"[..])).map_err(err)?;
            rt.host_mut().psbfile.decoding = Some((layer, job, member));
            rt.request_suspend();
            return Ok(Variant::Void);
        }
    }
    rt.host_mut().psbfile.pending = Some(PendingImage { layer, name: uri.to_owned(), bytes, member });
    rt.request_suspend();
    Ok(Variant::Void)
}

/// A native input callback cannot suspend, yet its script reads the new size at
/// once. Decode for the size and CPU source now; the shared image job publishes
/// the pixels at the next boundary, before the VM continues.
fn load_encoded_deferred(rt: &mut Rt, layer: ObjectHandle, uri: &str, bytes: Vec<u8>) -> Result<Variant> {
    let (size, opaque) = match crysirror_image::tlg_size(&bytes) {
        Some(size) => (size, false),
        None => {
            let surface = crysirror_image::decode_standard(&bytes)
                .map_err(|e| err(format!("decode `{uri}` in a native callback: {e}")))?;
            ([surface.width, surface.height], surface.rgba.chunks_exact(4).all(|p| p[3] == 255))
        }
    };
    let (w, h) = (i64::from(size[0]), i64::from(size[1]));
    let name = uri.rsplit('/').next().unwrap_or(uri).to_owned();
    let input = crysirror_core::image_processing::ImageInput::Encoded {
        name,
        bytes: bytes.into(),
        size,
    };
    let host = rt.host_mut();
    host.reset_surface(layer, w, h, [255, 255, 255, 0])?;
    let l = host.layer_mut(layer)?;
    (l.width, l.height) = (w, h);
    l.blank = false;
    l.image_source = Some(input.clone());
    l.full_alpha = opaque.then_some(255);
    host.psbfile.deferred.push((layer, input));
    Ok(Variant::Void)
}

/// Give `layer` an RGBA image of `size` now (its image surface is resized and
/// marked drawn) and publish the pixels through the shared image job at the
/// next boundary, like a deferred encoded load.
pub(crate) fn defer_rgba(rt: &mut Rt, layer: ObjectHandle, size: [u32; 2], rgba: Vec<u8>) -> Result<()> {
    let client = crysirror_core::state::ClientId(rt.host().client);
    let image = crysirror_core::image_processing::RgbaImage::new(client, size, rgba.into()).map_err(err)?;
    let opaque = crysirror_core::image_processing::ImageResource::opaque(&image);
    let input = crysirror_core::image_processing::ImageInput::Resource(std::sync::Arc::new(image));
    let host = rt.host_mut();
    host.reset_surface(layer, i64::from(size[0]), i64::from(size[1]), [255, 255, 255, 0])?;
    let l = host.layer_mut(layer)?;
    l.blank = false;
    l.image_source = Some(input.clone());
    l.full_alpha = opaque.then_some(255);
    host.psbfile.deferred.push((layer, input));
    Ok(())
}

pub fn poll(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<bool> {
    use crysirror_core::image_processing::{ImageInput, ImageProcessState};
    if let Some((layer, mut job, member)) = rt.host_mut().psbfile.decoding.take() {
        match job.poll()? {
            ImageProcessState::Pending => {
                rt.host_mut().psbfile.decoding = Some((layer, job, member));
                return Ok(false);
            }
            ImageProcessState::Failed(error) => anyhow::bail!("native encoded image: {error}"),
            ImageProcessState::Ready(image) => {
                if let Some((file, index)) = member {
                    let mut cache = file.decoded.lock().unwrap_or_else(|e| e.into_inner());
                    if cache.len() >= 1024 { cache.retain(|_, image| image.strong_count() > 0); }
                    if cache.len() < 1024 { cache.insert(index, Arc::downgrade(&image)); }
                    drop(cache);
                    let [w, h] = image.size();
                    let bytes = u64::from(w) * u64::from(h) * 4;
                    if bytes <= UI_MEMBER_BYTES {
                        let state = rt.host_mut();
                        state.psbfile.make_member_room(bytes, &mut state.retired_images);
                        let slot = state.layer(layer).map_err(err)?.slot;
                        let receipt = match host.publish_image_resource(client, slot, image.clone()) {
                            Ok(receipt) => receipt,
                            Err(crysirror_core::Error::Limit(_)) if !state.psbfile.members.is_empty() || !state.retired_images.is_empty() => {
                                state.psbfile.clear_members(&mut state.retired_images);
                                state.psbfile.ready = Some((layer, image));
                                return Ok(false);
                            }
                            Err(error) => return Err(error.into()),
                        };
                        let path = receipt.path.relative().to_owned();
                        state.psbfile.retain_member(CachedMember {
                            file: Arc::downgrade(&file), index, path: Some(path.clone()), image: image.clone(), bytes,
                        });
                        assign_member(rt, layer, image, &path)?;
                        return Ok(true);
                    }
                    if bytes <= MEMBER_CACHE_BYTES && file.psb.resource_bytes(&file.plain, index)
                        .is_some_and(|encoded| encoded.starts_with(PNG_SIGNATURE))
                    {
                        let state = rt.host_mut();
                        state.psbfile.retain_png(CachedMember {
                            file: Arc::downgrade(&file), index, path: None, image: image.clone(), bytes,
                        }, &mut state.retired_images);
                    }
                }
                rt.host_mut().psbfile.ready = Some((layer, image));
            }
        }
    }
    if let Some((layer, image)) = rt.host_mut().psbfile.ready.take() {
        let [w, h] = image.size().map(i64::from);
        rt.host_mut().reset_surface(layer, w, h, [255, 255, 255, 0]).map_err(err)?;
        let target = rt.host_mut().layer_mut(layer).map_err(err)?;
        (target.width, target.height) = (w, h);
        crate::image_api::commit_result(rt, host, client, layer.0, image, [false; 2], true)?;
        return Ok(true);
    }
    if !rt.host().psbfile.deferred.is_empty() {
        let (layer, input) = rt.host_mut().psbfile.deferred.remove(0);
        let job = rt.host().images.as_ref().ok_or_else(|| anyhow::anyhow!("image service absent"))?
            .begin_process(input, std::sync::Arc::from(&b"[]"[..]))?;
        rt.host_mut().image_job = Some((layer.0, job, [false; 2], true));
        return Ok(false);
    }
    let Some(p) = rt.host_mut().psbfile.pending.take() else {
        return Ok(true);
    };
    let name = p.name.rsplit('/').next().unwrap_or(&p.name).to_owned();
    if let Some(size) = crysirror_image::encoded_dimensions_hint(&p.bytes) {
        let input = ImageInput::Encoded { name, bytes: p.bytes.into(), size };
        let job = rt.host().images.as_ref().ok_or_else(|| anyhow::anyhow!("image service absent"))?
            .begin_process(input, Arc::from(&b"[]"[..]))?;
        rt.host_mut().psbfile.decoding = Some((p.layer, job, p.member));
        return Ok(false);
    }
    // Preserve decoder-registry support for formats without a header size hint.
    let state = &mut rt.host_mut().psbfile;
    let slot = 0x6000_0000u32
        .checked_add(state.next_slot)
        .ok_or_else(|| anyhow::anyhow!("psb image slots exhausted"))?;
    state.next_slot += 1;
    let image = host.upload_encoded_image(client, slot, &name, &p.bytes)?;
    rt.host_mut().retired_images.push(image.path.relative().to_owned());
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
    let layer = host_state.layer_mut(p.layer).map_err(e)?;
    layer.full_alpha = image.opaque.then_some(255);
    layer.image_source = Some(crysirror_core::image_processing::ImageInput::Encoded {
        name,
        bytes: p.bytes.into(),
        size: image.size,
    });
    host_state.api_calls += 1;
    Ok(true)
}

impl State {
    fn package_bytes(&self) -> usize {
        self.files.values().map(|file| file.plain.len()).sum()
    }

    fn contains_package(&self, file: &Weak<Loaded>) -> bool {
        self.files.values().any(|loaded| std::ptr::eq(Arc::as_ptr(loaded), file.as_ptr()))
    }

    fn cached_member(&mut self, file: &Arc<Loaded>, index: usize, can_suspend: bool)
        -> Option<(Option<String>, Arc<dyn ImageResource>)>
    {
        let key = Arc::downgrade(file);
        if let Some(member) = self.recent_large_png.as_ref().filter(|member| {
            can_suspend && member.index == index && member.file.ptr_eq(&key)
        }) {
            return Some((None, member.image.clone()));
        }
        let i = self.members.iter().position(|member| {
            member.index == index && member.file.ptr_eq(&key) && (member.path.is_some() || can_suspend)
        })?;
        let member = self.members.remove(i).expect("member index found above");
        let hit = (member.path.clone(), member.image.clone());
        self.members.push_back(member);
        Some(hit)
    }

    fn make_member_room(&mut self, bytes: u64, retired_images: &mut Vec<String>) {
        debug_assert!(bytes <= MEMBER_CACHE_BYTES);
        while self.member_bytes + bytes > MEMBER_CACHE_BYTES || self.members.len() >= MEMBER_CACHE_COUNT {
            let member = self.members.pop_front().expect("nonempty member cache at limit");
            self.member_bytes -= member.bytes;
            retired_images.extend(member.path);
        }
    }

    fn retain_member(&mut self, member: CachedMember) {
        self.member_bytes += member.bytes;
        self.members.push_back(member);
    }

    fn retain_png(&mut self, member: CachedMember, retired_images: &mut Vec<String>) {
        if member.bytes >= LARGE_PNG_BYTES {
            if self.package_bytes() as u64 + member.bytes <= RETAINED_BYTES as u64
                && self.contains_package(&member.file)
            {
                self.recent_large_png = Some(member);
            }
            return;
        }
        self.make_member_room(member.bytes, retired_images);
        self.retain_member(member);
    }

    fn clear_members(&mut self, retired_images: &mut Vec<String>) {
        retired_images.extend(self.members.drain(..).filter_map(|member| member.path));
        self.member_bytes = 0;
        self.recent_large_png = None;
    }

    pub(crate) fn invalidate(&mut self, object: usize) {
        self.owners.remove(&object);
    }

    pub(crate) fn gc_roots(&self, out: &mut Vec<ObjectHandle>) {
        // PSBFile instances are owned by scripts; a collected one drops its
        // entry through `invalidate`.
        if let Some(p) = &self.pending {
            out.push(p.layer);
        }
        if let Some((layer, _, _)) = &self.decoding {
            out.push(*layer);
        }
        if let Some((layer, _)) = &self.ready { out.push(*layer); }
        out.extend(self.deferred.iter().map(|(layer, _)| *layer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::image_processing::RgbaImage;
    use eluna::psb::PsbHeader;

    fn loaded_file() -> Arc<Loaded> {
        let mut header = [0; 40];
        header[..5].copy_from_slice(b"PSB\0\x02");
        Arc::new(Loaded {
            mounted_path: None,
            plain: Arc::from([]),
            psb: Arc::new(PsbFile {
                header: PsbHeader::read(&header).unwrap(),
                version: 2,
                encrypted: false,
                checksum: None,
                names: Vec::new(),
                strings: Vec::new(),
                resources: Vec::new(),
                extra_resources: Vec::new(),
                root: PsbValue::Null,
            }),
            decoded: Mutex::default(),
        })
    }

    fn member(file: &Arc<Loaded>, size: [u32; 2], path: Option<String>) -> CachedMember {
        let bytes = u64::from(size[0]) * u64::from(size[1]) * 4;
        CachedMember {
            file: Arc::downgrade(file),
            index: 0,
            path,
            image: Arc::new(RgbaImage::new(ClientId(1), size, vec![0; bytes as usize].into()).unwrap()),
            bytes,
        }
    }

    #[test]
    fn full_size_png_and_overlay_survive_under_existing_budgets() {
        let mut state = State::default();
        let mut retired = Vec::new();
        let first = loaded_file();
        let second = loaded_file();
        remember(&mut state, "first".into(), first.clone());
        remember(&mut state, "second".into(), second.clone());
        let ui = member(&second, [160, 160], Some("button".into()));
        state.retain_member(ui);

        let cg = member(&first, [1920, 1080], None);
        let old = Arc::downgrade(&cg.image);
        state.retain_png(cg, &mut retired);
        let mut overlay = member(&first, [380, 220], None);
        overlay.index = 1;
        state.retain_png(overlay, &mut retired);
        assert_eq!(state.member_bytes, (160 * 160 + 380 * 220) * 4);
        assert!(retired.is_empty());
        assert_eq!(state.cached_member(&second, 0, true).unwrap().0.as_deref(), Some("button"));
        let (path, image) = state.cached_member(&first, 0, true).unwrap();
        assert!(path.is_none());
        assert!(Weak::ptr_eq(&old, &Arc::downgrade(&image)));
        assert!(state.cached_member(&first, 1, true).is_some());
        assert!(state.cached_member(&second, 1, true).is_none());

        let replacement = member(&second, [1920, 1080], None);
        state.retain_png(replacement, &mut retired);
        assert!(state.cached_member(&first, 0, true).is_none());
        assert!(retired.is_empty());
        drop(image);
        assert!(old.upgrade().is_none());

        remember(&mut state, "second".into(), loaded_file());
        assert!(state.recent_large_png.is_none());
        state.clear_members(&mut retired);
        assert_eq!(retired, ["button"]);
        assert_eq!(state.member_bytes, 0);
    }

    #[test]
    fn png_offsets_are_signed_and_require_a_complete_chunk() {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x09oFFs".to_vec();
        bytes.extend((-17i32).to_be_bytes());
        bytes.extend(162i32.to_be_bytes());
        bytes.extend([0; 5]); // unit and CRC, checked by the image decoder
        assert_eq!(super::png_offset(&bytes), Some((-17, 162, "pixel")));
        assert_eq!(super::png_offset(&bytes[..bytes.len() - 1]), None);
    }
}
