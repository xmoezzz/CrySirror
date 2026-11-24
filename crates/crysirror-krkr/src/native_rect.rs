// SPDX-License-Identifier: MPL-2.0
//! TJS Rect uses signed edges, including empty and inverted rectangles.
//! Reference: krkrz fd5c4baa6a2ef5978db1bd043634351f48667daf,
//! visual/RectItf.cpp, RectItf.h and ComplexRect.{h,cpp}.
use crate::natives::{Rt, native_class, need_int, object_arg, read_only, this_of, unsupported};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{NativeArgCount, ObjectHandle, Variant},
};

const CLASS: &str = "__crysirror_native_rect_class";
const FIELDS: [&str; 4] = ["__rect_left", "__rect_top", "__rect_right", "__rect_bottom"];
type Edges = [i32; 4];

fn integer(value: i32) -> Variant {
    Variant::Integer(i64::from(value))
}

fn edges(rt: &Rt, object: ObjectHandle) -> Result<Edges> {
    if !rt.object_valid(object)
        || !rt.has_object_member(object, FIELDS[0])
        || !rt
            .object_class_infos(object)
            .iter()
            .any(|name| name == "Rect")
    {
        return Err(TjsError::runtime("Rect instance required"));
    }
    let mut result = [0; 4];
    for (field, edge) in FIELDS.into_iter().zip(&mut result) {
        *edge = rt.object_member(object, field).to_integer()? as i32;
    }
    Ok(result)
}

fn put(rt: &mut Rt, object: ObjectHandle, bounds: Edges) {
    for (field, edge) in FIELDS.into_iter().zip(bounds) {
        rt.set_object_member(object, field, integer(edge));
    }
}

fn coords(args: &[Variant], count: usize, method: &str) -> Result<Edges> {
    let mut result = [0; 4];
    for (i, edge) in result.iter_mut().take(count).enumerate() {
        *edge = need_int(args, i, method)? as i32;
    }
    Ok(result)
}

fn empty([left, top, right, bottom]: Edges) -> bool {
    left >= right || top >= bottom
}

