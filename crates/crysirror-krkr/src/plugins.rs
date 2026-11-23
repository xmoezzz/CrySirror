// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
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
    // TPM is the native autoload extension for the same plugin module.
    Ok(match name.strip_suffix(".tpm") {
        Some(stem) => format!("{stem}.dll"),
        None => name,
    })
}

type Install = fn(&mut Rt, &str) -> Result<()>;

/// One built-in plug-in interface: its canonical DLL name, the members it
/// installs on first link and the shared capability those members map onto.
struct Facade {
    name: &'static str,
    install: Option<Install>,
    capability: &'static str,
}

macro_rules! facade {
    ($name:literal, $cap:literal) => {
        Facade { name: $name, install: None, capability: $cap }
    };
    ($name:literal, $cap:literal, $f:expr) => {
        Facade { name: $name, install: Some($f), capability: $cap }
    };
}

const FACADES: &[Facade] = &[
    facade!("savestruct.dll", "builtin structured serializers", |rt, _| { crate::storage_plugins::install_save_struct(rt); Ok(()) }),
    facade!("fstat.dll", "client-scoped storage metadata", |rt, _| { crate::storage_plugins::install_fstat(rt); Ok(()) }),
    // MenuItem is built in for the KRKR2 native profile. KRKRZ's menu.dll
    // names the same semantic adapter onto the engine MenuSpec service.
    facade!("menu.dll", "UI.MenuSpec"),
    facade!("kagparser.dll", "native KAG scenario VM", |rt, _| { crate::kag::install(rt); Ok(()) }),
    facade!("layerexraster.dll", "shared ImageEffect sine row copy (full clip/source-resource profile)", |rt, _| { crate::image_api::install_raster(rt); Ok(()) }),
    facade!("json.dll", "shared structured_text (bounded Japanese JSON profile)", |rt, _| { crate::json_plugin::install(rt); Ok(()) }),
    facade!("layerexdraw.dll", "shared VectorPath solid-brush path profile", |rt, _| { crate::draw_plugin::install(rt); Ok(()) }),
    // krmovie.dll is VideoOverlay's decoder module and exports no TJS
    // members; the core VideoOverlay class maps onto engine video.
    facade!("krmovie.dll", "VideoOverlay decoder module (engine video service)"),
    facade!("alphamovie.dll", "AlphaMovie (AJPM) through the shared media service", |rt, _| { crate::alpha_movie::install(rt); Ok(()) }),
    facade!("layereximage.dll", "shared ImageEffect CxImage colour/noise/blur profile", |rt, _| { crate::image_api::install_ex_image(rt); Ok(()) }),
    facade!("layerexbtoa.dll", "clipAlphaRect via shared AlphaClipRect and Canvas preparation"),
    facade!("layerexmovie.dll", "plain RGB layer movies through shared Media metadata/frame/loop services", |rt, _| { crate::layer_movie::install(rt); Ok(()) }),
    facade!("kagparserex.dll", "native KAG scenario VM, KAGParserEx taglist profile", |rt, _| { crate::kag::install_ex(rt); Ok(()) }),
    facade!("extkagparser.dll", "native ExtKAGParser VM while/local dictionary/multiline/parameter macro profile", |rt, _| { crate::kag::install_ext(rt); Ok(()) }),
    facade!("kropus.dll", "shared worker decoder registry, immutable audio resources, exact seek and common channel mixer", crate::audio_plugin::link),
    facade!("wuvorbis.dll", "shared worker decoder registry, immutable audio resources, exact seek and common channel mixer", crate::audio_plugin::link),
    facade!("wumsadp.dll", "shared worker decoder registry, immutable audio resources, exact seek and common channel mixer", crate::audio_plugin::link),
    facade!("fftgraph.dll", "shared spectrum graph raster over decoded source PCM", |rt, _| { crate::fftgraph::install(rt); Ok(()) }),
    facade!("textrender.dll", "shared crysirror_text::markup layout", |rt, _| { crate::text_render_plugin::install(rt); Ok(()) }),
    facade!("motionplayer.dll", "shared M2 motion workers, immutable textures and dynamic PuppetFrame"),
    facade!("emoteplayer.dll", "shared M2 motion workers, immutable textures and dynamic PuppetFrame"),
    facade!("motionplayer_nod3d.dll", "shared M2 motion workers, immutable textures and dynamic PuppetFrame"),
    facade!("csvparser.dll", "shared crysirror_runtime::csv reader", |rt, _| crate::csv_plugin::install(rt)),
    facade!("lineparser.dll", "shared crysirror_resource::text_stream::LineCursor", |rt, _| { crate::line_plugin::install(rt); Ok(()) }),
    facade!("dirlist.dll", "getDirList through shared client-scoped directory metadata/listing", |rt, _| { crate::storage_plugins::install_dirlist(rt); Ok(()) }),
    facade!("varfile.dll", "dynamic global octet namespace through portable storage media and shared image decoding", |rt, _| crate::varfile_plugin::install(rt)),
    facade!("psbfile.dll", "PSB via shared eluna_rs; psb:// images via the shared decoder registry", |rt, _| { crate::psbfile_plugin::install(rt); Ok(()) }),
    facade!("toml.dll", "Scripts.tomlDecodeFromStorage via shared structured_text TOML", |rt, _| { crate::json_plugin::install_toml(rt); Ok(()) }),
    // msdfrender.dll: scripts feature-detect its members
    // (`typeof Layer.makeGlyphSDF == "Object"`) and switch PreRenderFontEx
    // to the SDF glyph path when they exist.
    facade!("msdfrender.dll", "Layer glyph bitmaps from rendered layers (SDF/MSDF contrast, province glyph compositing, font ascent)", |rt, _| { crate::msdf_plugin::install(rt); Ok(()) }),
    facade!("scriptsex.dll", "Scripts reflection/struct helpers", |rt, _| crate::plugin_ports::scripts_ex::install_scripts_ex(rt)),
    facade!("addfont.dll", "System.addFont asynchronous shared client font registration, portable outline measurement and rendering", |rt, _| { crate::natives::install_add_font(rt); Ok(()) }),
    facade!("windowex.dll", "windowEx virtual-window profile", |rt, _| { crate::plugin_ports::window_ex::install(rt); Ok(()) }),
    facade!("win32dialog.dll", "WIN32Dialog template/state profile", |rt, _| { crate::plugin_ports::win32_dialog::install(rt); Ok(()) }),
    // Transition providers: no TJS members; beginTransition validates
    // the requested method and rejects unimplemented ones.
    facade!("extrans.dll", "shared transition providers"),
    facade!("extnagano.dll", "shared transition providers"),
    // Resource-only: GetOptionDesc descriptors. The chosen values reach the
    // game as `-name=value` options from `<exe>.cf` / `<datapath>/<exe>.cfu`.
    facade!("kagexopt.dll", "option descriptors; values through the .cf/.cfu argument stock"),
    facade!("wuopus.dll", "shared worker decoder registry, immutable audio resources, exact seek and common channel mixer", crate::audio_plugin::link),
    facade!("getlangname.dll", "System.getCurrentUILangName / getCurrentLocaleName from the configured user locale", |rt, _| { crate::system_plugins::install_lang_name(rt); Ok(()) }),
    facade!("lzfs.dll", "lzfs storage media: engine search with shared LZ4 frame passthrough", |rt, _| crate::wrapper_plugins::install_lzfs(rt)),
    facade!("yuzuex.dll", "ProxyStorageMap and the proxy storage media over the engine search", |rt, _| crate::wrapper_plugins::install_yuzuex(rt)),
    facade!("getsample.dll", "WaveSoundBuffer sample windows from the shared source-PCM service", |rt, _| { crate::get_sample::install(rt); Ok(()) }),
    facade!("k2compat.dll", "Window.TouchMouse pointer forwarding and a virtual ModelessOwnerWindow", |rt, _| { crate::plugin_ports::k2compat::install(rt); Ok(()) }),
    facade!("dmmcloud.dll", "DMMCloud without a DMM GAME PLAYER bridge (IPC answers false)", |rt, _| { crate::offline_plugins::install_dmmcloud(rt); Ok(()) }),
    facade!("krkrsteam.dll", "Steam static class for a game not started by Steam", |rt, _| { crate::offline_plugins::install_steam(rt); Ok(()) }),
    facade!("drawdeviced2d.dll", "DrawDeviceD2D as the shared retained-layer presentation device", |rt, _| { crate::offline_plugins::install_draw_device(rt, "DrawDeviceD2D"); Ok(()) }),
    facade!("drawdeviced2dm.dll", "DrawDeviceD2D as the shared retained-layer presentation device", |rt, _| { crate::offline_plugins::install_draw_device(rt, "DrawDeviceD2D"); Ok(()) }),
    facade!("drawdeviced2dv2.dll", "DrawDeviceD2D as the shared retained-layer presentation device", |rt, _| { crate::offline_plugins::install_draw_device(rt, "DrawDeviceD2D"); Ok(()) }),
    facade!("steamdrawdevice.dll", "DualDrawDevice_2/_Z as the shared retained-layer presentation device", |rt, _| { crate::offline_plugins::install_draw_device(rt, "DualDrawDevice_2"); crate::offline_plugins::install_draw_device(rt, "DualDrawDevice_Z"); Ok(()) }),
    facade!("win32ole.dll", "WIN32OLE/ActiveX script plumbing; no OLE automation transport exists (objects fail like an unregistered ProgID)", |rt, _| { crate::offline_plugins::install_win32ole(rt); Ok(()) }),
    facade!("wfbasiceffect.dll", "WaveSoundBuffer.DelayEffect/StkFreeVerb/GraphicEqualizer as shared crysirror_audio::wave_filters stages", |rt, _| crate::wf_plugins::install_basic(rt)),
    facade!("wftypicaldsp.dll", "WaveSoundBuffer.WaveDSPFilter as shared crysirror_audio::iir_design cascades", |rt, _| crate::wf_plugins::install_dsp(rt)),
    facade!("layerstwcopy.dll", "Layer.stitchWrappedCopy over the shared StitchWrappedCopy image pair", |rt, _| { crate::layer_ex::install_stw_copy(rt); Ok(()) }),
    facade!("gamepad.dll", "GamepadPort with no controller devices (the engine exposes none)", |rt, _| { crate::offline_plugins::install_gamepad(rt); Ok(()) }),
    facade!("wumultitrack.dll", ".mtrk/.mchx multi-track music over the shared multi-track voice; mask, delayed mask, rebind and gain-callback members", |rt, _| crate::multitrack::install(rt)),
    facade!("glitcheffect.dll", "Layer.glitchCopy/doGlitch over the shared GlitchCopy image pair; glitch/fadeglitch/loopglitch transitions", |rt, _| { crate::layer_ex::install_glitch(rt); Ok(()) }),
    facade!("wvdecoder.dll", "WaveSoundData.F32/S16 sample buffers through the shared audio resource and source-sample services", |rt, _| { crate::wv_decoder::install(rt); Ok(()) }),
    facade!("packinone.dll", "bundled layer/system/storage members, fstat directory queries and the SliceLayer class", |rt, _| {
        crate::storage_plugins::install_fstat(rt);
        crate::system_ex::install_slice_layer(rt)
    }),
    facade!("psd.dll", "PSD class and psd:// layer media over the shared crysirror_image::psd decoder", |rt, _| crate::psd_plugin::install(rt)),
    facade!("httprequest.dll", "HttpRequest over the engine's owner-authorized HTTP service", |rt, _| { crate::http_plugin::install(rt); Ok(()) }),
    facade!("kaichotrans.dll", "shared GPU subtree byte-domain blur and grayscale/blur/negative rule dim transitions"),
];

