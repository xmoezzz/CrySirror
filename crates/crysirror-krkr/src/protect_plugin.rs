// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `Plugins.linkZ` protected-bootstrap facade. No native code is loaded.
//!
//! The compressed BOOTSTRAP module registers `System.bootStrap`,
//! `System.checkSignature`, `Storages.CompoundStorageMedia`,
//! `Storages.parseArchiveIndex`, `Storages.archiveUniqueKey` and
//! `Storages.isExistentStorageNoSearchNoNormalize` (member names read from the
//! module's own string table). Here each maps onto engine services: the HXV4
//! resource plugin validates the bootstrap and publishes its profile, the shared
//! `crysirror_resource::compound` media routes `arc://` lookups onto mounted
//! containers, and `crysirror_resource::signature` verifies `.sig` files.
//! Unmapped members (`setFile`, `getFile`, custom archive seeds ...) fail visibly.
//!
//! CXDEC titles instead receive `Storages.setupArchiveData` from their
//! protection module before `startup.tjs` runs. Its effect (Special name
//! index and content-filter parameters) is already applied by the shared
//! `kirikiri/xp3` mount; the facade only checks that the script supplies the
//! exact argument that mount authenticated, so a mismatch fails visibly.
use crate::natives::{Rt, methods, object_arg, string};
use crysirror_resource::{compound::CompoundStorage, signature::RsaPssSha256Verifier};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{Closure, ObjectHandle, Variant},
};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Profile {
    pub family: String,
    pub executable: String,
    pub bootstrap_storage: String,
    pub bootstrap_prefix: String,
    pub archive_unique_key: String,
    pub media: String,
    /// Directory of the mounted container that answers hashed names.
    #[serde(default)]
    pub hashed_root: Option<String>,
    pub context_token: String,
    pub signature_public_key: String,
}

/// Engine-published CXDEC profile (`cxdec_bootstrap_profile`).
#[derive(Clone, Debug, Deserialize)]
pub struct CxdecProfile {
    pub family: String,
    /// The argument whose derived seeds authenticated the Special index.
    pub setup_archive_data: String,
    /// Native startup redirect (`archive.xp3>member`) served as startup.tjs.
    pub startup: String,
}

#[derive(Default)]
pub struct State {
    profile: Option<Profile>,
    cxdec: Option<CxdecProfile>,
    archive_data_set: bool,
    linked: bool,
    booted: bool,
    token: Vec<u8>,
    callback: Option<Variant>,
    media: Option<ObjectHandle>,
    unique_key: Option<String>,
}

impl State {
    /// Companion executable recorded by the validated profile.
    pub fn executable(&self) -> Option<String> {
        self.profile.as_ref().map(|p| p.executable.clone())
    }
}

fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(e.to_string())
}

/// Load the engine-published profile named by the adapter configuration.
pub fn configure(state: &mut State, json: &[u8]) -> anyhow::Result<()> {
    let value: serde_json::Value = serde_json::from_slice(json)?;
    if value.get("family").and_then(|f| f.as_str()) == Some("cxdec-setup-archive-data") {
        let profile: CxdecProfile = serde_json::from_value(value)?;
        anyhow::ensure!(
            !profile.setup_archive_data.is_empty()
                && profile.setup_archive_data.encode_utf16().count() <= 32 * 1024,
            "CXDEC setupArchiveData profile length"
        );
        state.cxdec = Some(profile);
        return Ok(());
    }
    let profile: Profile = serde_json::from_value(value)?;
    anyhow::ensure!(
        profile.family == "hxv4-bootstrap",
        "unknown protected bootstrap family `{}`",
        profile.family
    );
    anyhow::ensure!(
        profile.context_token.len() == 64,
        "bootstrap context token length"
    );
    state.token = (0..32)
        .map(|i| u8::from_str_radix(&profile.context_token[i * 2..i * 2 + 2], 16))
        .collect::<std::result::Result<_, _>>()?;
    state.profile = Some(profile);
    Ok(())
}

fn profile(rt: &Rt) -> Result<Profile> {
    rt.host()
        .protect
        .profile
        .clone()
        .ok_or_else(|| err("protected bootstrap profile not configured"))
}