fn intersection(a: Edges, b: Edges) -> Edges {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

fn union(a: Edges, b: Edges) -> Edges {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

pub(crate) fn from_bounds(rt: &mut Rt, bounds: Edges) -> Result<Variant> {
    let class = rt
        .global_member(CLASS)
        .object_handle()
        .filter(|class| rt.object_valid(*class))
        .ok_or_else(|| TjsError::runtime("native Rect class unavailable"))?;
    let object = rt.alloc_ordinary_object();
    rt.set_object_super_class(object, class);
    rt.add_object_class_info(object, "Rect");
    put(rt, object, bounds);
    Ok(Variant::self_bound(object))
}

pub(crate) fn install(rt: &mut Rt) {
    let class = native_class(rt, "Rect", |rt, this, args| {
        let object = this_of(rt, this, "Rect")?;
        let bounds = match args.as_slice() {
            [source] => edges(
                rt,
                object_arg(source).ok_or_else(|| TjsError::runtime("Rect copy requires a Rect"))?,
            )?,
            [_, _, _, _] => coords(&args, 4, "Rect")?,
            _ => [0; 4],
        };
        put(rt, object, bounds);
        Ok(Variant::Void)
    });
    rt.set_global_member(CLASS, Variant::Object(class));
    for (i, name) in ["left", "top", "right", "bottom"].into_iter().enumerate() {
        rt.register_object_native_property(
            class,
            name,
            move |rt: &mut Rt, this| Ok(integer(edges(rt, this_of(rt, this, "Rect")?)?[i])),
            move |rt: &mut Rt, this, value| {
                let object = this_of(rt, this, "Rect")?;
                let mut bounds = edges(rt, object)?;
                bounds[i] = value.to_integer()? as i32;
                put(rt, object, bounds);
                Ok(())
            },
        );
    }
    for (i, name) in ["width", "height"].into_iter().enumerate() {
        rt.register_object_native_property(
            class,
            name,
            move |rt: &mut Rt, this| {
                let bounds = edges(rt, this_of(rt, this, "Rect")?)?;
                Ok(integer(bounds[i + 2].wrapping_sub(bounds[i])))
            },
            move |rt: &mut Rt, this, value| {
                let object = this_of(rt, this, "Rect")?;
                let mut bounds = edges(rt, object)?;
                bounds[i + 2] = bounds[i].wrapping_add(value.to_integer()? as i32);
                put(rt, object, bounds);
                Ok(())
            },
        );
    }
    rt.register_object_native_property(
        class,
        "nativeArray",
        |_: &mut Rt, _| Err(unsupported("Rect.nativeArray", "native pointer interop")),
        read_only,
    );
    rt.register_object_native_with_arg_count(
        class,
        "isEmpty",
        NativeArgCount::Any,
        |rt: &mut Rt, this, _| {
            Ok(integer(i32::from(empty(edges(
                rt,
                this_of(rt, this, "Rect.isEmpty")?,
            )?))))
        },
    );
    rt.register_object_native_with_arg_count(
        class,
        "includedPos",
        NativeArgCount::AtLeast(2),
        |rt: &mut Rt, this, args: Vec<Variant>| {
            let [left, top, right, bottom] = edges(rt, this_of(rt, this, "Rect.includedPos")?)?;
            let [x, y, ..] = coords(&args, 2, "Rect.includedPos")?;
            Ok(integer(i32::from(
                left <= x && x < right && top <= y && y < bottom,
            )))
        },
    );
    for (name, count) in [
        ("setSize", 2),
        ("setOffset", 2),
        ("addOffset", 2),
        ("clear", 0),
        ("set", 4),
    ] {
        rt.register_object_native_with_arg_count(
            class,
            name,
            NativeArgCount::AtLeast(count),
            move |rt: &mut Rt, this, args: Vec<Variant>| {
                let object = this_of(rt, this, name)?;
                let [l, t, r, b] = edges(rt, object)?;
                let values = coords(&args, count, name)?;
                let [x, y, ..] = values;
                let updated = match name {
                    "setSize" => [l, t, l.wrapping_add(x), t.wrapping_add(y)],
                    "setOffset" => [
                        x,
                        y,
                        x.wrapping_add(r.wrapping_sub(l)),
                        y.wrapping_add(b.wrapping_sub(t)),
                    ],
                    "addOffset" => [
                        l.wrapping_add(x),
                        t.wrapping_add(y),
                        r.wrapping_add(x),
                        b.wrapping_add(y),
                    ],
                    "clear" | "set" => values,
                    _ => unreachable!(),
                };
                put(rt, object, updated);
                Ok(Variant::Void)
            },
        );
    }
    for name in ["clip", "union", "intersects", "included", "equal"] {
        rt.register_object_native_with_arg_count(
            class,
            name,
            NativeArgCount::AtLeast(1),
            move |rt: &mut Rt, this, args: Vec<Variant>| {
                let object = this_of(rt, this, name)?;
                let current = edges(rt, object)?;
                if matches!(args[0], Variant::Null) {
                    return Ok(Variant::Void);
                }
                let other = edges(
                    rt,
                    object_arg(&args[0])
                        .ok_or_else(|| TjsError::runtime("Rect argument required"))?,
                )?;
                let matched = match name {
                    "clip" | "union" => {
                        let updated = if name == "clip" {
                            intersection(current, other)
                        } else {
                            union(current, other)
                        };
                        if empty(updated) {
                            false
                        } else {
                            put(rt, object, updated);
                            true
                        }
                    }
                    "intersects" => !empty(intersection(current, other)),
                    "included" => {
                        !empty(current) && !empty(other) && intersection(current, other) == current
                    }
                    "equal" => current == other,
                    _ => unreachable!(),
                };
                Ok(integer(i32::from(matched)))
            },
        );
    }
}
