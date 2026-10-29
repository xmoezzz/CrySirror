// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Native module registration, never an OS DLL loader. A registration-only
//! shell has no native exports and cannot masquerade as an implemented API.
use crate::natives::{Rt, methods, string, unsupported};
use crysirror_tjs2::{Result, TjsError, runtime::Variant};

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
    if matches!(
        name,
        "savestruct.dll" | "fstat.dll" | "menu.dll" | "kagparser.dll" | "layerexraster.dll" | "json.dll" | "layerexdraw.dll"
            // krmovie.dll is VideoOverlay's decoder module and exports no TJS
            // members; the core VideoOverlay class maps onto engine video.
            | "krmovie.dll"
            | "layereximage.dll"
            | "kagparserex.dll"
            | "textrender.dll"
            | "motionplayer.dll"
            | "motionplayer_nod3d.dll"
            | "csvparser.dll"
            | "psbfile.dll"
            | "toml.dll"
            // msdfrender.dll: its members are feature-detected by the scripts
            // (`typeof Layer.makeGlyphSDF == "Object"`). The SDF glyph
            // generators are not mapped, so linking installs none of them and
            // PreRenderFontEx takes its atlas stretchCopy path.
            | "msdfrender.dll"
            | "scriptsex.dll"
            | "addfont.dll"
            | "windowex.dll"
            | "win32dialog.dll"
            // Transition providers: no TJS members; beginTransition validates
            // the requested method and rejects unimplemented ones.
            | "extrans.dll"
            | "extnagano.dll"
    ) {
        Some("engine_facade")
    } else if crate::surfaces::known(name) {
        Some("known_surface")
    } else if rt.host().plugin_shells.contains(name) {
        Some("registration_only")
    } else {
        None
    }
}

pub fn install(rt: &mut Rt, plugins: crysirror_tjs2::runtime::ObjectHandle) {
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
                if name == "savestruct.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::storage_plugins::install_save_struct(rt);
                }
                if name == "fstat.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::storage_plugins::install_fstat(rt);
                }
                if name == "kagparser.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::kag::install(rt);
                }
                if name == "layerexraster.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::image_api::install_raster(rt);
                }
                if matches!(name.as_str(), "motionplayer.dll" | "motionplayer_nod3d.dll") {
                    crate::motion_plugin::install(rt);
                }
                if crate::surfaces::known(&name) && !rt.host().loaded_plugins.contains(&name) {
                    crate::surfaces::install(rt, &name);
                }
                if name == "win32dialog.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::kirakira::win32_dialog::install(rt);
                }
                if name == "windowex.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::kirakira::window_ex::install(rt);
                }
                if name == "addfont.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::natives::install_add_font(rt);
                }
                if name == "scriptsex.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::kirakira::scripts_ex::install_scripts_ex(rt);
                }
                if name == "csvparser.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::csv_plugin::install(rt);
                }
                if name == "toml.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::json_plugin::install_toml(rt);
                }
                if name == "psbfile.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::psbfile_plugin::install(rt);
                }
                if name == "textrender.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::text_render_plugin::install(rt);
                }
                if name == "kagparserex.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::kag::install_ex(rt);
                }
                if name == "layereximage.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::image_api::install_ex_image(rt);
                }
                if name == "json.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::json_plugin::install(rt);
                }
                if name == "layerexdraw.dll" && !rt.host().loaded_plugins.contains(&name) {
                    crate::draw_plugin::install(rt);
                }
                // MenuItem is built in for the KRKR2 native profile. KRKRZ's menu.dll
                // names the same semantic adapter onto the engine MenuSpec service.
                rt.host_mut().loaded_plugins.insert(name);
                Ok(Variant::Void)
            }),
            ("linkZ", |rt, _, args| crate::protect_plugin::link_z(rt, args)),
            ("unlink", |rt, _, args| {
                let name = canonical(&string(&args, 0, "Plugins.unlink")?)?;
                // A live engine facade owns native objects. Unloading their class
                // while those objects exist is unsupported, as opposed to a false
                // success that leaves an apparently unlinked working implementation.
                if matches!(
                    name.as_str(),
                    "menu.dll"
                        | "kagparser.dll"
                        | "layerexraster.dll"
                        | "json.dll"
                        | "layerexdraw.dll"
                        | "krmovie.dll"
            | "layereximage.dll"
            | "kagparserex.dll"
            | "textrender.dll"
            | "motionplayer.dll"
            | "motionplayer_nod3d.dll"
            | "csvparser.dll"
            | "psbfile.dll"
            | "toml.dll"
            // msdfrender.dll: its members are feature-detected by the scripts
            // (`typeof Layer.makeGlyphSDF == "Object"`). The SDF glyph
            // generators are not mapped, so linking installs none of them and
            // PreRenderFontEx takes its atlas stretchCopy path.
            | "msdfrender.dll"
            | "scriptsex.dll"
            | "addfont.dll"
            | "windowex.dll"
            | "win32dialog.dll"
            // Transition providers: no TJS members; beginTransition validates
            // the requested method and rejects unimplemented ones.
            | "extrans.dll"
            | "extnagano.dll"
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
                        if name == "layerexdraw.dll" {
                            "shared VectorPath solid-brush path profile"
                        } else if name.starts_with("motionplayer") {
                            "shared M2 motion workers, immutable textures and dynamic PuppetFrame; single-timeline profile"
                        } else if name == "addfont.dll" {
                            "System.addFont validated by the shared text loader"
                        } else if name == "scriptsex.dll" {
                            "Scripts reflection/struct helpers (Kirakira scriptsEx)"
                        } else if name == "csvparser.dll" {
                            "shared crysirror_runtime::csv reader"
                        } else if name == "msdfrender.dll" {
                            "linked without SDF members (feature-detected); atlas glyph path"
                        } else if name == "toml.dll" {
                            "Scripts.tomlDecodeFromStorage via shared structured_text TOML"
                        } else if name == "psbfile.dll" {
                            "PSB via shared eluna_rs; psb:// images via the shared decoder registry"
                        } else if name == "textrender.dll" {
                            "shared crysirror_text::markup layout"
                        } else if name == "kagparserex.dll" {
                            "native KAG scenario VM, KAGParserEx taglist profile"
                        } else if name == "layereximage.dll" {
                            "shared ImageEffect CxImage colour/noise/blur profile"
                        } else if name == "krmovie.dll" {
                            "VideoOverlay decoder module (engine video service)"
                        } else if name == "json.dll" {
                            "shared structured_text (bounded Japanese JSON profile)"
                        } else if name == "menu.dll" {
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
