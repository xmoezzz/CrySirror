// SPDX-License-Identifier: MPL-2.0
//! Dynamic values crossing the object API. They are plain data: an object is a
//! reference (`ObjectRef`) into the engine's object table, never a pointer.
use crate::object::ObjectId;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Serialized as `{"$object": id, "$class": name}`; parsed by [`Value::from_json`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ObjectRef {
    #[serde(rename = "$object")]
    pub id: u64,
    #[serde(rename = "$class", default, skip_serializing_if = "Option::is_none")]
    pub class: Option<ClassName>,
}
/// Interned class name; kept as a static string so values stay `Copy`.
pub type ClassName = &'static str;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Object(ObjectRef),
    Map(BTreeMap<String, Value>),
}
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let json = serde_json::Value::deserialize(d)?;
        Ok(Value::from_json(json))
    }
}
impl Value {
    pub fn from_json(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::Number(n) => n
                .as_i64()
                .map(Value::Int)
                .unwrap_or_else(|| Value::Float(n.as_f64().unwrap_or(f64::NAN))),
            serde_json::Value::String(s) => Value::Str(s),
            serde_json::Value::Array(a) => {
                Value::List(a.into_iter().map(Value::from_json).collect())
            }
            serde_json::Value::Object(m) => {
                if let Some(id) = m.get("$object").and_then(serde_json::Value::as_u64) {
                    if m.keys().all(|k| k == "$object" || k == "$class") {
                        return Value::Object(ObjectRef { id, class: None });
                    }
                }
                Value::Map(
                    m.into_iter()
                        .map(|(k, v)| (k, Value::from_json(v)))
                        .collect(),
                )
            }
        }
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
    pub fn object(id: ObjectId, class: ClassName) -> Self {
        Value::Object(ObjectRef {
            id: id.0,
            class: Some(class),
        })
    }
    pub fn as_f64(&self) -> Result<f64> {
        match self {
            Value::Int(i) => Ok(*i as f64),
            Value::Float(f) if f.is_finite() => Ok(*f),
            _ => bail!("expected a finite number, got {self:?}"),
        }
    }
    pub fn as_f32(&self) -> Result<f32> {
        let v = self.as_f64()?;
        anyhow::ensure!(v.abs() <= f32::MAX as f64, "number out of f32 range");
        Ok(v as f32)
    }
    pub fn as_i64(&self) -> Result<i64> {
        match self {
            Value::Int(i) => Ok(*i),
            Value::Float(f) if f.fract() == 0. && f.abs() < 9e15 => Ok(*f as i64),
            _ => bail!("expected an integer, got {self:?}"),
        }
    }
    pub fn as_bool(&self) -> Result<bool> {
        match self {
            Value::Bool(b) => Ok(*b),
            _ => bail!("expected a boolean, got {self:?}"),
        }
    }
    pub fn as_str(&self) -> Result<&str> {
        match self {
            Value::Str(s) => Ok(s),
            _ => bail!("expected a string, got {self:?}"),
        }
    }
    pub fn as_object(&self) -> Result<ObjectId> {
        match self {
            Value::Object(r) => Ok(ObjectId(r.id)),
            _ => bail!("expected an object, got {self:?}"),
        }
    }
    pub fn as_vec3(&self) -> Result<[f32; 3]> {
        match self {
            Value::List(l) if l.len() == 3 => Ok([l[0].as_f32()?, l[1].as_f32()?, l[2].as_f32()?]),
            Value::Map(m) => Ok([
                m.get("x").context("vec3.x")?.as_f32()?,
                m.get("y").context("vec3.y")?.as_f32()?,
                m.get("z").context("vec3.z")?.as_f32()?,
            ]),
            _ => bail!("expected [x,y,z] or {{x,y,z}}, got {self:?}"),
        }
    }
    pub fn vec3(v: [f32; 3]) -> Self {
        Value::List(v.iter().map(|c| Value::Float(*c as f64)).collect())
    }
    pub fn field<'a>(&'a self, name: &str) -> Option<&'a Value> {
        match self {
            Value::Map(m) => m.get(name).filter(|v| !matches!(v, Value::Null)),
            _ => None,
        }
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}
impl From<u64> for Value {
    fn from(i: u64) -> Self {
        i64::try_from(i)
            .map(Value::Int)
            .unwrap_or(Value::Float(i as f64))
    }
}
impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Float(f)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.into())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}
