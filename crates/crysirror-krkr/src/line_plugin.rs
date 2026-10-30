// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `lineParser.dll` semantic adapter onto shared resource LineCursor.
//! Interface reviewed against wamsoft/lineParser revision
//! d8b6ec56810f1332f3d5e322986203630559791d (complete Main.cpp/manual.tjs).
//! No native parser or stream implementation is copied into this adapter.
use crate::natives::{Rt, methods, object_arg, read_only, string, this_of};
use crysirror_resource::text_stream::LineCursor;
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct State {
    readers: BTreeMap<usize, (LineCursor, Option<ObjectHandle>)>,
}
fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(format!("LineParser: {e:#}"))
}
fn reader(rt: &mut Rt, h: ObjectHandle) -> Result<&mut (LineCursor, Option<ObjectHandle>)> {
    rt.host_mut()
        .line_parser
        .readers
        .get_mut(&h.0)
        .ok_or_else(|| err("instance not constructed"))
}
fn init(rt: &mut Rt, h: ObjectHandle, a: &[Variant], storage: bool) -> Result<()> {
    let Some(input) = a.first().filter(|v| !matches!(v, Variant::Void)) else {
        return Ok(());
    };
    let input = input.to_tjs_string()?;
    if storage {
        let utf8 = a.get(1).map(Variant::to_integer).transpose()?.unwrap_or(0) != 0;
        let bytes = rt
            .read_storage_binary(&input, "")
            .map_err(|e| err(format!("{e:#}")))?;
        let text = if utf8 && !bytes.starts_with(&[0xff, 0xfe]) && !bytes.starts_with(&[0xfe, 0xfe])
        {
            String::from_utf8(bytes)
                .map_err(err)?
                .trim_start_matches('\u{feff}')
                .to_owned()
        } else {
            crate::storage::decode_text(&bytes).map_err(err)?
        };
        reader(rt, h)?.0.reset(text, 64 * 1024 * 1024).map_err(err)
    } else {
        reader(rt, h)?.0.reset(input, 64 * 1024 * 1024).map_err(err)
    }
}
fn parse(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>, storage: bool) -> Result<Variant> {
    let h = this_of(rt, t, "LineParser.parse")?;
    init(rt, h, &a, storage)?;
    let target = reader(rt, h)?.1.unwrap_or(h);
    if !crate::natives::has_member(rt, target, "doLine") {
        return Ok(Variant::Void);
    }
    // A callback may invalidate the parser; no further line is delivered then.
    loop {
        let Some(r) = rt.host_mut().line_parser.readers.get_mut(&h.0) else {
            break;
        };
        let Some(text) = r.0.next_line().map(str::to_owned) else {
            break;
        };
        let line = Variant::Integer(r.0.line_number() as i64);
        rt.call_variant_method(
            Variant::Object(target),
            "doLine",
            vec![Variant::String(text), line],
        )?;
    }
    Ok(Variant::Void)
}
pub fn install(rt: &mut Rt) {
    let c = crate::natives::native_class(rt, "LineParser", |rt, t, a| {
        let h = this_of(rt, t, "LineParser")?;
        let r = LineCursor::default();
        let target = match a.first() {
            None | Some(Variant::Void | Variant::Null) => None,
            Some(v) => Some(object_arg(v).ok_or_else(|| err("target must be an object"))?),
        };
        if rt.host().line_parser.readers.len() >= 1024
            && !rt.host().line_parser.readers.contains_key(&h.0)
        {
            return Err(err("instance limit"));
        }
        rt.host_mut().line_parser.readers.insert(h.0, (r, target));
        Ok(Variant::Void)
    });
    methods(
        rt,
        c,
        &[
            ("finalize", |rt, t, _| {
                let h = this_of(rt, t, "LineParser.finalize")?;
                rt.host_mut().line_parser.readers.remove(&h.0);
                Ok(Variant::Void)
            }),
            ("init", |rt, t, a| {
                let h = this_of(rt, t, "LineParser.init")?;
                string(&a, 0, "LineParser.init")?;
                init(rt, h, &a, false)?;
                Ok(Variant::Void)
            }),
            ("initStorage", |rt, t, a| {
                let h = this_of(rt, t, "LineParser.initStorage")?;
                string(&a, 0, "LineParser.initStorage")?;
                init(rt, h, &a, true)?;
                Ok(Variant::Void)
            }),
            ("getNextLine", |rt, t, _| {
                let h = this_of(rt, t, "LineParser.getNextLine")?;
                match reader(rt, h)?.0.next_line().map(str::to_owned) {
                    Some(text) => Ok(Variant::String(text)),
                    None => Ok(Variant::Void),
                }
            }),
            ("parse", |rt, t, a| parse(rt, t, a, false)),
            ("parseStorage", |rt, t, a| parse(rt, t, a, true)),
        ],
    );
    rt.register_object_native_property(
        c,
        "currentLineNumber",
        |rt, t| {
            let h = this_of(rt, t, "LineParser.currentLineNumber")?;
            Ok(Variant::Integer(reader(rt, h)?.0.line_number() as i64))
        },
        read_only,
    );
}

impl State {
    pub(crate) fn invalidate(&mut self, object: usize) {
        self.readers.remove(&object);
    }

    pub(crate) fn gc_roots(&self, out: &mut Vec<ObjectHandle>) {
        // Readers are owned by scripts (a collected one drops its entry via
        // `invalidate`); a live reader keeps its callback target.
        out.extend(self.readers.values().filter_map(|(_, target)| *target));
    }
}
