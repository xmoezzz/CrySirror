// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! msdfrender.dll Layer members that turn a rendered layer rectangle into a
//! `Layer.drawGlyph` coverage bitmap.
//!
//! Native findings (IDA on game-kr `plugin/msdfrender.dll`): registration
//! 0x10002CC0, rectangle arguments 0x100024A0 (`x, y, width, height`,
//! defaulting to the whole image and clipped to it), conversion 0x10004A00
//! answering `[width, height, octet]`:
//!
//! * `makeGlyphSDF(zoom, x, y, w, h)` 0x10002300/0x100023F0: each alpha
//!   byte `d` becomes `clamp(c·d + ((0x10000 − c) << 7), 0, 0xFFFFFF) >> 16`
//!   with `c = int(zoom·65536)`, i.e. contrast around the 128 edge.
//! * `makeGlyphMSDF(...)`: the same on the median of the three colour bytes.
//! * `makeBitmapFromProvince(x, y, w, h)`: the province bytes unchanged.
//! * `operateGlyphToProvince(x, y, w, h, octet)` 0x10004E20: saturating add
//!   of the glyph bytes into the province image.
//! * `getAscentHeight()` 0x10004620: the layer font's ascent.
//!
//! * `Scripts.makeColorTable(array)` 0x10004800 and
//!   `copyGlyphByColorTable(table, x, y, w, h, glyph)` 0x100043D0.
use crate::natives::{Rt, this_of};
use crate::province_ex::{paint, pixels};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};

fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(e.to_string())
}
/// `[x, y, w, h]` from arguments starting at `at`, clipped to the image.
fn rect(rt: &Rt, h: ObjectHandle, a: &[Variant], at: usize) -> Result<Option<[u32; 4]>> {
    let l = rt.host().layer(h)?;
    let arg = |i: usize, default: i64| -> Result<i64> {
        match a.get(at + i) {
            Some(v) if !matches!(v, Variant::Void) => v.to_integer(),
            _ => Ok(default),
        }
    };
    let (x, y) = (arg(0, 0)?, arg(1, 0)?);
    let (w, hh) = (arg(2, l.image_width)?, arg(3, l.image_height)?);
    let (x0, y0) = (x.clamp(0, l.image_width), y.clamp(0, l.image_height));
    let (x1, y1) = ((x + w).clamp(x0, l.image_width), (y + hh).clamp(y0, l.image_height));
    Ok((x1 > x0 && y1 > y0).then_some([x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32]))
}
fn glyph(rt: &mut Rt, r: [u32; 4], bytes: Vec<u8>) -> Variant {
    let arr = rt.alloc_array_object(vec![Variant::Integer(r[2].into()), Variant::Integer(r[3].into()), Variant::Octet(bytes.into())]);
    Variant::Object(arr)
}
fn make(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>, what: &str, median: bool) -> Result<Variant> {
    let h = this_of(rt, t, what)?;
    let zoom = a.first().ok_or_else(TjsError::bad_param_count)?.to_real()?;
    let Some(r) = rect(rt, h, &a, 1)? else {
        return Err(err(format!("{what}: empty rectangle")));
    };
    let (size, rgba) = pixels(rt, h, what)?;
    let c = (zoom * 65536.) as i64;
    let bias = (0x10000 - c) << 7;
    let mut out = Vec::with_capacity((r[2] * r[3]) as usize);
    for y in r[1]..r[1] + r[3] {
        for x in r[0]..r[0] + r[2] {
            let p = &rgba[((y * size[0] + x) * 4) as usize..][..4];
            let d = if median {
                let (lo, hi) = (p[0].min(p[1]), p[0].max(p[1]));
                lo.max(hi.min(p[2]))
            } else {
                p[3]
            };
            out.push(((c * i64::from(d) + bias).clamp(0, 0xFF_FFFF) >> 16) as u8);
        }
    }
    Ok(glyph(rt, r, out))
}
pub(crate) fn install(rt: &mut Rt) {
    let Some(class) = rt.global_member("Layer").object_handle() else { return };
    crate::natives::methods(
        rt,
        class,
        &[
            ("makeGlyphSDF", |rt, t, a| make(rt, t, a, "Layer.makeGlyphSDF", false)),
            ("makeGlyphMSDF", |rt, t, a| make(rt, t, a, "Layer.makeGlyphMSDF", true)),
            ("makeBitmapFromProvince", |rt, t, a| {
                let h = this_of(rt, t, "Layer.makeBitmapFromProvince")?;
                let Some(r) = rect(rt, h, &a, 0)? else {
                    return Err(err("Layer.makeBitmapFromProvince: empty rectangle"));
                };
                let labels = rt.host().layer(h)?.regions.labels(rt.host().client, r).map_err(err)?;
                Ok(glyph(rt, r, labels.into_iter().map(|v| v as u8).collect()))
            }),
            ("operateGlyphToProvince", |rt, t, a| {
                const M: &str = "Layer.operateGlyphToProvince";
                let h = this_of(rt, t, M)?;
                if a.len() < 5 {
                    return Err(TjsError::bad_param_count());
                }
                let (x, y, w, hh) = (a[0].to_integer()?, a[1].to_integer()?, a[2].to_integer()?, a[3].to_integer()?);
                let Variant::Octet(bytes) = &a[4] else {
                    return Err(err(format!("{M}: glyph bitmap must be an octet")));
                };
                if w <= 0 || hh <= 0 || (bytes.len() as i64) < w * hh {
                    return Err(err(format!("{M}: glyph bitmap is smaller than its size")));
                }
                let Some(r) = rect(rt, h, &a, 0)? else { return Ok(Variant::Void) };
                let old = rt.host().layer(h)?.regions.labels(rt.host().client, r).map_err(err)?;
                let mut out = Vec::with_capacity(old.len());
                for (i, o) in old.iter().enumerate() {
                    let (px, py) = (i64::from(r[0]) + (i as i64 % i64::from(r[2])), i64::from(r[1]) + (i as i64 / i64::from(r[2])));
                    let g = bytes[((py - y) * w + (px - x)) as usize];
                    out.push((*o as u8).saturating_add(g));
                }
                paint(rt, h, r, out)?;
                Ok(Variant::Void)
            }),
            ("getAscentHeight", |rt, t, _| {
                let h = this_of(rt, t, "Layer.getAscentHeight")?;
                let font = crate::natives::layer_font(rt, h)?;
                let size = crate::natives::font_size(rt, font)?;
                let alias = crate::font_resources::alias(rt, font)?;
                let raster = crate::font_resources::raster_size(rt, font, &alias, size)?;
                let Some(measure) = rt.host().text_measurement.as_ref() else { return Ok(Variant::Void) };
                Ok(match measure.measure_simple(&alias, f32::from(raster.draw), "") {
                    Ok(m) => Variant::Integer((m.ascent + raster.offset_y as f32).round() as i64),
                    Err(_) => Variant::Void,
                })
            }),
            // copyGlyphByColorTable(table, x, y, w, h, glyph) 0x100043D0:
            // each glyph byte selects a raw ARGB pixel from the table.
            ("copyGlyphByColorTable", |rt, t, a| {
                const M: &str = "Layer.copyGlyphByColorTable";
                let h = this_of(rt, t, M)?;
                let Some(Variant::Octet(table)) = a.first() else {
                    return Err(err(format!("{M}: the colour table must be an octet")));
                };
                if table.len() < 0x400 || a.len() < 6 {
                    return Err(err(format!("{M}: a 256-entry table and a glyph are required")));
                }
                let (x, y, w, hh) = (a[1].to_integer()?, a[2].to_integer()?, a[3].to_integer()?, a[4].to_integer()?);
                let Variant::Octet(bytes) = &a[5] else {
                    return Err(err(format!("{M}: glyph bitmap must be an octet")));
                };
                if w <= 0 || hh <= 0 || (bytes.len() as i64) < w * hh {
                    return Err(err(format!("{M}: glyph bitmap is smaller than its size")));
                }
                let Some(r) = rect(rt, h, &a, 1)? else { return Ok(Variant::Void) };
                let (size, rgba) = pixels(rt, h, M)?;
                let mut out = rgba.to_vec();
                for py in r[1]..r[1] + r[3] {
                    for px in r[0]..r[0] + r[2] {
                        let g = bytes[((i64::from(py) - y) * w + (i64::from(px) - x)) as usize] as usize;
                        let c = &table[g * 4..g * 4 + 4];
                        let o = ((py * size[0] + px) * 4) as usize;
                        // Table entries are little-endian 0xAARRGGBB.
                        out[o..o + 4].copy_from_slice(&[c[2], c[1], c[0], c[3]]);
                    }
                }
                let image = crysirror_core::image_processing::RgbaImage::new(crysirror_core::state::ClientId(rt.host().client), size, out.into()).map_err(err)?;
                crate::image_api::queue_chain(rt, h, crysirror_core::image_processing::ImageInput::Resource(std::sync::Arc::new(image)), vec![])
            }),
        ],
    );
    // Scripts.makeColorTable(array) 0x10004800: 256 colours as an octet of
    // little-endian 32-bit values, 1028 bytes long.
    if let Some(scripts) = rt.global_member("Scripts").object_handle() {
        rt.register_object_native(scripts, "makeColorTable", |rt: &mut Rt, _, a: Vec<Variant>| {
            let list = a.first().and_then(crate::natives::object_arg).ok_or_else(|| err("Scripts.makeColorTable: an array is required"))?;
            let items = rt.array_elements(list).unwrap_or(&[]).to_vec();
            let mut out = vec![0u8; 1028];
            for (i, v) in items.iter().take(256).enumerate() {
                let c = match v {
                    Variant::Void => 0,
                    v => v.to_integer()? as u32,
                };
                out[i * 4..i * 4 + 4].copy_from_slice(&c.to_le_bytes());
            }
            Ok(Variant::Octet(out.into()))
        });
    }
}
