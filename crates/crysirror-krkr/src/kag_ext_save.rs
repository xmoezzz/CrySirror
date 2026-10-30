//! Original ExtKAGParser dictionary layout, including suspended call/macro
//! buffers and typed argument/local dictionaries. No private save appendix.
use super::*;
use crysirror_kag::{NativeCall, NativeExtControl, NativePosition};

fn put(rt: &mut Rt, h: ObjectHandle, name: &str, value: Variant) {
    rt.set_object_member(h, name, value);
}
fn number(rt: &Rt, h: ObjectHandle, name: &str, max: usize) -> Result<usize> {
    let n = rt.object_member(h, name).to_integer()?;
    if n < 0 || n as u64 > max as u64 {
        return Err(error(format!("native Ext save {name} bounds")));
    }
    Ok(n as usize)
}
fn text(rt: &Rt, h: ObjectHandle, name: &str) -> Result<String> {
    let value = rt.object_member(h, name).to_tjs_string()?;
    if value.len() > 4 * 1024 * 1024 {
        return Err(error("native Ext save string budget"));
    }
    Ok(value)
}
fn array(rt: &Rt, h: ObjectHandle, name: &str, max: usize) -> Result<Vec<Variant>> {
    let object = rt
        .object_member(h, name)
        .object_handle()
        .ok_or_else(|| error(format!("native Ext save {name} array missing")))?;
    let values = rt
        .array_elements(object)
        .ok_or_else(|| error("native Ext save requires Array"))?;
    if values.len() > max {
        return Err(error("native Ext save array budget"));
    }
    Ok(values.to_vec())
}
fn object(value: &Variant) -> Result<ObjectHandle> {
    value
        .object_handle()
        .ok_or_else(|| error("native Ext save dictionary required"))
}
fn position(rt: &Rt, h: ObjectHandle) -> Result<NativePosition> {
    Ok(NativePosition {
        storage: text(rt, h, "storage")?,
        label: text(rt, h, "label")?,
        offset: number(rt, h, "offset", 1_000_000)?,
        original_line: text(rt, h, "orgLineStr")?,
        line_buffer: text(rt, h, "lineBuffer")?,
        position: number(rt, h, "pos", 4 * 1024 * 1024)?,
        using_buffer: rt.object_member(h, "lineBufferUsing").is_truthy(),
        conditions: crate::kag_save::read_conditions(rt, h)?,
    })
}
fn write_position(rt: &mut Rt, h: ObjectHandle, p: &NativePosition, full: bool) {
    for (name, value) in [
        ("storage", p.storage.clone()),
        ("label", p.label.clone()),
        ("orgLineStr", p.original_line.clone()),
        ("lineBuffer", p.line_buffer.clone()),
    ] {
        put(rt, h, name, Variant::String(value));
    }
    for (name, value) in [
        ("offset", p.offset),
        ("pos", p.position),
        ("lineBufferUsing", p.using_buffer as usize),
    ] {
        put(rt, h, name, Variant::Integer(value as i64));
    }
    if full {
        crate::kag_save::conditions(rt, h, &p.conditions);
    } else {
        put(
            rt,
            h,
            "ExcludeLevel",
            Variant::Integer(p.conditions.exclude_level),
        );
        put(rt, h, "IfLevel", Variant::Integer(p.conditions.if_level));
    }
}
pub(super) fn augment(
    rt: &mut Rt,
    h: ObjectHandle,
    owner: ObjectHandle,
    p: &KagParser,
) -> Result<()> {
    let control = p.native_ext_control().map_err(error)?;
    let values = array(rt, owner, "__kag_arg_stack", 1025)?;
    if values.len() <= control.macro_depth {
        return Err(error("native Ext argument ownership mismatch"));
    }
    let mut args = Vec::new();
    for value in values.iter().skip(1).take(control.macro_depth) {
        args.push(Variant::Object(extended::copy_struct(rt, object(value)?)?));
    }
    let args = rt.alloc_array_object(args);
    put(rt, h, "macroArgs", Variant::Object(args));
    let mut calls = Vec::new();
    let mut whiles = Vec::new();
    for call in &control.calls {
        let row = rt.alloc_dictionary_object();
        write_position(rt, row, &call.position, true);
        for (name, value) in [
            ("macroArgStackBase", call.macro_base),
            ("macroArgStackDepth", call.macro_depth),
            ("WhileStackDepth", call.while_depth),
            ("LocalVariablesCount", call.local_count),
        ] {
            put(rt, row, name, Variant::Integer(value as i64));
        }
        calls.push(Variant::Object(row));
        // Native PushCallStack pushes this while sentinel before recording
        // its stack depth, even when no while loop is active.
        let sentinel = rt.alloc_dictionary_object();
        write_position(rt, sentinel, &call.position, false);
        for key in ["WhileLevelExp", "WhileLevelEach"] {
            put(rt, sentinel, key, Variant::String(String::new()));
        }
        whiles.push(Variant::Object(sentinel));
    }
    let calls = rt.alloc_array_object(calls);
    put(rt, h, "callStack", Variant::Object(calls));
    let whiles = rt.alloc_array_object(whiles);
    put(rt, h, "whileStack", Variant::Object(whiles));
    rt.delete_object_member(h, "WhileStack");
    put(
        rt,
        h,
        "macroArgStackBase",
        Variant::Integer(control.macro_base as i64),
    );
    put(
        rt,
        h,
        "macroArgStackDepth",
        Variant::Integer(control.macro_depth as i64),
    );
    put(
        rt,
        h,
        "lineBuffer",
        Variant::String(control.current.line_buffer),
    );
    put(
        rt,
        h,
        "lineBufferUsing",
        Variant::Integer(control.current.using_buffer as i64),
    );
    put(
        rt,
        h,
        "curPos",
        Variant::Integer(control.current.position as i64),
    );
    let line = p
        .label_line(&control.current.storage, &control.current.label)
        .unwrap_or(0)
        + control.current.offset;
    put(rt, h, "curLine", Variant::Integer(line as i64));
    Ok(())
}
fn decode(rt: &Rt, h: ObjectHandle) -> Result<(NativeExtControl, Vec<Vec<Attribute>>)> {
    let rows = array(rt, h, "callStack", 128)?;
    let loops = array(rt, h, "whileStack", 128)?;
    if rows.len() != loops.len()
        || !text(rt, h, "WhileLevelExp")?.is_empty()
        || !text(rt, h, "WhileLevelEach")?.is_empty()
    {
        return Err(unsupported(
            "ExtKAGParser.restore",
            "active original while-loop save mapping",
        ));
    }
    let mut calls = Vec::new();
    for (row, sentinel) in rows.iter().zip(loops.iter()) {
        let row = object(row)?;
        let sentinel = object(sentinel)?;
        let p = position(rt, row)?;
        // A genuine active while row is not interchangeable with a call's
        // sentinel. Reject it instead of clearing loops to make a save load.
        for (key, value) in [
            ("storage", p.storage.as_str()),
            ("label", p.label.as_str()),
            ("orgLineStr", p.original_line.as_str()),
            ("lineBuffer", p.line_buffer.as_str()),
        ] {
            if text(rt, sentinel, key)? != value {
                return Err(unsupported(
                    "ExtKAGParser.restore",
                    "active original while-loop checkpoint",
                ));
            }
        }
        for (key, value) in [
            ("offset", p.offset),
            ("pos", p.position),
            ("lineBufferUsing", p.using_buffer as usize),
        ] {
            if number(rt, sentinel, key, 4 * 1024 * 1024)? != value {
                return Err(error("native Ext call/while sentinel mismatch"));
            }
        }
        if !text(rt, sentinel, "WhileLevelExp")?.is_empty()
            || !text(rt, sentinel, "WhileLevelEach")?.is_empty()
        {
            return Err(unsupported(
                "ExtKAGParser.restore",
                "active original while-loop checkpoint",
            ));
        }
        calls.push(NativeCall {
            position: p,
            macro_base: number(rt, row, "macroArgStackBase", 1024)?,
            macro_depth: number(rt, row, "macroArgStackDepth", 1024)?,
            while_depth: number(rt, row, "WhileStackDepth", 128)?,
            local_count: number(rt, row, "LocalVariablesCount", 1024)?,
        });
    }
    let locals = array(rt, h, "LocalVariables", 1024)?;
    let current = NativePosition {
        storage: text(rt, h, "storageName")?,
        label: text(rt, h, "curLabel")?,
        offset: 0,
        original_line: String::new(),
        line_buffer: text(rt, h, "lineBuffer")?,
        position: number(rt, h, "curPos", 4 * 1024 * 1024)?,
        using_buffer: rt.object_member(h, "lineBufferUsing").is_truthy(),
        conditions: crate::kag_save::read_conditions(rt, h)?,
    };
    let args = array(rt, h, "macroArgs", 1024)?
        .iter()
        .map(|v| extended::attrs(rt, object(v)?))
        .collect::<Result<Vec<_>>>()?;
    let control = NativeExtControl {
        current,
        calls,
        macro_base: number(rt, h, "macroArgStackBase", 1024)?,
        macro_depth: number(rt, h, "macroArgStackDepth", 1024)?,
        local_count: locals.len(),
    };
    if control.local_count != control.calls.last().map_or(1, |call| call.local_count) {
        return Err(unsupported(
            "ExtKAGParser.restore",
            "current label-local scope rewind mapping",
        ));
    }
    Ok((control, args))
}
pub(super) fn restore(
    rt: &mut Rt,
    this: Option<ObjectHandle>,
    data: ObjectHandle,
) -> Result<Variant> {
    let (control, args) = decode(rt, data)?;
    operate(rt, this, |p, host| {
        let mut staged = p.clone();
        let multiline = p.multi_line_tag_enabled();
        let numeric = p.numeric_macro_arguments_enabled();
        let macro_value = host.rt.object_member(data, "macros");
        let macros = if matches!(macro_value, Variant::Void) {
            staged
                .macro_definitions()
                .map(|(name, source)| (name.to_owned(), source.to_owned()))
                .collect::<Vec<_>>()
        } else {
            let dictionary = object(&macro_value).map_err(|e| host.wrap(e))?;
            host.rt
                .object_members(dictionary)
                .into_iter()
                .map(|(name, value)| match value {
                    Variant::String(source) => Ok((name, source)),
                    _ => Err(error("native macro source must be string")),
                })
                .collect::<Result<Vec<_>>>()
                .map_err(|e| host.wrap(e))?
        };
        for storage in std::iter::once(&control.current.storage)
            .chain(control.calls.iter().map(|call| &call.position.storage))
        {
            if staged.line_text_for_offset(storage, 0).is_none() {
                let source = host
                    .rt
                    .read_storage_text(storage, "")
                    .map_err(|e| host.wrap(e))?;
                staged.cache_native_scenario(storage.clone(), source)?;
            }
        }
        staged.restore_native_ext_control(control.clone(), args.clone(), macros.clone())?;
        extended::restore_values(host.rt, host.owner, data, &mut staged, multiline, numeric)
            .map_err(|e| host.wrap(e))?;
        extended::restore_saved_arguments(host.rt, host.owner, data).map_err(|e| host.wrap(e))?;
        staged.reload_native_scenario_with(
            control.current.storage.clone(),
            args.last().cloned().unwrap_or_default(),
            host,
        )?;
        staged.restore_native_ext_control(control, args, macros)?;
        extended::read(host.rt, host.owner, &mut staged).map_err(|e| host.wrap(e))?;
        *p = staged;
        host.rt
            .host_mut()
            .parsers
            .get_mut(&host.owner.0)
            .unwrap()
            .boundary = true;
        Ok(Variant::Void)
    })
}
