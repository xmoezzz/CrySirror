//! Immutable dynamic faces shared by measurement and renderer views.
use crate::TextSystem;
use ab_glyph::FontArc;
use anyhow::{Context, Result, ensure};
use crysirror_core::state::ClientId;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock, Weak},
};
/// Maximum encoded size of one font face source.
pub const MAX_FONT_BYTES: usize = 32 * 1024 * 1024;
/// Worker-prepared outline/shaping font; publication performs no parsing.
pub struct PreparedFontFace {
    pub(crate) font: Option<FontArc>,
    pub(crate) precomputed: Option<Arc<crate::precomputed_font::PrecomputedFont>>,
    pub(crate) bytes: Arc<[u8]>,
    /// Selected collection face index.
    pub face_index: u32,
    /// Face count in the encoded source collection.
    pub collection_faces: u32,
    /// Immutable source and collection-face content hash.
    pub digest: String,
    /// Unicode family/full/PostScript names from the validated face.
    pub families: Vec<String>,
    pub(crate) style: Option<crysirror_core::service::FontFaceStyle>,
    retention: Option<Arc<dyn Send + Sync>>,
}
impl PreparedFontFace {
    /// Validate both outline and shaping backends on a CPU worker.
    pub fn prepare(bytes: Arc<[u8]>, face_index: u32) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_FONT_BYTES,
            "font byte limit"
        );
        // WOFF/WOFF2 sources are decoded to their sfnt once, here.
        let bytes: Arc<[u8]> = match crate::webfont::decode(&bytes)? {
            Some(sfnt) => sfnt.into(),
            None => bytes,
        };
        let face = rustybuzz::Face::from_slice(&bytes, face_index)
            .context("invalid shaping font face/index")?;
        let families = family_names(&face)?;
        let style = face_style(&face);
        let shared = crate::intern_font_arc(bytes);
        let bytes = shared.clone();
        let font = crate::font_arc(shared, face_index).map_err(|e| anyhow::anyhow!("font outline: {e}"))?;
        let digest = if face_index == 0 {
            blake3::hash(&bytes).to_hex().to_string()
        } else {
            let mut h = blake3::Hasher::new();
            h.update(&bytes);
            h.update(&face_index.to_le_bytes());
            h.finalize().to_hex().to_string()
        };
        Ok(Self {
            font: Some(font),
            precomputed: None,
            bytes: bytes.clone(),
            face_index,
            collection_faces: rustybuzz::ttf_parser::fonts_in_collection(&bytes).unwrap_or(1),
            digest,
            families,
            style,
            retention: None,
        })
    }
    /// Prepare a fixed-size coverage face without an outline/shaping backend.
    /// The caller reserves `PrecomputedFont::required_resident_bytes` first.
    pub fn prepare_precomputed(
        bytes: Arc<[u8]>,
        pixel_size: u16,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        let face = crate::precomputed_font::PrecomputedFont::from_tft(&bytes, pixel_size, check)?;
        let mut hash = blake3::Hasher::new();
        hash.update(b"precomputed-v1");
        hash.update(&pixel_size.to_le_bytes());
        hash.update(&bytes);
        Ok(Self {
            font: None,
            precomputed: Some(Arc::new(face)),
            bytes,
            face_index: 0,
            collection_faces: 1,
            digest: hash.finalize().to_hex().to_string(),
            families: Vec::new(),
            style: None,
            retention: None,
        })
    }
    /// Keep a host memory reservation alive with every font clone.
    pub fn retain_storage(&mut self, lease: Arc<dyn Send + Sync>) {
        if let Some(face) = &mut self.precomputed {
            // Worker preparation owns this Arc exclusively until publication.
            // Bitmap-only read clones must retain their accounting lease too.
            Arc::get_mut(face)
                .expect("prepared bitmap face shared before publication")
                .retain_storage(lease.clone());
        }
        self.retention = Some(lease);
    }
    /// Resident source size, excluding parsed outline backend overhead.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}
