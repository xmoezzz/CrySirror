// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Native parameter/value mapping only. Pixels and algorithms belong to shared
//! engine resources; composed Canvas/text image mutation is still unsupported.
use crate::natives::*;
use crysirror_core::{
    image_processing::{ImageInput, ImageProcessState},
    service::HostServices,
    state::ClientId,
};
use krkr_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::{Value, json};
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
    let h = this_of(rt, t, "Layer image processing")?;
    if !rt.host().can_suspend {
        return Err(unsupported(
            "native image processing in this callback",
            "a resumable TJS boundary",
        ));
    }
    if rt.host().image_job.is_some() {
        return Err(TjsError::runtime("native image operation already pending"));
    }
    let kind = rt.object_member(h, "type").to_integer()?;
    if !matches!(kind, 1 | 2) {
        return Err(unsupported(
            "native image processing blend type",
            "straight-alpha/opaque image profile",
        ));
    }
    let source = input(rt, h)?;
    queue(rt, h, source, effect)
}

pub fn poll(rt: &mut Rt, host: &mut dyn HostServices, client: ClientId) -> anyhow::Result<bool> {
    let Some((id, mut job, flip)) = rt.host_mut().image_job.take() else {
        return Ok(true);
    };
    match job.poll()? {
        ImageProcessState::Pending => {
            rt.host_mut().image_job = Some((id, job, flip));
            Ok(false)
        }
        ImageProcessState::Failed(e) => anyhow::bail!("native image processing failed: {e}"),
        ImageProcessState::Ready(image) => {
            anyhow::ensure!(image.client() == client, "foreign image result");
            let h = ObjectHandle(id);
            let l = rt
                .host()
                .layer(h)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .clone();
            let [w, height] = image.size();
            let slot = 0x4000_0000u32
                .checked_add(l.slot)
                .ok_or_else(|| anyhow::anyhow!("image resource slot exhausted"))?;
            let path = host.present_surface(client, slot, w, height, &image.rgba())?;
            rt.host_mut().canvas_op(json!({"op":"set","id":l.slot,"layer":{"id":0,"rect":[0,0,w,height],"paint":{"kind":"image","asset":path.relative()}}}));
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
            rt.host_mut().api_calls += 1;
            Ok(true)
        }
    }
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
fn input(rt: &Rt, h: ObjectHandle) -> Result<ImageInput> {
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
    Err(unsupported(
        "image processing source",
        "a shared loaded, processed or uniform image; composed Canvas readback remains open",
    ))
}
fn queue(rt: &mut Rt, h: ObjectHandle, input: ImageInput, effect: Value) -> Result<Variant> {
    if !rt.host().can_suspend {
        return Err(unsupported(
            "native image operation in this callback",
            "a resumable TJS boundary",
        ));
    }
    if rt.host().image_job.is_some() {
        return Err(TjsError::runtime("native image operation already pending"));
    }
    let flip = if effect["kind"] == "flip" {
        [
            effect["horizontal"].as_bool().unwrap_or(false),
            effect["vertical"].as_bool().unwrap_or(false),
        ]
    } else {
        [false; 2]
    };
    let data = serde_json::to_vec(&vec![effect]).map_err(|e| TjsError::runtime(e.to_string()))?;
    let job = rt
        .host()
        .images
        .as_ref()
        .ok_or_else(|| TjsError::runtime("image service absent"))?
        .begin_process(input, data.into())
        .map_err(|e| TjsError::runtime(e.to_string()))?;
    rt.host_mut().image_job = Some((h.0, job, flip));
    rt.request_suspend();
    Ok(Variant::Void)
}
