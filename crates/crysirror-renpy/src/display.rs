// SPDX-License-Identifier: MPL-2.0
use crate::files::ScopedFiles;
use crysirror_core::{Error, Result};
use crysirror_python::{Payload, Value, ops};
use serde_json::json;

pub(crate) fn resolve_reference(mut value: Value, state: &crate::engine::State) -> Result<Value> {
    for _ in 0..32 {
        let Some(object) = value
            .obj()
            .filter(|object| object.class_name() == "ImageReference")
        else {
            return Ok(value);
        };
        let name =
            ops::get_raw_attr(object, "name").ok_or(Error::Invalid("Ren'Py image reference"))?;
        let Some(Payload::Tuple(parts)) = name.payload() else {
            return Err(Error::Invalid("Ren'Py image reference"));
        };
        let key = parts
            .iter()
            .map(|part| {
                part.as_str()
                    .map(str::to_owned)
                    .ok_or(Error::Invalid("Ren'Py image name"))
            })
            .collect::<Result<Vec<_>>>()?;
        value = state
            .images
            .borrow()
            .get(&key)
            .cloned()
            .ok_or_else(|| Error::Io(format!("Ren'Py image not defined: {}", key.join(" "))))?;
    }
    Err(Error::Limit("Ren'Py image reference chain"))
}

pub fn paint(value: &Value, files: &ScopedFiles) -> Result<Option<serde_json::Value>> {
    let path = if let Some(path) = value.as_str() {
        path.to_owned()
    } else {
        let object = value
            .obj()
            .ok_or(Error::Unsupported("Ren'Py scene displayable"))?;
        match object.class_name().as_str() {
            "Null" => return Ok(None),
            "ATLTransform" => return Err(Error::Unsupported("Ren'Py ATL displayable evaluation")),
            "Text" => {
                let text = ops::get_raw_attr(object, "text")
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .ok_or(Error::Unsupported("Ren'Py non-string text displayable"))?;
                let properties = ops::get_raw_attr(object, "properties");
                let property = |name: &str| {
                    properties
                        .as_ref()
                        .and_then(Value::obj)
                        .and_then(|values| values.dict_get_str_raw(name))
                };
                let size = match property("size") {
                    Some(Value::Int(value)) => u16::try_from(value)
                        .ok()
                        .filter(|value| *value != 0)
                        .ok_or(Error::Invalid("Ren'Py text size"))?,
                    None => 22,
                    _ => return Err(Error::Invalid("Ren'Py text size")),
                };
                let color = match property("color") {
                    Some(value) => color_value(&value)?,
                    None => [255; 4],
                };
                return Ok(Some(
                    json!({"kind":"text","text":text,"font":"default","size":size,"color":color}),
                ));
            }
            "Solid" => {
                let color = ops::get_raw_attr(object, "color")
                    .ok_or(Error::Invalid("Ren'Py Solid color"))?;
                let rgba = color_value(&color)?;
                return Ok(Some(json!({"kind":"solid","color":rgba})));
            }
            "Image" => ops::get_raw_attr(object, "filename")
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or(Error::Invalid("Ren'Py image filename"))?,
            _ => return Err(Error::Unsupported("Ren'Py scene displayable")),
        }
    };
    let asset = files
        .asset_path(&path)
        .ok_or_else(|| Error::Io(format!("Ren'Py image not found: {path}")))?;
    Ok(Some(json!({"kind":"image","asset":asset.relative()})))
}

pub(crate) fn color_value(color: &Value) -> Result<[u8; 4]> {
    if let Some(text) = color.as_str().and_then(|text| text.strip_prefix('#')) {
        if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::Invalid("Ren'Py color"));
        }
        let mut rgba = [255; 4];
        match text.len() {
            3 | 4 => {
                for (i, output) in rgba.iter_mut().take(text.len()).enumerate() {
                    *output = 17
                        * u8::from_str_radix(&text[i..i + 1], 16)
                            .map_err(|_| Error::Invalid("Ren'Py color"))?;
                }
            }
            6 | 8 => {
                for (i, output) in rgba.iter_mut().take(text.len() / 2).enumerate() {
                    *output = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
                        .map_err(|_| Error::Invalid("Ren'Py color"))?;
                }
            }
            _ => return Err(Error::Invalid("Ren'Py color")),
        }
        return Ok(rgba);
    }
    let Some(Payload::Tuple(channels)) = color.payload() else {
        return Err(Error::Invalid("Ren'Py color"));
    };
    if !matches!(channels.len(), 3 | 4) {
        return Err(Error::Invalid("Ren'Py color"));
    }
    let mut rgba = [255; 4];
    for (out, value) in rgba.iter_mut().zip(channels) {
        let Value::Int(channel) = value else {
            return Err(Error::Invalid("Ren'Py color"));
        };
        *out = u8::try_from(*channel).map_err(|_| Error::Invalid("Ren'Py color"))?;
    }
    Ok(rgba)
}

pub(crate) fn solid_background(value: &Value) -> Result<[u8; 4]> {
    match value.obj().map(|object| object.class_name()).as_deref() {
        Some("Solid") => color_value(
            &ops::get_raw_attr(value.obj().unwrap(), "color")
                .ok_or(Error::Invalid("Ren'Py Solid color"))?,
        ),
        Some("Null") => Ok([0; 4]),
        _ => color_value(value),
    }
}
