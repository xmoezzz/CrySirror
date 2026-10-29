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
use crysirror_tjs2::{
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
    if rt.host().image_job.is_some() {
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
    if rt.host().image_job.is_some() {
        return Err(TjsError::runtime("native image operation already pending"));
    }
    let flip = effects.iter().fold([false; 2], |mut flags, e| {
        if e["kind"] == "flip" {
            flags[0] ^= e["horizontal"].as_bool().unwrap_or(false);
            flags[1] ^= e["vertical"].as_bool().unwrap_or(false);
        }
        flags
    });
    let data = serde_json::to_vec(&effects).map_err(|e| TjsError::runtime(e.to_string()))?;
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

/// layerExImage (wamsoft 3aff625): six Layer methods onto shared ImageEffects.
/// The adapter has no partial clip model, so effects cover the whole image as
/// the native does with a full clip rectangle.
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
                begin(rt, t, json!({"kind":"hsl_modulate","hue":hue,"saturation":saturation,"luminance":luminance}))
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
