// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! `csvParser.dll` CSVParser: TJS mapping onto shared `crysirror_runtime::csv`.
use crate::natives::{Rt, methods, object_arg, read_only, string, this_of};
use crysirror_runtime::csv::CsvReader;
use krkr_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct State {
    readers: BTreeMap<usize, (CsvReader, Option<ObjectHandle>)>,
}
fn err(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(format!("CSVParser: {e:#}"))
}
fn reader(rt: &mut Rt, h: ObjectHandle) -> Result<&mut (CsvReader, Option<ObjectHandle>)> {
    rt.host_mut()
        .csv
        .readers
        .get_mut(&h.0)
        .ok_or_else(|| err("instance not constructed"))
}
fn row(rt: &mut Rt, fields: Vec<Vec<u16>>) -> Variant {
    let items = fields
        .iter()
        .map(|f| Variant::String(String::from_utf16_lossy(f)))
        .collect();
    Variant::Object(rt.alloc_array_object(items))
}
fn init(rt: &mut Rt, h: ObjectHandle, a: &[Variant], storage: bool) -> Result<()> {
    let Some(input) = a.first().filter(|v| !matches!(v, Variant::Void)) else {
        return Ok(());
    };
    let input = input.to_tjs_string()?;
    if storage {
        let utf8 = a.get(1).map(Variant::to_integer).transpose()?.unwrap_or(0) != 0;
        let bytes = rt
            .host()
            .storage()?
            .read_named(&input)
            .map_err(|e| err(format!("{e:#}")))?;
        // The native reader is a KiriKiri TextStream: a BOM or the ciphered
        // `FE FE` header selects text decoding; otherwise raw bytes (CP932,
        // or UTF-8 when requested) are read line by line.
        if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xEF, 0xBB, 0xBF]) || bytes.starts_with(&[0xFE, 0xFE]) {
            let text = crate::storage::decode_text(&bytes).map_err(err)?;
            return reader(rt, h)?.0.init_text(text.encode_utf16().collect()).map_err(err);
        }
        reader(rt, h)?.0.init_bytes(bytes, utf8).map_err(err)
    } else {
        reader(rt, h)?.0.init_text(input.encode_utf16().collect()).map_err(err)
    }
}
fn parse(rt: &mut Rt, t: Option<ObjectHandle>, a: Vec<Variant>, storage: bool) -> Result<Variant> {
    let h = this_of(rt, t, "CSVParser.parse")?;
    init(rt, h, &a, storage)?;
    let target = reader(rt, h)?.1.unwrap_or(h);
    if !crate::natives::has_member(rt, target, "doLine") {
        return Ok(Variant::Void);
    }
    // A callback may invalidate the parser; no further row is delivered then.
    loop {
        let Some(r) = rt.host_mut().csv.readers.get_mut(&h.0) else {
            break;
        };
        let Some(fields) = r.0.next_record().map_err(err)? else {
            break;
        };
        let line = Variant::Integer(r.0.line_number);
        let fields = row(rt, fields);
        rt.call_variant_method(Variant::Object(target), "doLine", vec![fields, line])?;
    }
    Ok(Variant::Void)
}
pub fn install(rt: &mut Rt) {
    let c = crate::natives::native_class(rt, "CSVParser", |rt, t, a| {
        let h = this_of(rt, t, "CSVParser")?;
        let mut r = CsvReader::default();
        let target = match a.first() {
            None | Some(Variant::Void | Variant::Null) => None,
            Some(v) => Some(object_arg(v).ok_or_else(|| err("target must be an object"))?),
        };
        if let Some(sep) = a.get(1).filter(|v| !matches!(v, Variant::Void)) {
            r.separator = sep.to_integer()? as u16;
        }
        if let Some(nl) = a.get(2).filter(|v| !matches!(v, Variant::Void)) {
            r.newline = nl.to_tjs_string()?.encode_utf16().collect();
        }
        if rt.host().csv.readers.len() >= 1024 && !rt.host().csv.readers.contains_key(&h.0) {
            return Err(err("instance limit"));
        }
        rt.host_mut().csv.readers.insert(h.0, (r, target));
        Ok(Variant::Void)
    });
    methods(
        rt,
        c,
        &[
            ("finalize", |rt, t, _| {
                let h = this_of(rt, t, "CSVParser.finalize")?;
                rt.host_mut().csv.readers.remove(&h.0);
                Ok(Variant::Void)
            }),
            ("init", |rt, t, a| {
                let h = this_of(rt, t, "CSVParser.init")?;
                string(&a, 0, "CSVParser.init")?;
                init(rt, h, &a, false)?;
                Ok(Variant::Void)
            }),
            ("initStorage", |rt, t, a| {
                let h = this_of(rt, t, "CSVParser.initStorage")?;
                string(&a, 0, "CSVParser.initStorage")?;
                init(rt, h, &a, true)?;
                Ok(Variant::Void)
            }),
            ("getNextLine", |rt, t, _| {
                let h = this_of(rt, t, "CSVParser.getNextLine")?;
                match reader(rt, h)?.0.next_record().map_err(err)? {
                    Some(fields) => Ok(row(rt, fields)),
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
            let h = this_of(rt, t, "CSVParser.currentLineNumber")?;
            Ok(Variant::Integer(reader(rt, h)?.0.line_number))
        },
        read_only,
    );
}
