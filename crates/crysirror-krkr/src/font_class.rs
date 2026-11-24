//! Native Font objects use the shared engine measurement/drawing path.
//! Reference: krkrz fd5c4baa6a2ef5978db1bd043634351f48667daf,
//! LayerIntf.cpp:9897-10272, LayerBitmapImpl.cpp:59-100.
//! Native rasterizer indices remain compatibility preferences; both route to
//! CrySirror's portable outline backend. This does not implement Win32 GDI or
//! promise GDI/FreeType pixel identity. A selection reports that distinction.
use super::*;

pub(super) fn install(rt: &mut Rt) {
    let font = native_class(rt, "Font", |rt, this, _args| {
        let font = this_of(rt, this, "Font")?;
        initialize(rt, font);
        Ok(Variant::Void)
    });
    initialize(rt, font);
}
pub(super) fn initialize(rt: &mut Rt, font: ObjectHandle) {
    rt.add_object_class_info(font, "Font");
    font_properties(rt, font);
    // LayerIntf.cpp declares TJS_DECL_EMPTY_FINALIZE_METHOD for Font.
    rt.register_object_native(font, "finalize", |_: &mut Rt, _, _| Ok(Variant::Void));
    rt.register_object_native_with_arg_count(
        font,
        "getGlyphDrawRect",
        NativeArgCount::AtLeast(1),
        |rt: &mut Rt, t, a: Vec<Variant>| {
            let font = this_of(rt, t, "Font.getGlyphDrawRect")?;
            let text = string(&a, 0, "Font.getGlyphDrawRect")?;
            let size = font_metric_size(rt, font)?;
            let runs = crate::font_resources::text_runs(rt, font, size, &text)?;
            let styled = ["bold", "underline", "strikeout"].map(|name| font_property(rt, font, name).is_truthy());
            let measurement = rt
                .host()
                .text_measurement
                .as_ref()
                .ok_or_else(|| TjsError::runtime("text measurement absent"))?;
            let mut bounds = None::<[i32; 4]>;
            let mut pen = 0.0f32;
            let run_count = runs.len();
            for (i, run) in runs.into_iter().enumerate() {
                let [bold, underline, strikeout] = if run.mapped { [false; 3] } else { styled };
                let px = f32::from(run.raster.draw);
                let ink = measurement.ink_bounds_simple(&run.alias, px, run.text, bold, underline, strikeout)
                    .map_err(|e| TjsError::runtime(e.to_string()))?;
                if ink != [0; 4] {
                    let x = pen.round() as i32;
                    let y = run.raster.offset_y;
                    let next = [ink[0] + x, ink[1] + y, ink[2] + x, ink[3] + y];
                    bounds = Some(bounds.map_or(next, |previous| [
                        previous[0].min(next[0]), previous[1].min(next[1]),
                        previous[2].max(next[2]), previous[3].max(next[3]),
                    ]));
                }
                if i + 1 < run_count {
                    pen += measurement.measure_simple(&run.alias, px, run.text)
                        .map_err(|e| TjsError::runtime(e.to_string()))?.advance;
                }
            }
            let angle = font_property(rt, font, "angle").to_integer()?;
            let bounds = crysirror_text::decoration::rotate_bounds(
                bounds.unwrap_or([0; 4]),
                -(angle.rem_euclid(3600) as f64) / 10.,
            );
            crate::native_rect::from_bounds(rt, bounds)
        },
    );
    rt.register_object_native_with_arg_count(
        font,
        "getList",
        NativeArgCount::AtLeast(1),
        |rt: &mut Rt, t, a: Vec<Variant>| {
            let font = this_of(rt, t, "Font.getList")?;
            let flags = need_int(&a, 0, "Font.getList")? as u32;
            let alias = crate::font_resources::alias(rt, font)?;
            let catalog = rt
                .host()
                .fonts
                .library
                .as_ref()
                .ok_or_else(|| TjsError::runtime("font library absent"))?
                .catalog()
                .map_err(|e| TjsError::runtime(e.to_string()))?;
            let reference = catalog
                .iter()
                .find(|f| f.alias == alias)
                .and_then(|f| f.code_pages);
            let mut names = std::collections::BTreeSet::new();
            for face in catalog {
                if flags & 1 != 0 && !face.fixed_pitch
                    || flags & 8 != 0 && !face.outline
                    || flags & 16 != 0 && face.code_pages.is_some_and(|p| p[0] & (1 << 31) != 0)
                {
                    continue;
                }
                if flags & 2 != 0
                    && reference.is_some_and(|r| {
                        face.code_pages
                            .is_none_or(|p| (r[0] & p[0]) | (r[1] & p[1]) == 0)
                    })
                {
                    continue;
                }
                names.extend(
                    face.names
                        .into_iter()
                        .filter(|n| flags & 4 == 0 || !n.starts_with('@')),
                );
            }
            Ok(Variant::self_bound(rt.alloc_array_object(
                names.into_iter().map(Variant::String).collect(),
            )))
        },
    );
    rt.register_object_native_with_arg_count(
        font,
        "mapPrerenderedFont",
        NativeArgCount::AtLeast(1),
        |rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>| {
            let font = this_of(rt, t, "Font.mapPrerenderedFont")?;
            let storage = string(&a, 0, "Font.mapPrerenderedFont")?;
            crate::font_resources::map(rt, font, storage)
        },
    );
    rt.register_object_native_with_arg_count(
        font,
        "unmapPrerenderedFont",
        NativeArgCount::Any,
        |rt: &mut Rt, t: Option<ObjectHandle>, _: Vec<Variant>| {
            let font = this_of(rt, t, "Font.unmapPrerenderedFont")?;
            crate::font_resources::unmap(rt, font)
        },
    );
    rt.register_object_native_with_arg_count(
        font,
        "getTextWidth",
        NativeArgCount::AtLeast(1),
        font_text_width,
    );
    rt.register_object_native_with_arg_count(
        font,
        "getTextHeight",
        NativeArgCount::AtLeast(1),
        font_text_height,
    );
    // Native angle units are tenths of a degree. These are advance-vector
    // components, not glyph ink bounds. Keep all measurement in the shared
    // text service and apply only the native coordinate convention here.
    for (name, width, x) in [
        ("getEscWidthX", true, true),
        ("getEscWidthY", true, false),
        ("getEscHeightX", false, true),
        ("getEscHeightY", false, false),
    ] {
        rt.register_object_native_with_arg_count(
            font,
            name,
            NativeArgCount::AtLeast(1),
            move |rt: &mut Rt, this: Option<ObjectHandle>, args: Vec<Variant>| {
                let font = this_of(rt, this, name)?;
                let angle = font_property(rt, font, "angle").to_integer()?;
                let angle = (angle.rem_euclid(3600) as f64) * std::f64::consts::PI / 1800.;
                let extent = if width {
                    font_text_width(rt, Some(font), args)?.to_integer()? as f64
                } else {
                    font_text_height(rt, Some(font), args)?.to_integer()? as f64
                };
                let component = match (width, x) {
                    (true, true) | (false, false) => angle.cos(),
                    (true, false) => -angle.sin(),
                    (false, true) => angle.sin(),
                };
                Ok(Variant::Real(component * extent))
            },
        );
    }
    rt.register_object_native_property(font, "rasterizer",
        |rt, _| Ok(Variant::Integer(rt.host().font_rasterizer_preference)),
        |rt, _, value| {
            let preference = value.to_integer()?;
            if !matches!(preference, 0 | 1) {
                return Err(unsupported("Font.rasterizer", "known frFreeType/frGDI compatibility preference"));
            }
            rt.host_mut().font_rasterizer_preference = preference;
            rt.host_mut().push_log(format!("Font.rasterizer={preference}: CrySirror portable outline compatibility profile; native GDI/FreeType raster pixels are not implemented"));
            Ok(())
        });
}
