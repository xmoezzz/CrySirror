// SPDX-License-Identifier: MPL-2.0
//! Content keys of immutable scene declarations. Font generations and the
//! asset store are resolved at use time, independently of the declaration.
use super::*;
use crysirror_scene::layer::LayerSpec;
use std::{
    cell::RefCell,
    io::{self, Write},
    sync::Weak,
};

struct HashWriter {
    hash: blake3::Hasher,
    remaining: usize,
}

impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.remaining = self.remaining.checked_sub(bytes.len()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "snapshot definition byte budget",
            )
        })?;
        self.hash.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

enum Definition {
    Transparent,
    Draw {
        hash: Box<blake3::Hasher>,
        fonts: Vec<String>,
        child_depth: usize,
    },
}

impl Definition {
    fn child_depth(&self) -> usize {
        match self {
            Self::Transparent => 0,
            Self::Draw { child_depth, .. } => *child_depth,
        }
    }

    fn build(
        size: [u32; 2],
        layers: &[LayerSpec],
        graph: &mut PageGraph,
        depth: usize,
    ) -> Result<Self> {
        ensure!(depth <= 16, "snapshot definition nesting budget");
        ensure!(
            layers.len() <= crysirror_scene::layer::MAX_LAYERS,
            "snapshot page node budget"
        );
        if !layers.is_empty()
            && layers.iter().all(|l| {
                matches!(&l.paint,Paint::Solid{color} if *color==[0;4])
                    && l.filter.is_none()
                    && l.blend == crysirror_scene::layer::Blend::Normal
            })
        {
            Paint::Snapshot {
                size,
                layers: Arc::new(layers.to_vec()),
            }
            .validate()?;
            return Ok(Self::Transparent);
        }

        let mut writer = HashWriter {
            hash: blake3::Hasher::new(),
            remaining: (layers.len() * 2048).max(4 << 20),
        };
        let mut fonts = BTreeSet::new();
        let mut child_depth = 0;
        let mut chunks = io::BufWriter::with_capacity(32 << 10, &mut writer);
        // Hash a page's own declarations. A child contributes its content
        // digest at every placement; its backing is processed once. No tree
        // expansion or pointer value becomes part of the texture identity.
        chunks.write_all(b"CrySirror snapshot page v2\0")?;
        serde_json::to_writer(&mut chunks, &size)?;
        let empty = Arc::new(Vec::new());
        for layer in layers {
            let paint = match &layer.paint {
                Paint::CoverageWrite { paint } => paint.as_ref(),
                p => p,
            };
            if let Paint::Snapshot {
                size: child_size,
                layers: children,
            } = paint
            {
                let child = graph.page(*child_size, children, depth + 1)?;
                child_depth = child_depth.max(1 + child.child_depth());
                let mut placement = layer.clone();
                let paint = match &mut placement.paint {
                    Paint::CoverageWrite { paint } => paint.as_mut(),
                    p => p,
                };
                if let Paint::Snapshot { layers, .. } = paint {
                    *layers = empty.clone();
                }
                serde_json::to_writer(&mut chunks, &placement)?;
                match child.as_ref() {
                    Definition::Transparent => chunks.write_all(b"T")?,
                    Definition::Draw {
                        hash,
                        fonts: aliases,
                        ..
                    } => {
                        chunks.write_all(b"D")?;
                        chunks.write_all(hash.finalize().as_bytes())?;
                        fonts.extend(aliases.iter().cloned());
                    }
                }
            } else {
                serde_json::to_writer(&mut chunks, layer)?;
                match paint {
                    Paint::Text { font, .. } => {
                        fonts.insert(font.clone());
                    }
                    Paint::GlyphRun { font, glyphs, .. } => {
                        fonts.insert(font.clone());
                        fonts.extend(glyphs.iter().filter_map(|g| g.font.clone()));
                    }
                    _ => {}
                }
            }
            chunks.write_all(b"\n")?;
        }
        chunks.flush()?;
        drop(chunks);
        Ok(Self::Draw {
            hash: Box::new(writer.hash),
            fonts: fonts.into_iter().collect(),
            child_depth,
        })
    }

    fn key(&self, assets: &Assets, owner: u32, size: [u32; 2]) -> Result<String> {
        let Self::Draw { hash, fonts, .. } = self else {
            return Ok(format!(
                "snapshot:{}:{owner}:transparent:{}x{}",
                assets.store_id(),
                size[0],
                size[1]
            ));
        };
        let mut hash = (**hash).clone();
        for font in fonts {
            hash.update(font.as_bytes());
            hash.update(
                assets
                    .text
                    .for_client(crysirror_core::state::ClientId(owner))
                    .font_digest(font)?
                    .as_bytes(),
            );
        }
        Ok(format!(
            "snapshot:{}:{owner}:{}",
            assets.store_id(),
            hash.finalize()
        ))
    }
}

type Entry = (Weak<Vec<LayerSpec>>, Arc<Definition>);
thread_local! {
    // Weak keys do not retain retired drawing surfaces. A different Arc or a
    // copy-on-write edit cannot reuse a cached declaration.
    static DEFINITIONS: RefCell<BTreeMap<(usize, [u32; 2]), Entry>> = RefCell::new(BTreeMap::new());
}

