// SPDX-License-Identifier: MPL-2.0
//! Unversioned (schema-indexed) property deserialization for cooked UE5
//! exports, producing JSON. Mirrors the binary layouts CUE4Parse documents.
use crate::usmap::{PropType, Usmap};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Map, Value};

pub struct Reader<'a> {
    pub b: &'a [u8],
    pub p: usize,
    pub names: &'a [String],
}
impl<'a> Reader<'a> {
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(
            self.p.checked_add(n).is_some_and(|e| e <= self.b.len()),
            "read past end at {} (+{n})",
            self.p
        );
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into()?))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into()?))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into()?))
    }
    pub fn fname(&mut self) -> Result<String> {
        let i = self.u32()? as usize;
        let n = self.u32()?;
        let base = self
            .names
            .get(i & 0x3fff_ffff)
            .with_context(|| format!("name index {i}"))?;
        Ok(if n == 0 {
            base.clone()
        } else {
            format!("{base}_{}", n - 1)
        })
    }
    pub fn fstring(&mut self) -> Result<String> {
        let n = self.i32()?;
        if n == 0 {
            return Ok(String::new());
        }
        ensure!(n.unsigned_abs() < 1 << 20, "string length {n}");
        Ok(if n < 0 {
            let s = self.take((-n) as usize * 2)?;
            String::from_utf16_lossy(
                &s.chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>(),
            )
            .trim_end_matches('\0')
            .to_owned()
        } else {
            String::from_utf8_lossy(self.take(n as usize)?)
                .trim_end_matches('\0')
                .to_owned()
        })
    }
    fn count(&mut self) -> Result<usize> {
        let n = self.i32()?;
        ensure!((0..1 << 24).contains(&n), "bad element count {n}");
        Ok(n as usize)
    }
}

