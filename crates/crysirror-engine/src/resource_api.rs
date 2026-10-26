// SPDX-License-Identifier: MPL-2.0
//! Shared archive and image resources, decoded through client-scoped capabilities.
use crate::{
    object::{Class, ClassInfo, MethodInfo, ObjectId, TypeHint as T},
    EngineContext, EngineState, Value,
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::AssetReader,
    state::ClientId,
};
use crysirror_resource::{
    Archive, ArchiveEntry, ArchiveMount, DecodeContext, DecodeLimits, FileKind, FileRegistry,
    FormatRegistry, KeyProvider, MemoryKeyProvider, PipelineRules, TransformRegistry,
    TransformSpec,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

pub struct ResourceRuntime {
    pub formats: FormatRegistry,
    pub files: FileRegistry,
    pub jobs: crysirror_resource::ResourceTasks,
    pub transforms: Arc<TransformRegistry>,
    /// Hosts supply key providers in Rust; keys never enter project/save JSON.
    pub keys: Arc<dyn KeyProvider>,
    image_formats: Vec<&'static str>,
    contexts: BTreeMap<(ClientId, String), DecodeContext>,
    // Stable owner for weak companion resolvers; mounted providers never own this map.
    storage_authorities: BTreeMap<ClientId, Arc<dyn crysirror_core::service::ClientStorage>>,
}
impl Default for ResourceRuntime {
    fn default() -> Self {
        #[allow(unused_mut)]
        let mut formats = FormatRegistry::default();
        #[cfg(feature = "arc-formats")]
        crysirror_formats_arc::register(&mut formats)
            .expect("built-in archive identifiers are unique");
        #[allow(unused_mut)]
        let mut files = FileRegistry::default();
        #[cfg(feature = "arc-formats")]
        {
            crysirror_formats_arc::register_files(&mut files).expect("file decoder IDs are unique");
            crysirror_formats_arc::register_audio(&mut files)
                .expect("audio decoder IDs are unique");
        }
        Self {
            formats,
            files,
            jobs: Default::default(),
            transforms: Arc::new(TransformRegistry::default()),
            keys: Arc::new(MemoryKeyProvider::default()),
            image_formats: Vec::new(),
            contexts: BTreeMap::new(),
            storage_authorities: BTreeMap::new(),
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Options {
    format: Option<String>,
    profile: Option<String>,
    offset: usize,
    limit: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    format: String,
    #[serde(default)]
    path: Option<String>,
    transforms: Vec<TransformSpec>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Profile {
    limits: DecodeLimits,
    rules: Vec<Rule>,
    parameters: BTreeMap<String, serde_json::Value>,
}
impl ResourceRuntime {
    fn context(&mut self, client: ClientId, name: Option<&str>) -> Result<DecodeContext> {
        let name = name.unwrap_or("default");
        if !self.contexts.contains_key(&(client, name.into())) {
            ensure!(name == "default", "unknown resource profile");
            self.define(client, name, Profile::default())?;
        }
        Ok(self.contexts[&(client, name.into())].clone())
    }
    fn define(&mut self, client: ClientId, name: &str, profile: Profile) -> Result<()> {
        ensure!(
            !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control),
            "invalid resource profile name"
        );
        ensure!(
            self.contexts.contains_key(&(client, name.into()))
                || self.contexts.keys().filter(|(c, _)| *c == client).count() < 32,
            "resource profile limit"
        );
        let l = &profile.limits;
        ensure!(
            (1..=1_000_000).contains(&l.max_entries)
                && (1..=1024 * 1024 * 1024).contains(&l.max_entry_size)
                && (1..=64 * 1024 * 1024).contains(&l.max_index_size)
                && (1..=2 * 1024 * 1024 * 1024).contains(&l.max_resident_bytes)
                && (1..=16 * 1024 * 1024).contains(&l.max_probe_bytes),
            "resource profile limits out of range"
        );
        ensure!(profile.rules.len() <= 1024, "resource profile rule limit");
        let mut rules = PipelineRules::default();
        for rule in profile.rules {
            ensure!(
                self.formats.names().contains(&rule.format.as_str())
                    || self.image_formats.contains(&rule.format.as_str())
                    || self.files.names().contains(&rule.format.as_str()),
                "unregistered resource format in profile"
            );
            ensure!(
                rule.transforms.len() <= 16,
                "resource transform chain limit"
            );
            for spec in &rule.transforms {
                ensure!(
                    self.transforms.names().contains(&spec.algorithm.as_str()),
                    "unsupported transform in profile"
                );
            }
            ensure!(
                rules
                    .rules
                    .insert((rule.format, rule.path), rule.transforms)
                    .is_none(),
                "duplicate resource profile rule"
            );
        }
        // Replacing an installed profile would otherwise reset the resident
        // budget while old mounted archives retain its decoded allocations.
        ensure!(
            !self.contexts.contains_key(&(client, name.into())),
            "resource profile already defined; choose another name"
        );
        // All profiles belonging to a client share retained decoded-byte
        // accounting, so creating another profile cannot reset that budget.
        let mut context = self
            .contexts
            .iter()
            .find(|((owner, _), _)| *owner == client)
            .map(|(_, context)| context.clone())
            .unwrap_or_else(|| DecodeContext::new(client));
        context.limits = profile.limits;
        context.transforms = self.transforms.clone();
        context.keys = self.keys.clone();
        context.pipelines = Arc::new(rules);
        ensure!(
            profile.parameters.len() <= 256,
            "resource format-parameter count limit"
        );
        for id in profile.parameters.keys() {
            ensure!(
                self.formats.names().contains(&id.as_str())
                    || self.files.names().contains(&id.as_str())
                    || self.image_formats.contains(&id.as_str()),
                "unregistered format parameters: {id}"
            );
        }
        context.parameters = profile.parameters;
        self.contexts.insert((client, name.into()), context);
        Ok(())
    }
}
fn source(state: &EngineState, path: &str) -> Result<Arc<dyn AssetReader>> {
    let path = VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
    state
        .runtime
        .host
        .fs
        .storage(state.client)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .open_asset(&path)
        .map_err(|e| anyhow::anyhow!("{e}"))
}
fn open(
    state: &mut EngineState,
    path: &str,
    options: &Options,
) -> Result<(&'static str, Box<dyn Archive>, DecodeContext)> {
    let source = source(state, path)?;
    let context = scoped_context(state, path, options.profile.as_deref())?;
    let (format, archive) =
        state
            .resources
            .formats
            .open(source, options.format.as_deref(), &context)?;
    Ok((format, archive, context))
}
fn scoped_context(
    state: &mut EngineState,
    path: &str,
    profile: Option<&str>,
) -> Result<DecodeContext> {
    let mut context = state.resources.context(state.client, profile)?;
    if !state
        .resources
        .storage_authorities
        .contains_key(&state.client)
    {
        let authority = state
            .runtime
            .host
            .fs
            .storage(state.client)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        state
            .resources
            .storage_authorities
            .insert(state.client, authority);
    }
    let storage = &state.resources.storage_authorities[&state.client];
    // A mounted provider retains its DecodeContext. Keep a weak dependency
    // authority to avoid provider -> resolver -> VFS provider-table cycles.
    let storage = Arc::downgrade(storage);
    context.source_name = Some(path.into());
    context.resolver = Some(Arc::new(
        move |name: &str| -> Result<Arc<dyn AssetReader>> {
            let path = VfsPath::new(Mount::Asset, name).map_err(|e| anyhow::anyhow!("{e}"))?;
            storage
                .upgrade()
                .context("resource companion authority was released")?
                .open_asset(&path)
                .map_err(|e| anyhow::anyhow!("{e}"))
        },
    ));
    Ok(context)
}
struct DecodedArchive {
    entries: Vec<ArchiveEntry>,
    reader: Arc<dyn AssetReader>,
}
impl Archive for DecodedArchive {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.entries
    }
    fn open(&self, id: u32, _: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        ensure!(id == 0, "decoded file entry id");
        Ok(self.reader.clone())
    }
}
struct SharedArchive(Arc<dyn Archive>);
impl Archive for SharedArchive {
    fn entries(&self) -> &[ArchiveEntry] {
        self.0.entries()
    }
    fn open(&self, id: u32, cx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        self.0.open(id, cx)
    }
}
fn mount_decoded(
    state: &mut EngineState,
    format: &str,
    file: crysirror_resource::DecodedFile,
    context: DecodeContext,
    target: &str,
) -> Result<serde_json::Value> {
    let canonical = VfsPath::new(Mount::Asset, target).map_err(|e| anyhow::anyhow!("{e}"))?;
    ensure!(
        canonical.relative() == target,
        "decoded mount path must be canonical"
    );
    let (parent, filename) = target.rsplit_once('/').unwrap_or(("", target));
    let size = file.reader.len().map_err(|e| anyhow::anyhow!("{e}"))?;
    let archive = DecodedArchive {
        entries: vec![ArchiveEntry {
            id: 0,
            path: filename.into(),
            size,
            stored_size: size,
            metadata: file.metadata.clone(),
        }],
        reader: file.reader,
    };
    let mount = ArchiveMount::new(Box::new(archive), context)?;
    state
        .runtime
        .mount_provider(state.client, parent, Arc::new(mount))?;
    Ok(
        json!({"format":format,"extension":file.extension,"size":size,"path":target,"metadata":file.metadata}),
    )
}
fn arg<'a>(args: &'a [Value], i: usize) -> Result<&'a Value> {
    args.get(i).context("missing resource argument")
}
fn options(args: &[Value], i: usize) -> Result<Options> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(Options::default()),
        Some(v) => Ok(serde_json::from_value(v.to_json())?),
    }
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
pub struct ResourcesClass;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_share_client_budget_without_sharing_another_clients_authority() {
        let mut resources = ResourceRuntime::default();
        let profile = || Profile {
            limits: DecodeLimits {
                max_resident_bytes: 10,
                ..Default::default()
            },
            rules: vec![],
            parameters: Default::default(),
        };
        resources.define(ClientId(1), "a", profile()).unwrap();
        resources.define(ClientId(1), "b", profile()).unwrap();
        resources.define(ClientId(2), "a", profile()).unwrap();
        let first = resources.context(ClientId(1), Some("a")).unwrap();
        let held = first.decode_with(8, || Ok(vec![0; 8])).unwrap();
        let second = resources.context(ClientId(1), Some("b")).unwrap();
        assert!(second.decode_with(4, || Ok(vec![0; 4])).is_err());
        let other = resources.context(ClientId(2), Some("a")).unwrap();
        assert!(other.decode_with(4, || Ok(vec![0; 4])).is_ok());
        drop(held);
        assert!(second.decode_with(4, || Ok(vec![0; 4])).is_ok());
    }
}
impl Class for ResourcesClass {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(||ClassInfo{
            name:"Resources",doc:"Client-scoped archive and native image formats, transform/key profiles, dependency reads and decoded image surfaces. Enable the host's arc-formats feature for migrated plugins.",properties:vec![],
            methods:vec![
                method("formats",vec![],T::List(Box::new(T::String)),"List installed archive decoder IDs; unavailable formats are never reported as supported."),
                method("fileFormats",vec![],T::List(Box::new(T::String)),"List installed encoded-file converters. They return shared resource bytes, not a private extractor process."),
                method("audioFormats",vec![],T::List(Box::new(T::String)),"List installed native audio converters producing standard WAVE resources for the engine decoder/mixer."),
                method("detectFile",vec![("asset",T::String),("options",T::Optional(Box::new(T::Map)))],T::List(Box::new(T::Map)),"Offline probe of native file/audio decoders with per-plugin read budgets. Options {profile?}; live hosts use beginDecode with automatic detection, and ambiguous formats require an explicit decoder."),
                method("readDecoded",vec![("asset",T::String),("options",T::Optional(Box::new(T::Map)))],T::Map,"Decode a file/audio resource and return metadata plus a bounded byte page. Options {format?,profile?,offset?,limit?}, default 4096 and maximum 65536 bytes. Decoding is CPU work; live callers should schedule it as an asset task."),
                method("mountDecoded",vec![("asset",T::String),("target",T::String),("options",T::Optional(Box::new(T::Map)))],T::Map,"Decode once and publish a client-scoped VFS resource at target. Normal Bitmap/Assets/Audio calls can consume it. Options {format?,profile?}; retained output shares the client's decode budget and no extracted file is written."),
                method("beginDecode",vec![("asset",T::String),("kind",T::String),("options",T::Optional(Box::new(T::Map)))],T::Int,"Schedule file/audio conversion (kind=file) or archive indexing (kind=archive) on the bounded CPU pool. Options {format?,profile?}. Returns a scoped ticket; owner calls never join. At most 64 retained tickets per client and 256 per host."),
                method("decodeStatus",vec![("ticket",T::Int)],T::Map,"Poll pending/ready/failed resource work without blocking. A completed decoder failure remains visible until cancelled."),
                method("readDecode",vec![("ticket",T::Int),("offset",T::Optional(Box::new(T::Int))),("limit",T::Optional(Box::new(T::Int)))],T::Map,"Read a ready file's bounded byte page (maximum 65536) or archive metadata page (maximum 1024 entries). Pending work fails visibly rather than waiting or returning fabricated data."),
                method("mountDecode",vec![("ticket",T::Int),("target",T::String)],T::Map,"Publish a completed worker result into this client's VFS. For files target is a full logical filename; for archives it is a mount directory. No CPU decoder runs during publication."),
                method("cancelDecode",vec![("ticket",T::Int)],T::Void,"Cancel pending work or release a retained result. Late worker output is discarded. Already mounted readers retain their own decoded-resource reservation."),
                method("transforms",vec![],T::List(Box::new(T::String)),"List installed byte transform IDs; profiles refer to keys supplied by the Rust host."),
                method("imageFormats",vec![],T::Map,"List installed native image format IDs and standard image extensions; the runtime, Bitmap, text icons and renderer use this same registry."),
                method("probeImage",vec![("asset",T::String)],T::List(Box::new(T::String)),"Return recognized native image IDs from bounded client asset bytes. Specific wrappers outrank their generic embedded codec during automatic decode."),
                method("decodeImage",vec![("slot",T::Int),("asset",T::String),("options",T::Optional(Box::new(T::Map)))],T::String,"Decode a client image into __surface/<slot> for Bitmap/Layer/UI/3D use. Options {format?,profile?}; external bases/masks use the same scoped VFS and keys remain in the host provider."),
                method("defineProfile",vec![("name",T::String),("profile",T::Map)],T::Void,"Define an immutable client profile {limits?,parameters?:{format:options},rules:[{format,path?,transforms:[{algorithm,key_id?,parameters?,expected_size?}]}]}. Native options name client key aliases/companions; secret bytes remain in the host KeyProvider. Up to 32 profiles; default is created lazily."),
                method("probe",vec![("asset",T::String)],T::List(Box::new(T::Map)),"Probe an asset using installed formats with bounded reads. Ambiguous signatures require an explicit format when opening."),
                method("list",vec![("asset",T::String),("options",T::Optional(Box::new(T::Map)))],T::Map,"Inspect archive metadata. Options {format?,profile?,offset?,limit?}; default 128 and at most 1024 entries per page."),
                method("mount",vec![("asset",T::String),("mountPoint",T::String),("options",T::Optional(Box::new(T::Map)))],T::Map,"Mount archive entries in this client's asset VFS, then use normal bitmap, audio and model APIs. Options {format?,profile?}. Native paths are validated; keys and decoded bytes are not saved."),
            ]})
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, p: &str) -> Result<Value> {
        bail!("unknown Resources property {p}")
    }
    fn set(&self, _: &mut EngineContext, _: ObjectId, p: &str, _: Value) -> Result<()> {
        bail!("unknown Resources property {p}")
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        name: &str,
        args: &[Value],
    ) -> Result<Value> {
        let state = cx.state()?;
        let out = match name {
            "beginDecode" => {
                let path = arg(args, 0)?.as_str()?;
                let kind = arg(args, 1)?.as_str()?;
                ensure!(
                    ["file", "archive"].contains(&kind),
                    "resource task kind must be file or archive"
                );
                let opts = options(args, 2)?;
                let canonical =
                    VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
                let context = scoped_context(state, canonical.relative(), opts.profile.as_deref())?;
                let pool = state.runtime.assets.cpu_tasks();
                let storage = state.resources.storage_authorities[&state.client].clone();
                let source_name = canonical.relative().to_owned();
                // Opening a provider-backed resource may decompress its entry;
                // retain the authority here and perform that work on the worker.
                let input = move || {
                    storage
                        .open_asset(&canonical)
                        .map_err(|e| anyhow::anyhow!("{e}"))
                };
                let ticket = if kind == "file" {
                    state.resources.jobs.begin_file_with_source(
                        state.client,
                        state.resources.files.clone(),
                        source_name,
                        input,
                        opts.format,
                        context,
                        &pool,
                    )?
                } else {
                    state.resources.jobs.begin_archive_with_source(
                        state.client,
                        state.resources.formats.clone(),
                        input,
                        opts.format,
                        context,
                        &pool,
                    )?
                };
                json!(ticket)
            }
            "decodeStatus" | "cancelDecode" | "readDecode" | "mountDecode" => {
                let ticket =
                    u64::try_from(arg(args, 0)?.as_i64()?).context("resource task ticket")?;
                if name == "decodeStatus" {
                    state.resources.jobs.status(state.client, ticket)?
                } else if name == "cancelDecode" {
                    state.resources.jobs.cancel(state.client, ticket)?;
                    json!(null)
                } else {
                    let result = state
                        .resources
                        .jobs
                        .poll(state.client, ticket)?
                        .context("resource task is pending")?
                        .clone();
                    match result {
                        crysirror_resource::ResourceResult::File {
                            format,
                            decoded,
                            context,
                        } => {
                            if name == "mountDecode" {
                                mount_decoded(
                                    state,
                                    format,
                                    decoded,
                                    context,
                                    arg(args, 1)?.as_str()?,
                                )?
                            } else {
                                let offset = args
                                    .get(1)
                                    .filter(|v| !matches!(v, Value::Null))
                                    .map(Value::as_i64)
                                    .transpose()?
                                    .unwrap_or(0);
                                let limit = args
                                    .get(2)
                                    .filter(|v| !matches!(v, Value::Null))
                                    .map(Value::as_i64)
                                    .transpose()?
                                    .unwrap_or(4096);
                                ensure!(
                                    offset >= 0 && (1..=65536).contains(&limit),
                                    "decoded file page bounds"
                                );
                                let size =
                                    decoded.reader.len().map_err(|e| anyhow::anyhow!("{e}"))?;
                                ensure!(offset as u64 <= size, "decoded resource page offset");
                                let mut bytes =
                                    vec![0; (size - offset as u64).min(limit as u64) as usize];
                                crysirror_resource::read_exact_at(
                                    decoded.reader.as_ref(),
                                    offset as u64,
                                    &mut bytes,
                                )?;
                                json!({"kind":"file","format":format,"extension":decoded.extension,"size":size,"metadata":decoded.metadata,"offset":offset,"bytes":bytes})
                            }
                        }
                        crysirror_resource::ResourceResult::Archive {
                            format,
                            archive,
                            context,
                        } => {
                            if name == "mountDecode" {
                                let target = arg(args, 1)?.as_str()?;
                                if !target.is_empty() {
                                    VfsPath::new(Mount::Asset, target)
                                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                                }
                                let count = archive.entries().len();
                                let mount =
                                    ArchiveMount::new(Box::new(SharedArchive(archive)), context)?;
                                state.runtime.mount_provider(
                                    state.client,
                                    target,
                                    Arc::new(mount),
                                )?;
                                json!({"kind":"archive","format":format,"entries":count,"mount_point":target})
                            } else {
                                let offset = args
                                    .get(1)
                                    .filter(|v| !matches!(v, Value::Null))
                                    .map(Value::as_i64)
                                    .transpose()?
                                    .unwrap_or(0);
                                let limit = args
                                    .get(2)
                                    .filter(|v| !matches!(v, Value::Null))
                                    .map(Value::as_i64)
                                    .transpose()?
                                    .unwrap_or(128);
                                ensure!(
                                    offset >= 0 && (1..=1024).contains(&limit),
                                    "archive metadata page bounds"
                                );
                                let offset = usize::try_from(offset)?;
                                let end = offset
                                    .saturating_add(limit as usize)
                                    .min(archive.entries().len());
                                let entries = archive.entries().get(offset..end).unwrap_or(&[]);
                                json!({"kind":"archive","format":format,"total":archive.entries().len(),"offset":offset,"entries":entries})
                            }
                        }
                    }
                }
            }
            "formats" => json!(state.resources.formats.names()),
            "fileFormats" => json!(state.resources.files.formats(FileKind::File)),
            "audioFormats" => json!(state.resources.files.formats(FileKind::Audio)),
            "detectFile" => {
                ensure!(!state.runtime.assets.is_nonblocking(),"synchronous file probing is offline-only in a live host; use beginDecode with automatic detection");
                let path = arg(args, 0)?.as_str()?;
                let opts = options(args, 1)?;
                let input = source(state, path)?;
                let context = scoped_context(state, path, opts.profile.as_deref())?;
                json!(state
                    .resources
                    .files
                    .probe(path, input, &context)?
                    .into_iter()
                    .map(|(format, probe)| json!({"format":format,"confidence":probe.confidence}))
                    .collect::<Vec<_>>())
            }
            "readDecoded" | "mountDecoded" => {
                ensure!(!state.runtime.assets.is_nonblocking(),"synchronous resource conversion is offline-only in a live host; use beginDecode and poll completion");
                let path = arg(args, 0)?.as_str()?;
                let opts = options(args, if name == "mountDecoded" { 2 } else { 1 })?;
                let limit = opts.limit.unwrap_or(4096);
                ensure!((1..=65536).contains(&limit), "decoded resource page limit");
                let input = source(state, path)?;
                let context = scoped_context(state, path, opts.profile.as_deref())?;
                let (format, file) =
                    state
                        .resources
                        .files
                        .decode(path, input, opts.format.as_deref(), &context)?;
                let size = file.reader.len().map_err(|e| anyhow::anyhow!("{e}"))?;
                if name == "mountDecoded" {
                    mount_decoded(state, format, file, context, arg(args, 1)?.as_str()?)?
                } else {
                    ensure!(opts.offset as u64 <= size, "decoded resource page offset");
                    let n = usize::try_from((size - opts.offset as u64).min(limit as u64))?;
                    let mut bytes = vec![0; n];
                    crysirror_resource::read_exact_at(
                        file.reader.as_ref(),
                        opts.offset as u64,
                        &mut bytes,
                    )?;
                    json!({"format":format,"extension":file.extension,"size":size,"metadata":file.metadata,"offset":opts.offset,"bytes":bytes})
                }
            }
            "imageFormats" => {
                json!({"native":state.runtime.assets.decoders.format_ids(),"standard_extensions":state.runtime.assets.decoders.extensions()})
            }
            "probeImage" => {
                let path = arg(args, 0)?.as_str()?;
                let bytes = crysirror_resource::read_all(
                    source(state, path)?.as_ref(),
                    crysirror_image::MAX_DECODED_BYTES as u64,
                )?;
                json!(state.runtime.assets.decoders.probe(path, &bytes)?)
            }
            "decodeImage" => {
                let slot = u32::try_from(arg(args, 0)?.as_i64()?).context("image surface slot")?;
                let path = arg(args, 1)?.as_str()?;
                let opts = options(args, 2)?;
                let bytes = crysirror_resource::read_all(
                    source(state, path)?.as_ref(),
                    crysirror_image::MAX_DECODED_BYTES as u64,
                )?;
                let resources = state
                    .resources
                    .context(state.client, opts.profile.as_deref())?;
                let surface = state.runtime.assets.decode_image_with(
                    state.client,
                    path,
                    &bytes,
                    Some(&resources),
                    opts.format.as_deref(),
                )?;
                json!(state.runtime.assets.put_surface(
                    state.client,
                    slot,
                    surface.width,
                    surface.height,
                    surface.rgba
                )?)
            }
            "transforms" => json!(state.resources.transforms.names()),
            "defineProfile" => {
                state.resources.image_formats = state.runtime.assets.decoders.format_ids();
                let profile = arg(args, 1)?.to_json();
                ensure!(
                    serde_json::to_vec(&profile)?.len() <= 1024 * 1024,
                    "resource profile byte limit"
                );
                state.resources.define(
                    state.client,
                    arg(args, 0)?.as_str()?,
                    serde_json::from_value(profile)?,
                )?;
                json!(null)
            }
            "probe" => {
                let src = source(state, arg(args, 0)?.as_str()?)?;
                let context = state.resources.context(state.client, None)?;
                json!(state
                    .resources
                    .formats
                    .probe(src, &context)?
                    .into_iter()
                    .map(|(format, probe)| json!({"format":format,"confidence":probe.confidence}))
                    .collect::<Vec<_>>())
            }
            "list" => {
                let options = options(args, 1)?;
                let limit = options.limit.unwrap_or(128);
                ensure!((1..=1024).contains(&limit), "archive list page limit");
                let (format, archive, _) = open(state, arg(args, 0)?.as_str()?, &options)?;
                let end = options
                    .offset
                    .saturating_add(limit)
                    .min(archive.entries().len());
                let entries = archive.entries().get(options.offset..end).unwrap_or(&[]);
                json!({"format":format,"total":archive.entries().len(),"offset":options.offset,"entries":entries})
            }
            "mount" => {
                let point = arg(args, 1)?.as_str()?;
                if !point.is_empty() {
                    VfsPath::new(Mount::Asset, point).map_err(|e| anyhow::anyhow!("{e}"))?;
                }
                let (format, archive, context) =
                    open(state, arg(args, 0)?.as_str()?, &options(args, 2)?)?;
                let count = archive.entries().len();
                let mount = ArchiveMount::new(archive, context)?;
                state
                    .runtime
                    .mount_provider(state.client, point, Arc::new(mount))?;
                json!({"format":format,"entries":count,"mount_point":point})
            }
            _ => bail!("unknown Resources method {name}"),
        };
        Ok(Value::from_json(out))
    }
}
