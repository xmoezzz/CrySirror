// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! One shared CPU image pipeline for native adapters and public bitmap jobs.
use crysirror_core::{image_processing::*, service::ClientStorage, state::ClientId, Error, Result};
use crysirror_image::{effects::ImageEffect, DecoderRegistry, ImageSurface};
use crysirror_tasks::{Task, TaskPool};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
const BUDGET: usize = 512 * 1024 * 1024;
struct Charge {
    memory: Arc<AtomicUsize>,
    bytes: usize,
}
impl Charge {
    fn new(memory: Arc<AtomicUsize>, bytes: usize) -> Result<Self> {
        memory
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                v.checked_add(bytes).filter(|v| *v <= BUDGET)
            })
            .map_err(|_| Error::Limit("image processing memory budget"))?;
        Ok(Self { memory, bytes })
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.memory.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
struct Resource {
    owner: ClientId,
    size: [u32; 2],
    opaque: bool,
    pixels: Arc<[u8]>,
    _charge: Charge,
}
impl std::fmt::Debug for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageResource")
            .field("client", &self.owner)
            .field("size", &self.size())
            .finish()
    }
}
impl ImageResource for Resource {
    fn client(&self) -> ClientId {
        self.owner
    }
    fn size(&self) -> [u32; 2] {
        self.size
    }
    fn rgba(&self) -> Arc<[u8]> {
        self.pixels.clone()
    }
    fn opaque(&self) -> bool {
        self.opaque
    }
}
struct Job {
    task: Option<Task<Arc<dyn ImageResource>>>,
    state: ImageProcessState,
}
impl ImageProcess for Job {
    fn poll(&mut self) -> Result<ImageProcessState> {
        if let Some(t) = &mut self.task {
            match t.try_take() {
                Ok(None) => (),
                Ok(Some(r)) => {
                    self.task = None;
                    self.state = ImageProcessState::Ready(r);
                }
                Err(e) => {
                    self.task = None;
                    self.state = ImageProcessState::Failed(e.to_string());
                }
            }
        }
        Ok(self.state.clone())
    }
}
pub(crate) fn begin(
    owner: ClientId,
    input: ImageInput,
    effects: Arc<[u8]>,
    storage: Arc<dyn ClientStorage>,
    decoders: DecoderRegistry,
    pool: TaskPool,
    memory: Arc<AtomicUsize>,
) -> Result<ImageJob> {
    if effects.len() > 65536 {
        return Err(Error::Limit("image effect specification"));
    }
    let effects: Vec<ImageEffect> =
        serde_json::from_slice(&effects).map_err(|e| Error::Io(e.to_string()))?;
    if effects.is_empty() || effects.len() > 16 {
        return Err(Error::Limit("image effect chain length"));
    }
    for effect in &effects {
        effect.validate().map_err(|e| Error::Io(e.to_string()))?;
    }
    let size = input_size(owner, &input)?;
    let pair = matches!(
        &input,
        ImageInput::Transition { .. } | ImageInput::Raster { .. }
    );
    let pair_op = |e: &ImageEffect| {
        matches!(
            e,
            ImageEffect::Transition { .. } | ImageEffect::SineRowCopy { .. }
        )
    };
    if effects.iter().skip(1).any(pair_op) || effects.first().is_some_and(pair_op) != pair {
        return Err(Error::Invalid("image input/effect context"));
    }
    if matches!(&input, ImageInput::Transition { .. })
        && !matches!(effects.first(), Some(ImageEffect::Transition { .. }))
    {
        return Err(Error::Invalid("transition input context"));
    }
    if matches!(&input, ImageInput::Raster { .. })
        && !matches!(effects.first(), Some(ImageEffect::SineRowCopy { .. }))
    {
        return Err(Error::Invalid("row-copy input context"));
    }
    let bytes =
        crysirror_image::rgba_len(size[0], size[1]).map_err(|e| Error::Io(e.to_string()))?;
    if u64::from(size[0]) * u64::from(size[1]) > crysirror_image::effects::MAX_EFFECT_PIXELS {
        return Err(Error::Limit("image effect pixel budget"));
    }
    let output = Charge::new(memory.clone(), bytes)?;
    // Gaussian's two f32 planes, two RGBA products and a bounded encoded asset.
    let scratch = Charge::new(
        memory,
        bytes * 16
            + if matches!(input, ImageInput::Asset { .. } | ImageInput::Raster { .. }) {
                128 * 1024 * 1024
            } else {
                0
            },
    )?;
    let permit = pool.reserve().map_err(|e| Error::Io(e.to_string()))?;
    let task = permit
        .spawn(move |cancel| {
            let _scratch = scratch;
            cancel.check()?;
            let (mut image, other, mask) = match input {
                ImageInput::Transition { from, to, mask } => (
                    load(&ImageInput::Resource(from), &storage, &decoders, &cancel)?,
                    Some(load(
                        &ImageInput::Resource(to),
                        &storage,
                        &decoders,
                        &cancel,
                    )?),
                    mask.map(|r| load(&ImageInput::Resource(r), &storage, &decoders, &cancel))
                        .transpose()?,
                ),
                ImageInput::Raster {
                    destination,
                    source,
                } => (
                    load(&destination, &storage, &decoders, &cancel)?,
                    Some(load(&source, &storage, &decoders, &cancel)?),
                    None,
                ),
                input => (load(&input, &storage, &decoders, &cancel)?, None, None),
            };
            for effect in effects {
                cancel.check()?;
                image = if let ImageEffect::SineRowCopy { spec } = &effect {
                    image.sine_copy_from(
                        other
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("row-copy source absent"))?,
                        spec,
                        &|| cancel.check(),
                    )?
                } else if let ImageEffect::Transition { spec } = &effect {
                    image.transition_to(
                        other
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("transition source absent"))?,
                        mask.as_ref(),
                        spec,
                        &|| cancel.check(),
                    )?
                } else {
                    image.apply_effect_cancellable(&effect, &|| cancel.check())?
                };
            }
            let opaque = image.rgba.chunks_exact(4).all(|p| p[3] == 255);
            let pixels = Arc::from(std::mem::take(&mut image.rgba));
            Ok(Arc::new(Resource {
                owner,
                size: [image.width, image.height],
                opaque,
                pixels,
                _charge: output,
            }) as Arc<dyn ImageResource>)
        })
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(Box::new(Job {
        task: Some(task),
        state: ImageProcessState::Pending,
    }))
}

