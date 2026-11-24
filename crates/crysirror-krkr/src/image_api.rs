// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native parameter/value mapping only. Pixels and algorithms belong to shared
//! engine resources; alpha clips prepare composed inputs through shared Canvases.
// Native image lifetime and dependency policy:
// Keep encoded Asset descriptions immutable; hidden PSD cache layers must not
// eagerly acquire duplicate RGBA buffers. Before admitting pointer input,
// prepare visible positive-alpha hit candidates through the shared image jobs.
// A composed Canvas has no direct CPU image: use the shared capture dependency
// for hit coverage, saves and pair effects, then retain its completed generation.
// Never acknowledge an undecoded image as an opaque hit or a successful read.
use crate::natives::*;
use crysirror_core::{
    image_processing::{ImageInput, ImageProcessState},
    service::HostServices,
    state::ClientId,
};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::{Value, json};

/// Consecutive native bitmap operations, retained as an engine image DAG.
/// The native owner keeps only the target and a bounded operation count.
pub(crate) struct BitmapBatch {
    pub target: usize,
    pub operations: usize,
}

fn seal_bitmap_batch(rt: &mut Rt) -> Result<()> {
    let Some(batch) = rt.host().bitmap_batch.as_ref() else { return Ok(()); };
    let target = ObjectHandle(batch.target);
    let source = input(rt, target)?;
    if rt.host().image_job.is_some() || rt.host().image_pair.is_some() {
        return Err(TjsError::runtime("bitmap batch overlaps image preparation"));
    }
    let job = rt.host().images.as_ref().ok_or_else(|| TjsError::runtime("image service absent"))?
        .begin_process(source, std::sync::Arc::from(&b"[]"[..]))
        .map_err(|e| TjsError::runtime(e.to_string()))?;
    rt.host_mut().bitmap_batch = None;
    rt.host_mut().image_job = Some((target.0, job, [false; 2], true));
    Ok(())
}

/// A non-batched call must observe all prior bitmap writes. ResourcePending
/// retries this same instruction before it has performed any side effect.
pub(crate) fn before_bitmap_call(rt: &mut Rt) -> Result<()> {
    if rt.host().bitmap_batch.is_none() { return Ok(()); }
    seal_bitmap_batch(rt)?;
    Err(TjsError::resource_pending("ordered bitmap batch"))
}
pub fn gamma(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.adjustGamma")?;
    rt.host().layer(h)?;
    if a.len() > 9 {
        return Err(TjsError::runtime(
            "adjustGamma needs at most nine parameters",
        ));
    }
    let mut gamma = [1.; 3];
    let mut floor = [0i32; 3];
    let mut ceiling = [255i32; 3];
    for c in 0..3 {
        if let Some(v) = a.get(c * 3).filter(|v| !matches!(v, Variant::Void)) {
            gamma[c] = v.to_real()?;
        }
        if let Some(v) = a.get(c * 3 + 1).filter(|v| !matches!(v, Variant::Void)) {
            floor[c] = i32::try_from(v.to_integer()?)
                .map_err(|_| TjsError::runtime("gamma floor overflow"))?;
        }
        if let Some(v) = a.get(c * 3 + 2).filter(|v| !matches!(v, Variant::Void)) {
            ceiling[c] = i32::try_from(v.to_integer()?)
                .map_err(|_| TjsError::runtime("gamma ceiling overflow"))?;
        }
    }
    if gamma == [1.; 3] && floor == [0; 3] && ceiling == [255; 3] {
        return Ok(Variant::Void);
    }
    begin(
        rt,
        Some(h),
        json!({"kind":"gamma","gamma":gamma,"floor":floor,"ceiling":ceiling}),
    )
}
pub fn begin(rt: &mut Rt, t: Option<ObjectHandle>, effect: Value) -> Result<Variant> {
    begin_chain(rt, t, vec![effect])
}
/// Submit an ordered native effect list as one shared immutable worker product.
pub(crate) fn begin_chain(
    rt: &mut Rt,
    t: Option<ObjectHandle>,
    effects: Vec<Value>,
) -> Result<Variant> {
    let h = this_of(rt, t, "Layer image processing")?;
    if !rt.host().can_suspend {
        return Err(unsupported(
            "native image processing in this callback",
            "a resumable TJS boundary",
        ));
    }
    if rt.host().image_job.is_some() || rt.host().image_pair.is_some() {
        return Err(TjsError::runtime("native image operation already pending"));
    }
    let kind = rt.host().layer(h)?.kind;
    if !matches!(kind, 1 | 2) {
        return Err(unsupported(
            "native image processing blend type",
            "straight-alpha/opaque image profile",
        ));
    }
    let source = input(rt, h)?;
    queue_chain(rt, h, source, effects)
}

