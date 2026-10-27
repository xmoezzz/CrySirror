// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Original KAGParser dictionary names/layout for plain scenario/label boundaries.
//! Active macro/call frames are refused, never replaced by a private save blob.
use crate::natives::*;
use krkr_kag::{KagParser, Krkr2ConditionState, ParserSnapshot};
use krkr_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
fn put(rt: &mut Rt, h: ObjectHandle, name: &str, v: impl Into<Variant>) {
    rt.set_object_member(h, name, v.into());
}
fn conditions(rt: &mut Rt, h: ObjectHandle, c: &Krkr2ConditionState) {
    put(rt, h, "ExcludeLevel", Variant::Integer(c.exclude_level));
    put(rt, h, "IfLevel", Variant::Integer(c.if_level));
    put(
        rt,
        h,
        "ExcludeLevelStack",
        Variant::String(
            c.exclude_level_stack
                .iter()
                .map(|v| format!("{:08x}", *v as i32 as u32))
                .collect(),
        ),
    );
    put(
        rt,
        h,
        "IfLevelExecutedStack",
        Variant::String(
            c.if_level_executed_stack
                .iter()
                .map(|v| if *v { '1' } else { '0' })
                .collect(),
        ),
    );
}
pub fn capture(rt: &mut Rt, p: &KagParser) -> Result<Variant> {
    if p.call_stack_depth() != 0 || !p.macro_params().is_empty() {
        return Err(unsupported(
            "KAGParser.store",
            "native active call/macro save mapping",
        ));
    }
    let h = rt.alloc_dictionary_object();
    let macros = rt.alloc_dictionary_object();
    for (n, s) in p.macro_definitions() {
        put(rt, macros, n, Variant::String(s.into()));
    }
    put(rt, h, "macros", Variant::Object(macros));
    for name in ["macroArgs", "callStack"] {
        let a = rt.alloc_array_object(vec![]);
        put(rt, h, name, Variant::Object(a));
    }
    let storage = p.cur_storage().unwrap_or("");
    put(rt, h, "storageName", Variant::String(storage.into()));
    put(
        rt,
        h,
        "storageShortName",
        Variant::String(
            storage
                .rsplit(['/', '\\', '>'])
                .next()
                .unwrap_or(storage)
                .into(),
        ),
    );
    put(
        rt,
        h,
        "curLabel",
        Variant::String(p.cur_label().unwrap_or("").into()),
    );
    put(
        rt,
        h,
        "curLine",
        Variant::Integer(p.cur_line().unwrap_or(1).saturating_sub(1) as i64),
    );
    put(
        rt,
        h,
        "curPos",
        Variant::Integer(p.cur_pos().unwrap_or(0) as i64),
    );
    put(rt, h, "lineBuffer", Variant::String(String::new()));
    for n in ["lineBufferUsing", "macroArgStackBase", "macroArgStackDepth"] {
        put(rt, h, n, Variant::Integer(0));
    }
    conditions(rt, h, &p.store().krkr2_condition_state());
    Ok(Variant::Object(h))
}
fn integer(rt: &Rt, h: ObjectHandle, n: &str) -> Result<i64> {
    rt.object_member(h, n).to_integer()
}
fn string_field(rt: &Rt, h: ObjectHandle, n: &str) -> Result<String> {
    rt.object_member(h, n).to_tjs_string()
}
pub fn validate(rt: &Rt, h: ObjectHandle) -> Result<()> {
    for n in ["lineBufferUsing", "macroArgStackBase", "macroArgStackDepth"] {
        if integer(rt, h, n)? != 0 {
            return Err(unsupported(
                "KAGParser.restore",
                "active native macro/line-buffer save mapping",
            ));
        }
    }
    for n in ["callStack", "macroArgs"] {
        let Some(a) = rt.object_member(h, n).object_handle() else {
            return Err(TjsError::runtime("native parser save array missing"));
        };
        if rt.array_elements(a).is_none_or(|v| !v.is_empty()) {
            return Err(unsupported(
                "KAGParser.restore",
                "native active call/macro save mapping",
            ));
        }
    }
    Ok(())
}
pub fn decode(rt: &Rt, p: &KagParser, h: ObjectHandle) -> Result<ParserSnapshot> {
    validate(rt, h)?;
    let storage = string_field(rt, h, "storageName")?;
    let label = string_field(rt, h, "curLabel")?;
    let line = if label.is_empty() {
        0
    } else {
        p.label_line(&storage, &label)
            .ok_or_else(|| TjsError::runtime("native parser saved label absent"))?
    };
    // The original Restore reloads storage and GoToLabel; curLine/curPos are
    // inspection fields, not an exact runtime cursor restoration contract.
    let offset = p
        .offset_for_line_pos(&storage, line, 0)
        .map_err(|e| TjsError::runtime(e.to_string()))?;
    let macros = match rt.object_member(h, "macros") {
        Variant::Void => p
            .macro_definitions()
            .map(|(n, s)| (n.to_owned(), s.to_owned()))
            .collect::<Vec<_>>(),
        Variant::Object(m) => rt
            .object_members(m)
            .into_iter()
            .map(|(n, v)| {
                if let Variant::String(s) = v {
                    Ok((n, s))
                } else {
                    Err(TjsError::runtime(
                        "native saved macro body must be a string",
                    ))
                }
            })
            .collect::<Result<Vec<_>>>()?,
        _ => return Err(TjsError::runtime("native saved macro dictionary")),
    };
    let hex = string_field(rt, h, "ExcludeLevelStack")?;
    let flags = string_field(rt, h, "IfLevelExecutedStack")?;
    let level = integer(rt, h, "IfLevel")?;
    if !(0..=128).contains(&level)
        || hex.len() != level as usize * 8
        || flags.len() != level as usize
        || !flags.bytes().all(|v| v == b'0' || v == b'1')
    {
        return Err(TjsError::runtime("native saved condition stack bounds"));
    }
    let stack = hex
        .as_bytes()
        .chunks_exact(8)
        .map(|c| {
            std::str::from_utf8(c)
                .ok()
                .and_then(|s| u32::from_str_radix(s, 16).ok())
                .map(|v| v as i32 as i64)
                .ok_or_else(|| TjsError::runtime("native saved condition hex"))
        })
        .collect::<Result<Vec<_>>>()?;
    let condition = Krkr2ConditionState {
        exclude_level: integer(rt, h, "ExcludeLevel")?,
        if_level: level,
        exclude_level_stack: stack,
        if_level_executed_stack: flags.bytes().map(|v| v == b'1').collect(),
    };
    Ok(ParserSnapshot::from_krkr2_compatible(
        Some(storage),
        offset,
        (!label.is_empty()).then_some(label),
        vec![],
        macros,
        condition,
    ))
}