fn input_size(owner: ClientId, input: &ImageInput) -> Result<[u32; 2]> {
    let size = match input {
        ImageInput::Asset { path, size, .. } => {
            crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, path)?;
            *size
        }
        ImageInput::Solid { size, .. } => *size,
        ImageInput::Resource(r) => {
            if r.client() != owner {
                return Err(Error::Invalid("foreign image resource"));
            }
            r.size()
        }
        ImageInput::Transition { from, to, mask } => {
            let size = from.size();
            for r in std::iter::once(from)
                .chain(std::iter::once(to))
                .chain(mask.iter())
            {
                if r.client() != owner {
                    return Err(Error::Invalid("foreign transition resource"));
                }
                if r.size() != size {
                    return Err(Error::Invalid("transition dimensions"));
                }
            }
            size
        }
        ImageInput::Raster {
            destination,
            source,
        } => {
            if [destination.as_ref(), source.as_ref()]
                .iter()
                .any(|v| matches!(v, ImageInput::Raster { .. } | ImageInput::Transition { .. }))
            {
                return Err(Error::Invalid("nested image input"));
            }
            let size = input_size(owner, destination)?;
            input_size(owner, source)?;
            size
        }
    };
    crysirror_image::rgba_len(size[0], size[1]).map_err(|e| Error::Io(e.to_string()))?;
    if u64::from(size[0]) * u64::from(size[1]) > crysirror_image::effects::MAX_EFFECT_PIXELS {
        return Err(Error::Limit("image effect pixel budget"));
    }
    Ok(size)
}
fn load(
    input: &ImageInput,
    storage: &Arc<dyn ClientStorage>,
    decoders: &DecoderRegistry,
    cancel: &crysirror_tasks::Cancellation,
) -> anyhow::Result<ImageSurface> {
    cancel.check()?;
    match input {
        ImageInput::Solid { size, color } => ImageSurface::filled(size[0], size[1], *color),
        ImageInput::Resource(r) => {
            let size = r.size();
            let pixels = r.rgba();
            anyhow::ensure!(
                pixels.len() == crysirror_image::rgba_len(size[0], size[1])?,
                "invalid image resource pixels"
            );
            Ok(ImageSurface {
                width: size[0],
                height: size[1],
                rgba: pixels.to_vec(),
            })
        }
        ImageInput::Asset {
            path,
            size,
            fingerprint,
        } => {
            use crysirror_core::path::{Mount, VfsPath};
            let path = VfsPath::new(Mount::Asset, path).map_err(|e| anyhow::anyhow!("{e}"))?;
            let reader = storage
                .open_asset(&path)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let n = reader.len().map_err(|e| anyhow::anyhow!("{e}"))?;
            anyhow::ensure!(n <= 64 * 1024 * 1024, "image encoded byte budget");
            let mut bytes = vec![0; n as usize];
            let mut at = 0;
            while at < bytes.len() {
                cancel.check()?;
                let end = (at + 65536).min(bytes.len());
                let n = reader
                    .read_at(at as u64, &mut bytes[at..end])
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                anyhow::ensure!(n > 0 && n <= end - at, "truncated image asset");
                at += n;
            }
            if let Some(hash) = fingerprint {
                anyhow::ensure!(
                    blake3::hash(&bytes).as_bytes() == hash,
                    "image asset changed since native load"
                );
            }
            let image = decoders.decode(path.relative(), &bytes)?;
            anyhow::ensure!(
                [image.width, image.height] == *size,
                "image dimensions changed during processing"
            );
            Ok(image)
        }
        _ => anyhow::bail!("nested image input is not supported"),
    }
}
