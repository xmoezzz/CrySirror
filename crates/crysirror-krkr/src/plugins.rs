// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Native module registration, never an OS DLL loader. A registration-only
//! shell has no native exports and cannot masquerade as an implemented API.
use crate::natives::{Rt, methods, string, unsupported};
use krkr_tjs2::{Result, TjsError, runtime::Variant};

pub fn canonical(name: &str) -> Result<String> {
    let name = name.replace('\\', "/");
    let name = name
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.is_empty() || name.len() > 128 || name.contains('\0') {
        return Err(TjsError::runtime("invalid native plugin name"));
    }
    Ok(name)
}

fn kind(rt: &Rt, name: &str) -> Option<&'static str> {
    if matches!(name, "menu.dll" | "kagparser.dll" | "layerexraster.dll") {
        Some("engine_facade")
    } else if rt.host().plugin_shells.contains(name) {
        Some("registration_only")
    } else {
        None
    }
}

pub fn install(rt: &mut Rt, plugins: krkr_tjs2::runtime::ObjectHandle) {
    methods(
        rt,
        plugins,
        &[
            ("finalize", |_, _, _| Ok(Variant::Void)), // Original native base has an empty finalizer.
            ("link", |rt, _, args| {
                let name = canonical(&string(&args, 0, "Plugins.link")?)?;
                if kind(rt, &name).is_none() {
                    return Err(unsupported(
                        &format!("Plugins.link(\"{name}\")"),
                        "an engine facade or explicitly configured registration-only shell; native DLLs are never loaded",
                    ));
                }
                if name == "kagparser.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::kag::install(rt);
                }
                if name == "layerexraster.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::image_api::install_raster(rt);
                }
                // MenuItem is built in for the KRKR2 native profile. KRKRZ's menu.dll
                // names the same semantic adapter onto the engine MenuSpec service.
                rt.host_mut().loaded_plugins.insert(name);
                Ok(Variant::Void)
            }),
            ("unlink", |rt, _, args| {
                let name = canonical(&string(&args, 0, "Plugins.unlink")?)?;
                // A live engine facade owns native objects. Unloading their class
                // while those objects exist is unsupported, as opposed to a false
                // success that leaves an apparently unlinked working implementation.
                if matches!(
                    name.as_str(),
                    "menu.dll" | "kagparser.dll" | "layerexraster.dll"
                ) && rt.host().loaded_plugins.contains(&name)
                {
                    return Ok(Variant::Integer(0));
                }
                Ok(Variant::Integer(
                    rt.host_mut().loaded_plugins.remove(&name) as i64
                ))
            }),
            ("getList", |rt, _, _| {
                let names = rt
                    .host()
                    .loaded_plugins
                    .iter()
                    .cloned()
                    .map(Variant::String)
                    .collect();
                Ok(Variant::Object(rt.alloc_array_object(names)))
            }),
            ("getStatus", |rt, _, args| {
                let name = canonical(&string(&args, 0, "Plugins.getStatus")?)?;
                let h = rt.alloc_dictionary_object();
                rt.set_object_member(h, "name", Variant::String(name.clone()));
                rt.set_object_member(
                    h,
                    "kind",
                    Variant::String(kind(rt, &name).unwrap_or("unsupported").into()),
                );
                rt.set_object_member(
                    h,
                    "loaded",
                    Variant::Integer(rt.host().loaded_plugins.contains(&name) as i64),
                );
                rt.set_object_member(
                    h,
                    "capability",
                    Variant::String(
                        if name == "menu.dll" {
                            "UI.MenuSpec"
                        } else if name == "layerexraster.dll" {
                            "shared ImageEffect sine row copy (full clip/source-resource profile)"
                        } else if name == "kagparser.dll" {
                            "native KAG scenario VM"
                        } else {
                            ""
                        }
                        .into(),
                    ),
                );
                Ok(Variant::Object(h))
            }),
        ],
    );
    rt.register_global_native("CanLoadPlugin", |rt: &mut Rt, _, args: Vec<Variant>| {
        let name = canonical(&string(&args, 0, "CanLoadPlugin")?)?;
        // Availability means a supported registration, never the presence of
        // an arbitrary binary that this process cannot execute.
        Ok(Variant::Integer(kind(rt, &name).is_some() as i64))
    });
}
