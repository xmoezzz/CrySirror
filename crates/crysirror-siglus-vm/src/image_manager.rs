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
    pub fn load_g00(&mut self, name: &str, cut: usize) -> Result<ImageHandle> {
        let path = if Path::new(name).extension().is_some() {
            format!("asset/{name}")
        } else {
            format!("asset/{name}.g00")
        };
        if !self.cache.contains_key(&path) {
            let bytes = crate::resource::read_file_bytes(Path::new(&path))?;
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
