//! Native Font objects use the shared engine measurement/drawing path.
//! Reference: krkrz fd5c4baa6a2ef5978db1bd043634351f48667daf,
//! LayerIntf.cpp:9897-10272, LayerBitmapImpl.cpp:59-100.
//! Native rasterizer indices remain compatibility preferences; both route to
//! CrySirror's portable outline backend. This does not implement Win32 GDI or
//! promise GDI/FreeType pixel identity. A selection reports that distinction.
use super::*;

pub(super) fn install(rt: &mut Rt) {
    let font = native_class(rt, "Font", |rt, this, args| {
        if !args.is_empty() {
            return Err(unsupported(
                "Font(layer)",
                "an explicitly bound engine font handle",
            ));
        }
        let font = this_of(rt, this, "Font")?;
        initialize(rt, font);
        Ok(Variant::Void)
    });
    initialize(rt, font);
}
pub(super) fn initialize(rt: &mut Rt, font: ObjectHandle) {
    rt.add_object_class_info(font, "Font");
    font_properties(rt, font);
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