/// Startup natives a validated profile provides before `startup.tjs` runs
/// (the CXDEC protection module registers them at process start).
pub fn install_startup(rt: &mut Rt) {
    if rt.host().protect.cxdec.is_none() {
        return;
    }
    let storages = rt
        .global_member("Storages")
        .object_handle()
        .expect("Storages");
    methods(rt, storages, &[("setupArchiveData", setup_archive_data)]);
}

fn setup_archive_data(rt: &mut Rt, _: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let text = string(&a, 0, "Storages.setupArchiveData")?;
    let state = &rt.host().protect;
    let p = state
        .cxdec
        .as_ref()
        .ok_or_else(|| err("Storages.setupArchiveData: no validated CXDEC profile"))?;
    if text != p.setup_archive_data {
        return Err(err(
            "Storages.setupArchiveData: argument does not derive the mounted archives' index",
        ));
    }
    if state.archive_data_set {
        return Err(err("Storages.setupArchiveData: archive data already set up"));
    }
    rt.host_mut().protect.archive_data_set = true;
    Ok(Variant::Void)
}

pub fn link_z(rt: &mut Rt, args: Vec<Variant>) -> Result<Variant> {
    let storage = string(&args, 0, "Plugins.linkZ")?;
    let Some(p) = rt.host().protect.profile.clone() else {
        return Err(crate::natives::unsupported(
            &format!("Plugins.linkZ({storage:?})"),
            "a validated protected-bootstrap profile from the shared HXV4 resource plugin",
        ));
    };
    if !storage.eq_ignore_ascii_case(&p.bootstrap_storage) {
        return Err(err(format!(
            "Plugins.linkZ({storage:?}): not the validated bootstrap module {:?}",
            p.bootstrap_storage
        )));
    }
    if rt.host().protect.linked {
        return Err(err("Plugins.linkZ: bootstrap module already linked"));
    }
    install(rt);
    rt.host_mut().protect.linked = true;
    rt.host_mut().loaded_plugins.insert("bootstrap".into());
    Ok(Variant::Void)
}

fn install(rt: &mut Rt) {
    let system = rt.global_member("System").object_handle().expect("System");
    methods(
        rt,
        system,
        &[("bootStrap", boot_strap), ("checkSignature", check_signature)],
    );
    let storages = rt
        .global_member("Storages")
        .object_handle()
        .expect("Storages");
    methods(
        rt,
        storages,
        &[
            ("parseArchiveIndex", parse_archive_index),
            ("isExistentStorageNoSearchNoNormalize", exists_exact),
        ],
    );
    rt.register_object_native_property(
        storages,
        "archiveUniqueKey",
        |rt, _| {
            let key = profile(rt)?.archive_unique_key;
            Ok(Variant::String(key))
        },
        |rt, _, v| {
            let value = v.to_tjs_string()?;
            if value != profile(rt)?.archive_unique_key {
                return Err(err("Storages.archiveUniqueKey does not match the archive"));
            }
            rt.host_mut().protect.unique_key = Some(value);
            Ok(())
        },
    );
    install_media_class(rt, storages);
}

fn boot_strap(rt: &mut Rt, _: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let prefix = string(&a, 0, "System.bootStrap")?;
    let p = profile(rt)?;
    if rt.host().protect.booted {
        return Err(err("System.bootStrap: already bootstrapped"));
    }
    if prefix != p.bootstrap_prefix {
        return Err(err("System.bootStrap: prefix does not match the validated bootstrap"));
    }
    if matches!(a.get(3), Some(v) if !matches!(v, Variant::Void)) {
        return Err(crate::natives::unsupported(
            "System.bootStrap custom archive seed",
            "a validated per-archive seed profile",
        ));
    }
    let callback = a.get(1).cloned().unwrap_or_default();
    if !matches!(callback, Variant::Void) && object_arg(&callback).is_none() {
        return Err(err("System.bootStrap: auto-path callback must be callable"));
    }
    let state = &mut rt.host_mut().protect;
    state.booted = true;
    state.callback = (!matches!(callback, Variant::Void)).then_some(callback);
    Ok(Variant::Octet(state.token.clone()))
}

