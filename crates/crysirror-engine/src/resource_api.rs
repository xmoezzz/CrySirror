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
    Archive, ArchiveMount, DecodeContext, DecodeLimits, FormatRegistry, KeyProvider,
    MemoryKeyProvider, PipelineRules, TransformRegistry, TransformSpec,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

pub struct ResourceRuntime {
    pub formats: FormatRegistry,
    pub transforms: Arc<TransformRegistry>,
    /// Hosts supply key providers in Rust; keys never enter project/save JSON.
    pub keys: Arc<dyn KeyProvider>,
    image_formats: Vec<&'static str>,
    contexts: BTreeMap<(ClientId, String), DecodeContext>,
}
impl Default for ResourceRuntime {
    fn default() -> Self {
        #[allow(unused_mut)]
        let mut formats = FormatRegistry::default();
        #[cfg(feature = "arc-formats")]
        crysirror_formats_arc::register(&mut formats)
            .expect("built-in archive identifiers are unique");
        Self {
            formats,
            transforms: Arc::new(TransformRegistry::default()),
            keys: Arc::new(MemoryKeyProvider::default()),
            image_formats: Vec::new(),
            contexts: BTreeMap::new(),
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
                    || self.image_formats.contains(&rule.format.as_str()),
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
    let context = state
        .resources
        .context(state.client, options.profile.as_deref())?;
    let (format, archive) =
        state
            .resources
            .formats
            .open(source, options.format.as_deref(), &context)?;
    Ok((format, archive, context))
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
                method("transforms",vec![],T::List(Box::new(T::String)),"List installed byte transform IDs; profiles refer to keys supplied by the Rust host."),
                method("imageFormats",vec![],T::Map,"List installed native image format IDs and standard image extensions; the runtime, Bitmap, text icons and renderer use this same registry."),
                method("probeImage",vec![("asset",T::String)],T::List(Box::new(T::String)),"Return recognized native image IDs from bounded client asset bytes. Specific wrappers outrank their generic embedded codec during automatic decode."),
                method("decodeImage",vec![("slot",T::Int),("asset",T::String),("options",T::Optional(Box::new(T::Map)))],T::String,"Decode a client image into __surface/<slot> for Bitmap/Layer/UI/3D use. Options {format?,profile?}; external bases/masks use the same scoped VFS and keys remain in the host provider."),
                method("defineProfile",vec![("name",T::String),("profile",T::Map)],T::Void,"Define an immutable client profile {limits?,rules:[{format,path?,transforms:[{algorithm,key_id?,parameters?,expected_size?}]}]}. No raw keys. Up to 32 profiles; default is created lazily."),
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
            "formats" => json!(state.resources.formats.names()),
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
