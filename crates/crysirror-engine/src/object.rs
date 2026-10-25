// SPDX-License-Identifier: MPL-2.0
//! Reflective classes and the object table (ADR 0004).
//!
//! A [`Class`] is compiled Rust that implements properties and methods; an
//! object is an [`ObjectId`] plus plain-data payload in the [`ObjectTable`].
//! Every engine feature is reached through classes, so Rust behaviours, script
//! hosts and inspection tools share one validated path. Classes describe
//! themselves ([`ClassInfo`]) so hosts can generate bindings and typings.
use crate::{context::EngineContext, value::Value};
use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use std::{collections::BTreeMap, fmt::Write, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ObjectId(pub u64);

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeHint {
    Any,
    Void,
    Bool,
    Int,
    Number,
    String,
    Vec3,
    Map,
    List(Box<TypeHint>),
    Object(&'static str),
    Optional(Box<TypeHint>),
}
impl TypeHint {
    pub fn typescript(&self) -> String {
        match self {
            TypeHint::Any => "any".into(),
            TypeHint::Void => "void".into(),
            TypeHint::Bool => "boolean".into(),
            TypeHint::Int | TypeHint::Number => "number".into(),
            TypeHint::String => "string".into(),
            TypeHint::Vec3 => "Vec3".into(),
            TypeHint::Map => "Record<string, any>".into(),
            TypeHint::List(t) => format!("{}[]", t.typescript()),
            TypeHint::Object(c) => (*c).into(),
            TypeHint::Optional(t) => format!("{} | null", t.typescript()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PropertyInfo {
    pub name: &'static str,
    pub ty: TypeHint,
    pub writable: bool,
    pub doc: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub struct MethodInfo {
    pub name: &'static str,
    pub params: Vec<(&'static str, TypeHint)>,
    pub returns: TypeHint,
    pub doc: &'static str,
}
#[derive(Clone, Debug, Serialize)]
pub struct ClassInfo {
    pub name: &'static str,
    pub doc: &'static str,
    pub properties: Vec<PropertyInfo>,
    pub methods: Vec<MethodInfo>,
}
impl ClassInfo {
    pub fn property(&self, name: &str) -> Result<&PropertyInfo> {
        self.properties
            .iter()
            .find(|p| p.name == name)
            .with_context(|| format!("{}.{name}: no such property", self.name))
    }
    pub fn method(&self, name: &str) -> Result<&MethodInfo> {
        self.methods
            .iter()
            .find(|m| m.name == name)
            .with_context(|| format!("{}.{name}(): no such method", self.name))
    }
}

/// Compiled behaviour for one class. Implementations receive the whole engine
/// context so a method can mutate the world transactionally, but they must
/// validate `this` (stale handles fail) and all arguments.
pub trait Class: Send + Sync {
    fn info(&self) -> &ClassInfo;
    fn get(&self, cx: &mut EngineContext, this: ObjectId, property: &str) -> Result<Value>;
    fn set(
        &self,
        cx: &mut EngineContext,
        this: ObjectId,
        property: &str,
        value: Value,
    ) -> Result<()> {
        let _ = (cx, this, value);
        bail!("{}.{property} is read-only", self.info().name)
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        this: ObjectId,
        method: &str,
        args: &[Value],
    ) -> Result<Value>;
}

#[derive(Clone, Debug, Serialize)]
pub struct ObjectEntry {
    pub class: &'static str,
    /// Plain-data identity, e.g. `{"id":3,"generation":1}` for an entity.
    pub payload: Value,
}

/// Live objects. Ids are never reused within a table, so a dropped id stays dead.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ObjectTable {
    next: u64,
    objects: BTreeMap<u64, ObjectEntry>,
    singletons: BTreeMap<&'static str, u64>,
}
pub const MAX_OBJECTS: usize = 1 << 20;
impl ObjectTable {
    pub fn insert(&mut self, class: &'static str, payload: Value) -> Result<ObjectId> {
        ensure!(self.objects.len() < MAX_OBJECTS, "object table limit");
        self.next = self.next.checked_add(1).context("object id exhausted")?;
        self.objects
            .insert(self.next, ObjectEntry { class, payload });
        Ok(ObjectId(self.next))
    }
    /// Returns the existing object with identical class/payload, if any, so a
    /// handle resolves to one stable object id while it lives.
    pub fn intern(&mut self, class: &'static str, payload: Value) -> Result<ObjectId> {
        if let Some((id, _)) = self
            .objects
            .iter()
            .find(|(_, e)| e.class == class && e.payload == payload)
        {
            return Ok(ObjectId(*id));
        }
        self.insert(class, payload)
    }
    pub fn singleton(&mut self, class: &'static str) -> Result<ObjectId> {
        if let Some(id) = self.singletons.get(class) {
            return Ok(ObjectId(*id));
        }
        let id = self.insert(class, Value::Null)?;
        self.singletons.insert(class, id.0);
        Ok(id)
    }
    pub fn singletons(&self) -> impl Iterator<Item = (&'static str, ObjectId)> + '_ {
        self.singletons.iter().map(|(c, id)| (*c, ObjectId(*id)))
    }
    pub fn get(&self, id: ObjectId) -> Result<&ObjectEntry> {
        self.objects.get(&id.0).context("dead or unknown object")
    }
    pub fn remove(&mut self, id: ObjectId) -> Result<ObjectEntry> {
        self.objects.remove(&id.0).context("dead or unknown object")
    }
    pub fn len(&self) -> usize {
        self.objects.len()
    }
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

#[derive(Clone, Default)]
pub struct ClassRegistry {
    classes: BTreeMap<&'static str, Arc<dyn Class>>,
}
impl ClassRegistry {
    pub fn register(&mut self, class: Arc<dyn Class>) -> Result<()> {
        let name = class.info().name;
        ensure!(
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && name.chars().next().is_some_and(|c| c.is_ascii_uppercase()),
            "class name '{name}' must be an UpperCamel identifier"
        );
        ensure!(
            !self.classes.contains_key(name),
            "class '{name}' already registered"
        );
        self.classes.insert(name, class);
        Ok(())
    }
    pub fn get(&self, name: &str) -> Result<Arc<dyn Class>> {
        self.classes
            .get(name)
            .cloned()
            .with_context(|| format!("class '{name}' is not registered"))
    }
    pub fn infos(&self) -> Vec<&ClassInfo> {
        self.classes.values().map(|c| c.info()).collect()
    }
    /// TypeScript declarations for every registered class, used by the
    /// authoring tools and as the contract script hosts bind against.
    pub fn typescript_declarations(&self, singletons: &[(&str, &str)]) -> String {
        let mut out = String::from(
            "// Generated by crysirror-engine from the class registry. Do not edit.\n\
             type Vec3 = [number, number, number];\n",
        );
        for info in self.infos() {
            let _ = writeln!(out, "/** {} */\ndeclare class {} {{", info.doc, info.name);
            for p in &info.properties {
                let ro = if p.writable { "" } else { "readonly " };
                let _ = writeln!(
                    out,
                    "  /** {} */\n  {ro}{}: {};",
                    p.doc,
                    p.name,
                    p.ty.typescript()
                );
            }
            for m in &info.methods {
                let params = m
                    .params
                    .iter()
                    .map(|(n, t)| format!("{n}: {}", t.typescript()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = writeln!(
                    out,
                    "  /** {} */\n  {}({params}): {};",
                    m.doc,
                    m.name,
                    m.returns.typescript()
                );
            }
            out.push_str("}\n");
        }
        for (global, class) in singletons {
            let _ = writeln!(out, "declare const {global}: {class};");
        }
        out
    }
}
