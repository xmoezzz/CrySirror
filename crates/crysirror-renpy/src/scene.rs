// SPDX-License-Identifier: MPL-2.0
//! Logical Ren'Py image tags. GPU and texture ownership stay with the host.
use crysirror_python::Value;
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub struct SceneLists {
    pub layers: BTreeMap<String, Vec<Image>>,
}

#[derive(Clone)]
pub struct Image {
    pub tag: String,
    pub name: Vec<String>,
    pub value: Value,
    pub zorder: i64,
    pub(crate) transforms: Vec<crate::atl_runtime::Instance>,
}

impl SceneLists {
    pub fn clone_for_new_context(&self) -> Self {
        self.clone()
    }

    pub fn show(&mut self, layer: String, image: Image, behind: &[String]) {
        let images = self.layers.entry(layer).or_default();
        images.retain(|current| current.tag != image.tag);
        let position = images
            .iter()
            .position(|current| {
                current.zorder > image.zorder
                    || current.zorder == image.zorder && behind.contains(&current.tag)
            })
            .unwrap_or(images.len());
        images.insert(position, image);
    }
}