pub(crate) fn face_style(
    face: &rustybuzz::Face<'_>,
) -> Option<crysirror_core::service::FontFaceStyle> {
    face.tables().os2.as_ref()?;
    Some(crysirror_core::service::FontFaceStyle {
        weight: face.weight().to_number(),
        italic: face.is_italic(),
    })
}
pub(crate) fn family_names(face: &rustybuzz::Face<'_>) -> Result<Vec<String>> {
    let mut families = Vec::new();
    for name in face.names() {
        if matches!(name.name_id, 1 | 4 | 6 | 16) {
            if let Some(value) = name.to_string() {
                if !value.is_empty()
                    && value.len() <= 256
                    && !value.chars().any(char::is_control)
                    && !families.contains(&value)
                {
                    ensure!(
                        families.len() < 128
                            && families.iter().map(String::len).sum::<usize>() + value.len()
                                <= 16384,
                        "font family metadata limit"
                    );
                    families.push(value);
                }
            }
        }
    }
    Ok(families)
}
struct Registration {
    owner: ClientId,
    face: Arc<PreparedFontFace>,
}
#[derive(Default)]
struct CatalogState {
    active: BTreeMap<String, Arc<Registration>>,
    // Does not own the face. Only actual admitted snapshots/resources keep a
    // released canonical render identity valid. Friendly aliases/handles are
    // removed independently by the resource owner.
    retired: BTreeMap<String, Weak<Registration>>,
}
impl CatalogState {
    fn prune(&mut self) {
        self.retired.retain(|_, entry| entry.strong_count() != 0);
    }
    fn frozen(&self) -> Self {
        let mut active = self.active.clone();
        for (alias, entry) in &self.retired {
            if let Some(entry) = entry.upgrade() {
                active.insert(alias.clone(), entry);
            }
        }
        Self {
            active,
            retired: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Default)]
pub(crate) struct Catalog(Arc<RwLock<CatalogState>>);
impl TextSystem {
    /// Freeze only one authorized canonical font. A metadata job must not keep
    /// unrelated client faces alive merely because they share the catalog.
    pub fn frozen_font(&self, alias: &str) -> Result<Self> {
        self.font_digest(alias)?;
        let mut frozen = self.clone();
        if let Some(face) = self.fonts.get(alias) {
            frozen.fonts = Arc::new(BTreeMap::from([(alias.to_owned(), face.clone())]));
            frozen.font_sources = Arc::new(
                self.font_sources
                    .get(alias)
                    .map(|v| BTreeMap::from([(alias.to_owned(), v.clone())]))
                    .unwrap_or_default(),
            );
            frozen.font_families = Arc::new(
                self.font_families
                    .get(alias)
                    .map(|v| BTreeMap::from([(alias.to_owned(), v.clone())]))
                    .unwrap_or_default(),
            );
            frozen.font_styles = Arc::new(
                self.font_styles
                    .get(alias)
                    .map(|v| BTreeMap::from([(alias.to_owned(), *v)]))
                    .unwrap_or_default(),
            );
            frozen.dynamic = Catalog::default();
        } else {
            let entry = self.dynamic_registration(alias)?;
            frozen.fonts = Default::default();
            frozen.font_sources = Default::default();
            frozen.font_families = Default::default();
            frozen.font_styles = Default::default();
            frozen.dynamic = Catalog(Arc::new(RwLock::new(CatalogState {
                active: BTreeMap::from([(alias.to_owned(), entry)]),
                retired: BTreeMap::new(),
            })));
        }
        Ok(frozen)
    }
    /// Freeze catalog membership while sharing immutable prepared font faces.
    /// Later publication/release cannot change an admitted render job's fonts.
    pub fn frozen(&self) -> Result<Self> {
        let entries = self
            .dynamic
            .0
            .read()
            .map_err(|_| anyhow::anyhow!("font catalog poisoned"))?
            .frozen();
        let mut frozen = self.clone();
        frozen.dynamic = Catalog(Arc::new(RwLock::new(entries)));
        Ok(frozen)
    }
    /// Share the dynamic resource catalog; declared static fonts remain unchanged.
    pub fn attach_dynamic_fonts(&mut self, source: &Self) {
        self.dynamic = source.dynamic.clone();
    }
    /// Cached aliases for static and active client-owned faces. Retired faces
    /// and faces owned by other clients do not become new selectable bindings.
    pub fn font_aliases(&self) -> Result<Vec<String>> {
        let catalog = self
            .dynamic
            .0
            .read()
            .map_err(|_| anyhow::anyhow!("font catalog poisoned"))?;
        let mut aliases = self
            .fonts
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        aliases.extend(
            catalog
                .active
                .iter()
                .filter(|(_, entry)| self.scope == Some(entry.owner))
                .map(|(alias, _)| alias.clone()),
        );
        ensure!(aliases.len() <= 4096, "font enumeration budget");
        Ok(aliases.into_iter().collect())
    }
    /// A cheap client view. Dynamic aliases belonging to other clients are denied.
    pub fn for_client(&self, owner: ClientId) -> Self {
        let mut view = self.clone();
        view.scope = Some(owner);
        view
    }
    fn dynamic_registration(&self, alias: &str) -> Result<Arc<Registration>> {
        let map = self
            .dynamic
            .0
            .read()
            .map_err(|_| anyhow::anyhow!("font catalog poisoned"))?;
        let entry = match map.active.get(alias) {
            Some(entry) => entry.clone(),
            None => map
                .retired
                .get(alias)
                .and_then(Weak::upgrade)
                .with_context(|| format!("font alias '{alias}' is not loaded or retained"))?,
        };
        ensure!(
            self.scope == Some(entry.owner),
            "font alias belongs to another client"
        );
        Ok(entry)
    }
    pub(crate) fn dynamic_face(&self, alias: &str) -> Result<Arc<PreparedFontFace>> {
        Ok(self.dynamic_registration(alias)?.face.clone())
    }
    /// Publish an immutable client-owned face. Repeated identical publication is idempotent.
    pub fn publish_font(
        &self,
        owner: ClientId,
        alias: &str,
        face: Arc<PreparedFontFace>,
    ) -> Result<()> {
        ensure!(
            alias.starts_with(&format!("__font/{}/", owner.0)),
            "invalid canonical font alias"
        );
        let mut map = self
            .dynamic
            .0
            .write()
            .map_err(|_| anyhow::anyhow!("font catalog poisoned"))?;
        if let Some(old) = map.active.get(alias) {
            ensure!(
                old.owner == owner && old.face.digest == face.digest,
                "font alias content collision"
            );
            return Ok(());
        }
        map.prune();
        if let Some(old) = map.retired.get(alias).and_then(Weak::upgrade) {
            ensure!(
                old.owner == owner && old.face.digest == face.digest,
                "retained font alias content collision"
            );
            map.retired.remove(alias);
            map.active.insert(alias.into(), old);
            return Ok(());
        }
        ensure!(
            map.active.len() + map.retired.len() < 128,
            "dynamic font catalog limit"
        );
        map.active
            .insert(alias.into(), Arc::new(Registration { owner, face }));
        Ok(())
    }
    /// Release the live catalog reference. A canonical rendering lookup can
    /// still upgrade the face while an admitted immutable snapshot/resource
    /// holds it. Last-snapshot release invalidates that lookup; weak entries
    /// retain no font bytes and are collected before the next publication.
    pub fn unpublish_font(&self, owner: ClientId, alias: &str) -> Result<()> {
        let mut map = self
            .dynamic
            .0
            .write()
            .map_err(|_| anyhow::anyhow!("font catalog poisoned"))?;
        ensure!(
            map.active
                .get(alias)
                .is_some_and(|entry| entry.owner == owner),
            "unknown/foreign font alias"
        );
        let entry = map.active.remove(alias).expect("validated active font");
        map.retired.insert(alias.to_owned(), Arc::downgrade(&entry));
        Ok(())
    }
}