/// Archive auto paths (`archive>domain`) go through the bootstrap callback.
/// Returns `None` when the path is not an archive path or no callback exists.
pub fn auto_path_hook(rt: &mut Rt, path: &str, mounting: bool) -> Result<Option<bool>> {
    let Some(callback) = rt.host().protect.callback.clone() else {
        return Ok(None);
    };
    let Some((archive, domain)) = path.split_once('>') else {
        return Ok(None);
    };
    let archive = if archive.is_empty() {
        ">".to_owned()
    } else {
        archive.to_owned()
    };
    let r = rt.call_function(
        callback,
        vec![
            Variant::String(archive),
            Variant::String(domain.to_owned()),
            Variant::Integer(mounting as i64),
        ],
    )?;
    Ok(Some(r.is_truthy()))
}

fn check_signature(rt: &mut Rt, _: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let file = string(&a, 0, "System.checkSignature")?;
    let p = profile(rt)?;
    let sig = match a.get(1) {
        None | Some(Variant::Void) => format!("{file}.sig"),
        Some(v) => {
            let s = v.to_tjs_string()?;
            if s.is_empty() { format!("{file}.sig") } else { s }
        }
    };
    let storage = rt.host().storage()?;
    let resolve = |name: &str| {
        storage
            .resolve(name)
            .ok_or_else(|| err(format!("System.checkSignature: {name}: file not found.")))
    };
    let (file_path, sig_path) = (resolve(&file)?, resolve(&sig)?);
    let verifier = RsaPssSha256Verifier::from_pkcs1_pem(p.signature_public_key.as_bytes())
        .map_err(|e| err(format!("System.checkSignature: {e}")))?;
    let blob = storage
        .read_limited(&sig_path, 16 * 1024)
        .map_err(|e| err(format!("{e:#}")))?;
    // Synchronous bounded read: the ADR 0050 worker split for whole-file
    // signature checks is still open; the cap keeps a stall bounded.
    let message = storage
        .read_limited(&file_path, 64 * 1024 * 1024)
        .map_err(|e| err(format!("{e:#}")))?;
    let ok = verifier
        .verify(&message, &blob)
        .map_err(|e| err(format!("System.checkSignature: {e}")))?;
    Ok(Variant::Integer(ok as i64))
}

fn parse_archive_index(rt: &mut Rt, _: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let name = string(&a, 0, "Storages.parseArchiveIndex")?;
    let source = rt
        .host()
        .storage()?
        .archive_source(&name)
        .map_err(|e| err(format!("{e:#}")))?;
    // Index layout is private to this facade: only CompoundStorageMedia reads it.
    Ok(match source {
        Some(source) => {
            let h = rt.alloc_array_object(vec![
                Variant::String("XP3ArchiveIndex".into()),
                Variant::String(source),
            ]);
            Variant::Object(h)
        }
        None => Variant::Void,
    })
}

fn exists_exact(rt: &mut Rt, _: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let name = string(&a, 0, "Storages.isExistentStorageNoSearchNoNormalize")?;
    Ok(Variant::Integer(
        rt.host().storage()?.exists_exact(&name) as i64,
    ))
}

// ------------------------------------------------------ CompoundStorageMedia

