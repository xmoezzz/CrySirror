// SPDX-License-Identifier: MPL-2.0
//! Ren'Py 6.99's zlib-wrapped persistent pickle under the client save root.
use crate::engine::{Engine, state};
use crysirror_core::{
    Error,
    path::{Mount, VfsPath},
};
use crysirror_python::{PyResult, Value, Vm, builtins::Kw, import, ops, pickle};
use flate2::{Compression, bufread::ZlibDecoder, write::ZlibEncoder};
use std::io::{Read, Write};

const MAX_BYTES: usize = 8 * 1024 * 1024;

pub fn restore(engine: &mut Engine) -> Result<(), String> {
    let files = engine.st.files.clone();
    let Some(storage) = files.temporary_storage() else {
        return Ok(());
    };
    let mut failure = None;
    for name in ["persistent.new", "persistent"] {
        let path = VfsPath::new(Mount::Save, name).map_err(|error| error.to_string())?;
        if let Ok(Some(info)) = storage.metadata(Mount::Save, name)
            && (info.is_directory || info.size.is_some_and(|size| size > MAX_BYTES as u64))
        {
            failure = Some(format!("{name} exceeds the persistent file limit"));
            continue;
        }
        let bytes = match storage.read(&path) {
            Ok(bytes) => bytes,
            Err(Error::NotFound) => continue,
            Err(error) => {
                failure = Some(format!("{name}: {error}"));
                continue;
            }
        };
        match decode(&mut engine.vm, &bytes) {
            Ok(value) => {
                engine.call_glue("_install_persistent", vec![value])?;
                return Ok(());
            }
            Err(error) => {
                failure = Some(error);
            }
        }
    }
    if let Some(error) = failure {
        engine
            .st
            .log
            .borrow_mut()
            .push(format!("Ren'Py persistent profile: {error}"));
    }
    Ok(())
}

fn decode(vm: &mut Vm, bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_BYTES {
        return Err("compressed data exceeds limit".into());
    }
    let mut raw = Vec::new();
    let mut decoder = ZlibDecoder::new(bytes);
    (&mut decoder)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    if raw.len() > MAX_BYTES || decoder.total_in() as usize != bytes.len() {
        return Err("invalid persistent ZLIB stream length".into());
    }
    let options = pickle::UnpickleOptions::default();
    let mut find_class = |vm: &mut Vm, module: &str, name: &str| -> PyResult<Value> {
        match (module, name) {
            ("renpy.persistent", "Persistent") | ("renpy.preferences", "Preferences") => {
                pickle::default_find_class(vm, module, name)
            }
            ("__builtin__" | "builtins", "set" | "frozenset" | "object") => {
                pickle::default_find_class(vm, "builtins", name)
            }
            ("renpy.python", "RevertableSet") => Ok(Value::Obj(vm.types.set.clone())),
            ("renpy.python", "RevertableList") => Ok(Value::Obj(vm.types.list.clone())),
            ("renpy.python", "RevertableDict") => Ok(Value::Obj(vm.types.dict.clone())),
            _ => Err(vm.value_error(format!("unsupported persistent class {module}.{name}"))),
        }
    };
    let decoded = pickle::loads(vm, &raw, &mut find_class, &options)
        .map_err(|error| crate::rpyc::py_err(vm, error))?;
    let class = import::get_module(vm, "renpy.persistent")
        .and_then(|module| ops::get_raw_attr(module.obj()?, "Persistent"));
    if !decoded.obj().is_some_and(|object| {
        class
            .as_ref()
            .and_then(Value::obj)
            .is_some_and(|class| std::rc::Rc::ptr_eq(&object.class(), class))
    }) {
        return Err("unexpected persistent root".into());
    }
    Ok(decoded)
}

pub fn save(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [persistent] = args else {
        return Err(vm.type_error("save_persistent expects the current persistent object"));
    };
    let st = state(vm);
    let storage = st
        .files
        .temporary_storage()
        .ok_or_else(|| vm.exc_msg("OSError", "Ren'Py save storage is unavailable"))?;
    let path = VfsPath::new(Mount::Save, "persistent")
        .map_err(|error| vm.exc_msg("OSError", error.to_string()))?;
    let serialized = pickle::dumps(vm, persistent, 2)?;
    if serialized.len() > MAX_BYTES {
        return Err(vm.value_error("Ren'Py persistent file exceeds limit"));
    }
    let mut output = ZlibEncoder::new(Vec::new(), Compression::default());
    output
        .write_all(&serialized)
        .map_err(|error| vm.exc_msg("OSError", error.to_string()))?;
    let bytes = output
        .finish()
        .map_err(|error| vm.exc_msg("OSError", error.to_string()))?;
    if bytes.len() > MAX_BYTES {
        return Err(vm.value_error("Ren'Py persistent file exceeds compressed limit"));
    }
    storage
        .write_atomic(&path, &bytes)
        .map_err(|error| vm.exc_msg("OSError", error.to_string()))?;
    let recovery = VfsPath::new(Mount::Save, "persistent.new")
        .map_err(|error| vm.exc_msg("OSError", error.to_string()))?;
    match storage.remove(&recovery) {
        Ok(()) | Err(Error::NotFound) => {}
        Err(error) => return Err(vm.exc_msg("OSError", error.to_string())),
    }
    Ok(Value::None)
}
