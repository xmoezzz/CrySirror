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
/// Resource decoding/composition has a separate bounded budget from expensive
/// image effects. Native sprite sheets routinely exceed a4MP effect target.
const MAX_RESOURCE_PIXELS: u64 = 16 * 1024 * 1024;
pub(crate) struct Charge {
    memory: Arc<AtomicUsize>,
    bytes: usize,
}
impl Charge {
    pub(crate) fn new(memory: Arc<AtomicUsize>, bytes: usize) -> Result<Self> {
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
    // An empty chain is resource preparation: decoding/ownership/publication
    // still follows this worker's normal reservation and cancellation fence.
    // Pair inputs retain their mandatory operation/context checks below.
    if effects.len() > 16 {
        return Err(Error::Limit("image effect chain length"));
    }
    for effect in &effects {
        effect.validate().map_err(|e| Error::Io(e.to_string()))?;
    }
    let mut graph_nodes = 0;
    let mut graph_bytes = 0;
    let size = input_size(owner, &input, 0, &mut graph_nodes, &mut graph_bytes)?;
    if !effects.is_empty() && u64::from(size[0])*u64::from(size[1]) > crysirror_image::effects::MAX_EFFECT_PIXELS {
        return Err(Error::Limit("image effect pixel budget"));
    }
    let pair = matches!(
        &input,
        ImageInput::Transition { .. } | ImageInput::Raster { .. }
    );
    let pair_op = |e: &ImageEffect| {
        matches!(
            e,
            ImageEffect::Transition { .. } | ImageEffect::SineRowCopy { .. } | ImageEffect::AlphaClipRect {..}
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
        && !matches!(effects.first(), Some(ImageEffect::SineRowCopy { .. } | ImageEffect::AlphaClipRect {..}))
    {
        return Err(Error::Invalid("row-copy input context"));
    }
    let bytes =
        crysirror_image::rgba_len(size[0], size[1]).map_err(|e| Error::Io(e.to_string()))?;
    let output = Charge::new(memory.clone(), bytes)?;
    // Effect jobs reserve Gaussian planes/products. Decode-only jobs reserve
    // decoded products and bounded encoded input, without nonexistent blur planes.
    let scratch_factor = if effects.is_empty() { 3 } else { 16 };
    let scratch = Charge::new(
        memory.clone(),
        bytes * scratch_factor + graph_bytes
            + if matches!(input,ImageInput::Raster{..}|ImageInput::Composite{..}) && has_encoded_dependencies(&input) {
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
            let load_input=|input:&ImageInput|load_inner(owner,input,&storage,&decoders,&cancel,Some(&memory));
            let (mut image, other, mask) = match input {
                ImageInput::Transition { from, to, mask } => (
                    load_input(&ImageInput::Resource(from))?,
                    Some(load_input(&ImageInput::Resource(to))?),
                    mask.map(|r| load_input(&ImageInput::Resource(r)))
                        .transpose()?,
                ),
                ImageInput::Raster {
                    destination,
                    source,
                } => (
                    load_input(&destination)?,
                    Some(load_input(&source)?),
                    None,
                ),
                input => (load_input(&input)?, None, None),
            };
            for effect in effects {
                cancel.check()?;
                image = if let ImageEffect::AlphaClipRect {spec}=&effect {
                    spec.apply(&image,other.as_ref().ok_or_else(||anyhow::anyhow!("alpha clip source absent"))?,&||cancel.check())?
                } else if let ImageEffect::SineRowCopy { spec } = &effect {
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
            resource_from_surface(owner,image,output,&||cancel.check())
        })
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(Box::new(Job {
        task: Some(task),
        state: ImageProcessState::Pending,
    }))
}

// Composition graphs with encoded plugin formats may request auxiliary sources
// during decode. Keep their existing dependency reservation. Asset inputs
// use bounded streams and reserve their actual encoded length on the worker.
fn has_encoded_dependencies(input:&ImageInput)->bool {
    match input {
        ImageInput::Encoded{..}=>true,
        ImageInput::Composite{base,layers}=>has_encoded_dependencies(base)
            || layers.iter().any(|l|has_encoded_dependencies(&l.input)),
        ImageInput::Raster{destination,source}=>has_encoded_dependencies(destination)||has_encoded_dependencies(source),
        _=>false,
    }
}

pub(crate) fn input_size(owner: ClientId, input: &ImageInput, depth: usize, nodes: &mut usize, bytes: &mut usize) -> Result<[u32; 2]> {
    *nodes += 1;
    if depth > 32 || *nodes > 256 { return Err(Error::Limit("image composition graph")); }

    let size = match input {
        ImageInput::Encoded{name,bytes:encoded,size}=>{
            crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset,name)?;
            if encoded.is_empty()||encoded.len()>64*1024*1024{return Err(Error::Limit("image encoded byte budget"));}
            *bytes=bytes.checked_add(encoded.len()).filter(|n|*n<=128*1024*1024).ok_or(Error::Limit("image composition byte budget"))?;
            *size
        }
        ImageInput::Composite { base, layers } => {
            let size = input_size(owner, base, depth + 1, nodes, bytes)?;
            for layer in layers {
                if matches!(layer.math,ImageCompositeMath::MaskSeparate256)&&layer.opacity!=255{return Err(Error::Invalid("mask_separate256 requires opacity255"));}
                if let ImageCompositeMath::MaskPiecewise256{weight}=layer.math{
                    if weight>256||layer.opacity!=255{return Err(Error::Invalid("piecewise mask weight/opacity"));}
                }
                if layer.source[2] < 0 || layer.source[3] < 0
                    || layer.source.iter().chain(&layer.destination).any(|v| v.unsigned_abs() > 1_000_000) {
                    return Err(Error::Invalid("image composite coordinates"));
                }
                if layer.clip.is_some_and(|clip| clip[2] < 0 || clip[3] < 0
                    || clip.iter().any(|v| v.unsigned_abs() > 1_000_000)) {
                    return Err(Error::Invalid("image composite clip"));
                }
                if matches!(layer.input.as_ref(), ImageInput::Raster { .. } | ImageInput::Transition { .. }) {
                    return Err(Error::Invalid("composite pair-effect input"));
                }
                input_size(owner, &layer.input, depth + 1, nodes, bytes)?;
            }
            if matches!(base.as_ref(), ImageInput::Raster { .. } | ImageInput::Transition { .. }) {
                return Err(Error::Invalid("composite pair-effect base"));
            }
            size
        }
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
            let size = input_size(owner, destination, depth + 1, nodes, bytes)?;
            input_size(owner, source, depth + 1, nodes, bytes)?;
            size
        }
    };
    *bytes = bytes.checked_add(crysirror_image::rgba_len(size[0], size[1]).map_err(|e| Error::Io(e.to_string()))?)
        .filter(|n| *n <= 128 * 1024 * 1024).ok_or(Error::Limit("image composition byte budget"))?;
    if u64::from(size[0]) * u64::from(size[1]) > MAX_RESOURCE_PIXELS {
        return Err(Error::Limit("image resource pixel budget"));
    }
    Ok(size)
}
pub(crate) fn load(
    owner:ClientId,
    input: &ImageInput,
    storage: &Arc<dyn ClientStorage>,
    decoders: &DecoderRegistry,
    cancel: &crysirror_tasks::Cancellation,
) -> anyhow::Result<ImageSurface> {
    load_inner(owner,input,storage,decoders,cancel,None)
}

fn load_inner(
    owner:ClientId,
    input:&ImageInput,
    storage:&Arc<dyn ClientStorage>,
    decoders:&DecoderRegistry,
    cancel:&crysirror_tasks::Cancellation,
    encoded_memory:Option<&Arc<AtomicUsize>>,
)->anyhow::Result<ImageSurface>{
    cancel.check()?;
    match input {
        ImageInput::Encoded{name,bytes,size}=>{
            cancel.check()?;
            let resources=crysirror_resource::DecodeContext::new(owner).with_cancellation(cancel.clone());
            let source=|path:&str| { let path=crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset,path)?;storage.read(&path) };
            let image=decoders.decode_with(name,bytes,Some(&source),Some(&resources),None)?;
            cancel.check()?;
            anyhow::ensure!([image.width,image.height]==*size,"encoded image dimensions changed during processing");
            Ok(image)
        }
        ImageInput::Composite { base, layers } => {
            use crysirror_image::bitmap::{BitmapRect, BlitOptions, PixelMath};
            let mut image = load_inner(owner,base, storage, decoders, cancel,encoded_memory)?;
            for layer in layers {
                cancel.check()?;
                let (mut rect, mut destination) = (layer.source, layer.destination);
                if let Some([x, y, width, height]) = layer.clip {
                    let left = destination[0].max(x);
                    let top = destination[1].max(y);
                    let right = (destination[0] + rect[2]).min(x + width);
                    let bottom = (destination[1] + rect[3]).min(y + height);
                    if right <= left || bottom <= top { continue; }
                    rect[0] += left - destination[0];
                    rect[1] += top - destination[1];
                    rect[2] = right - left;
                    rect[3] = bottom - top;
                    destination = [left, top];
                }
                let source = load_inner(owner,&layer.input, storage, decoders, cancel,encoded_memory)?;
                let mode = match layer.math {
                    ImageCompositeMath::Copy => PixelMath::Copy,
                    ImageCompositeMath::CopyRgb => PixelMath::CopyRgb,
                    ImageCompositeMath::MaskSeparate256 => PixelMath::MaskSeparate256,
                    ImageCompositeMath::OpaqueScalar256 => PixelMath::OpaqueScalar256,
                    ImageCompositeMath::MaskScalar256 => PixelMath::MaskScalar256,
                    ImageCompositeMath::MaskHoldAlphaScalar256 => PixelMath::MaskHoldAlphaScalar256,
                    ImageCompositeMath::MaskPiecewise256{weight}=>PixelMath::MaskPiecewise256{weight},
                    ImageCompositeMath::Ps { blend, hold_alpha } => {
                        use crysirror_core::image_processing::PsBlend as C;
                        use crysirror_image::bitmap::PsBlend as I;
                        let blend = match blend {
                            C::Normal => I::Normal,
                            C::Overlay => I::Overlay,
                            C::HardLight => I::HardLight,
                            C::SoftLight => I::SoftLight,
                            C::ColorDodge => I::ColorDodge,
                            C::ColorBurn => I::ColorBurn,
                            C::Lighten => I::Lighten,
                            C::Darken => I::Darken,
                        };
                        PixelMath::Ps { blend, hold_alpha }
                    }
                    ImageCompositeMath::OpaqueOnAlphaScalar256 => PixelMath::OpaqueOnAlphaScalar256,
                    ImageCompositeMath::AlphaOnAlphaScalar256 => PixelMath::AlphaOnAlphaScalar256,
                };
                // A bounded row is the cancellation granularity; no worker owns
                // native VM state and publication occurs only after all rows.
                for row in 0..rect[3] {
                    if row % 32 == 0 { cancel.check()?; }
                    image.blit(&source, BlitOptions {
                        source_rect: BitmapRect::new(rect[0], rect[1] + row, rect[2] as u32, 1),
                        destination: [destination[0], destination[1] + row],
                        opacity: layer.opacity, mode, color_key: None,
                    })?;
                }
            }
            Ok(image)
        }
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
            let reader = if encoded_memory.is_some() {storage.open_read(&path)} else {storage.open_asset(&path)}
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let n = reader.len().map_err(|e| anyhow::anyhow!("{e}"))?;
            anyhow::ensure!(n <= 64 * 1024 * 1024, "image encoded byte budget");
            let _encoded_charge=encoded_memory.map(|memory|Charge::new(memory.clone(),n as usize)).transpose()?;
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


pub(crate) fn resource_from_surface(owner:ClientId,mut image:ImageSurface,charge:Charge,check:&dyn Fn()->anyhow::Result<()>)->anyhow::Result<Arc<dyn ImageResource>> {
    image.validate()?;
    let mut opaque=true;
    for block in image.rgba.chunks(256*1024) {
        check()?;
        opaque &= block.chunks_exact(4).all(|p|p[3]==255);
    }
    check()?;
    let pixels=Arc::from(std::mem::take(&mut image.rgba));
    Ok(Arc::new(Resource{owner,size:[image.width,image.height],opaque,pixels,_charge:charge}))
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crysirror_core::{path::{Mount,VfsPath},service::{AssetReader,MemoryAssetReader}};
    struct Stream {
        bytes:Arc<[u8]>, reads:Arc<AtomicUsize>, declared:Option<u64>,
    }
    impl AssetReader for Stream {
        fn len(&self)->Result<u64>{Ok(self.declared.unwrap_or(self.bytes.len() as u64))}
        fn read_at(&self,offset:u64,out:&mut[u8])->Result<usize>{
            self.reads.fetch_add(1,Ordering::SeqCst);
            MemoryAssetReader(self.bytes.clone()).read_at(offset,out)
        }
    }
    struct Storage(Arc<Stream>);
    impl ClientStorage for Storage {
        fn read(&self,_:&VfsPath)->Result<Vec<u8>>{panic!("budgeted asset pipeline must not allocate via whole-file fallback")}
        fn open_read(&self,_:&VfsPath)->Result<Arc<dyn AssetReader>>{Ok(self.0.clone())}
        fn write_atomic(&self,_:&VfsPath,_:&[u8])->Result<()>{Err(Error::ReadOnly)}
        fn remove(&self,_:&VfsPath)->Result<()>{Err(Error::ReadOnly)}
        fn list(&self,_:Mount,_:&str)->Result<Vec<String>>{Ok(vec![])}
    }
    fn wait(job:&mut ImageJob)->ImageProcessState {
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {
            let state=job.poll().unwrap();
            if !matches!(state,ImageProcessState::Pending){return state;}
            assert!(std::time::Instant::now()<deadline,"image worker did not complete");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    fn start(memory:Arc<AtomicUsize>,stream:Arc<Stream>,fingerprint:Option<[u8;32]>)->ImageJob{
        begin(ClientId(7),ImageInput::Asset{path:"tiny.png".into(),size:[1,1],fingerprint},
            Arc::from(&b"[]"[..]),Arc::new(Storage(stream)),DecoderRegistry::default(),
            TaskPool::new("image-budget-test",1,8).unwrap(),memory).unwrap()
    }
    #[test]
    fn asset_reserves_actual_encoded_bytes_without_flat_128_mib_penalty(){
        let memory=Arc::new(AtomicUsize::new(0));
        let occupied=Charge::new(memory.clone(),BUDGET-1024*1024).unwrap();
        let png=crysirror_image::encode_png(&ImageSurface::filled(1,1,[8,16,32,255]).unwrap()).unwrap();
        let reads=Arc::new(AtomicUsize::new(0));
        let stream=Arc::new(Stream{bytes:png.into(),reads:reads.clone(),declared:None});
        let mut job=start(memory.clone(),stream,None);
        let result=wait(&mut job);
        let ImageProcessState::Ready(image)=&result else{panic!("{result:?}")};
        assert_eq!(&*image.rgba(),&[8,16,32,255]);
        assert!(reads.load(Ordering::SeqCst)>0);
        assert_eq!(memory.load(Ordering::Acquire),BUDGET-1024*1024+4);
        drop(result);drop(job);
        assert_eq!(memory.load(Ordering::Acquire),BUDGET-1024*1024);
        drop(occupied);assert_eq!(memory.load(Ordering::Acquire),0);
    }
    #[test]
    fn encoded_budget_is_checked_before_allocating_or_reading_stream(){
        let memory=Arc::new(AtomicUsize::new(0));
        let occupied=Charge::new(memory.clone(),BUDGET-8192).unwrap();
        let reads=Arc::new(AtomicUsize::new(0));
        let stream=Arc::new(Stream{bytes:Arc::from([]),reads:reads.clone(),declared:Some(65536)});
        let mut job=start(memory.clone(),stream,None);
        let result=wait(&mut job);
        assert!(matches!(result,ImageProcessState::Failed(ref e) if e.contains("image processing memory budget")),"{result:?}");
        assert_eq!(reads.load(Ordering::SeqCst),0);
        assert_eq!(memory.load(Ordering::Acquire),BUDGET-8192);
        drop(occupied);assert_eq!(memory.load(Ordering::Acquire),0);
    }
    #[test]
    fn changed_stream_releases_encoded_and_scratch_reservations(){
        let memory=Arc::new(AtomicUsize::new(0));
        let png=crysirror_image::encode_png(&ImageSurface::filled(1,1,[1,2,3,255]).unwrap()).unwrap();
        let stream=Arc::new(Stream{bytes:png.into(),reads:Arc::new(AtomicUsize::new(0)),declared:None});
        let mut job=start(memory.clone(),stream,Some([0;32]));
        let result=wait(&mut job);
        assert!(matches!(result,ImageProcessState::Failed(ref e) if e.contains("asset changed")),"{result:?}");
        assert_eq!(memory.load(Ordering::Acquire),0);
    }
}