/// A Photoshop-mode operateRect waiting for canvas captures of its layers.
pub(crate) struct PendingComposite {
    target: ObjectHandle,
    source: ObjectHandle,
    rect: [i64; 4],
    at: [i64; 2],
    opacity: u8,
    math: crysirror_core::image_processing::ImageCompositeMath,
    capture: Option<(ObjectHandle, crysirror_core::image_processing::ImageJob)>,
}
impl PendingComposite {
    pub(crate) fn new(
        target: ObjectHandle,
        source: ObjectHandle,
        rect: [i64; 4],
        at: [i64; 2],
        opacity: u8,
        math: crysirror_core::image_processing::ImageCompositeMath,
    ) -> Self {
        Self { target, source, rect, at, opacity, math, capture: None }
    }
    pub(crate) fn layers(&self) -> [ObjectHandle; 2] {
        [self.target, self.source]
    }
}

fn poll_composite(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<bool> {
    let e = |e: TjsError| anyhow::anyhow!("{e}");
    if rt.host().composite_pending.is_empty() { return Ok(true); }
    let mut pending = rt.host_mut().composite_pending.remove(0);
    // A script may invalidate a temporary layer while the operation is queued.
    if pending.layers().iter().any(|l| rt.host().layer(*l).is_err()) {
        return Ok(false);
    }
    if let Some((layer, mut job)) = pending.capture.take() {
        match job.poll()? {
            ImageProcessState::Pending => {
                pending.capture = Some((layer, job));
                rt.host_mut().composite_pending.insert(0, pending);
                return Ok(false);
            }
            ImageProcessState::Failed(error) => anyhow::bail!("native layer capture failed: {error}"),
            ImageProcessState::Ready(image) => {
                anyhow::ensure!(image.client() == client, "foreign image result");
                rt.host_mut().layer_mut(layer).map_err(e)?.image_source = Some(ImageInput::Resource(image));
            }
        }
    }
    for layer in [pending.target, pending.source] {
        if input(rt, layer).is_err() {
            let slot = rt.host().layer(layer).map_err(e)?.slot;
            pending.capture = Some((layer, host.begin_canvas_image(client, slot)?));
            rt.host_mut().composite_pending.insert(0, pending);
            return Ok(false);
        }
    }
    let dest = rt.host().layer(pending.target).map_err(e)?.clone();
    let source = rt.host().layer(pending.source).map_err(e)?.clone();
    let composite = crate::natives::composite_with(
        rt, pending.target, pending.source, &dest, &source,
        pending.rect, pending.at, pending.opacity, pending.math,
    ).ok_or_else(|| anyhow::anyhow!("native layer composite has no CPU source after capture"))?;
    let job = rt.host().images.as_ref().ok_or_else(|| anyhow::anyhow!("image service absent"))?
        .begin_process(composite, std::sync::Arc::from(&b"[]"[..]))?;
    rt.host_mut().image_job = Some((pending.target.0, job, [false; 2], true));
    Ok(false)
}

pub fn poll(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<bool> {
    seal_bitmap_batch(rt).map_err(|e|anyhow::anyhow!("{e}"))?;
    if !poll_save(rt,host,client)?{return Ok(false);}
    if !poll_pair(rt,host,client)?{return Ok(false);}
    if rt.host().image_job.is_none() && !poll_composite(rt,host,client)?{return Ok(false);}
    let Some((id, mut job, flip, publish_surface)) = rt.host_mut().image_job.take() else {
        return Ok(true);
    };
    match job.poll()? {
        ImageProcessState::Pending => {
            rt.host_mut().image_job = Some((id, job, flip, publish_surface));
            Ok(false)
        }
        ImageProcessState::Failed(e) => anyhow::bail!("native image processing failed: {e}"),
        ImageProcessState::Ready(image) => {
            commit_result(rt,host,client,id,image,flip,publish_surface)?;
            Ok(true)
        }
    }
}

pub(crate) fn commit_result(rt:&mut Rt,host:&mut dyn HostServices,client:ClientId,id:usize,image:std::sync::Arc<dyn crysirror_core::image_processing::ImageResource>,flip:[bool;2],publish_surface:bool)->anyhow::Result<()> {
    anyhow::ensure!(image.client() == client, "foreign image result");
    let h = ObjectHandle(id);
    let l = rt
        .host()
        .layer(h)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .clone();
    let [w, height] = image.size();
    if publish_surface {
        let uploaded=host.publish_image_resource(client,l.slot,image.clone())?;
        rt.host_mut().retired_images.push(uploaded.path.relative().to_owned());
        rt.host_mut().canvas_op(json!({"op":"set","id":l.slot,"layer":{"id":0,"rect":[0,0,w,height],"paint":{"kind":"image","asset":uploaded.path.relative()}}}));
    }
    let l = rt
        .host_mut()
        .layer_mut(h)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    l.regions
        .flip(client.0, flip[0], flip[1])
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    l.image_source = Some(ImageInput::Resource(image.clone()));
    l.full_alpha = image.opaque().then_some(255);
    l.blank = false;
    if publish_surface {l.image_modified=true;}
    rt.host_mut().api_calls += 1;
    Ok(())
}

/// `Layer.saveLayerImage` (krkrz TVPSaveImage): a loaded image goes through a
/// shared job, a composed one through the Canvas capture presenter; encoding
/// and the atomic write then use shared image and storage services.
pub(crate) struct LayerImageSave{
    pub layer:usize,
    // A bookmark encoder returns its octet to the suspended script. A
    // saveLayerImage call writes it atomically through the Save capability.
    destination:LayerImageDestination,
    format:SaveFormat,
    input:Option<ImageInput>,
    job:Option<crysirror_core::image_processing::ImageJob>,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum SaveFormat{Bmp(crysirror_image::bmp_encode::BmpDepth),Png,Jpeg(u8)}
enum LayerImageDestination { File(String), Bookmark }
/// Layer.saveLayerImage handler modes. The PackinOne bookmark encoder selects
/// its JPEG quality separately from this mode string.
fn save_format(mode:&str)->Option<SaveFormat>{
    use crysirror_image::bmp_encode::BmpDepth as D;
    match mode.to_ascii_lowercase().trim_start_matches('.'){
        "bmp"|"bmp32"|"dib"=>Some(SaveFormat::Bmp(D::Bgra32)),
        "bmp24"=>Some(SaveFormat::Bmp(D::Bgr24)),
        "bmp8"=>Some(SaveFormat::Bmp(D::Dither252)),
        "png"=>Some(SaveFormat::Png),
        _=>None,
    }
}
pub(crate) fn save_layer_image(rt:&mut Rt,t:Option<ObjectHandle>,args:Vec<Variant>)->Result<Variant>{
    const M:&str="Layer.saveLayerImage";
    if args.is_empty(){return Err(TjsError::bad_param_count());}
    if !rt.host().can_suspend{return Err(unsupported(M,"a resumable native boundary"));}
    if rt.host().image_save.is_some(){return Err(TjsError::runtime("native layer image save already pending"));}
    let h=this_of(rt,t,M)?;
    let name=crate::natives::string(&args,0,M)?;
    let mode=if matches!(args.get(1),None|Some(Variant::Void)){"bmp".to_owned()}else{crate::natives::string(&args,1,M)?};
    let Some(format)=save_format(&mode) else{
        return Err(if matches!(mode.to_ascii_lowercase().trim_start_matches('.'),"jpg"|"jpeg"|"tlg"|"tlg5"|"tlg6"){
            unsupported(&format!("{M} type {mode:?}"),"a registered layer-save handler for this mode")
        }else{
            TjsError::runtime(format!("unknown graphic format {mode:?}"))
        });
    };
    let layer=rt.host().layer(h)?.clone();
    if !layer.has_image{return Err(TjsError::runtime("not a drawable layer type"));}
    if crate::storage::save_path(&name).map_err(|e|TjsError::runtime(e.to_string()))?.is_none(){
        return Err(TjsError::runtime(format!("{M}: {name:?} is outside the save capability root")));
    }
    rt.host_mut().image_save=Some(LayerImageSave{layer:h.0,destination:LayerImageDestination::File(name),format,input:layer.image_source,job:None});
    rt.request_suspend();Ok(Variant::Void)
}

/// Legacy bookmarks append the saved variables after a BMP. PackinOne
/// bookmarks use their own JPEG or PNG boundary.
pub(crate) fn load_save_thumbnail(
    rt: &mut Rt,
    layer: ObjectHandle,
    name: &str,
    mut bytes: Vec<u8>,
) -> Result<Variant> {
    let end = if bytes.starts_with(b"BM") {
        let field = |at: usize| {
            let value = bytes.get(at..at + 4)?.try_into().ok()?;
            usize::try_from(u32::from_le_bytes(value)).ok()
        };
        field(2).zip(field(10)).and_then(|(size, pixels)| {
            (pixels >= 14 && pixels < size && size <= bytes.len()).then_some(size)
        })
    } else {
        crate::plugin_ports::packinone::image_end(&bytes)
    };
    let end = end.ok_or_else(|| TjsError::runtime(format!("no valid BMP, JPEG or PNG thumbnail in `{name}`")))?;
    bytes.truncate(end);
    crate::psbfile_plugin::load_encoded_image(rt, layer, name, bytes)
}

pub(crate) fn capture_bookmark_thumbnail(rt: &mut Rt, layer: ObjectHandle, jpeg: bool, quality: u8) -> Result<Variant> {
    before_bitmap_call(rt)?;
    if !rt.host().can_suspend {
        return Err(unsupported("makeDataPackThumb", "a resumable image capture"));
    }
    if rt.host().image_save.is_some() {
        return Err(TjsError::runtime("native layer image save already pending"));
    }
    let source = rt.host().layer(layer)?;
    if !source.has_image { return Ok(Variant::Void); }
    let input = source.image_source.clone();
    rt.host_mut().image_save = Some(LayerImageSave {
        layer: layer.0,
        destination: LayerImageDestination::Bookmark,
        format: if jpeg { SaveFormat::Jpeg(quality) } else { SaveFormat::Png },
        input, job: None,
    });
    rt.request_suspend();
    Ok(Variant::Void)
}
fn poll_save(rt:&mut Rt,host:&mut dyn HostServices,client:ClientId)->anyhow::Result<bool>{
    let Some(mut save)=rt.host_mut().image_save.take() else{return Ok(true)};
    if save.job.is_none(){
        save.job=Some(match save.input.take(){
            Some(input)=>rt.host().images.as_ref().ok_or_else(||anyhow::anyhow!("image service absent"))?.begin_process(input,std::sync::Arc::from(&b"[]"[..]))?,
            None=>{
                let layer=rt.host().layer(ObjectHandle(save.layer)).map_err(|e|anyhow::anyhow!("{e}"))?;
                host.begin_canvas_image(client,layer.slot)?
            }
        });
    }
    match save.job.as_mut().unwrap().poll()?{
        ImageProcessState::Pending=>{rt.host_mut().image_save=Some(save);Ok(false)}
        ImageProcessState::Failed(e)=>anyhow::bail!("Layer.saveLayerImage capture failed: {e}"),
        ImageProcessState::Ready(image)=>{
            let [width,height]=image.size();
            let bytes=match save.format{
                SaveFormat::Bmp(depth)=>crysirror_image::bmp_encode::encode_bmp(width,height,&image.rgba(),depth)?,
                SaveFormat::Png=>crysirror_image::encode_png(&crysirror_image::ImageSurface{width,height,rgba:image.rgba().to_vec()})?,
                SaveFormat::Jpeg(quality)=>crysirror_image::encode_jpeg(&crysirror_image::ImageSurface{width,height,rgba:image.rgba().to_vec()},quality)?,
            };
            match save.destination {
                LayerImageDestination::File(name) => {
                    let job=rt.host().storage().map_err(|e|anyhow::anyhow!("{e}"))?.begin_write(&name,bytes.into()).map_err(|e|anyhow::anyhow!("{e}"))?;
                    rt.host_mut().writes.push_back(job);
                }
                LayerImageDestination::Bookmark => rt.set_resume_value(Variant::Octet(bytes)),
            }
            Ok(true)
        }
    }
}
#[cfg(test)]
mod save_format_tests{
    use super::*;
    use crysirror_image::bmp_encode::BmpDepth as D;
    #[test]
    fn modes_follow_the_original_handlers(){
        assert_eq!(save_format("bmp"),Some(SaveFormat::Bmp(D::Bgra32)));
        assert_eq!(save_format("bmp24"),Some(SaveFormat::Bmp(D::Bgr24)));
        assert_eq!(save_format("bmp8"),Some(SaveFormat::Bmp(D::Dither252)));
        assert_eq!(save_format(".PNG"),Some(SaveFormat::Png));
        assert_eq!(save_format("gif"),None);
    }
}

/// Native suspension policy only. Every missing CPU input is a client Canvas
/// capture through the shared presenter, followed by an ordinary shared job.
pub(crate) struct PairPreparation{
    pub target:usize,pub source:usize,
    inputs:[Option<ImageInput>;2],
    captures:[Option<crysirror_core::image_processing::ImageJob>;2],
    effects:std::sync::Arc<[u8]>,
}
fn poll_pair(rt:&mut Rt,host:&mut dyn HostServices,client:ClientId)->anyhow::Result<bool>{
    let Some(mut pair)=rt.host_mut().image_pair.take() else{return Ok(true)};
    for index in 0..2 {
        if index==1&&pair.target==pair.source{pair.inputs[1]=pair.inputs[0].clone();}
        if pair.inputs[index].is_some(){continue;}
        let id=[pair.target,pair.source][index];
        if pair.captures[index].is_none(){
            let layer=rt.host().layer(ObjectHandle(id)).map_err(|e|anyhow::anyhow!("{e}"))?;
            pair.captures[index]=Some(host.begin_canvas_image(client,layer.slot)?);
        }
        match pair.captures[index].as_mut().unwrap().poll()?{
            ImageProcessState::Pending=>{rt.host_mut().image_pair=Some(pair);return Ok(false);}
            ImageProcessState::Failed(e)=>anyhow::bail!("native alpha input capture failed: {e}"),
            ImageProcessState::Ready(image)=>{pair.inputs[index]=Some(ImageInput::Resource(image));pair.captures[index]=None;}
        }
    }
    let [destination,source]=pair.inputs;
    let input=ImageInput::Raster{destination:Box::new(destination.unwrap()),source:Box::new(source.unwrap())};
    let job=rt.host().images.as_ref().ok_or_else(||anyhow::anyhow!("image service absent"))?.begin_process(input,pair.effects)?;
    rt.host_mut().image_job=Some((pair.target,job,[false;2],true));Ok(true)
}
pub fn clip_alpha(rt:&mut Rt,t:Option<ObjectHandle>,args:Vec<Variant>)->Result<Variant>{
    const M:&str="Layer.clipAlphaRect";
    if args.len()<7{return Err(TjsError::bad_param_count());}
    if !rt.host().can_suspend{return Err(unsupported(M,"a resumable native boundary"));}
    if rt.host().image_job.is_some()||rt.host().image_pair.is_some(){return Err(TjsError::runtime("native image operation already pending"));}
    let target=this_of(rt,t,M)?;
    let source=object_arg(&args[2]).map(|h|rt.bound_this(h).unwrap_or(h)).ok_or_else(||TjsError::runtime("clipAlphaRect: src must be Layer"))?;
    let d=rt.host().layer(target)?.clone();let s=rt.host().layer(source)?.clone();
    if !d.has_image||!s.has_image{return Err(TjsError::runtime("clipAlphaRect requires drawable images"));}
    let coordinate=|index|need_int(&args,index,M).map(|n|n as i32);
    let destination=[coordinate(0)?,coordinate(1)?];
    let source_rect=[coordinate(3)?,coordinate(4)?,coordinate(5)?,coordinate(6)?];
    if source_rect[2]<=0||source_rect[3]<=0{return Err(TjsError::runtime("clipAlphaRect: invalid source extent"));}
    let clip=clip_of(&d).map(|n|n as i32);
    if clip[2]<=0||clip[3]<=0{return Err(TjsError::runtime("clipAlphaRect: destination clip is empty"));}
    let clear=match args.get(7){None|Some(Variant::Void)=>None,Some(v)=>{let n=v.to_integer()? as i32;(0..=255).contains(&n).then_some(n as u8)}};
    let spec=crysirror_image::alpha_clip::AlphaClipRect{destination,source:source_rect,clip,clear,multiply_rgb:d.kind==12,self_source:target==source,clear_empty_alpha_only:true};
    let effects=serde_json::to_vec(&[crysirror_image::effects::ImageEffect::AlphaClipRect{spec}]).map_err(|e|TjsError::runtime(e.to_string()))?;
    let host = rt.host_mut();
    host.image_pair=Some(PairPreparation{target:target.0,source:source.0,inputs:[d.image_source,s.image_source],captures:[None,None],effects:effects.into()});
    // onPaint often draws the unmasked source into the destination first.
    // The Canvas worker needs to see those writes, but the presenter must
    // keep the previous finished picture until this redraw returns.
    host.bitmap_redraw_pending = true;
    rt.request_suspend();Ok(Variant::Void)
}

/// Queue a two-layer shared effect (target image as destination, `source`
/// layer image as the second input) and suspend until it commits.
pub(crate) fn begin_pair(rt:&mut Rt,target:ObjectHandle,source:ObjectHandle,effect:crysirror_image::effects::ImageEffect,what:&str)->Result<Variant>{
    if !rt.host().can_suspend{return Err(unsupported(what,"a resumable native boundary"));}
    if rt.host().image_job.is_some()||rt.host().image_pair.is_some(){return Err(TjsError::runtime("native image operation already pending"));}
    let d=rt.host().layer(target)?.clone();let s=rt.host().layer(source)?.clone();
    if !d.has_image||!s.has_image{return Err(TjsError::runtime(format!("{what} requires drawable images")));}
    let effects=serde_json::to_vec(&[effect]).map_err(|e|TjsError::runtime(e.to_string()))?;
    rt.host_mut().image_pair=Some(PairPreparation{target:target.0,source:source.0,inputs:[d.image_source,s.image_source],captures:[None,None],effects:effects.into()});
    rt.request_suspend();Ok(Variant::Void)
}

/// Prepare native mask coverage before consuming an input receipt. The same
/// shared ImageJob evaluates immutable copy/fill/resize graphs; no Canvas paint
/// is retargeted, and no pointer callback or TJS continuation is replayed.
pub(crate) fn prepare_hit_image(
    rt: &mut Rt, host: &mut dyn HostServices, client: ClientId, id: usize, x: i64, y: i64,
) -> anyhow::Result<bool> {
    anyhow::ensure!(rt.host().image_job.is_none(), "hit preparation overlaps native image job");
    let h = ObjectHandle(id);
    let kind = rt.object_member(h, "__lp_hitType").to_integer().map_err(|e|anyhow::anyhow!("{e}"))?;
    let threshold = rt.object_member(h, "__lp_hitThreshold").to_integer().map_err(|e|anyhow::anyhow!("{e}"))?;
    let layer = rt.host().layer(h).map_err(|e|anyhow::anyhow!("{e}"))?;
    let px=x-layer.image_left; let py=y-layer.image_top;
    if kind!=0 || !(1..256).contains(&threshold) || !layer.has_image
        || layer.kind==1 || layer.full_alpha.is_some()
        || px<0 || py<0 || px>=layer.image_width || py>=layer.image_height {
        return Ok(true);
    }
    // A layer composed by retained canvas drawing has no CPU source: its
    // coverage comes from the same shared capture Layer.saveLayerImage uses.
    let Some(source)=layer.image_source.as_ref() else {
        let slot=layer.slot;
        let job=host.begin_canvas_image(client,slot)?;
        rt.host_mut().image_job=Some((id,job,[false;2],false));
        return Ok(false);
    };
    if matches!(source,ImageInput::Resource(_)) { return Ok(true); }
    let source=source.clone();
    let job=rt.host().images.as_ref().ok_or_else(||anyhow::anyhow!("image service absent"))?
        .begin_process(source,std::sync::Arc::from(&b"[]"[..]))
        .map_err(|e|anyhow::anyhow!("{e}"))?;
    // Only coverage becomes ready. Existing retained Canvas nodes and copied
    // immutable resources continue to own the visible image.
    rt.host_mut().image_job=Some((id,job,[false;2],false));
    Ok(false)
}

/// A real layerExRaster facade. Only full clip boxes and shared source resources
/// are admitted; raw pointer buffers are never exposed to TJS or plugins.
pub fn install_raster(rt: &mut Rt) {
    if let Some(class) = rt.global_member("Layer").object_handle() {
        if !matches!(rt.object_member(class, "copyRaster"), Variant::Closure(_)) {
            rt.register_object_native(class,"copyRaster",|rt:&mut Rt,t,a:Vec<Variant>|{
    if a.len()<5{return Err(TjsError::bad_param_count())}
    let h=this_of(rt,t,"Layer.copyRaster")?;
    let source=object_arg(&a[0]).map(|v|rt.bound_this(v).unwrap_or(v)).ok_or_else(||TjsError::runtime("copyRaster source must be a Layer"))?;
    if h==source{return Err(unsupported("self copyRaster","native overlapping in-place row-copy profile"))}
    let d=rt.host().layer(h)?;let s=rt.host().layer(source)?;
    if !d.has_image||!s.has_image{return Err(TjsError::runtime("Not drawable layer type"))}
    if [d.image_width,d.image_height]!=[s.image_width,s.image_height]{return Ok(Variant::Void)}
    let input=ImageInput::Raster{destination:Box::new(input(rt,h)?),source:Box::new(input(rt,source)?)};
    let amplitude=i32::try_from(a[1].to_integer()?).map_err(|_|TjsError::runtime("row-copy amplitude overflow"))?;
    let lines=i32::try_from(a[2].to_integer()?).map_err(|_|TjsError::runtime("row-copy line count overflow"))?;
    let cycle=i32::try_from(a[3].to_integer()?).map_err(|_|TjsError::runtime("row-copy cycle overflow"))?;
    let time=a[4].to_integer()?;
    queue(rt,h,input,json!({"kind":"sine_row_copy","spec":{"amplitude":amplitude,"lines":lines,"cycle":cycle,"time":time}}))
   });
        }
    }
}
pub(crate) fn input(rt: &Rt, h: ObjectHandle) -> Result<ImageInput> {
    let l = rt.host().layer(h)?;
    if let Some(input) = &l.image_source {
        return Ok(input.clone());
    }
    if l.blank && l.has_image {
        return Ok(ImageInput::Solid {
            size: [l.image_width as u32, l.image_height as u32],
            color: [255, 255, 255, 0],
        });
    }
    if std::env::var_os("CRYSIRROR_KRKR_TRACE_SOURCE").is_some() {
        eprintln!("[source] layer {} slot {} lost: {:?}", h.0, l.slot, l.source_lost);
    }
    Err(unsupported(
        "image processing source",
        "a shared loaded, processed or uniform image; composed Canvas readback remains open",
    ))
}
fn queue(rt: &mut Rt, h: ObjectHandle, input: ImageInput, effect: Value) -> Result<Variant> {
    queue_chain(rt, h, input, vec![effect])
}
pub(crate) fn queue_chain(
    rt: &mut Rt,
    h: ObjectHandle,
    input: ImageInput,
    effects: Vec<Value>,
) -> Result<Variant> {
    if !rt.host().can_suspend {
        return Err(unsupported(
            "native image operation in this callback",
            "a resumable TJS boundary",
        ));
    }
    if rt.host().image_job.is_some() || rt.host().image_pair.is_some() {
        return Err(TjsError::runtime("native image operation already pending"));
    }
    let flip = effects.iter().fold([false; 2], |mut flags, e| {
        if e["kind"] == "flip" {
            flags[0] ^= e["horizontal"].as_bool().unwrap_or(false);
            flags[1] ^= e["vertical"].as_bool().unwrap_or(false);
        }
        flags
    });
    // Preparing coverage of an unchanged immutable asset must not retarget its
    // Canvas paint to a mutable per-Layer surface. Canvas copies retain the
    // original asset independently of later loads into a temporary Layer.
    let publish_surface = !effects.is_empty() || !matches!(&input, ImageInput::Asset { .. });
    let data = serde_json::to_vec(&effects).map_err(|e| TjsError::runtime(e.to_string()))?;
    let job = rt
        .host()
        .images
        .as_ref()
        .ok_or_else(|| TjsError::runtime("image service absent"))?
        .begin_process(input, data.into())
        .map_err(|e| TjsError::runtime(e.to_string()))?;
    rt.host_mut().image_job = Some((h.0, job, flip, publish_surface));
    rt.request_suspend();
    Ok(Variant::Void)
}

/// layerExImage (wamsoft 3aff625): six Layer methods onto shared ImageEffects.
pub fn install_ex_image(rt: &mut Rt) {
    let Some(class) = rt.global_member("Layer").object_handle() else {
        return;
    };
    methods(
        rt,
        class,
        &[
            ("light", |rt, t, a| {
                let brightness = need_int(&a, 0, "Layer.light")? as i32;
                let contrast = need_int(&a, 1, "Layer.light")? as i32;
                begin(rt, t, json!({"kind":"brightness_contrast","brightness":brightness,"contrast":contrast}))
            }),
            ("colorize", |rt, t, a| {
                // Native parameters are BYTE hue/sat; out-of-range values wrap.
                let hue = need_int(&a, 0, "Layer.colorize")? as u8;
                let saturation = need_int(&a, 1, "Layer.colorize")? as u8;
                let blend = a.get(2).ok_or_else(|| TjsError::bad_param_count())?.to_real()?;
                begin(rt, t, json!({"kind":"colorize","hue":hue,"saturation":saturation,"blend":blend}))
            }),
            ("modulate", |rt, t, a| {
                let hue = need_int(&a, 0, "Layer.modulate")? as i32;
                let saturation = need_int(&a, 1, "Layer.modulate")? as i32;
                let luminance = need_int(&a, 2, "Layer.modulate")? as i32;
                let h = this_of(rt, t, "Layer.modulate")?;
                let layer = rt.host().layer(h)?;
                // layerExImage::reset offsets the buffer by the drawing clip.
                // Expression redraws can reuse the already modulated body.
                let size = [0, 0, layer.image_width, layer.image_height];
                let Some(rect) = clip_rect(layer, size) else {
                    return Ok(Variant::Void);
                };
                let rect = (rect != size).then_some(rect.map(|part| part as u32));
                begin(rt, Some(h), json!({"kind":"hsl_modulate","hue":hue,"saturation":saturation,"luminance":luminance,"rect":rect}))
            }),
            ("noise", |rt, t, a| {
                // Native: per channel (rand/RAND_MAX - 0.5) * level.
                let level = need_int(&a, 0, "Layer.noise")?.clamp(0, 255);
                let seed = (rt.host().api_calls as u32).wrapping_mul(2654435761) | 1;
                begin(rt, t, json!({"kind":"noise","seed":seed,"amplitude":level / 2,"monochrome":false}))
            }),
            ("generateWhiteNoise", |rt, t, _| {
                let seed = (rt.host().api_calls as u32).wrapping_mul(2246822519) | 1;
                begin(rt, t, json!({"kind":"white_noise","seed":seed}))
            }),
            ("gaussianBlur", |rt, t, a| {
                // CxImage gen_convolve_matrix: std_dev = |0.5 * radius| + 0.25.
                // The shared kernel is a sampled Gaussian of that deviation.
                let radius = a.first().ok_or_else(TjsError::bad_param_count)?.to_real()?;
                let sigma = ((0.5 * radius).abs() + 0.25).min(16.0);
                begin(rt, t, json!({"kind":"gaussian_blur","sigma":sigma}))
            }),
        ],
    );
}
