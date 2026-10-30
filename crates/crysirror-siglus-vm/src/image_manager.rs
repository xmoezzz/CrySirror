// SPDX-License-Identifier: MPL-2.0
use crate::assets::RgbaImage;
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, path::Path};
pub type ImageHandle = usize;
#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct ImageManager {
    pub images: Vec<std::sync::Arc<RgbaImage>>,
    pub originals: Vec<(u32, u32)>,
    cache: BTreeMap<String, Vec<usize>>,
    #[serde(skip)]
    pub published: BTreeMap<usize, String>,
}
impl ImageManager {
    pub(crate) fn album_frames(&mut self, name: &str) -> Result<Vec<ImageHandle>> {
        self.load_g00(name, 0)?;
        let file = if Path::new(name).extension().is_some() {
            name.to_owned()
        } else {
            format!("{name}.g00")
        };
        self.cache
            .get(&format!("asset/{file}"))
            .cloned()
            .context("native image album missing")
    }
    pub fn load_g00(&mut self, name: &str, cut: usize) -> Result<ImageHandle> {
        let file = if Path::new(name).extension().is_some() {
            name.to_owned()
        } else {
            format!("{name}.g00")
        };
        let path = format!("asset/{file}");
        if !self.cache.contains_key(&path) {
            // Native PCT names resolve in g00/. Explicit relative paths keep
            // their meaning; the root fallback preserves authored fixtures.
            let candidates = if Path::new(&file).components().count() > 1 {
                vec![path.clone()]
            } else {
                vec![format!("asset/g00/{file}"), path.clone()]
            };
            let mut found = None;
            for candidate in candidates {
                match crate::resource::read_file_bytes(Path::new(&candidate)) {
                    Ok(bytes) => {
                        found = Some(bytes);
                        break;
                    }
                    Err(error) => {
                        if !error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                        {
                            return Err(error);
                        }
                    }
                }
            }
            let bytes = found.context("native G00 resource missing")?;
            let decoded = crysirror_siglus::g00::decode_g00(&bytes)?;
            let current: usize = self.images.iter().map(|i| i.rgba.len()).sum();
            ensure!(
                current + decoded.frames.iter().map(|i| i.rgba.len()).sum::<usize>()
                    <= 128 * 1024 * 1024,
                "native image cache limit"
            );
            let ids = (self.images.len()..self.images.len() + decoded.frames.len()).collect();
            self.images
                .extend(decoded.frames.into_iter().map(std::sync::Arc::new));
            self.originals.extend(decoded.original_sizes);
            self.cache.insert(path.clone(), ids);
        }
        self.cache[&path]
            .get(cut)
            .copied()
            .context("Siglus PATNO missing")
    }
}
