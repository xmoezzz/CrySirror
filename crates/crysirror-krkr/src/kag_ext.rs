//! Native ExtKAGParser object/value policy; scenario execution stays in the
//! first-class KAG VM and all rendering/storage remain CrySirror services.
use super::*;

fn member(rt: &Rt, h: ObjectHandle, key: &str) -> Result<ObjectHandle> {
    rt.object_member(h, key)
        .object_handle()
        .ok_or_else(|| error(format!("ExtKAGParser {key} dictionary missing")))
}
pub(super) fn attrs(rt: &Rt, h: ObjectHandle) -> Result<Vec<Attribute>> {
    rt.object_members(h)
        .into_iter()
        .map(|(name, value)| {
            Ok(Attribute::named(
                name,
                match value {
                    Variant::Void => AttributeValue::Void,
                    value => AttributeValue::Literal(value.to_tjs_string()?),
                },
            ))
        })
        .collect()
}
pub(super) fn copy_struct(rt: &mut Rt, from: ObjectHandle) -> Result<ObjectHandle> {
    let destination = rt.alloc_dictionary_object();
    let class = rt
        .global_member("Dictionary")
        .object_handle()
        .ok_or_else(|| error("Dictionary class missing"))?;
    let method = rt
        .object_member(class, "assignStruct")
        .object_handle()
        .ok_or_else(|| error("Dictionary.assignStruct missing"))?;
    rt.call_function(
        Variant::Closure(crysirror_tjs2::runtime::Closure::new(
            method,
            Some(destination),
        )),
        vec![Variant::Object(from)],
    )?;
    Ok(destination)
}
fn copy_array_struct(rt: &mut Rt, from: ObjectHandle) -> Result<ObjectHandle> {
    let destination = rt.alloc_array_object(Vec::new());
    let class = rt
        .global_member("Array")
        .object_handle()
        .ok_or_else(|| error("Array class missing"))?;
    let method = rt
        .object_member(class, "assignStruct")
        .object_handle()
        .ok_or_else(|| error("Array.assignStruct missing"))?;
    rt.call_function(
        Variant::Closure(crysirror_tjs2::runtime::Closure::new(
            method,
            Some(destination),
        )),
        vec![Variant::Object(from)],
    )?;
    Ok(destination)
}
fn copy_dict(rt: &mut Rt, from: ObjectHandle) -> ObjectHandle {
    let to = rt.alloc_dictionary_object();
    for (n, v) in rt.object_members(from) {
        rt.set_object_member(to, n, v);
    }
    to
}
/// ExtKAGParser's native operator= uses Array.assign for LocalVariables and
/// Dictionary.assign for ParamMacros, preserving contained object identities.
/// Its active MacroArgs dictionaries are separately allocated shallow copies.
pub(super) fn assign_values(
    rt: &mut Rt,
    destination: ObjectHandle,
    source: ObjectHandle,
) -> Result<()> {
    let locals = rt
        .array_elements(member(rt, source, "__kag_locals")?)
        .ok_or_else(|| error("ExtKAGParser localVariables array missing"))?
        .to_vec();
    let source_args = rt
        .array_elements(member(rt, source, "__kag_arg_stack")?)
        .ok_or_else(|| error("ExtKAGParser argument stack missing"))?
        .to_vec();
    let depth = rt.object_member(source, "__kag_arg_depth").to_integer()? as usize;
    let mut arguments = Vec::new();
    for value in source_args.into_iter().take(depth + 1) {
        arguments.push(Variant::Object(copy_dict(
            rt,
            value
                .object_handle()
                .ok_or_else(|| error("ExtKAGParser macro argument dictionary missing"))?,
        )));
    }
    let params = arguments
        .last()
        .cloned()
        .ok_or_else(|| error("ExtKAGParser argument base missing"))?;
    let macros = rt.object_members(member(rt, source, "__kag_pmacros")?);
    let destination_locals = member(rt, destination, "__kag_locals")?;
    rt.array_clear(destination_locals);
    for value in locals {
        rt.array_push(destination_locals, value);
    }
    let stack = rt.alloc_array_object(arguments);
    rt.set_object_member(destination, "__kag_arg_stack", Variant::Object(stack));
    rt.set_object_member(
        destination,
        "__kag_arg_depth",
        Variant::Integer(depth as i64),
    );
    rt.set_object_member(destination, "__kag_params", params);
    let destination_macros = member(rt, destination, "__kag_pmacros")?;
    for (name, _) in rt.object_members(destination_macros) {
        rt.delete_object_member(destination_macros, &name);
    }
    for (name, value) in macros {
        rt.set_object_member(destination_macros, name, value);
    }
    Ok(())
}
pub(super) fn initialize(rt: &mut Rt, h: ObjectHandle) {
    let base = rt.alloc_dictionary_object();
    let array = rt.alloc_array_object(vec![Variant::Object(base)]);
    rt.set_object_member(h, "__kag_locals", Variant::Object(array));
    for key in ["__kag_params", "__kag_pmacros"] {
        let object = rt.alloc_dictionary_object();
        rt.set_object_member(h, key, Variant::Object(object));
    }
    let params = rt.object_member(h, "__kag_params");
    let stack = rt.alloc_array_object(vec![params]);
    rt.set_object_member(h, "__kag_arg_stack", Variant::Object(stack));
    rt.set_object_member(h, "__kag_arg_depth", Variant::Integer(0));
}
pub(super) fn macro_values(rt: &Rt, h: ObjectHandle) -> Result<Vec<Attribute>> {
    attrs(rt, member(rt, h, "__kag_params")?)
}
pub(super) fn reset_arguments(rt: &mut Rt, h: ObjectHandle) {
    let base = rt.alloc_dictionary_object();
    let frames = rt.alloc_array_object(vec![Variant::Object(base)]);
    rt.set_object_member(h, "__kag_arg_stack", Variant::Object(frames));
    rt.set_object_member(h, "__kag_arg_depth", Variant::Integer(0));
    rt.set_object_member(h, "__kag_params", Variant::Object(base));
    rt.delete_object_member(h, "__kag_tag_values");
}
pub(super) fn restore_arguments(rt: &mut Rt, h: ObjectHandle, p: &KagParser) -> Result<()> {
    let depth = p.macro_argument_depth();
    let current = rt
        .array_elements(member(rt, h, "__kag_arg_stack")?)
        .and_then(|frames| frames.get(depth))
        .cloned()
        .ok_or_else(|| error("ExtKAGParser pending argument checkpoint missing"))?;
    rt.set_object_member(h, "__kag_arg_depth", Variant::Integer(depth as i64));
    rt.set_object_member(h, "__kag_params", current);
    Ok(())
}
pub(super) fn prepare_tag_values(
    rt: &mut Rt,
    h: ObjectHandle,
    original: &Tag,
    resolved: &Tag,
) -> Result<()> {
    let args = member(rt, h, "__kag_params")?;
    let mut overrides = std::collections::BTreeMap::new();
    for attribute in &original.attributes {
        match attribute {
            Attribute::Spread => {
                for (name, value) in rt.object_members(args) {
                    overrides.insert(name, value);
                }
            }
            Attribute::Named {
                name,
                value: AttributeValue::MacroArgument(spec),
            } => {
                let (source, default) = spec
                    .split_once('|')
                    .map_or((spec.as_str(), None), |(s, d)| (s, Some(d)));
                let value = match rt.object_member(args, source) {
                    Variant::Void => default.map_or(Variant::Void, |v| Variant::String(v.into())),
                    value => value,
                };
                overrides.insert(name.clone(), value);
            }
            Attribute::Named { name, .. } => {
                overrides.remove(name);
            }
        }
    }
    let values = rt.alloc_dictionary_object();
    for (name, value) in overrides {
        if name != "tagname"
            && name != "cond"
            && resolved
                .attributes
                .iter()
                .any(|a| a.name() == Some(name.as_str()))
        {
            rt.set_object_member(values, name, value);
        }
    }
    rt.set_object_member(h, "__kag_tag_values", Variant::Object(values));
    Ok(())
}
pub(super) fn apply_tag_values(
    rt: &mut Rt,
    owner: ObjectHandle,
    destination: ObjectHandle,
) -> Result<()> {
    if let Some(values) = rt.object_member(owner, "__kag_tag_values").object_handle() {
        for (name, value) in rt.object_members(values) {
            rt.set_object_member(destination, name, value);
        }
    }
    Ok(())
}
pub(super) fn reset_locals(rt: &mut Rt, h: ObjectHandle) -> Result<()> {
    let array = member(rt, h, "__kag_locals")?;
    let base = rt.alloc_dictionary_object();
    rt.array_clear(array);
    rt.array_push(array, Variant::Object(base));
    Ok(())
}
pub(super) fn read(rt: &Rt, h: ObjectHandle, parser: &mut KagParser) -> Result<()> {
    if !parser.is_ext_kag() {
        return Ok(());
    }
    parser.set_macro_params(attrs(rt, member(rt, h, "__kag_params")?)?);
    let macros = member(rt, h, "__kag_pmacros")?;
    let mut entries = Vec::new();
    for (n, v) in rt.object_members(macros) {
        entries.push((
            n,
            attrs(
                rt,
                v.object_handle()
                    .ok_or_else(|| error("ExtKAGParser parameter macro must be Dictionary"))?,
            )?,
        ));
    }
    parser.set_parameter_macros(entries).map_err(error)
}
pub(super) fn publish(rt: &mut Rt, h: ObjectHandle, parser: &KagParser) -> Result<()> {
    if !parser.is_ext_kag() {
        return Ok(());
    }
    let locals = member(rt, h, "__kag_locals")?;
    let mut values = rt
        .array_elements(locals)
        .ok_or_else(|| error("ExtKAGParser localVariables array missing"))?
        .to_vec();
    if values.len() < parser.local_variables_depth() {
        return Err(error("ExtKAGParser native local scope was not created"));
    }
    values.truncate(parser.local_variables_depth());
    rt.array_clear(locals);
    for v in values {
        rt.array_push(locals, v);
    }
    let stack = member(rt, h, "__kag_arg_stack")?;
    let mut frames = rt
        .array_elements(stack)
        .ok_or_else(|| error("ExtKAGParser argument stack missing"))?
        .to_vec();
    let depth = parser.macro_argument_depth();
    let previous = rt.object_member(h, "__kag_arg_depth").to_integer()? as usize;
    if depth > previous {
        if depth != previous + 1 {
            return Err(error("ExtKAGParser unobserved macro argument push"));
        }
        let incoming = dict(rt, parser.macro_params())?;
        apply_tag_values(rt, h, incoming)?;
        if depth == frames.len() {
            frames.push(Variant::Object(incoming));
        } else {
            let destination = frames[depth]
                .object_handle()
                .ok_or_else(|| error("ExtKAGParser macro argument dictionary missing"))?;
            for (name, _) in rt.object_members(destination) {
                rt.delete_object_member(destination, &name);
            }
            for (name, value) in rt.object_members(incoming) {
                rt.set_object_member(destination, name, value);
            }
        }
    }
    let current = frames
        .get(depth)
        .cloned()
        .ok_or_else(|| error("ExtKAGParser argument base missing"))?;
    rt.array_clear(stack);
    for frame in frames {
        rt.array_push(stack, frame);
    }
    rt.set_object_member(h, "__kag_params", current);
    rt.set_object_member(h, "__kag_arg_depth", Variant::Integer(depth as i64));
    let macros = member(rt, h, "__kag_pmacros")?;
    for (n, _) in rt.object_members(macros) {
        if !parser.parameter_macros().any(|(name, _)| name == n) {
            rt.delete_object_member(macros, &n);
        }
    }
    for (n, a) in parser.parameter_macros() {
        // A native shallow assign shares the contained dictionaries. Ordinary
        // parsing must not replace them just to publish an unchanged VM view.
        if let Some(existing) = rt.object_member(macros, n).object_handle() {
            if attrs(rt, existing)?.as_slice() == a {
                continue;
            }
        }
        let d = dict(rt, a)?;
        rt.set_object_member(macros, n, Variant::Object(d));
    }
    Ok(())
}
fn local(rt: &Rt, h: ObjectHandle, previous: bool) -> Result<Option<ObjectHandle>> {
    let values = rt
        .array_elements(member(rt, h, "__kag_locals")?)
        .ok_or_else(|| error("ExtKAGParser locals missing"))?;
    Ok(values
        .len()
        .checked_sub(if previous { 2 } else { 1 })
        .and_then(|i| values[i].object_handle()))
}
pub(super) fn get_local(rt: &Rt, h: ObjectHandle, name: &str) -> Result<Option<String>> {
    match rt.object_member(
        local(rt, h, false)?.ok_or_else(|| error("ExtKAGParser local variable stack empty"))?,
        name,
    ) {
        Variant::Void => Ok(None),
        v => v.to_tjs_string().map(Some),
    }
}
pub(super) fn update_locals(
    rt: &mut Rt,
    h: ObjectHandle,
    tag: &Tag,
    push: bool,
    copy: bool,
) -> Result<()> {
    let array = member(rt, h, "__kag_locals")?;
    let destination = if push {
        let d = if copy {
            copy_dict(
                rt,
                local(rt, h, false)?.ok_or_else(|| error("ExtKAGParser local stack empty"))?,
            )
        } else {
            rt.alloc_dictionary_object()
        };
        rt.array_push(array, Variant::Object(d));
        d
    } else {
        local(rt, h, false)?.ok_or_else(|| error("ExtKAGParser local stack empty"))?
    };
    let source = dict(rt, &tag.attributes)?;
    for (n, v) in rt.object_members(source) {
        rt.set_object_member(destination, n, v);
    }
    // Native CopyDicToCurrentLocalVariables includes tagname/storage/target.
    rt.set_object_member(destination, "tagname", Variant::String(tag.tagname.clone()));
    Ok(())
}
pub(super) fn install(rt: &mut Rt) {
    rt.host_mut().ext_kag = true;
    // V2Link replaces the class; existing classic objects retain their dialect.
    super::install(rt);
    let c = rt.global_member("KAGParser").object_handle().unwrap();
    for name in [
        "multiLineTagEnabled",
        "numericMacroArgumentsEnabled",
        "currentLocalVariables",
        "lf",
        "prelf",
        "localVariables",
        "localVariablesDepth",
        "whileStackDepth",
        "ifLevel",
        "pmacros",
        "paramMacros",
        "enableNP",
        "fuzzyReturn",
        "returnErrorStorage",
    ] {
        rt.register_object_native_property(
            c,
            name,
            move |rt, t| {
                let h = this_of(rt, t, "ExtKAGParser")?;
                let p = &state(rt, h)?.parser;
                if !p.is_ext_kag() {
                    return Err(error("ExtKAGParser instance required"));
                }
                Ok(match name {
                    "multiLineTagEnabled" => Variant::Integer(p.multi_line_tag_enabled() as i64),
                    "numericMacroArgumentsEnabled" => {
                        Variant::Integer(p.numeric_macro_arguments_enabled() as i64)
                    }
                    "localVariablesDepth" => Variant::Integer(p.local_variables_depth() as i64),
                    "whileStackDepth" => Variant::Integer(p.while_stack_depth() as i64),
                    "ifLevel" => Variant::Integer(p.if_level() as i64),
                    "currentLocalVariables" | "lf" => Variant::self_bound(
                        local(rt, h, false)?
                            .ok_or_else(|| error("ExtKAGParser local variable stack empty"))?,
                    ),
                    "prelf" => Variant::self_bound(local(rt, h, true)?.ok_or_else(|| {
                        error("ExtKAGParser previous local variable stack underflow")
                    })?),
                    "localVariables" => Variant::Object(member(rt, h, "__kag_locals")?),
                    "pmacros" | "paramMacros" => Variant::Object(member(rt, h, "__kag_pmacros")?),
                    "enableNP" | "fuzzyReturn" => Variant::Integer(0),
                    "returnErrorStorage" => Variant::String(String::new()),
                    _ => return Err(error("unknown ExtKAGParser property")),
                })
            },
            move |rt, t, v| match name {
                "multiLineTagEnabled" | "numericMacroArgumentsEnabled" => operate(rt, t, |p, _| {
                    if name == "multiLineTagEnabled" {
                        p.set_multi_line_tag_enabled(v.is_truthy())?;
                    } else {
                        p.set_numeric_macro_arguments_enabled(v.is_truthy())?;
                    }
                    Ok(Variant::Void)
                })
                .map(|_| ()),
                "enableNP" | "fuzzyReturn" => {
                    if !v.is_truthy() {
                        Ok(())
                    } else {
                        Err(unsupported("ExtKAGParser property", name))
                    }
                }
                "returnErrorStorage" => {
                    if v.to_tjs_string()?.is_empty() {
                        Ok(())
                    } else {
                        Err(unsupported(
                            "ExtKAGParser.returnErrorStorage",
                            "alternate return failure storage",
                        ))
                    }
                }
                _ => Err(error(format!("ExtKAGParser {name} is read-only"))),
            },
        );
    }
}