/// Whether `name` (canonical) is a built-in interface, never a plain shell.
pub(crate) fn is_facade(name: &str) -> bool {
    facade(name).is_some()
}

fn facade(name: &str) -> Option<&'static Facade> {
    FACADES.iter().find(|f| f.name == name)
}

fn kind(rt: &Rt, name: &str) -> Option<&'static str> {
    if facade(name).is_some() {
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
                let first = !rt.host().loaded_plugins.contains(&name);
                // The M2 Motion distributions share one global class set.
                if matches!(name.as_str(), "motionplayer.dll" | "motionplayer_nod3d.dll" | "emoteplayer.dll") {
                    crate::motion_plugin::install(rt);
                }
                if first && crate::surfaces::known(&name) {
                    crate::surfaces::install(rt, &name);
                }
                if first && let Some(install) = facade(&name).and_then(|f| f.install) {
                    install(rt, &name)?;
                }
                rt.host_mut().loaded_plugins.insert(name);
                Ok(Variant::Void)
            }),
            ("linkZ", |rt, _, args| crate::protect_plugin::link_z(rt, args)),
            ("unlink", |rt, _, args| {
                let name = canonical(&string(&args, 0, "Plugins.unlink")?)?;
                // A live engine facade owns native objects. Unloading their class
                // while those objects exist is unsupported, as opposed to a false
                // success that leaves an apparently unlinked working implementation.
                if facade(&name).is_some() && rt.host().loaded_plugins.contains(&name) {
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
                let capability = facade(&name).map_or("", |f| f.capability);
                rt.set_object_member(h, "capability", Variant::String(capability.into()));
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

/// PluginImpl::TVPLoadPluigins: ASCII filename order, executable/system/plugin
/// directories, TPM files only. Implemented facades register through Plugins.link;
/// unknown modules fail visibly before executing startup.tjs.
pub(crate) fn autoload(rt: &mut Rt) -> anyhow::Result<()> {
    let modules = rt.host().storage()?.autoload_modules(rt.host().exe_bits == 64)?;
    let plugins = rt.global_member("Plugins").object_handle()
        .ok_or_else(|| anyhow::anyhow!("Plugins namespace absent"))?;
    for module in modules {
        rt.call_object_method(plugins, "link", vec![Variant::String(module.clone())])
            .map_err(|e| anyhow::anyhow!("autoload {module}: {e}"))?;
    }
    Ok(())
}
