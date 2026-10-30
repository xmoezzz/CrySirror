// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or later.
//! saveStruct/fstat native semantics over CrySirror's client storage services.
//! saveStruct reference: wtnbgo/saveStruct f03a81a46018d89d20d1e4614006fc502fd12d2c.
use crate::natives::{Rt, int, methods, string, this_of, unsupported};
use crysirror_runtime::text_storage::{self, LineEnding, TextEncoding};
use crysirror_tjs2::{Result, TjsError, runtime::Variant};
fn error(e: impl std::fmt::Display) -> TjsError {
    TjsError::runtime(e.to_string())
}

pub fn install_save_struct(rt: &mut Rt) {
    let Some(array) = rt.global_member("Array").object_handle() else {
        return;
    };
    let save =
        rt.register_object_native(array, "save2", |rt: &mut Rt, this, args: Vec<Variant>| {
            if args.is_empty() {
                return Err(TjsError::bad_param_count());
            }
            let target = this_of(rt, this, "Array.save2")?;
            let name = string(&args, 0, "Array.save2")?;
            let elements = rt
                .array_elements(target)
                .ok_or_else(|| error("Array.save2 requires an Array"))?;
            let lines = elements
                .iter()
                .map(|v| match v {
                    Variant::String(s) => Ok(s.as_str()),
                    _ => Err(error("Array.save2 requires string elements")),
                })
                .collect::<Result<Vec<_>>>()?;
            let encoding = if int(&args, 1, 0)? != 0 {
                TextEncoding::Utf8
            } else {
                TextEncoding::Cp932
            };
            let ending = if int(&args, 2, 0)? == 1 {
                LineEnding::Lf
            } else {
                LineEnding::CrLf
            };
            let bytes = text_storage::encode_lines(lines, encoding, ending).map_err(error)?;
            if !rt.host().can_suspend {
                return Err(unsupported(
                    "Array.save2 in this callback",
                    "resumable atomic write",
                ));
            }
            if rt.host().writes.len() >= 64 {
                return Err(error("native text write queue budget"));
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
    rt.register_array_extension("save2", Variant::Object(save));
    for class_name in ["Array", "Dictionary", "Scripts"] {
        let Some(class) = rt.global_member(class_name).object_handle() else {
            continue;
        };
        for method in ["saveStruct2", "toStructString"] {
            if class_name == "Scripts" && method == "saveStruct2" {
                continue;
            }
            let h =
                rt.register_object_native(class, method, move |_: &mut Rt, _, _: Vec<Variant>| {
                    Err(unsupported(method, "saveStruct2 typed expression writer"))
                });
            if class_name == "Array" {
                rt.register_array_extension(method, Variant::Object(h));
            }
        }
    }
}

pub fn install_fstat(rt: &mut Rt) {
    let Some(storages) = rt.global_member("Storages").object_handle() else {
        return;
    };
    rt.register_object_native(storages, "dirlist", |rt: &mut Rt, _, args: Vec<Variant>| {
        let name = string(&args, 0, "Storages.dirlist")?;
        if !name.ends_with('/') {
            return Err(error("Missing slash at end of directory name."));
        }
        let entries = if crate::varfile_plugin::is_uri(&name) {
            crate::varfile_plugin::list(rt, &name)?
        } else {
            rt.host()
                .storage()?
                .directory_entries(&name)
                .map_err(error)?
        };
        Ok(Variant::Object(rt.alloc_array_object(
            entries.into_iter().map(Variant::String).collect(),
        )))
    });
    rt.register_object_native(
        storages,
        "isExistentDirectory",
        |rt: &mut Rt, _, args: Vec<Variant>| {
            let name = string(&args, 0, "Storages.isExistentDirectory")?;
            if !name.ends_with('/') {
                return Err(error("Missing slash at end of directory name."));
            }
            let info = rt.host().storage()?.metadata(&name).map_err(error)?;
            Ok(Variant::Integer(info.is_some_and(|m| m.is_directory) as i64))
        },
    );
    rt.register_object_native(storages, "fstat", |rt: &mut Rt, _, args: Vec<Variant>| {
        let name = string(&args, 0, "Storages.fstat")?;
        let info = rt
            .host()
            .storage()?
            .metadata(&name)
            .map_err(error)?
            .ok_or_else(|| error(format!("cannot open : {name}")))?;
        let dict = rt.alloc_dictionary_object();
        if let Some(size) = info.size {
            rt.set_object_member(
                dict,
                "size",
                Variant::Integer(i64::try_from(size).map_err(error)?),
            );
        }
        for (key, millis) in [
            ("ctime", info.created_ms),
            ("mtime", info.modified_ms),
            ("atime", info.accessed_ms),
        ] {
            if let Some(millis) = millis {
                let date =
                    rt.call_function(rt.global_member("Date"), vec![Variant::Integer(millis)])?;
                rt.set_object_member(dict, key, date);
            }
        }
        Ok(Variant::Object(dict))
    });
    // Save-mount file operations (safeSaveStruct backup rotation).
    methods(
        rt,
        storages,
        &[
            ("deleteFile", |rt, _, a| {
                let name = string(&a, 0, "Storages.deleteFile")?;
                let storage = rt.host().storage()?;
                Ok(Variant::Integer(
                    storage.delete_save(&name).map_err(error)? as i64
                ))
            }),
            ("moveFile", |rt, _, a| {
                let from = string(&a, 0, "Storages.moveFile")?;
                let to = string(&a, 1, "Storages.moveFile")?;
                let storage = rt.host().storage()?;
                Ok(Variant::Integer(
                    storage.move_save(&from, &to).map_err(error)? as i64,
                ))
            }),
            ("copyFile", |rt, _, a| {
                let from = string(&a, 0, "Storages.copyFile")?;
                let to = string(&a, 1, "Storages.copyFile")?;
                let storage = rt.host().storage()?;
                Ok(Variant::Integer(
                    storage.copy_to_save(&from, &to).map_err(error)? as i64,
                ))
            }),
            // Save-mount directories exist implicitly (writes create parents).
            ("createDirectory", |_, _, a| {
                let name = string(&a, 0, "Storages.createDirectory")?;
                Ok(Variant::Integer(
                    crate::storage::save_path(name.trim_end_matches('/'))
                        .map_err(error)?
                        .is_some() as i64,
                ))
            }),
        ],
    );
    for method in [
        "getTime",
        "setTime",
        "getLastModifiedFileTime",
        "setLastModifiedFileTime",
        "exportFile",
        "truncateFile",
        "dirlistEx",
        "removeDirectory",
        "createDirectoryNoNormalize",
        "changeDirectory",
        "setFileAttributes",
        "resetFileAttributes",
        "getFileAttributes",
        "selectDirectory",
        "copyFile",
        "copyFileNoNormalize",
        "getDisplayName",
        "getMD5HashString",
        "searchPath",
        "getTemporaryName",
        "clearStorageCaches",
    ] {
        rt.register_object_native(storages, method, move |_: &mut Rt, _, _: Vec<Variant>| {
            Err(unsupported(
                method,
                "scoped fstat operation with native completion semantics",
            ))
        });
    }
}

/// dirlist.dll's global getDirList. Reference: complete krkrz/krkr2 Main.cpp
/// at dec49af97e174d31059c3ccd7efc700ba3c6b788, native dirlist folder.
/// Native local names are mapped to the client's scoped engine filesystem;
/// listing never grants access outside that capability (including ../).
pub fn install_dirlist(rt: &mut Rt) {
    rt.register_global_native("getDirList", |rt: &mut Rt, _, args: Vec<Variant>| {
        let name = string(&args, 0, "getDirList")?;
        if !name.ends_with('/') {
            return Err(error(
                "'/' must be specified at the end of given directory name.",
            ));
        }
        if crate::varfile_plugin::is_uri(&name) || name.contains('>') {
            return Err(unsupported(
                "getDirList non-file media",
                "a capability-scoped local directory",
            ));
        }
        let mut entries = rt
            .host()
            .storage()?
            .directory_entries(&name)
            .map_err(error)?;
        if entries.len() > 65534 {
            return Err(error("getDirList entry budget"));
        }
        // Win32 FindFirstFile also enumerates these directory names. Returning
        // a name does not grant the capability to follow it outside the root.
        entries.insert(0, "../".into());
        entries.insert(0, "./".into());
        Ok(Variant::Object(rt.alloc_array_object(
            entries.into_iter().map(Variant::String).collect(),
        )))
    });
}
