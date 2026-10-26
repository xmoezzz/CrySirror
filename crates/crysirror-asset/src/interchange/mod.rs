// SPDX-License-Identifier: MPL-2.0
//! Interchange importers: FBX (binary) and MikuMikuDance PMX are converted
//! into an in-memory GLB, recognised by content rather than file name, so
//! every glTF consumer in the engine accepts them. VRM is glTF already.
//!
//! What a source format carries that glTF and the engine cannot represent is
//! listed in the GLB's `extras` and surfaces in `ImportReport::dropped`.
pub mod fbx;
pub mod glb;
pub mod pmx;

use anyhow::Result;
use std::{borrow::Cow, collections::BTreeMap};

pub const DROPPED_EXTRA: &str = "crysirror_import_dropped";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Gltf,
    FbxBinary,
    FbxAscii,
    Pmx,
}

pub fn sniff(bytes: &[u8]) -> Format {
    if bytes.starts_with(b"Kaydara FBX Binary  \0") {
        Format::FbxBinary
    } else if bytes.starts_with(b"PMX ") {
        Format::Pmx
    } else if bytes.len() < 4096 && bytes.windows(13).any(|w| w == b"FBXHeaderExte")
        || bytes.starts_with(b"; FBX ")
    {
        Format::FbxAscii
    } else {
        Format::Gltf
    }
}

/// Return glTF bytes for any supported model format. glTF/GLB/VRM input is
/// borrowed unchanged. `external` resolves texture paths relative to the
/// model, through the caller's scoped storage.
pub fn to_gltf<'a>(
    bytes: &'a [u8],
    external: &mut dyn FnMut(&str) -> Result<Vec<u8>>,
) -> Result<Cow<'a, [u8]>> {
    match sniff(bytes) {
        Format::Gltf => Ok(Cow::Borrowed(bytes)),
        Format::FbxBinary => Ok(Cow::Owned(fbx::convert(bytes, external)?.to_glb()?)),
        Format::Pmx => Ok(Cow::Owned(pmx::convert(bytes, external)?.to_glb()?)),
        Format::FbxAscii => {
            anyhow::bail!("ASCII FBX is not supported; re-export as binary FBX or glTF")
        }
    }
}

/// Features recorded by a converter in the GLB `extras`.
pub fn dropped_from(doc: &gltf::Document) -> Vec<String> {
    let Some(raw) = doc.as_json().extras.as_ref() else {
        return vec![];
    };
    serde_json::from_str::<serde_json::Value>(raw.get())
        .ok()
        .and_then(|v| v.get(DROPPED_EXTRA).cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Loads, normalises and embeds source textures once per path.
pub struct Textures<'e> {
    external: &'e mut dyn FnMut(&str) -> Result<Vec<u8>>,
    cache: BTreeMap<String, Option<usize>>,
}

impl<'e> Textures<'e> {
    pub fn new(external: &'e mut dyn FnMut(&str) -> Result<Vec<u8>>) -> Self {
        Self {
            external,
            cache: BTreeMap::new(),
        }
    }

    /// Embed `path` (or inline `content`) into `doc`, returning its image
    /// index. Missing or undecodable textures are reported as dropped.
    pub fn embed(
        &mut self,
        doc: &mut glb::Document,
        path: &str,
        content: Option<&[u8]>,
    ) -> Option<usize> {
        let path = path.replace('\\', "/");
        if let Some(hit) = self.cache.get(&path) {
            return *hit;
        }
        let base = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let bytes = match content {
            Some(c) if !c.is_empty() => Ok(c.to_vec()),
            _ => {
                let relative = path.trim_start_matches("./");
                let absolute = relative.starts_with('/') || relative.get(1..2) == Some(":");
                let first = if absolute {
                    Err(anyhow::anyhow!("absolute path"))
                } else {
                    (self.external)(relative)
                };
                first.or_else(|_| (self.external)(&base))
            }
        };
        let result = bytes.and_then(|b| encode(&base, b)).map(|(data, mime)| {
            doc.images.push((data, mime));
            doc.images.len() - 1
        });
        let index = match result {
            Ok(i) => Some(i),
            Err(e) => {
                doc.dropped.push(format!("texture `{path}`: {e:#}"));
                None
            }
        };
        self.cache.insert(path, index);
        index
    }

    /// True when any texel of an embedded image is not fully opaque.
    pub fn has_alpha(doc: &glb::Document, image: usize) -> bool {
        let Some((bytes, _)) = doc.images.get(image) else {
            return false;
        };
        image::load_from_memory(bytes)
            .map(|i| i.color().has_alpha() && i.to_rgba8().pixels().any(|p| p[3] < 255))
            .unwrap_or(false)
    }
}

/// glTF images must be PNG or JPEG. Other formats go through the engine's
/// shared decoder registry and are re-encoded losslessly as PNG.
fn encode(name: &str, bytes: Vec<u8>) -> Result<(Vec<u8>, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok((bytes, "image/png"));
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok((bytes, "image/jpeg"));
    }
    let surface =
        crysirror_image::DecoderRegistry::default().decode_with(name, &bytes, None, None, None)?;
    let rgba = image::RgbaImage::from_raw(surface.width, surface.height, surface.rgba)
        .ok_or_else(|| anyhow::anyhow!("decoded image size mismatch"))?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(rgba).write_to(
        &mut std::io::Cursor::new(&mut png),
        image::ImageOutputFormat::Png,
    )?;
    Ok((png, "image/png"))
}
