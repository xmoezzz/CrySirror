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
            let alias = crate::font_resources::alias(rt, font)?;
            let raster = crate::font_resources::raster_size(rt, font, &alias, size)?;
            let mapped = crate::font_resources::is_mapped(rt, font)?;
            let bold = !mapped && rt.resolve_object_member(font, "bold")?.is_truthy();
            let underline = !mapped && rt.resolve_object_member(font, "underline")?.is_truthy();
            let strikeout = !mapped && rt.resolve_object_member(font, "strikeout")?.is_truthy();
            let mut bounds = rt
                .host()
                .text_measurement
                .as_ref()
                .ok_or_else(|| TjsError::runtime("text measurement absent"))?
                .ink_bounds_simple(&alias, f32::from(raster.draw), &text, bold, underline, strikeout)
                .map_err(|e| TjsError::runtime(e.to_string()))?;
            if bounds != [0; 4] {
                bounds[1] += raster.offset_y;
                bounds[3] += raster.offset_y;
            }
            let angle = rt.resolve_object_member(font, "angle")?.to_integer()?;
            let bounds = crysirror_text::decoration::rotate_bounds(
                bounds,
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
                let angle = rt.resolve_object_member(font, "angle")?.to_integer()?;
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
