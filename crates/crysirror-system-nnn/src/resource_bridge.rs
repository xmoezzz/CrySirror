// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Reviewed native resource precedence over shared archive/file decoders.
//! All routines run on admitted workers; no VM parser/codec or ambient path.
use crate::{
    native_audio::{NativeAudioLookup, NativeSource},
    native_resources::{ImageLookup, PackedMember, ResourceNames},
};
use anyhow::{bail, ensure, Context, Result};
use crysirror_core::{
    image_processing::ImageInput,
    service::{AssetReader, EncodedAudio},
    state::ClientId,
};
use crysirror_resource::{read_all, DecodeContext, FileRegistry, FormatRegistry};
use crysirror_tasks::Cancellation;
use std::sync::Arc;
pub type SourceOpener =
    Arc<dyn Fn(&str, &Cancellation) -> Result<Option<Arc<dyn AssetReader>>> + Send + Sync>;
#[derive(Clone, Debug)]
pub enum ResolvedAudio {
    Asset(crysirror_core::path::VfsPath),
    Encoded(EncodedAudio),
}
#[derive(Clone, Debug)]
pub struct NativeImageSource {
    pub input: ImageInput,
    pub has_mask: bool,
}
#[derive(Clone)]
pub struct NativeResources {
    client: ClientId,
    open: SourceOpener,
    archives: Arc<FormatRegistry>,
    files: Arc<FileRegistry>,
}
impl NativeResources {
    pub fn blackcyc(client: ClientId, open: SourceOpener) -> Result<Self> {
        ensure!(client.0 != 0, "NNN resource client identity");
        let mut archives = FormatRegistry::default();
        crysirror_formats_blackcyc::register_archives(&mut archives)?;
        let mut files = FileRegistry::default();
        crysirror_formats_blackcyc::register_audio(&mut files)?;
        Ok(Self {
            client,
            open,
            archives: Arc::new(archives),
            files: Arc::new(files),
        })
    }
    fn context(&self, path: &str, cancel: &Cancellation) -> DecodeContext {
        let mut cx = DecodeContext::new(self.client).with_cancellation(cancel.clone());
        cx.source_name = Some(path.to_owned());
        let open = self.open.clone();
        let token = cancel.clone();
        cx.resolver = Some(Arc::new(move |path: &str| {
            token.check()?;
            open(path, &token)?.with_context(|| format!("NNN companion absent: {path}"))
        }));
        cx
    }
    fn packed(
        &self,
        member: &PackedMember,
        kind: &str,
        cancel: &Cancellation,
    ) -> Result<Option<Arc<dyn AssetReader>>> {
        cancel.check()?;
        let Some(body) = (self.open)(&member.archive, cancel)? else {
            return Ok(None);
        };
        let cx = self.context(&member.archive, cancel);
        let (_, archive) = self.archives.open(body, Some(kind), &cx)?;
        let Some(entry) = archive
            .entries()
            .iter()
            .find(|entry| entry.path.eq_ignore_ascii_case(&member.member))
        else {
            return Ok(None);
        };
        Ok(Some(archive.open(entry.id, &cx)?))
    }
    pub fn image(&self, lookup: &ImageLookup, cancel: &Cancellation) -> Result<ImageInput> {
        Ok(self.image_source(lookup, cancel)?.input)
    }
    pub fn image_source(
        &self,
        lookup: &ImageLookup,
        cancel: &Cancellation,
    ) -> Result<NativeImageSource> {
        cancel.check()?;
        let mut selected = None;
        for member in &lookup.packed {
            let Some(table) = (self.open)(&member.table, cancel)? else {
                continue;
            };
            let cx = self.context(&member.archive, cancel);
            let table = read_all(table.as_ref(), cx.limits.max_index_size)
                .with_context(|| format!("NNN image directory read {}", member.table))?;
            let directory = crysirror_formats_blackcyc::GpkIndex::parse(&table, &cx)
                .with_context(|| format!("NNN image directory {} bytes={}", member.table, table.len()))?;
            if directory.find(&member.member).is_none() {
                continue;
            }
            // Original highest matching directory commits precedence. If its
            // GPK is absent, fall back to loose, never search an older pack.
            selected = self
                .packed(member, "black-cyc/gpk", cancel)?
                .map(|reader| (member.member.clone(), reader));
            break;
        }
        let (name, reader) = match selected {
            Some(found) => found,
            None => (
                lookup.loose.clone(),
                (self.open)(&lookup.loose, cancel)?
                    .with_context(|| format!("NNN image resource absent: {}", lookup.loose))?,
            ),
        };
        let bytes = read_all(reader.as_ref(), 64 * 1024 * 1024)?;
        cancel.check()?;
        let info = crysirror_formats_blackcyc::Dwq::metadata(&bytes)?;
        let has_mask = match info.pack {
            2 | 3 | 7 => {
                ensure!(
                    info.pack != 2 || info.alpha_suffix,
                    "NNN legacy pack2 without A needs explicit shared mask decoding profile"
                );
                true
            }
            8 => {
                if !matches!(info.png_color_type, Some(4 | 6)) {
                    ensure!(
                        !info.png_transparency,
                        "NNN legacy indexed/RGB PNG transparency requires explicit profile"
                    );
                }
                matches!(info.png_color_type, Some(4 | 6))
            }
            _ => false,
        };
        Ok(NativeImageSource {
            has_mask,
            input: ImageInput::Encoded {
                name,
                bytes: bytes.into(),
                size: info.size,
            },
        })
    }
    pub fn audio(
        &self,
        source: &NativeSource,
        names: &ResourceNames,
        cancel: &Cancellation,
    ) -> Result<ResolvedAudio> {
        cancel.check()?;
        let (path, reader) = match source.lookup(names)? {
            NativeAudioLookup::Music(lookup) => {
                if (self.open)(&lookup.control, cancel)?.is_some() {
                    bail!(
                        "NNN native MML playlist requires original segment/loop policy: {}",
                        lookup.control
                    )
                }
                let reader = (self.open)(&lookup.audio, cancel)?
                    .with_context(|| format!("NNN music resource absent: {}", lookup.audio))?;
                (lookup.audio, reader)
            }
            NativeAudioLookup::Sound { loose } => {
                let reader = (self.open)(&loose, cancel)?
                    .with_context(|| format!("NNN sound resource absent: {loose}"))?;
                (loose, reader)
            }
            NativeAudioLookup::Voice { packed, loose } => {
                if let Some(reader) = self.packed(&packed, "black-cyc/vpk", cancel)? {
                    (packed.member, reader)
                } else {
                    let reader = (self.open)(&loose, cancel)?
                        .with_context(|| format!("NNN voice resource absent: {loose}"))?;
                    (loose, reader)
                }
            }
        };
        let cx = self.context(&path, cancel);
        let (_, decoded) = self
            .files
            .decode(&path, reader, Some("black-cyc/vaw"), &cx)?;
        ensure!(
            matches!(decoded.extension.as_str(), "wav" | "ogg"),
            "NNN shared audio wrapper output profile"
        );
        let bytes = read_all(decoded.reader.as_ref(), 64 * 1024 * 1024)?;
        cancel.check()?;
        Ok(ResolvedAudio::Encoded(EncodedAudio {
            bytes: bytes.into(),
            frame_limit: None,
        }))
    }
}