/// Original-format saves at a native quiescent boundary. Active native loop,
/// macro/call line buffers are rejected until their original dictionaries map.
pub(super) fn capture(rt: &mut Rt, owner: ObjectHandle, p: &KagParser) -> Result<Variant> {
    p.native_ext_control().map_err(error)?;
    let value = crate::kag_save::capture(rt, p)?;
    let h = value.object_handle().unwrap();
    let a = copy_array_struct(rt, member(rt, owner, "__kag_locals")?)?;
    rt.set_object_member(h, "LocalVariables", Variant::Object(a));
    let d = copy_struct(rt, member(rt, owner, "__kag_pmacros")?)?;
    rt.set_object_member(h, "paramMacros", Variant::Object(d));
    rt.set_object_member(h, "IgnoreCR", Variant::Integer(p.ignore_cr() as i64));
    rt.set_object_member(h, "EnableNP", Variant::Integer(0));
    for key in ["WhileLevelExp", "WhileLevelEach"] {
        rt.set_object_member(h, key, Variant::String(String::new()));
    }
    super::extended_save::augment(rt, h, owner, p)?;
    Ok(value)
}
pub(super) fn restore_values(
    rt: &mut Rt,
    owner: ObjectHandle,
    data: ObjectHandle,
    parser: &mut KagParser,
    multiline: bool,
    numeric: bool,
) -> Result<()> {
    if rt.object_member(data, "EnableNP").is_truthy() {
        return Err(unsupported(
            "ExtKAGParser.restore",
            "enableNP blank paragraph macro",
        ));
    }
    parser.enable_ext_kag();
    parser
        .set_multi_line_tag_enabled(multiline)
        .map_err(error)?;
    parser
        .set_numeric_macro_arguments_enabled(numeric)
        .map_err(error)?;
    parser.set_ignore_cr(rt.object_member(data, "IgnoreCR").is_truthy());
    let source = rt
        .object_member(data, "LocalVariables")
        .object_handle()
        .ok_or_else(|| error("ExtKAGParser original LocalVariables missing"))?;
    let values = rt
        .array_elements(source)
        .ok_or_else(|| error("ExtKAGParser LocalVariables must be array"))?
        .to_vec();
    if values.is_empty() || values.len() > 1024 || values.len() != parser.local_variables_depth() {
        return Err(error("native Ext saved local stack count mismatch"));
    }
    if values.iter().any(|value| value.object_handle().is_none()) {
        return Err(error("native local Dictionary required"));
    }
    let cloned = copy_array_struct(rt, source)?;
    let locals = rt.array_elements(cloned).unwrap().to_vec();
    let a = member(rt, owner, "__kag_locals")?;
    rt.array_clear(a);
    for local in locals {
        rt.array_push(a, local);
    }
    let source = rt
        .object_member(data, "paramMacros")
        .object_handle()
        .ok_or_else(|| error("ExtKAGParser original paramMacros missing"))?;
    let mut entries = Vec::new();
    let copied = copy_struct(rt, source)?;
    let destination = member(rt, owner, "__kag_pmacros")?;
    for (name, _) in rt.object_members(destination) {
        rt.delete_object_member(destination, &name);
    }
    for (name, value) in rt.object_members(copied) {
        rt.set_object_member(destination, name, value);
    }
    for (n, v) in rt.object_members(destination) {
        entries.push((
            n,
            attrs(
                rt,
                v.object_handle()
                    .ok_or_else(|| error("parameter macro dictionary missing"))?,
            )?,
        ));
    }
    parser.set_parameter_macros(entries).map_err(error)
}
pub(super) fn restore_saved_arguments(
    rt: &mut Rt,
    owner: ObjectHandle,
    data: ObjectHandle,
) -> Result<()> {
    let source = member(rt, data, "macroArgs")?;
    let values = rt
        .array_elements(source)
        .ok_or_else(|| error("native macroArgs must be Array"))?
        .to_vec();
    if values.len() > 1024 {
        return Err(error("native macroArgs stack budget"));
    }
    let base = rt.alloc_dictionary_object();
    let mut frames = vec![Variant::Object(base)];
    for value in values {
        frames.push(Variant::Object(copy_struct(
            rt,
            value
                .object_handle()
                .ok_or_else(|| error("native macroArgs Dictionary required"))?,
        )?));
    }
    let current = frames.last().cloned().unwrap();
    let depth = frames.len() - 1;
    let stack = rt.alloc_array_object(frames);
    rt.set_object_member(owner, "__kag_arg_stack", Variant::Object(stack));
    rt.set_object_member(owner, "__kag_params", current);
    rt.set_object_member(owner, "__kag_arg_depth", Variant::Integer(depth as i64));
    rt.delete_object_member(owner, "__kag_tag_values");
    Ok(())
}
