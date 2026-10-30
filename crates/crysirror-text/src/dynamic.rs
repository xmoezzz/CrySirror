//! Immutable dynamic faces shared by measurement and renderer views.
use crate::TextSystem;
use ab_glyph::FontArc;
use anyhow::{Context, Result, ensure};
use crysirror_core::state::ClientId;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
/// Maximum encoded size of one font face source.
pub const MAX_FONT_BYTES: usize = 32 * 1024 * 1024;
/// Worker-prepared outline/shaping font; publication performs no parsing.
pub struct PreparedFontFace {
    pub(crate) font: FontArc,
    pub(crate) bytes: Arc<[u8]>,
    /// Selected collection face index.
    pub face_index: u32,
    /// Face count in the encoded source collection.
    pub collection_faces: u32,
    /// Immutable source and collection-face content hash.
    pub digest: String,
    /// Unicode family/full/PostScript names from the validated face.
    pub families: Vec<String>,
    retention: Option<Arc<dyn Send + Sync>>,
}
impl PreparedFontFace {
    /// Validate both outline and shaping backends on a CPU worker.
    pub fn prepare(bytes: Arc<[u8]>, face_index: u32) -> Result<Self> {
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_FONT_BYTES,
            "font byte limit"
        );
        let face = rustybuzz::Face::from_slice(&bytes, face_index)
            .context("invalid shaping font face/index")?;
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
        let font: FontArc = ab_glyph::FontVec::try_from_vec_and_index(bytes.to_vec(), face_index)
            .map_err(|e| anyhow::anyhow!("font outline: {e}"))?
            .into();
        let digest = if face_index == 0 {
            blake3::hash(&bytes).to_hex().to_string()
        } else {
            let mut h = blake3::Hasher::new();
            h.update(&bytes);
            h.update(&face_index.to_le_bytes());
            h.finalize().to_hex().to_string()
        };
        Ok(Self {
            font,
            bytes: bytes.clone(),
            face_index,
            collection_faces: rustybuzz::ttf_parser::fonts_in_collection(&bytes).unwrap_or(1),
            digest,
            families,
            retention: None,
        })
    }
    /// Keep a host memory reservation alive with every font clone.
    pub fn retain_storage(&mut self, lease: Arc<dyn Send + Sync>) {
        self.retention = Some(lease);
    }
    /// Resident source size, excluding parsed outline backend overhead.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}
#[derive(Clone, Default)]
pub(crate) struct Catalog(Arc<RwLock<BTreeMap<String, (ClientId, Arc<PreparedFontFace>)>>>);
impl TextSystem {
    /// Share the dynamic resource catalog; declared static fonts remain unchanged.
    pub fn attach_dynamic_fonts(&mut self, source: &Self) {
        self.dynamic = source.dynamic.clone();
    }
    /// A cheap client view. Dynamic aliases belonging to other clients are denied.
    pub fn for_client(&self, owner: ClientId) -> Self {
        let mut view = self.clone();
        view.scope = Some(owner);
        view
    }
    pub(crate) fn dynamic_face(&self, alias: &str) -> Result<Arc<PreparedFontFace>> {
        let map = self
            .dynamic
            .0
            .read()
            .map_err(|_| anyhow::anyhow!("font catalog poisoned"))?;
        let (owner, face) = map
            .get(alias)
            .with_context(|| format!("font alias '{alias}' is not loaded"))?;
        ensure!(
            self.scope == Some(*owner),
            "font alias belongs to another client"
        );
        Ok(face.clone())
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
        if let Some((old_owner, old)) = map.get(alias) {
            ensure!(
                *old_owner == owner && old.digest == face.digest,
                "font alias content collision"
            );
            return Ok(());
        }
        ensure!(map.len() < 128, "dynamic font catalog limit");
        map.insert(alias.into(), (owner, face));
        Ok(())
    }
    /// Release the catalog reference. Existing worker clones remain valid until dropped.
    pub fn unpublish_font(&self, owner: ClientId, alias: &str) -> Result<()> {
        let mut map = self
            .dynamic
            .0
            .write()
            .map_err(|_| anyhow::anyhow!("font catalog poisoned"))?;
        ensure!(
            map.get(alias).is_some_and(|(o, _)| *o == owner),
            "unknown/foreign font alias"
        );
        map.remove(alias);
        Ok(())
    }
}