/// Structs serialized natively (binary), not as property lists.
fn native_struct(r: &mut Reader, name: &str) -> Result<Option<Value>> {
    let v3 = |r: &mut Reader| -> Result<Value> { Ok(json!([r.f64()?, r.f64()?, r.f64()?])) };
    Ok(Some(match name {
        "Vector" | "Rotator" => v3(r)?,
        "Vector3f" => json!([r.f32()?, r.f32()?, r.f32()?]),
        "Vector2D" => json!([r.f64()?, r.f64()?]),
        "Vector2f" => json!([r.f32()?, r.f32()?]),
        "Vector4" | "Quat" | "Plane" => json!([r.f64()?, r.f64()?, r.f64()?, r.f64()?]),
        "Vector4f" | "Quat4f" => json!([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
        "LinearColor" => json!([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
        "Color" => {
            let c = r.take(4)?;
            json!([c[2], c[1], c[0], c[3]])
        }
        "Guid" => json!(r
            .take(16)?
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>()),
        "IntPoint" => json!([r.i32()?, r.i32()?]),
        "IntVector" => json!([r.i32()?, r.i32()?, r.i32()?]),
        "Box" => {
            let (a, b) = (v3(r)?, v3(r)?);
            json!({"min": a, "max": b, "valid": r.u8()?})
        }
        "Box2D" => {
            json!({"min": [r.f64()?, r.f64()?], "max": [r.f64()?, r.f64()?], "valid": r.u8()?})
        }
        "DateTime" | "Timespan" => json!(r.i64()?),
        "FrameNumber" => json!(r.i32()?),
        "SoftObjectPath" | "SoftClassPath" => soft_path(r)?,
        "GameplayTagContainer" => {
            let n = r.count()?;
            Value::Array(
                (0..n)
                    .map(|_| r.fname().map(Value::String))
                    .collect::<Result<_>>()?,
            )
        }
        "PerPlatformFloat" => json!({"cooked": r.u8()?, "value": r.f32()?}),
        "PerPlatformInt" => json!({"cooked": r.u8()?, "value": r.i32()?}),
        "PerPlatformBool" => json!({"cooked": r.u8()?, "value": r.u8()?}),
        "Matrix" | "Matrix44d" => json!((0..16).map(|_| r.f64()).collect::<Result<Vec<_>>>()?),
        _ => return Ok(None),
    }))
}
fn soft_path(r: &mut Reader) -> Result<Value> {
    let package = r.fname()?;
    let asset = r.fname()?;
    let sub = r.fstring()?;
    Ok(json!({"asset": format!("{package}.{asset}"), "sub": sub}))
}
fn text(r: &mut Reader) -> Result<Value> {
    let _flags = r.u32()?;
    let history = r.u8()? as i8;
    Ok(match history {
        -1 => {
            let has = r.u32()? != 0;
            json!(if has { r.fstring()? } else { String::new() })
        }
        0 => {
            let ns = r.fstring()?;
            let key = r.fstring()?;
            json!({"namespace": ns, "key": key, "source": r.fstring()?})
        }
        11 => json!({"table": r.fname()?, "key": r.fstring()?}),
        h => bail!("unsupported FText history {h}"),
    })
}

pub fn value(r: &mut Reader, m: &Usmap, ty: &PropType) -> Result<Value> {
    Ok(match ty {
        PropType::Bool | PropType::Int8 | PropType::Byte { .. } => json!(r.u8()?),
        PropType::Int16 | PropType::UInt16 => json!(r.u16()?),
        PropType::Int => json!(r.i32()?),
        PropType::UInt32 => json!(r.u32()?),
        PropType::Int64 | PropType::UInt64 => json!(r.i64()?),
        PropType::Float => json!(r.f32()?),
        PropType::Double => json!(r.f64()?),
        PropType::Object | PropType::Weak => json!({"index": r.i32()?}),
        PropType::Interface => json!({"index": r.i32()?}),
        PropType::Lazy => json!(r
            .take(16)?
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect::<String>()),
        PropType::SoftObject => soft_path(r)?,
        PropType::Name => json!(r.fname()?),
        PropType::Str | PropType::Utf8Str | PropType::AnsiStr => json!(r.fstring()?),
        PropType::Text => text(r)?,
        PropType::Delegate => json!({"object": r.i32()?, "function": r.fname()?}),
        PropType::MulticastDelegate => {
            let n = r.count()?;
            Value::Array(
                (0..n)
                    .map(|_| Ok(json!({"object": r.i32()?, "function": r.fname()?})))
                    .collect::<Result<_>>()?,
            )
        }
        PropType::FieldPath => {
            let n = r.count()?;
            let path: Vec<String> = (0..n).map(|_| r.fname()).collect::<Result<_>>()?;
            json!({"path": path, "owner": r.i32()?})
        }
        PropType::Enum { inner, name } => {
            let raw = match inner.as_ref() {
                PropType::Int | PropType::UInt32 => r.u32()? as u64,
                PropType::Int64 | PropType::UInt64 => r.i64()? as u64,
                PropType::Int16 | PropType::UInt16 => r.u16()? as u64,
                _ => r.u8()? as u64,
            };
            m.enums
                .get(name)
                .and_then(|vals| vals.iter().find(|(v, _)| *v == raw))
                .map(|(_, n)| json!(n))
                .unwrap_or(json!(raw))
        }
        PropType::Struct(name) => match native_struct(r, name)? {
            Some(v) => v,
            None => properties(r, m, name)?,
        },
        PropType::Array(inner) => {
            let n = r.count()?;
            Value::Array((0..n).map(|_| value(r, m, inner)).collect::<Result<_>>()?)
        }
        PropType::Set(inner) => {
            let removed = r.count()?;
            for _ in 0..removed {
                value(r, m, inner)?;
            }
            let n = r.count()?;
            Value::Array((0..n).map(|_| value(r, m, inner)).collect::<Result<_>>()?)
        }
        PropType::Map(k, v) => {
            let removed = r.count()?;
            for _ in 0..removed {
                value(r, m, k)?;
            }
            let n = r.count()?;
            Value::Array(
                (0..n)
                    .map(|_| Ok(json!([value(r, m, k)?, value(r, m, v)?])))
                    .collect::<Result<_>>()?,
            )
        }
        PropType::Optional(inner) => {
            if r.u32()? != 0 {
                value(r, m, inner)?
            } else {
                Value::Null
            }
        }
    })
}

/// Deserialize one unversioned property block for `class` (schema chain).
pub fn properties(r: &mut Reader, m: &Usmap, class: &str) -> Result<Value> {
    let slots = m.slots(class)?;
    // Header fragments.
    let mut frags = Vec::new();
    loop {
        let f = r.u16()?;
        let (skip, zeros, last, num) = (
            (f & 0x7f) as usize,
            f & 0x80 != 0,
            f & 0x100 != 0,
            (f >> 9) as usize,
        );
        frags.push((skip, zeros, num));
        if last {
            break;
        }
        ensure!(frags.len() < 4096, "unversioned header too long");
    }
    let zero_count: usize = frags.iter().filter(|f| f.1).map(|f| f.2).sum();
    let mut zero_mask = Vec::new();
    if zero_count > 0 {
        let bytes = if zero_count <= 8 {
            1
        } else if zero_count <= 16 {
            2
        } else {
            zero_count.div_ceil(32) * 4
        };
        let raw = r.take(bytes)?;
        zero_mask = (0..zero_count)
            .map(|i| raw[i / 8] >> (i % 8) & 1 == 1)
            .collect();
    }
    let mut out = Map::new();
    let (mut idx, mut zi) = (0usize, 0usize);
    for (skip, zeros, num) in frags {
        idx += skip;
        for _ in 0..num {
            let is_zero = if zeros {
                let z = zero_mask.get(zi).copied().unwrap_or(false);
                zi += 1;
                z
            } else {
                false
            };
            let (name, ty) = slots
                .get(idx)
                .cloned()
                .flatten()
                .with_context(|| format!("{class}: no property at schema slot {idx}"))?;
            if !is_zero {
                let v = value(r, m, &ty).with_context(|| format!("{class}.{name}"))?;
                out.insert(name, v);
            } else {
                out.insert(name, Value::Null);
            }
            idx += 1;
        }
    }
    Ok(Value::Object(out))
}
