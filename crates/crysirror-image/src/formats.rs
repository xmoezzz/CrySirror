// SPDX-License-Identifier: MPL-2.0
//! Content-recognized CPU image plugins, with scoped dependency reads.
use crate::{DecoderRegistry, ImageSurface, MAX_DECODED_BYTES};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    Error,
};
use crysirror_resource::{ArchiveEntry, DecodeContext};
use std::{cell::Cell, sync::Arc};

/// The host binds this callback to one client's capability storage.
pub type ImageSource<'a> = dyn Fn(&str) -> crysirror_core::Result<Vec<u8>> + 'a;
pub trait ImageFormat: Send + Sync {
    fn id(&self) -> &'static str;
    /// A specific native wrapper outranks its generic embedded image codec.
    fn priority(&self) -> u8 {
        50
    }
    fn priority_for(&self, _path: &str, _bytes: &[u8]) -> u8 {
        self.priority()
    }
    fn recognize(&self, path: &str, bytes: &[u8]) -> bool;
    fn decode(&self, bytes: &[u8], context: &ImageDecodeContext<'_>) -> Result<ImageSurface>;
}
struct Work {
    encoded: Cell<usize>,
    decoded: Cell<usize>,
}
pub struct ImageDecodeContext<'a> {
    pub path: &'a str,
    registry: &'a DecoderRegistry,
    source: Option<&'a ImageSource<'a>>,
    resource: Option<&'a DecodeContext>,
    depth: u8,
    work: Option<&'a Work>,
}
impl<'a> ImageDecodeContext<'a> {
    pub fn new(path: &'a str, registry: &'a DecoderRegistry) -> Self {
        Self {
            path,
            registry,
            source: None,
            resource: None,
            depth: 0,
            work: None,
        }
    }
    pub fn with_source(mut self, source: &'a ImageSource<'a>) -> Self {
        self.source = Some(source);
        self
    }
    pub fn with_resources(mut self, resources: &'a DecodeContext) -> Self {
        self.resource = Some(resources);
        self
    }
    pub fn key(&self, id: &str) -> Result<crysirror_resource::KeyMaterial> {
        let resources = self
            .resource
            .context("encrypted image needs a client-scoped key provider")?;
        resources.keys.key(resources.client, id)
    }
    fn relative(&self, name: &str) -> Result<String> {
        let native = name.replace('\\', "/");
        VfsPath::new(Mount::Asset, &native)
            .map_err(|e| anyhow::anyhow!("image dependency path: {e}"))?;
        let prefix = self.path.rsplit_once('/').map_or("", |(prefix, _)| prefix);
        let full = if prefix.is_empty() {
            native
        } else {
            format!("{prefix}/{native}")
        };
        Ok(VfsPath::new(Mount::Asset, &full)
            .map_err(|e| anyhow::anyhow!("image dependency path: {e}"))?
            .relative()
            .into())
    }
    pub fn try_read_relative(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = self.relative(name)?;
        let source = self
            .source
            .context("image needs a client-scoped dependency resolver")?;
        match source(&path) {
            Ok(bytes) => {
                ensure!(
                    bytes.len() <= MAX_DECODED_BYTES,
                    "image dependency byte limit"
                );
                Ok(Some(bytes))
            }
            Err(Error::NotFound) => Ok(None),
            Err(e) => bail!("image dependency {path}: {e}"),
        }
    }
    pub fn read_relative(&self, name: &str) -> Result<Vec<u8>> {
        self.try_read_relative(name)?
            .context("image dependency not found")
    }
    pub fn decode_relative(&self, name: &str) -> Result<ImageSurface> {
        let path = self.relative(name)?;
        let bytes = self.read_relative(name)?;
        self.child(&path, &bytes)
    }
    pub fn decode_embedded(&self, path_hint: &str, bytes: &[u8]) -> Result<ImageSurface> {
        let path = self.relative(path_hint)?;
        self.child(&path, bytes)
    }
    fn child(&self, path: &str, bytes: &[u8]) -> Result<ImageSurface> {
        ensure!(self.depth < 16, "nested image decode depth limit");
        let context = ImageDecodeContext {
            path,
            registry: self.registry,
            source: self.source,
            resource: self.resource,
            depth: self.depth + 1,
            work: self.work,
        };
        self.registry.decode_context(bytes, None, &context)
    }
    /// Resolve an explicit format/path transform profile through the common
    /// resource service. A cipher name alone does not implement a native cipher.
    pub fn transform(
        &self,
        format: &str,
        bytes: &[u8],
        metadata: &serde_json::Value,
        required: bool,
    ) -> Result<Vec<u8>> {
        let Some(resource) = self.resource else {
            ensure!(
                !required,
                "encrypted image needs an explicit transform/key profile"
            );
            return Ok(bytes.to_vec());
        };
        let entry = ArchiveEntry {
            id: 0,
            path: self.path.into(),
            size: bytes.len() as u64,
            stored_size: bytes.len() as u64,
            metadata: metadata.clone(),
        };
        let source = resource.apply_pipeline(
            format,
            &entry,
            crysirror_resource::io::memory(bytes.to_vec()),
            required,
        )?;
        crysirror_resource::read_all(source.as_ref(), MAX_DECODED_BYTES as u64)
    }
}
impl DecoderRegistry {
    pub fn register_format(&mut self, format: Arc<dyn ImageFormat>) -> Result<()> {
        let id = format.id();
        ensure!(
            !id.is_empty() && id.len() <= 128 && !self.formats.contains_key(id),
            "invalid or duplicate image format ID"
        );
        self.formats.insert(id, format);
        Ok(())
    }
    pub fn format_ids(&self) -> Vec<&'static str> {
        self.formats.keys().copied().collect()
    }
    pub fn extensions(&self) -> Vec<&str> {
        self.decoders.keys().map(String::as_str).collect()
    }
    pub fn probe(&self, path: &str, bytes: &[u8]) -> Result<Vec<&'static str>> {
        ensure!(bytes.len() <= MAX_DECODED_BYTES, "encoded image byte limit");
        Ok(self
            .formats
            .iter()
            .filter_map(|(&id, decoder)| decoder.recognize(path, bytes).then_some(id))
            .collect())
    }
    pub fn decode_with(
        &self,
        path: &str,
        bytes: &[u8],
        source: Option<&ImageSource<'_>>,
        resource: Option<&DecodeContext>,
        format: Option<&str>,
    ) -> Result<ImageSurface> {
        let work = Work {
            encoded: Cell::new(0),
            decoded: Cell::new(0),
        };
        let context = ImageDecodeContext {
            path,
            registry: self,
            source,
            resource,
            depth: 0,
            work: Some(&work),
        };
        self.decode_context(bytes, format, &context)
    }
    fn decode_context(
        &self,
        bytes: &[u8],
        format: Option<&str>,
        context: &ImageDecodeContext<'_>,
    ) -> Result<ImageSurface> {
        ensure!(bytes.len() <= MAX_DECODED_BYTES, "encoded image byte limit");
        if let Some(work) = context.work {
            let n = work
                .encoded
                .get()
                .checked_add(bytes.len())
                .context("image dependency byte overflow")?;
            ensure!(
                n <= 2 * MAX_DECODED_BYTES,
                "cumulative image dependency byte limit"
            );
            work.encoded.set(n);
        }
        let decoder = if let Some(id) = format {
            Some(
                self.formats
                    .get(id)
                    .context("image format not registered")?,
            )
        } else {
            let mut matches = self.probe(context.path, bytes)?;
            if let Some(priority) = matches
                .iter()
                .map(|id| self.formats[id].priority_for(context.path, bytes))
                .max()
            {
                matches.retain(|id| self.formats[id].priority_for(context.path, bytes) == priority);
            }
            ensure!(
                matches.len() <= 1,
                "ambiguous image formats {matches:?}; specify a format ID"
            );
            matches.first().map(|id| &self.formats[id])
        };
        let surface = if let Some(decoder) = decoder {
            ensure!(
                decoder.recognize(context.path, bytes),
                "image does not match selected format {}",
                decoder.id()
            );
            decoder
                .decode(bytes, context)
                .with_context(|| format!("decode {} as {}", context.path, decoder.id()))?
        } else {
            let extension = context
                .path
                .rsplit_once('.')
                .map(|(_, e)| e.to_ascii_lowercase())
                .unwrap_or_default();
            self.decoders
                .get(&extension)
                .with_context(|| format!("unsupported image format: {extension}"))?
                .decode(bytes)
                .with_context(|| format!("decode {}", context.path))?
        };
        surface.validate()?;
        if let Some(work) = context.work {
            let n = work
                .decoded
                .get()
                .checked_add(surface.rgba.len())
                .context("decoded image work overflow")?;
            ensure!(
                n <= 2 * MAX_DECODED_BYTES,
                "cumulative decoded image byte limit"
            );
            work.decoded.set(n);
        }
        Ok(surface)
    }
}
