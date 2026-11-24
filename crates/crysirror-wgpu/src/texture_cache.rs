// SPDX-License-Identifier: MPL-2.0
//! Retire old GPU cache generations before isolated-render budget admission.
//! Cache residency follows declared dependencies, not the most recent upload.
use super::*;
use std::collections::VecDeque;

/// Optional residency for small immutable bitmap cells across isolate passes.
/// Active frame entries are moved here, so an output is allocated only once.
#[derive(Default)]
pub(super) struct RecentSnapshots {
    entries: VecDeque<(String, Texture)>,
    bytes: u64,
}

impl RecentSnapshots {
    fn park(&mut self, key: String, texture: Texture) {
        const MAX_BYTES: u64 = 2 * 1024 * 1024;
        const MAX_ENTRIES: usize = 128;
        if !key.starts_with("snapshot:") || texture.texture.width() > 96
            || texture.texture.height() > 96
        {
            return;
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes + texture.bytes > MAX_BYTES {
            let Some((_, evicted)) = self.entries.pop_front() else { return };
            self.bytes -= evicted.bytes;
        }
        self.bytes += texture.bytes;
        self.entries.push_back((key, texture));
    }

    pub(super) fn take(&mut self, key: &str) -> Option<Texture> {
        let index = self.entries.iter().position(|(name, _)| name == key)?;
        let (_, texture) = self.entries.remove(index)?;
        self.bytes -= texture.bytes;
        Some(texture)
    }
}

impl Renderer {
    pub(super) fn retain_frame_textures(&mut self, used: &BTreeSet<String>) {
        for (key, texture) in self.textures.extract_if(.., |key, _| !used.contains(key)) {
            self.recent_snapshots.park(key, texture);
        }
    }

    pub(super) fn resident_texture_bytes(&self) -> u64 {
        self.textures.values().map(|texture| texture.bytes).sum::<u64>()
            + self.recent_snapshots.bytes
    }

    pub(super) fn trim_recent_snapshots_for(&mut self, additional: u64) -> u64 {
        let resident = self.resident_texture_bytes();
        if resident + additional > 256 * 1024 * 1024 {
            let live = resident - self.recent_snapshots.bytes;
            self.recent_snapshots = Default::default();
            live
        } else {
            resident
        }
    }

    pub(super) fn trim_isolated_snapshot_caches(
        &mut self,
        child: &mut Self,
        work_bytes: u64,
    ) -> (u64, u64) {
        let (mut resident, mut isolated) =
            (self.resident_texture_bytes(), child.resident_texture_bytes());
        if resident + isolated + work_bytes > 256 * 1024 * 1024 {
            resident -= self.recent_snapshots.bytes;
            isolated -= child.recent_snapshots.bytes;
            self.recent_snapshots = Default::default();
            child.recent_snapshots = Default::default();
        }
        (resident, isolated)
    }

    pub(super) fn scene_cache_keys(assets: &Assets, scene: &SceneState) -> Result<BTreeSet<String>> {
        let mut keys = BTreeSet::new();
        for (id, node) in scene.draw_order() {
            match &node.visual {
                Visual::Sprite(sprite) => {
                    keys.insert(Self::image_key(assets, id.client.0, sprite.asset.relative())?);
                }
                Visual::Text(text) => {
                    let mut local = text.clone();
                    local.bounds.x = 0;
                    local.bounds.y = 0;
                    keys.insert(assets.text.for_client(id.client).key(&local)?);
                }
            }
        }
        Ok(keys)
    }

