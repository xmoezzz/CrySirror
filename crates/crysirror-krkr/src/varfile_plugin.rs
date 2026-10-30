// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or later.
//! varfile's native namespace/URI policy over portable TJS media hooks.
//! Reviewed complete wamsoft/varfile Main.cpp and readme at
//! e8742b4d4a8f9d5744ed91116d00ffa93f265001. No COM/Win32 stream is hosted.
use crate::natives::Rt;
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, StorageAction, StorageRequest, Variant},
};
const MAX_BYTES: usize = 64 * 1024 * 1024;
fn err(text: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(format!("varfile: {text}"))
}
pub fn is_uri(name: &str) -> bool {
    name.get(..6)
        .is_some_and(|s| s.eq_ignore_ascii_case("var://"))
}
fn parts(name: &str) -> Result<Vec<&str>> {
    if name.len() > 4096 || name.contains('\0') || !is_uri(name) {
        return Err(err("invalid URI"));
    }
    let path = name[6..]
        .strip_prefix("./")
        .ok_or_else(|| err("only the current script domain var://./ is allowed"))?;
    let segments: Vec<_> = path.split('/').collect();
    if segments.len() > 128
        || segments.iter().enumerate().any(|(i, s)| {
            matches!(*s, "." | "..")
                || (s.is_empty() && i + 1 < segments.len())
                || s.contains(['\\', ':'])
        })
    {
        return Err(err("invalid namespace path"));
    }
    Ok(segments)
}
pub fn validate_uri(name: &str) -> Result<()> {
    parts(name).map(|_| ())
}
fn parent(rt: &mut Rt, name: &str) -> Result<Option<(ObjectHandle, String)>> {
    let mut segments = parts(name)?;
    let leaf = segments.pop().unwrap_or_default().to_owned();
    let mut object = rt.global_handle();
    for segment in segments {
        let value = rt.resolve_object_member(object, segment)?;
        let Some(next) = value.object_handle().filter(|h| rt.object_valid(*h)) else {
            return Ok(None);
        };
        object = next;
    }
    Ok(Some((object, leaf)))
}
fn value(rt: &mut Rt, name: &str) -> Result<Variant> {
    let Some((object, leaf)) = parent(rt, name)? else {
        return Ok(Variant::Void);
    };
    if leaf.is_empty() {
        Ok(Variant::Object(object))
    } else {
        rt.resolve_object_member(object, &leaf)
    }
}
pub fn exists(rt: &mut Rt, name: &str) -> Result<bool> {
    if !rt.host().loaded_plugins.contains("varfile.dll") {
        return Err(err("var media is not linked"));
    }
    Ok(matches!(value(rt, name)?, Variant::Octet(_)))
}
pub fn list(rt: &mut Rt, name: &str) -> Result<Vec<String>> {
    if !rt.host().loaded_plugins.contains("varfile.dll") {
        return Err(err("var media is not linked"));
    }
    let Some(object) = value(rt, name)?.object_handle() else {
        return Ok(Vec::new());
    };
    Ok(rt
        .object_members(object)
        .into_iter()
        .filter_map(|(key, value)| matches!(value, Variant::Octet(_)).then_some(key))
        .collect())
}
fn bytes(rt: &mut Rt, name: &str) -> Result<Vec<u8>> {
    let Variant::Octet(bytes) = value(rt, name)? else {
        return Err(err(format!("not an octet file: {name}")));
    };
    if bytes.len() > MAX_BYTES {
        return Err(err("octet byte limit"));
    }
    Ok(bytes)
}
fn write(rt: &mut Rt, name: &str, bytes: Vec<u8>) -> Result<()> {
    if bytes.len() > MAX_BYTES {
        return Err(err("octet byte limit"));
    }
    let (parent, leaf) =
        parent(rt, name)?.ok_or_else(|| err("parent namespace is not an object"))?;
    if leaf.is_empty() {
        return Err(err("cannot write a directory"));
    }
    if rt.object_member_is_property(parent, &leaf) {
        return Err(err(
            "property-backed var writes require a native property setter mapping",
        ));
    }
    if rt.array_elements(parent).is_some() {
        let index = leaf
            .parse::<usize>()
            .map_err(|_| err("array storage index must be a non-negative integer"))?;
        if index >= 1_000_000 {
            return Err(err("array storage index limit"));
        }
    }
    rt.set_object_member(parent, leaf, Variant::Octet(bytes));
    Ok(())
}
fn request(rt: &mut Rt, request: StorageRequest) -> Result<Variant> {
    match request.action {
        StorageAction::ReadBinary => {
            if !matches!(request.mode.as_str(), "" | "b") {
                return Err(err("offset/read stream modes are not mapped"));
            }
            Ok(Variant::Octet(bytes(rt, &request.name)?))
        }
        StorageAction::ReadText => {
            if !request.mode.is_empty() {
                return Err(err("text stream read mode is not mapped"));
            }
            let text =
                crysirror_resource::text_stream::decode_auto(&bytes(rt, &request.name)?, MAX_BYTES)
                    .map_err(err)?;
            Ok(Variant::String(text))
        }
        StorageAction::WriteBinary(bytes) => {
            if !matches!(request.mode.as_str(), "" | "b") {
                return Err(err("offset/append stream modes are not mapped"));
            }
            write(rt, &request.name, bytes)?;
            Ok(Variant::Void)
        }
        StorageAction::WriteText(text) => {
            let bytes = crate::host::encode_native_text(&text,&request.mode)?;
            write(rt, &request.name, bytes)?;
            Ok(Variant::Void)
        }
    }
}
pub fn install(rt: &mut Rt) -> Result<()> {
    rt.register_storage_media("var", request)
}
pub fn load_image(rt: &mut Rt, layer: ObjectHandle, name: &str) -> Result<Variant> {
    let encoded = rt.read_storage_binary(name, "")?;
    crate::psbfile_plugin::load_encoded_image(rt, layer, name, encoded)
}
