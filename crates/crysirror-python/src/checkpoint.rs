// SPDX-License-Identifier: MPL-2.0
//! Bounded in-process snapshots that restore mutable Python objects in place.

use crate::{ObjRef, Payload, Value, dict::Dict};
use std::collections::HashSet;

pub struct Checkpoint {
    objects: Vec<ObjectState>,
    bytes: usize,
}

struct ObjectState {
    target: ObjRef,
    attrs: Option<Dict>,
    contents: Contents,
}

enum Contents {
    None,
    List(Vec<Value>),
    Map(Dict),
    Bytes(Vec<u8>),
    Cell(Option<Value>),
}

impl Checkpoint {
    /// Captures object identities and mutable contents without invoking Python.
    /// The embedding runtime selects the roots and any excluded subgraphs.
    pub fn capture(
        roots: impl IntoIterator<Item = Value>,
        exclude: impl Fn(&ObjRef) -> bool,
        max_bytes: usize,
    ) -> Result<Self, &'static str> {
        let mut checkpoint = Self {
            objects: Vec::new(),
            bytes: 0,
        };
        let mut pending = Vec::new();
        let mut visited = HashSet::new();
        let max_objects = (max_bytes / 32).min(250_000);
        push(&mut pending, roots, max_objects)?;
        while let Some(value) = pending.pop() {
            let Value::Obj(object) = value else {
                continue;
            };
            if !visited.insert(std::rc::Rc::as_ptr(&object)) || exclude(&object) {
                continue;
            }
            if visited.len() > max_objects {
                return Err("Python checkpoint object limit");
            }
            if matches!(
                object.payload,
                Payload::Type(_) | Payload::Module(..) | Payload::Native(_)
            ) {
                continue;
            }
            let attrs = object
                .dict
                .as_ref()
                .map(|table| {
                    let table = table.borrow();
                    checkpoint.map(&table, &mut pending, max_objects, max_bytes)?;
                    Ok(table.clone())
                })
                .transpose()?;
            let contents = match &object.payload {
                Payload::List(items) => {
                    let items = items.borrow();
                    checkpoint
                        .reserve(items.len().saturating_mul(size_of::<Value>()), max_bytes)?;
                    push(&mut pending, items.iter().cloned(), max_objects)?;
                    Contents::List(items.clone())
                }
                Payload::Dict(items) | Payload::Set(items) => {
                    let items = items.borrow();
                    checkpoint.map(&items, &mut pending, max_objects, max_bytes)?;
                    Contents::Map(items.clone())
                }
                Payload::ByteArray(data) => {
                    let data = data.borrow();
                    checkpoint.reserve(data.len(), max_bytes)?;
                    Contents::Bytes(data.clone())
                }
                Payload::Cell(cell) => {
                    let cell = cell.borrow().clone();
                    push(&mut pending, cell.iter().cloned(), max_objects)?;
                    Contents::Cell(cell)
                }
                Payload::Tuple(items) => {
                    push(&mut pending, items.iter().cloned(), max_objects)?;
                    Contents::None
                }
                Payload::FrozenSet(items) => {
                    push(
                        &mut pending,
                        items.iter().map(|entry| entry.key.clone()),
                        max_objects,
                    )?;
                    Contents::None
                }
                Payload::Function(function) => {
                    push(&mut pending, function.defaults.iter().cloned(), max_objects)?;
                    push(
                        &mut pending,
                        function.closure.iter().cloned().map(Value::Obj),
                        max_objects,
                    )?;
                    push(
                        &mut pending,
                        function.kwdefaults.iter().cloned().map(Value::Obj),
                        max_objects,
                    )?;
                    Contents::None
                }
                Payload::Method { func, slf } => {
                    push(&mut pending, [func.clone(), slf.clone()], max_objects)?;
                    Contents::None
                }
                _ => Contents::None,
            };
            if attrs.is_some() || !matches!(contents, Contents::None) {
                checkpoint.reserve(size_of::<ObjectState>(), max_bytes)?;
                checkpoint.objects.push(ObjectState {
                    target: object,
                    attrs,
                    contents,
                });
            }
        }
        Ok(checkpoint)
    }

    fn reserve(&mut self, bytes: usize, limit: usize) -> Result<(), &'static str> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|bytes| *bytes <= limit)
            .ok_or("Python checkpoint byte limit")?;
        Ok(())
    }

    fn map(
        &mut self,
        contents: &Dict,
        pending: &mut Vec<Value>,
        max_objects: usize,
        max_bytes: usize,
    ) -> Result<(), &'static str> {
        self.reserve(contents.slots().saturating_mul(96), max_bytes)?;
        push(
            pending,
            contents
                .iter()
                .flat_map(|entry| [entry.key.clone(), entry.value.clone()]),
            max_objects,
        )
    }

    pub fn byte_count(&self) -> usize {
        self.bytes
    }

    pub fn restore(&self) {
        for state in self.objects.iter().rev() {
            if let (Some(live), Some(checkpoint)) = (&state.target.dict, &state.attrs) {
                live.replace(checkpoint.clone());
            }
            match (&state.target.payload, &state.contents) {
                (Payload::List(live), Contents::List(value)) => {
                    live.replace(value.clone());
                }
                (Payload::Dict(live) | Payload::Set(live), Contents::Map(value)) => {
                    live.replace(value.clone());
                }
                (Payload::ByteArray(live), Contents::Bytes(value)) => {
                    live.replace(value.clone());
                }
                (Payload::Cell(live), Contents::Cell(value)) => {
                    live.replace(value.clone());
                }
                _ => {}
            }
        }
    }
}

fn push(
    pending: &mut Vec<Value>,
    values: impl IntoIterator<Item = Value>,
    limit: usize,
) -> Result<(), &'static str> {
    for value in values {
        if let Value::Obj(_) = value {
            if pending.len() >= limit {
                return Err("Python checkpoint traversal limit");
            }
            pending.push(value);
        }
    }
    Ok(())
}