fn install_media_class(rt: &mut Rt, storages: ObjectHandle) {
    const NAME: &str = "CompoundStorageMedia";
    let class = rt.alloc_native_constructor(|rt: &mut Rt, this: Option<ObjectHandle>, a| {
        let existing = this
            .map(|h| rt.bound_this(h).unwrap_or(h))
            .filter(|h| *h != rt.global_handle());
        let h = existing.unwrap_or_else(|| rt.alloc_ordinary_object());
        let storages = rt.global_member("Storages").object_handle().expect("Storages");
        if let Variant::Object(class) = rt.object_member(storages, NAME) {
            if rt.object_super_class(h).is_none() && class != h {
                rt.set_object_super_class(h, class);
            }
        }
        rt.add_object_class_info(h, NAME);
        construct(rt, h, a)?;
        Ok(Variant::Object(h))
    });
    rt.add_object_class_info(class, NAME);
    methods(
        rt,
        class,
        &[
            ("finalize", |rt, t, _| {
                let h = media_this(rt, t).ok();
                if h.is_some() && rt.host().protect.media == h {
                    rt.host_mut().protect.media = None;
                    rt.host_mut().storage_mut()?.compound = None;
                }
                Ok(Variant::Void)
            }),
            ("pathHash", |rt, t, a| {
                let path = string(&a, 0, "CompoundStorageMedia.pathHash")?;
                media_this(rt, t)?;
                let h = with_media(rt, |m| m.path_hash(&path))?;
                Ok(Variant::Octet(h.to_vec()))
            }),
            ("fileHash", |rt, t, a| {
                let name = string(&a, 0, "CompoundStorageMedia.fileHash")?;
                media_this(rt, t)?;
                let h = with_media(rt, |m| Ok(m.file_hash(&name)))?;
                Ok(Variant::Octet(h.to_vec()))
            }),
            ("mount", |rt, t, a| {
                let path = string(&a, 0, "CompoundStorageMedia.mount")?;
                media_this(rt, t)?;
                let source = index_source(rt, a.get(1))?;
                let expected = rt.host().storage()?.archive_source(&path).ok().flatten();
                if expected.as_deref() != Some(source.as_str()) {
                    return Err(err("CompoundStorageMedia.mount: index belongs to another archive"));
                }
                let i = with_media(rt, |m| m.mount(&source))?;
                Ok(Variant::Integer(i as i64))
            }),
            ("unmount", |rt, t, a| {
                let i = crate::natives::need_int(&a, 0, "CompoundStorageMedia.unmount")?;
                media_this(rt, t)?;
                let i = usize::try_from(i).map_err(err)?;
                with_media(rt, |m| m.unmount(i))?;
                Ok(Variant::Void)
            }),
            ("unmountAll", |rt, t, _| {
                media_this(rt, t)?;
                with_media(rt, |m| Ok(m.unmount_all()))?;
                Ok(Variant::Void)
            }),
            ("entryDomain", |rt, t, a| {
                let domain = string(&a, 0, "CompoundStorageMedia.entryDomain")?;
                media_this(rt, t)?;
                let source = index_source(rt, a.get(1))?;
                with_media(rt, |m| {
                    let i = m
                        .mount_index(&source)
                        .ok_or_else(|| anyhow::anyhow!("entryDomain: archive not mounted"))?;
                    m.entry_domain(&domain, i)
                })?;
                Ok(Variant::Void)
            }),
            ("assignDomain", |rt, t, a| {
                let domain = string(&a, 0, "CompoundStorageMedia.assignDomain")?;
                let target = string(&a, 1, "CompoundStorageMedia.assignDomain")?;
                media_this(rt, t)?;
                with_media(rt, |m| m.assign_domain(&domain, &target))?;
                Ok(Variant::Void)
            }),
            ("assignFiles", |rt, t, a| {
                let domain = string(&a, 0, "CompoundStorageMedia.assignFiles")?;
                let source = hash_arg(a.get(1))?;
                let target = string(&a, 2, "CompoundStorageMedia.assignFiles")?;
                let target_hash = hash_arg(a.get(3))?;
                media_this(rt, t)?;
                with_media(rt, |m| m.assign_files(&domain, source, &target, target_hash))?;
                Ok(Variant::Void)
            }),
            ("clearDomain", |rt, t, a| {
                let target = string(&a, 0, "CompoundStorageMedia.clearDomain")?;
                media_this(rt, t)?;
                with_media(rt, |m| Ok(m.clear_domain(&target)))?;
                Ok(Variant::Void)
            }),
            ("clearAll", |rt, t, _| {
                media_this(rt, t)?;
                with_media(rt, |m| Ok(m.clear_all()))?;
                Ok(Variant::Void)
            }),
            ("clearFiles", |_, _, _| Err(unmapped("clearFiles"))),
            ("entryFiles", |_, _, _| Err(unmapped("entryFiles"))),
            ("setFile", |_, _, _| Err(unmapped("setFile"))),
            ("getFile", |_, _, _| Err(unmapped("getFile"))),
            ("getHashes", |_, _, _| Err(unmapped("getHashes"))),
            ("getDomains", |_, _, _| Err(unmapped("getDomains"))),
        ],
    );
    rt.set_object_member(class, NAME, Variant::Closure(Closure::new(class, None)));
    rt.set_object_member(storages, NAME, Variant::Object(class));
}

