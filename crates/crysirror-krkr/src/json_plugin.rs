// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! json.dll semantic/value/storage facade over shared document APIs.
use crate::natives::{Rt, int, string, unsupported};
use crysirror_runtime::structured_text::{self, DocumentValue as D, JsonDialect};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::Variant,
};
use std::collections::BTreeSet;
fn error(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(e.to_string())
}
fn parse(rt: &mut Rt, text: &str) -> Result<Variant> {
    let value = structured_text::parse_json(text, JsonDialect::KiriKiri)
        .map_err(|e| error(format!("JSONファイル のパースに失敗しました: {e}")))?;
    to_native(rt, value)
}
fn to_native(rt: &mut Rt, value: D) -> Result<Variant> {
    Ok(match value {
        D::Void => Variant::Void,
        D::Integer(i) => Variant::Integer(i),
        D::Real(f) => Variant::Real(f),
        D::Text(s) => Variant::String(s),
        D::Array(a) => {
            let items = a
                .into_iter()
                .map(|v| to_native(rt, v))
                .collect::<Result<Vec<_>>>()?;
            Variant::Object(rt.alloc_array_object(items))
        }
        D::Object(a) => {
            let h = rt.alloc_dictionary_object();
            for (k, v) in a {
                let value = to_native(rt, v)?;
                rt.set_object_member(h, &k, value)
            }
            Variant::Object(h)
        }
    })
}
fn from_native(
    rt: &Rt,
    value: &Variant,
    depth: usize,
    nodes: &mut usize,
    stack: &mut BTreeSet<usize>,
) -> Result<D> {
    *nodes += 1;
    if *nodes > structured_text::MAX_DOCUMENT_NODES || depth > structured_text::MAX_DOCUMENT_DEPTH {
        return Err(error("JSON document shape budget"));
    }
    Ok(match value {
        Variant::Void | Variant::Null | Variant::Octet(_) | Variant::CodeObject(_) => D::Void,
        Variant::Integer(i) => D::Integer(*i),
        Variant::Real(f) => {
            if !f.is_finite() {
                return Err(error("nonfinite JSON number"));
            }
            D::Real(*f)
        }
        Variant::String(s) => {
            if s.len() > structured_text::MAX_DOCUMENT_BYTES {
                return Err(error("JSON string byte budget"));
            }
            D::Text(s.clone())
        }
        Variant::Object(h) | Variant::Closure(crysirror_tjs2::runtime::Closure { object: h, .. }) => {
            if !rt.object_valid(*h) {
                return Ok(D::Void);
            }
            if rt.object_is_callable(*h) {
                return Err(unsupported(
                    "JSON callable serialization",
                    "plain data objects",
                ));
            }
            if !stack.insert(h.0) {
                return Err(error("cyclic JSON document"));
            }
            let result = if let Some(items) = rt.array_elements(*h) {
                D::Array(
                    items
                        .iter()
                        .map(|v| from_native(rt, v, depth + 1, nodes, stack))
                        .collect::<Result<_>>()?,
                )
            } else {
                D::Object(
                    rt.object_members(*h)
                        .into_iter()
                        .map(|(k, v)| Ok((k, from_native(rt, &v, depth + 1, nodes, stack)?)))
                        .collect::<Result<_>>()?,
                )
            };
            stack.remove(&h.0);
            result
        }
    })
}
fn text(rt: &Rt, value: &Variant, lf: bool) -> Result<String> {
    let value = from_native(rt, value, 0, &mut 0, &mut BTreeSet::new())?;
    structured_text::write_json(&value, lf).map_err(error)
}
pub fn install(rt: &mut Rt) {
    let Some(scripts) = rt.global_member("Scripts").object_handle() else {
        return;
    };
    rt.register_object_native(scripts, "evalJSON", |rt: &mut Rt, _, a: Vec<Variant>| {
        if a.is_empty() {
            return Err(TjsError::bad_param_count());
        }
        parse(rt, &a[0].to_tjs_string()?)
    });
    rt.register_object_native(
        scripts,
        "toJSONString",
        |rt: &mut Rt, _, a: Vec<Variant>| {
            if a.is_empty() {
                return Err(TjsError::bad_param_count());
            }
            Ok(Variant::String(text(rt, &a[0], int(&a, 1, 0)? == 1)?))
        },
    );
    rt.register_object_native(
        scripts,
        "evalJSONStorage",
        |rt: &mut Rt, _, a: Vec<Variant>| {
            if a.is_empty() {
                return Err(TjsError::bad_param_count());
            }
            let name = string(&a, 0, "Scripts.evalJSONStorage")?;
            let storage = rt.host().storage()?;
            let path = storage
                .resolve(&name)
                .ok_or_else(|| error(format!("JSON storage `{name}` absent")))?;
            let bytes = storage
                .read_limited(&path, structured_text::MAX_DOCUMENT_BYTES)
                .map_err(error)?;
            if bytes.len() > structured_text::MAX_DOCUMENT_BYTES {
                return Err(error("JSON document byte budget"));
            }
            let content = if int(&a, 1, 0)? != 0 {
                std::str::from_utf8(&bytes).map_err(error)?.to_owned()
            } else {
                // Explicit Japanese ACP profile. Never replace malformed bytes.
                let (s, _, bad) = encoding_rs::SHIFT_JIS.decode(&bytes);
                if bad {
                    return Err(error("invalid CP932 JSON storage"));
                }
                s.into_owned()
            };
            parse(rt, &content)
        },
    );
    rt.register_object_native(scripts, "saveJSON", |rt: &mut Rt, _, a: Vec<Variant>| {
        if a.len() < 2 {
            return Err(TjsError::bad_param_count());
        }
        if !rt.host().can_suspend {
            return Err(unsupported(
                "Scripts.saveJSON in this callback",
                "a resumable atomic file boundary",
            ));
        }
        let name = string(&a, 0, "Scripts.saveJSON")?;
        let content = text(rt, &a[1], int(&a, 3, 0)? == 1)?;
        let bytes = if int(&a, 2, 0)? != 0 {
            content.into_bytes()
        } else {
            let (b, _, bad) = encoding_rs::SHIFT_JIS.encode(&content);
            if bad {
                return Err(error("JSON text cannot be encoded as CP932"));
            }
            b.into_owned()
        };
        if bytes.len() > 4 * 1024 * 1024 || rt.host().writes.len() >= 64 {
            return Err(error("native JSON write budget"));
        }
        let job = rt
            .host()
            .storage()?
            .begin_write(&name, bytes.into())
            .map_err(error)?;
        rt.host_mut().writes.push_back(job);
        rt.request_suspend();
        Ok(Variant::Void)
    });
}

/// `toml.dll`: Scripts.tomlDecodeFromStorage onto the shared TOML parser.
/// Storage text uses the native script decoding (BOM / FE FE / UTF-8).
pub fn install_toml(rt: &mut Rt) {
    let Some(scripts) = rt.global_member("Scripts").object_handle() else {
        return;
    };
    rt.register_object_native(
        scripts,
        "tomlDecodeFromStorage",
        |rt: &mut Rt, _, a: Vec<Variant>| {
            let name = string(&a, 0, "Scripts.tomlDecodeFromStorage")?;
            let storage = rt.host().storage()?;
            let path = storage
                .resolve(&name)
                .ok_or_else(|| error(format!("TOML storage `{name}` absent")))?;
            let bytes = storage
                .read_limited(&path, structured_text::MAX_DOCUMENT_BYTES)
                .map_err(error)?;
            let text = crate::storage::decode_text(&bytes).map_err(error)?;
            let value = structured_text::parse_toml(&text)
                .map_err(|e| error(format!("{name}: TOML: {e:#}")))?;
            to_native(rt, value)
        },
    );
}