    pub(super) fn frame_cache_keys(&self, assets: &Assets, frame: &[Primitive]) -> Result<BTreeSet<String>> {
        fn collect(renderer: &Renderer, assets: &Assets, frame: &[Primitive], keys: &mut BTreeSet<String>) -> Result<()> {
            for p in frame {
                for target in p.target.iter().chain(p.output.iter()) {
                    keys.insert(Renderer::target_key(assets,target));
                }
                if let Some(source) = p.target.as_ref().filter(|t|p.output.as_ref()==Some(*t)) { keys.insert(format!("{}:read",Renderer::target_key(assets,source))); }
                // Backing is the sampled image once promoted. Its original
                // Paint and one-shot seed are no longer GPU dependencies.
                if p.target.is_none() { match &p.paint {
                    Paint::Solid { .. } => { keys.insert("solid:white".into()); }
                    Paint::Backdrop { .. } => { keys.insert(BACKDROP_KEY.into()); }
                    Paint::Image { asset, .. } => {
                        keys.insert(Renderer::image_key(assets, p.owner, asset)?);
                        keys.insert(Renderer::sampled_image_key(assets, p.owner, asset, p.sampling)?);
                    }
                    Paint::Text { text, font, size, color, layout } => {
                        keys.insert(assets.text.for_client(ClientId(p.owner)).key(&TextRun {
                            text: text.clone(), font: font.clone(), size_px: *size,
                            bounds: Rect { x: 0, y: 0, width: p.size[0], height: p.size[1] },
                            color: Color(*color), layout_profile: layout.clone(),
                        })?);
                    }
                    Paint::GlyphRun { .. } => { glyph_run::cache_keys(assets, p.owner, &p.paint, keys)?; }
                    Paint::Snapshot { size, layers } => {
                        // A completed snapshot owns its immutable pixels. Its
                        // input draws are needed only while a result is missing.
                        let key = Renderer::snapshot_key(assets, p.owner, *size, layers)?;
                        let ready = renderer.textures.contains_key(&key);
                        if keys.insert(key) && !ready {
                            let mut isolated = crysirror_scene::layer::LayerSystem::default();
                            isolated.install(ClientId(p.owner), layers)?;
                            collect(renderer, assets, &isolated.frame(*size)?, keys)?;
                        }
                    }
                    Paint::Puppet { .. } | Paint::Mesh { .. } => {
                        let [w,h] = Renderer::puppet_size(p);
                        keys.insert(format!("puppet:{}:{}:{w}x{h}", p.owner, p.id));
                    }
                    Paint::ProjectedMesh { frame, .. } => {
                        keys.insert("solid:white".into());
                        keys.insert("curve:identity".into());
                        for part in &frame.parts {
                            if let Some(asset) = &part.texture {
                                keys.insert(Renderer::image_key(assets,p.owner,asset)?);
                                keys.insert(Renderer::sampled_image_key_filter(assets,p.owner,asset,p.sampling,part.linear)?);
                            }
                        }
                    }
                    Paint::LegacyMesh { .. } => anyhow::bail!("legacy mesh must be evaluated by shared layer extraction"),
                    Paint::CoverageWrite{..}|Paint::CoverageReset{..}|Paint::GlyphView{..}|Paint::Group { .. } | Paint::OrderedState { .. } => anyhow::bail!("composition command must be consumed by shared layer extraction"),
                }
                }
                if let Some(mesh) = &p.puppet {
                    // History can attach a mesh to a solid placeholder. Keep
                    // its actual output alive across capture/presentation or
                    // the same declaration will sample and tint itself twice.
                    let [w, h] = Renderer::puppet_size(p);
                    keys.insert(format!("puppet:{}:{}:{w}x{h}", p.owner, p.id));
                    for asset in &mesh.textures {
                        keys.insert(match crysirror_core::service::frame_history_layer(asset) {
                            Some(id) => format!("history:{}:{id}", p.owner),
                            None if asset == BACKDROP_KEY => BACKDROP_KEY.into(),
                            None => Renderer::image_key(assets, p.owner, asset)?,
                        });
                    }
                }
                if let Some(filter) = &p.filter {
                    if let Some(stencil)=&filter.stencil {keys.insert(Renderer::image_key(assets,p.owner,&stencil.asset)?);}
                    keys.insert(match &filter.mask { Some(mask) => Renderer::image_key(assets, p.owner, &mask.asset)?, None => "solid:white".into() });
                    keys.insert(match &filter.tone_curve { Some(curve) => Renderer::image_key(assets, p.owner, &curve.asset)?, None => "curve:identity".into() });
                }
                if let Some(composite) = &p.composite {
                    keys.insert(Renderer::image_key(assets, p.owner, &format!("__crysirror_composite/{}",p.id))?);
                    if let Some((rule,_)) = &composite.rule {
                        keys.insert(Renderer::image_key(assets, p.owner, &rule.asset)?);
                    }
                    collect(renderer, assets, &composite.before, keys)?;
                    collect(renderer, assets, &composite.after, keys)?;
                }
            }
            Ok(())
        }
        let mut keys = BTreeSet::new();
        collect(self, assets, frame, &mut keys)?;
        Ok(keys)
    }

    pub(super) fn preserve_capture_textures(&mut self, used: &mut BTreeSet<String>) {
        used.extend(self.capture_texture_keys.iter().cloned());
        used.extend(self.snapshot_readbacks.iter().cloned());
        self.snapshot_readers.retain(|key, readers| {
            readers.retain(|reader| reader.strong_count() != 0);
            if readers.is_empty() { false } else { used.insert(key.clone()); true }
        });
    }

    pub(super) fn retire_unused_textures(&mut self, assets: &Assets) -> Result<()> {
        let mut used = self.frame_cache_keys(assets, &self.engine_frame)?;
        used.extend(self.scene_texture_keys.iter().cloned());
        self.preserve_capture_textures(&mut used);
        self.retain_frame_textures(&used);
        self.target_write_revisions.retain(|key,_|used.contains(key));
        Ok(())
    }
}