fn unmapped(member: &str) -> TjsError {
    crate::natives::unsupported(
        &format!("CompoundStorageMedia.{member}"),
        "its native file-table layout, which is not established",
    )
}

fn construct(rt: &mut Rt, h: ObjectHandle, a: Vec<Variant>) -> Result<()> {
    if a.is_empty() {
        return Ok(());
    }
    let kind = string(&a, 0, "CompoundStorageMedia")?;
    let media = string(&a, 1, "CompoundStorageMedia")?;
    let p = profile(rt)?;
    let state = &rt.host().protect;
    if !state.booted {
        return Err(err("CompoundStorageMedia requires System.bootStrap"));
    }
    if state.media.is_some() {
        return Err(err("CompoundStorageMedia: one active media per bootstrap"));
    }
    if !matches!(a.get(2), Some(Variant::Octet(k)) if *k == state.token) {
        return Err(err("CompoundStorageMedia: hash key is not this bootstrap's result"));
    }
    if media != p.media {
        return Err(crate::natives::unsupported(
            &format!("CompoundStorageMedia media name {media:?}"),
            "the archive's validated media name",
        ));
    }
    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_lowercase()) || kind == "file" {
        return Err(err("CompoundStorageMedia: invalid media type"));
    }
    let compound = CompoundStorage::new(&media, &state.token).map_err(err)?;
    let storage = rt.host_mut().storage_mut()?;
    storage.compound = Some((kind, compound));
    storage.hashed_root = p.hashed_root.clone();
    rt.host_mut().protect.media = Some(h);
    Ok(())
}

fn media_this(rt: &Rt, this: Option<ObjectHandle>) -> Result<ObjectHandle> {
    let h = crate::natives::this_of(rt, this, "CompoundStorageMedia")?;
    if rt.host().protect.media != Some(h) {
        return Err(err("CompoundStorageMedia: invalidated or foreign instance"));
    }
    Ok(h)
}

fn with_media<T>(
    rt: &mut Rt,
    f: impl FnOnce(&mut CompoundStorage) -> anyhow::Result<T>,
) -> Result<T> {
    let (_, media) = rt
        .host_mut()
        .storage_mut()?
        .compound
        .as_mut()
        .ok_or_else(|| err("CompoundStorageMedia: media not registered"))?;
    f(media).map_err(|e| err(format!("CompoundStorageMedia: {e:#}")))
}

fn index_source(rt: &Rt, v: Option<&Variant>) -> Result<String> {
    let items = v
        .and_then(object_arg)
        .and_then(|h| rt.array_elements(h))
        .ok_or_else(|| err("CompoundStorageMedia: archive index required"))?;
    match items {
        [Variant::String(tag), Variant::String(source)] if tag == "XP3ArchiveIndex" => {
            Ok(source.clone())
        }
        _ => Err(err("CompoundStorageMedia: not a parseArchiveIndex result")),
    }
}

fn hash_arg(v: Option<&Variant>) -> Result<[u8; 8]> {
    match v {
        Some(Variant::Octet(b)) if b.len() == 8 => Ok(b.as_slice().try_into().unwrap()),
        _ => Err(err("CompoundStorageMedia: path hash Octet required")),
    }
}

impl State {
    pub(crate) fn gc_roots(&self, out: &mut Vec<ObjectHandle>) {
        out.extend(self.media);
        match &self.callback {
            Some(Variant::Object(h)) => out.push(*h),
            Some(Variant::Closure(c)) => out.extend(std::iter::once(c.object).chain(c.this_obj)),
            _ => {}
        }
    }
}