#[derive(Default)]
struct PageGraph {
    seen: BTreeMap<(usize, [u32; 2]), Arc<Definition>>,
}

impl PageGraph {
    fn page(
        &mut self,
        size: [u32; 2],
        layers: &Arc<Vec<LayerSpec>>,
        depth: usize,
    ) -> Result<Arc<Definition>> {
        ensure!(depth <= 16, "snapshot definition nesting budget");
        let key = (Arc::as_ptr(layers) as usize, size);
        if let Some(value) = self.seen.get(&key) {
            ensure!(
                depth + value.child_depth() <= 16,
                "snapshot definition nesting budget"
            );
            return Ok(value.clone());
        }
        ensure!(
            self.seen.len() < crysirror_scene::layer::MAX_LAYERS,
            "distinct snapshot page budget"
        );
        let cached = DEFINITIONS.with(|e| e.borrow().get(&key).map(|(_, v)| v.clone()));
        let value = if let Some(value) = cached {
            value
        } else {
            let value = Arc::new(Definition::build(size, layers, self, depth)?);
            if !matches!(value.as_ref(), Definition::Draw{fonts,..} if fonts.iter().map(String::len).sum::<usize>()>8192)
            {
                DEFINITIONS.with(|e| {
                    let mut entries = e.borrow_mut();
                    if entries.len() >= 128 {
                        entries.retain(|_, (weak, _)| weak.strong_count() != 0);
                    }
                    if entries.len() >= 128 {
                        entries.pop_first();
                    }
                    entries.insert(key, (Arc::downgrade(layers), value.clone()));
                });
            }
            value
        };
        ensure!(
            depth + value.child_depth() <= 16,
            "snapshot definition nesting budget"
        );
        ensure!(
            self.seen.len() < crysirror_scene::layer::MAX_LAYERS,
            "distinct snapshot page budget"
        );
        self.seen.insert(key, value.clone());
        Ok(value)
    }
}

fn definition_for(size: [u32; 2], layers: &Arc<Vec<LayerSpec>>) -> Result<Arc<Definition>> {
    PageGraph::default().page(size, layers, 0)
}

pub(super) fn cached(
    assets: &Assets,
    owner: u32,
    size: [u32; 2],
    layers: &Arc<Vec<LayerSpec>>,
) -> Result<String> {
    definition_for(size, layers)?.key(assets, owner, size)
}

pub(super) fn uncached(
    assets: &Assets,
    owner: u32,
    size: [u32; 2],
    layers: &[LayerSpec],
) -> Result<String> {
    Definition::build(size, layers, &mut PageGraph::default(), 0)?.key(assets, owner, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_pages_hash_without_an_expanded_json_allocation() {
        let leaf: LayerSpec = serde_json::from_value(serde_json::json!({
            "id":1,"rect":[0,0,1,1],"paint":{"kind":"solid","color":[111,22,33,255]}
        }))
        .unwrap();
        let mut children = vec![leaf.clone()];
        for id in 2..=3200 {
            let mut child = leaf.clone();
            child.id = id;
            child.parent = Some(1);
            children.push(child);
        }
        let mut page = leaf.clone();
        page.parent = Some(1);
        page.id = 2;
        page.paint = Paint::Snapshot {
            size: [1, 1],
            layers: Arc::new(children),
        };
        let mut placements = vec![leaf];
        for id in 2..=9 {
            let mut copy = page.clone();
            copy.id = id;
            placements.push(copy);
        }
        let mut layers = Arc::new(placements); // >25,000 expanded layers
        let mut graph = PageGraph::default();
        let previous = graph.page([1, 1], &layers, 0).unwrap();
        assert_eq!(graph.seen.len(), 2);
        assert!(graph.page([1, 1], &layers, 16).is_err());
        assert!(PageGraph::default().page([1, 1], &layers, 16).is_err());
        assert!(Arc::ptr_eq(
            &previous,
            &definition_for([1, 1], &layers).unwrap()
        ));
        fn digest(value: &Definition) -> blake3::Hash {
            let Definition::Draw { hash, .. } = value else {
                panic!("visible scene")
            };
            hash.finalize()
        }
        let duplicate = Arc::new(
            layers
                .iter()
                .cloned()
                .map(|mut layer| {
                    if let Paint::Snapshot { layers, .. } = &mut layer.paint {
                        *layers = Arc::new((**layers).clone());
                    }
                    layer
                })
                .collect(),
        );
        assert_eq!(
            digest(&previous),
            digest(&definition_for([1, 1], &duplicate).unwrap())
        );
        Arc::make_mut(&mut layers)[2].opacity = 128;
        assert_ne!(
            digest(&previous),
            digest(&definition_for([1, 1], &layers).unwrap())
        );
        let mut changed = (*layers).clone();
        let Paint::Snapshot { layers: child, .. } = &mut changed[1].paint else {
            panic!("nested page")
        };
        Arc::make_mut(child)[1].rect[0] = 0.5;
        assert_ne!(
            digest(&definition_for([1, 1], &layers).unwrap()),
            digest(&definition_for([1, 1], &Arc::new(changed)).unwrap())
        );
    }
}
