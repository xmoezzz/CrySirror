//! Remaining native Layer/Window operations over engine-owned resources.
//! Semantics: krkrz LayerIntf.cpp and WindowImpl.cpp at the existing source pin.
//! Native handles are ordinary VM objects; bitmap pixels never live in the VM.
use super::*;
use crysirror_core::image_processing::{ImageCompositeLayer, ImageCompositeMath, ImageInput};
#[path = "native_bitmap.rs"]
pub(super) mod bitmap;

#[derive(Default)]
pub(crate) struct State {
    pub prepare: Option<ObjectHandle>,
    pub posted: std::collections::VecDeque<(ObjectHandle, &'static str, i64, i64)>,
    pub touches: std::collections::BTreeMap<u32, usize>,
    pub attention: Option<(usize, [i32; 4])>,
}
impl State {
    pub(crate) fn roots(&self, out: &mut Vec<ObjectHandle>) {
        out.extend(self.prepare);
        out.extend(self.posted.iter().map(|v| v.0));
    }
    pub(crate) fn invalidate(&mut self, h: ObjectHandle) {
        if self.attention.is_some_and(|(id, _)| id == h.0) {
            self.attention = None;
        }
        if self.prepare == Some(h) {
            self.prepare = None;
        }
        self.posted.retain(|v| v.0 != h);
        self.touches.retain(|_, owner| *owner != h.0);
    }
}
/// A pending read retries the original instruction after a shared capture or
/// immutable graph worker completes. It never invents an opaque pixel.
pub(crate) fn prepare(rt: &mut Rt, h: ObjectHandle) -> Result<()> {
    let l = rt.host().layer(h)?;
    if !l.has_image {
        return Err(TjsError::runtime("Not drawable layer type"));
    }
    if matches!(
        &l.image_source,
        Some(ImageInput::Resource(_) | ImageInput::Solid { .. })
    ) || (l.blank && l.image_source.is_none())
    {
        return Ok(());
    }
    if !rt.host().can_suspend {
        return Err(unsupported(
            "bitmap read in synchronous callback",
            "resumable image preparation",
        ));
    }
    rt.host_mut().extra.prepare = Some(h);
    Err(TjsError::resource_pending("native bitmap read preparation"))
}
pub(crate) fn poll(
    rt: &mut Rt,
    host: &mut dyn crysirror_core::service::HostServices,
    client: crysirror_core::state::ClientId,
) -> anyhow::Result<()> {
    let Some(h) = rt.host_mut().extra.prepare.take() else {
        return Ok(());
    };
    let l = rt.host().layer(h).map_err(|e| anyhow::anyhow!("{e}"))?;
    let job = if let Some(input) = l.image_source.clone() {
        rt.host()
            .images
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("image service absent"))?
            .begin_process(input, std::sync::Arc::from(&b"[]"[..]))?
    } else {
        host.begin_canvas_image(client, l.slot)?
    };
    anyhow::ensure!(
        rt.host().image_job.is_none(),
        "bitmap read overlaps image job"
    );
    rt.host_mut().image_job = Some((h.0, job, [false; 2], false));
    Ok(())
}
pub(super) fn mask_pixel(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.getMaskPixel")?;
    let [x, y] = [
        need_int(&a, 0, "getMaskPixel")?,
        need_int(&a, 1, "getMaskPixel")?,
    ];
    let l = rt.host().layer(h)?;
    if x < 0 || y < 0 || x >= l.image_width || y >= l.image_height {
        return Err(TjsError::runtime("pixel out of rectangle"));
    }
    prepare(rt, h)?;
    let source = crate::image_api::input(rt, h)?;
    let p = source
        .resident_pixel(
            crysirror_core::state::ClientId(rt.host().client),
            [x as u32, y as u32],
        )
        .map_err(|e| TjsError::runtime(e.to_string()))?
        .ok_or_else(|| TjsError::runtime("pixel out of rectangle"))?;
    Ok(Variant::Integer(p[3] as i64))
}
pub(super) fn set_mask_pixel(
    rt: &mut Rt,
    t: Option<ObjectHandle>,
    a: Vec<Variant>,
) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.setMaskPixel")?;
    let [x, y, alpha] = [
        need_int(&a, 0, "setMaskPixel")?,
        need_int(&a, 1, "setMaskPixel")?,
        need_int(&a, 2, "setMaskPixel")?,
    ];
    let l = rt.host().layer(h)?.clone();
    if !l.has_image {
        return Err(TjsError::runtime("Not drawable layer type"));
    }
    // Native clipping precedes SetPointMask's bounds validation.
    if clip_rect(&l, [x, y, 1, 1]).is_none() {
        return Ok(Variant::Void);
    }
    prepare(rt, h)?;
    let input = crate::image_api::input(rt, h)?.with_composite_layer(ImageCompositeLayer {
        input: Box::new(ImageInput::Solid {
            size: [1, 1],
            color: [0, 0, 0, alpha as u8],
        }),
        source: [0, 0, 1, 1],
        destination: [x as i32, y as i32],
        clip: None,
        opacity: 255,
        math: ImageCompositeMath::CopyAlpha,
    });
    crate::image_api::queue_chain(rt, h, input, vec![])
}
pub(super) fn dump(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.dump")?;
    rt.host().layer(h)?;
    let mut pending = vec![(h.0, 0usize)];
    let mut count = 0;
    while let Some((id, depth)) = pending.pop() {
        count += 1;
        if count > crate::host::MAX_LAYERS {
            return Err(TjsError::runtime("layer dump traversal limit"));
        }
        let l = rt.host().layer(ObjectHandle(id))?;
        let line = format!(
            "{}Layer {}: pos=({}, {}) size={}x{} image={}x{} visible={} type={}",
            "  ".repeat(depth),
            id,
            l.left,
            l.top,
            l.width,
            l.height,
            l.image_width,
            l.image_height,
            l.visible,
            l.kind
        );
        pending.extend(l.children.iter().rev().map(|c| (*c, depth + 1)));
        rt.host_mut().push_log(line);
    }
    Ok(Variant::Void)
}
pub(super) fn attention(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.setAttentionPos")?;
    rt.host().layer(h)?;
    let x = need_int(&a, 0, "setAttentionPos")?;
    let y = need_int(&a, 1, "setAttentionPos")?;
    rt.set_object_member(h, "__lp_attentionLeft", Variant::Integer(x));
    rt.set_object_member(h, "__lp_attentionTop", Variant::Integer(y));
    rt.host_mut().attention_dirty = true;
    Ok(Variant::Void)
}
pub(crate) fn sync_attention(rt: &mut Rt) -> Result<()> {
    rt.host_mut().attention_dirty = false;
    let mut selected = None;
    let windows: Vec<_> = rt.host().windows.keys().copied().collect();
    for window in windows {
        let Some(h) = focused_layer(rt, ObjectHandle(window)) else {
            continue;
        };
        if !rt.object_member(h, "__lp_useAttention").is_truthy() {
            continue;
        }
        let mut point = [
            rt.object_member(h, "__lp_attentionLeft").to_integer()?,
            rt.object_member(h, "__lp_attentionTop").to_integer()?,
        ];
        let mut current = Some(h.0);
        let mut depth = 0;
        while let Some(id) = current {
            let l = rt.host().layer(ObjectHandle(id))?;
            point[0] = point[0]
                .checked_add(l.left)
                .ok_or_else(|| TjsError::runtime("attention position overflow"))?;
            point[1] = point[1]
                .checked_add(l.top)
                .ok_or_else(|| TjsError::runtime("attention position overflow"))?;
            current = l.parent;
            depth += 1;
            if depth > 64 {
                return Err(TjsError::runtime("attention ancestry"));
            }
        }
        let x = i32::try_from(point[0]).map_err(|_| TjsError::runtime("attention x"))?;
        let y = i32::try_from(point[1]).map_err(|_| TjsError::runtime("attention y"))?;
        let height = layer_font(rt, h).ok()
            .and_then(|font| rt.resolve_object_member(font, "height").ok())
            .and_then(|v| v.to_integer().ok())
            .unwrap_or(24)
            .unsigned_abs()
            .clamp(1, 512);
        selected = Some((h.0, [x, y, 1, height as i32]));
        break;
    }
    if selected != rt.host().extra.attention {
        if let Some((_, rect)) = selected {
            rt.host_mut()
                .ui_op(json!({"op":"set_ime_cursor_area","rect":rect}));
        }
        rt.host_mut().extra.attention = selected;
    }
    Ok(())
}
pub(super) fn release_touch(
    rt: &mut Rt,
    t: Option<ObjectHandle>,
    a: Vec<Variant>,
) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.releaseTouchCapture")?;
    rt.host().layer(h)?;
    if let Some(id) = a.first() {
        let id = id.to_integer()? as u32;
        if rt.host().extra.touches.get(&id) == Some(&h.0) {
            rt.host_mut().extra.touches.remove(&id);
        }
    } else {
        rt.host_mut().extra.touches.retain(|_, owner| *owner != h.0);
    }
    Ok(Variant::Void)
}
pub(super) fn begin_move(rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Window.beginMove")?;
    if !is_output_window(rt, h) {
        return Err(unsupported(
            "secondary Window.beginMove",
            "a native presenting window",
        ));
    }
    rt.host_mut().ui_op(json!({"op":"begin_window_drag"}));
    Ok(Variant::Void)
}
pub(super) fn remove_mask(
    rt: &mut Rt,
    t: Option<ObjectHandle>,
    _: Vec<Variant>,
) -> Result<Variant> {
    let h = this_of(rt, t, "Window.removeMaskRegion")?;
    window_of(rt, Some(h))?;
    rt.set_object_member(h, "__window_mask", Variant::Void);
    if is_output_window(rt, h) {
        rt.host_mut().ui_op(json!({"op":"reset_window_shape"}));
    }
    Ok(Variant::Void)
}
pub(super) fn post_input(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Window.postInputEvent")?;
    window_of(rt, Some(h))?;
    let name = string(&a, 0, "Window.postInputEvent")?;
    let method = match name.as_str() {
        "onKeyDown" => "onKeyDown",
        "onKeyUp" => "onKeyUp",
        "onKeyPress" => "onKeyPress",
        _ => {
            return Err(TjsError::runtime(format!(
                "Specified event name is unknown: {name}"
            )));
        }
    };
    let params = a
        .get(1)
        .and_then(object_arg)
        .ok_or_else(TjsError::bad_param_count)?;
    let key = rt.resolve_object_member(params, "key")?.to_integer()?;
    let shift = if method == "onKeyPress" {
        0
    } else {
        rt.resolve_object_member(params, "shift")?.to_integer()?
    };
    if rt.host().extra.posted.len() >= 256 {
        return Err(TjsError::runtime("posted input queue limit"));
    }
    rt.host_mut()
        .extra
        .posted
        .push_back((h, method, key, shift));
    Ok(Variant::Void)
}
pub(super) fn install_layer(rt: &mut Rt, h: ObjectHandle) {
    methods(
        rt,
        h,
        &[
            ("getMaskPixel", mask_pixel),
            ("setMaskPixel", set_mask_pixel),
            ("dump", dump),
            ("setAttentionPos", attention),
            ("releaseTouchCapture", release_touch),
            ("copy9Patch", nine_patch),
            ("drawGlyph", draw_glyph),
            ("copyToBitmapFromMainImage", bitmap::copy_to),
            ("copyFromBitmapToMainImage", bitmap::copy_from),
        ],
    );
}
pub(super) fn nine_patch(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.copy9Patch")?;
    let source = a
        .first()
        .and_then(object_arg)
        .map(|h| rt.bound_this(h).unwrap_or(h))
        .ok_or_else(TjsError::bad_param_count)?;
    let s = rt.host().layer(source)?;
    let d = rt.host().layer(h)?;
    let size = [s.image_width as u32, s.image_height as u32];
    let output = [d.image_width as u32, d.image_height as u32];
    if !d.has_image || !s.has_image {
        return Err(TjsError::runtime("Not drawable layer or bitmap"));
    }
    if size[0] < 11 || size[1] < 11 || output[0] < size[0] - 2 || output[1] < size[1] - 2 {
        return Ok(Variant::Void);
    }
    prepare(rt, source)?;
    let input = crate::image_api::input(rt, source)?;
    let owner = crysirror_core::state::ClientId(rt.host().client);
    let layout = crysirror_image::nine_patch::layout(size, |x, y| {
        input
            .resident_pixel(owner, [x, y])
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .map(|p| p[3])
            .ok_or_else(|| anyhow::anyhow!("nine-patch border read"))
    })
    .map_err(|e| TjsError::runtime(e.to_string()))?;
    let Some(layout) = layout else {
        return Ok(Variant::Void);
    };
    crate::image_api::queue_chain(
        rt,
        h,
        input,
        vec![json!({"kind":"nine_patch","width":output[0],"height":output[1]})],
    )?;
    Ok(rect(rt, layout.margins))
}
pub(super) fn draw_glyph(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>) -> Result<Variant> {
    let h = this_of(rt, t, "Layer.drawGlyph")?;
    check_face(rt, h, "Layer.drawGlyph")?;
    if a.len() < 4 {
        return Err(TjsError::bad_param_count());
    }
    let x = need_int(&a, 0, "drawGlyph")?;
    let y = need_int(&a, 1, "drawGlyph")?;
    let g = object_arg(&a[2])
        .ok_or_else(|| TjsError::runtime("drawGlyph requires an array descriptor"))?;
    let items = rt
        .array_elements(g)
        .ok_or_else(|| TjsError::runtime("drawGlyph descriptor is not an array"))?
        .to_vec();
    if items.len() < 8 {
        return Err(TjsError::runtime(
            "drawGlyph descriptor needs eight elements",
        ));
    }
    let w = u32::try_from(items[0].to_integer()?).map_err(|_| TjsError::runtime("glyph width"))?;
    let height =
        u32::try_from(items[1].to_integer()?).map_err(|_| TjsError::runtime("glyph height"))?;
    if w == 0 || height == 0 {
        return Ok(Variant::Void);
    }
    let origin = [items[2].to_integer()?, items[3].to_integer()?];
    for v in &items[4..7] {
        v.to_integer()?;
    }
    // Keep the bitmap before reading the optional colour count. The original
    // implementation overwrites its temporary variant with that count and
    // then reads it as an octet; do not reproduce that invalid-memory bug.
    let Variant::Octet(bytes) = &items[7] else {
        return Err(TjsError::runtime("glyph bitmap must be an octet"));
    };
    let levels = items
        .get(8)
        .map(Variant::to_integer)
        .transpose()?
        .unwrap_or(256) as u32;
    let spec = crysirror_image::coverage_bitmap::CoverageBitmap {
        size: [w, height],
        bytes: bytes.clone(),
        levels,
        color: argb(a[3].to_integer()?, Some(255)),
    };
    spec.validate()
        .map_err(|e| TjsError::runtime(e.to_string()))?;
    let opacity = int(&a, 4, 255)?;
    if opacity < 0 {
        return Err(unsupported(
            "drawGlyph negative opacity",
            "alpha-erasure glyph profile",
        ));
    }
    if opacity == 0 {
        return Ok(Variant::Void);
    }
    let font = layer_font(rt, h)?;
    let size = font_size(rt, font)?;
    let alias = crate::font_resources::alias(rt, font)?;
    let raster = crate::font_resources::raster_size(rt, font, &alias, size)?;
    let ascent = rt
        .host()
        .text_measurement
        .as_ref()
        .ok_or_else(|| TjsError::runtime("text measurement absent"))?
        .measure_simple(&alias, f32::from(raster.draw), "")
        .map_err(|e| TjsError::runtime(e.to_string()))?
        .ascent.round() as i64 + i64::from(raster.offset_y);
    let dx = x
        .checked_add(origin[0])
        .ok_or_else(|| TjsError::runtime("glyph x overflow"))?;
    let dy = y
        .checked_sub(origin[1])
        .and_then(|y| y.checked_add(ascent))
        .ok_or_else(|| TjsError::runtime("glyph y overflow"))?;
    let to_i32 = |v: i64| i32::try_from(v).map_err(|_| TjsError::runtime("glyph coordinate range"));
    prepare(rt, h)?;
    let l = rt.host().layer(h)?.clone();
    let mut input = crate::image_api::input(rt, h)?;
    let mask = |color: [u8; 4], shadow: Option<(u32, i32)>| -> Result<ImageInput> {
        let mut s = spec.clone();
        s.color = color;
        let mut effects = vec![json!({"kind":"coverage_bitmap","spec":s})];
        if let Some((radius, level)) = shadow {
            effects.push(json!({"kind":"glyph_coverage_blur","radius":radius,"level":level}));
        }
        Ok(ImageInput::Process {
            input: Box::new(ImageInput::Solid {
                size: [w, height],
                color: [0; 4],
            }),
            effects: serde_json::to_vec(&effects)
                .map_err(|e| TjsError::runtime(e.to_string()))?
                .into(),
        })
    };
    let clip = l
        .draw_clip
        .map(|r| r.map(|v| i32::try_from(v).unwrap_or(i32::MAX)));
    let math = if rt.object_member(h, "__lp_holdAlpha").is_truthy() {
        ImageCompositeMath::MaskHoldAlphaScalar256
    } else if draw_face(rt, h) == 1 {
        ImageCompositeMath::MaskScalar256
    } else {
        ImageCompositeMath::AlphaOnAlphaScalar256
    };
    let level = int(&a, 6, 0)?;
    let radius = int(&a, 8, 0)?.unsigned_abs();
    if level != 0 {
        if radius > 64 || !(0..=65535).contains(&level) {
            return Err(TjsError::runtime("glyph shadow range"));
        }
        let sw = w + 2 * radius as u32;
        let sh = height + 2 * radius as u32;
        input = input.with_composite_layer(ImageCompositeLayer {
            input: Box::new(mask(
                argb(int(&a, 7, 0)?, Some(255)),
                Some((radius as u32, level as i32)),
            )?),
            source: [0, 0, sw as i32, sh as i32],
            destination: [
                to_i32(dx + int(&a, 9, 0)? - radius as i64)?,
                to_i32(dy + int(&a, 10, 0)? - radius as i64)?,
            ],
            clip,
            opacity: opacity.min(255) as u8,
            math,
        });
    }
    input = input.with_composite_layer(ImageCompositeLayer {
        input: Box::new(mask(spec.color, None)?),
        source: [0, 0, w as i32, height as i32],
        destination: [to_i32(dx)?, to_i32(dy)?],
        clip,
        opacity: opacity.min(255) as u8,
        math,
    });
    crate::image_api::queue_chain(rt, h, input, vec![])
}
pub(super) fn rect(rt: &mut Rt, bounds: [i32; 4]) -> Variant {
    let h = rt.alloc_ordinary_object();
    rt.add_object_class_info(h, "Rect");
    for (key, value) in ["left", "top", "right", "bottom"].into_iter().zip(bounds) {
        rt.set_object_member(h, key, Variant::Integer(value as i64));
    }
    for (name, a, b) in [("width", "left", "right"), ("height", "top", "bottom")] {
        rt.register_object_native_property(
            h,
            name,
            move |rt: &mut Rt, t| {
                let h = this_of(rt, t, "Rect extent")?;
                Ok(Variant::Integer(
                    rt.resolve_object_member(h, b)?.to_integer()?
                        - rt.resolve_object_member(h, a)?.to_integer()?,
                ))
            },
            move |rt: &mut Rt, t, v| {
                let h = this_of(rt, t, "Rect extent")?;
                let end = rt
                    .resolve_object_member(h, a)?
                    .to_integer()?
                    .checked_add(v.to_integer()?)
                    .ok_or_else(|| TjsError::runtime("Rect extent overflow"))?;
                rt.set_object_member(h, b, Variant::Integer(end));
                Ok(())
            },
        );
    }
    Variant::self_bound(h)
}
pub(super) fn install_window(rt: &mut Rt, h: ObjectHandle) {
    methods(
        rt,
        h,
        &[
            ("beginMove", begin_move),
            ("removeMaskRegion", remove_mask),
            ("postInputEvent", post_input),
        ],
    );
}
