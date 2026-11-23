// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! lzfs / yuzuex wrapper media, getLangName and the option-only plug-ins.
//! Self-authored fixtures only.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;
use std::io::Write;

fn run(files: &[(&str, Vec<u8>)], config: serde_json::Value, marker: &str) {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("game");
    for (name, bytes) in files {
        let path = assets.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let mut config = config;
    config["canvas"] = json!([64, 64]);
    let project = serde_json::from_value(json!({"schema":2,"name":"wrappers",
        "content_revision":"1","assets":"game","canvas":[64,64],
        "clients":[{"name":"game","game":"authored","adapter":"krkr","config":config}]}))
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut rt = Runtime::with_asset_root(project, &assets, &temp.path().join("state"), registry).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        rt.tick(1, InputFrame::default()).unwrap();
        let info = rt.inspect();
        if info["sessions"][0]["native"]["log_tail"].to_string().contains(marker) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "{info}");
        std::thread::yield_now();
    }
}

#[test]
fn lzfs_serves_frames_decoded_and_other_files_raw() {
    let mut encoder = lz4_flex::frame::FrameEncoder::new(Vec::new());
    encoder.write_all("packed text 圧縮".as_bytes()).unwrap();
    let packed = encoder.finish().unwrap();
    run(
        &[
            ("data/packed.txt", packed),
            ("data/plain.txt", "plain text".as_bytes().to_vec()),
            (
                "startup.tjs",
                r#"
                var missing = false;
                try { Storages.getFullPath("lzfs://./x"); } catch(e) { missing = true; }
                if (!missing) throw "lzfs before link";
                Plugins.link("lzfs.dll");
                Storages.addAutoPath("data/");
                if (!Storages.isExistentStorage("lzfs://./packed.txt")) throw "lzfs miss";
                if ([].load("lzfs://./packed.txt")[0] !== "packed text 圧縮") throw "not decoded";
                if ([].load("lzfs://./plain.txt")[0] !== "plain text") throw "not raw";
                if (Storages.isExistentStorage("lzfs://./absent.txt")) throw "invented file";
                Debug.message("lzfs passed");
            "#
                .as_bytes()
                .to_vec(),
            ),
        ],
        json!({}),
        "lzfs passed",
    );
}

#[test]
fn proxy_media_maps_exact_lowercased_keys_through_auto_paths() {
    run(
        &[
            ("plugin/target.txt", b"target".to_vec()),
            (
                "startup.tjs",
                br#"
                Plugins.link("yuzuex.dll");
                if (typeof global.ProxyStorageMap != "Object") throw "map absent";
                ProxyStorageMap["./alias.txt"] = System.exePath + "plugin/target.txt";
                ProxyStorageMap["./thumb/a"] = <% 00 01 %>;
                Storages.addAutoPath("proxy://./");
                if ([].load("proxy://./ALIAS.txt")[0] !== "target") throw "proxy read";
                if ([].load("alias.txt")[0] !== "target") throw "auto path";
                if (Storages.isExistentStorage("proxy://./thumb/a")) throw "octet mapped";
                if (Storages.isExistentStorage("proxy://./absent")) throw "absent mapped";
                Debug.message("proxy passed");
            "#
                .to_vec(),
            ),
        ],
        json!({}),
        "proxy passed",
    );
}

#[test]
fn lang_name_follows_configured_locale_and_option_plugins_link() {
    run(
        &[(
            "startup.tjs",
            br#"
            Plugins.link("getLangName.dll");
            if (System.getCurrentUILangName() !== "ja") throw "ui " + System.getCurrentUILangName();
            if (System.getCurrentLocaleName() !== "ja-JP") throw "locale";
            Plugins.link("kagexopt.dll");
            Plugins.link("wuopus.dll");
            if (Plugins.getStatus("kagexopt.dll").kind != "engine_facade") throw "kagexopt";
            Debug.message("lang passed");
        "#
            .to_vec(),
        )],
        json!({"locale": "ja_JP.UTF-8"}),
        "lang passed",
    );
}

fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut out = Vec::new();
    out.extend(b"RIFF");
    out.extend((36 + data.len() as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

#[test]
fn get_sample_reads_the_playing_window() {
    // A constant positive level, then silence: the reference average of the
    // non-negative samples is the level itself.
    let samples: Vec<i16> = (0..44100 * 4).map(|i| if i % 2 == 0 { 1000 } else { -1000 }).collect();
    run(
        &[
            ("voice.wav", wav(&samples, 44100)),
            (
                "startup.tjs",
                br#"
                Plugins.link("getSample.dll");
                var w = new WaveSoundBuffer(null);
                if (w.getSample() !== 0) throw "stopped buffer has samples";
                if (w.sampleCount !== 100 || w.sampleAhead !== 0) throw "defaults";
                WaveSoundBuffer.setDefaultCounts(50);
                var w2 = new WaveSoundBuffer(null);
                if (w2.sampleCount !== 50) throw "static default";
                w.open("voice.wav");
                w.play();
                var t = new Timer(function() {
                    var level = w.getSample(64);
                    var peak = w.sampleValue;
                    if (level == 0) return;
                    if (level !== 1000) throw "level " + level;
                    if (Math.abs(peak - (1000.0/32768)*(1000.0/32768)) > 1e-9) throw "peak " + peak;
                    t.enabled = false;
                    Debug.message("getSample passed");
                }, "");
                t.interval = 1;
                t.enabled = true;
            "#
                .to_vec(),
            ),
        ],
        json!({}),
        "getSample passed",
    );
}

#[test]
fn launcher_draw_device_and_compat_plugins_report_their_profiles() {
    run(
        &[(
            "startup.tjs",
            br#"
            var w = new Window();
            Plugins.link("k2compat.dll");
            var hits = 0;
            w.onMouseDown = function(x, y, b, s) { if (x == 3 && y == 4) hits++; };
            var tm = new Window.TouchMouse(w);
            if (tm.enabled !== 0) throw "touch default";
            tm.enabled = true;
            tm.onTouchMouseDown(3, 4, 0, 0);
            if (hits != 1) throw "touch forward";
            var owner = new System.ModelessOwnerWindow();
            owner.show(true);
            owner.stayOnTop = true;
            if (owner.stayOnTop !== 1) throw "owner";
            Plugins.link("dmmcloud.dll");
            if ((new DMMCloud()).open_store("x") !== 0) throw "dmm";
            Plugins.link("krkrsteam.dll");
            if (Steam.requestInitialize() !== 0 || Steam.initialized !== 0) throw "steam";
            if (Steam.getLanguage() !== "japanese") throw "steam language";
            if (Steam.getAchievement("A").achieved !== 0) throw "achievement record";
            var refused = false; try { new Steam(); } catch(e) { refused = true; }
            if (!refused) throw "steam instance";
            Plugins.link("DrawDeviceD2Dm.dll");
            w.drawDevice = new DrawDeviceD2D();
            var main = new Layer(w, null);
            var view = new Layer(w, null);
            view.viewState.setRect(4, 2, 28, 26, 0);
            view.viewState.visible = 0;
            view.viewState.opacity = 128;
            view.viewState.order = -1;
            if (w.primaryLayer !== main || view.viewState.visible || view.viewState.opacity != 128
                || view.viewState.order != -1) throw "secondary view replaced the primary or lost its state";
            invalidate view;
            if (w.primaryLayer !== main) throw "removing a secondary view detached the primary";
            Plugins.link("DrawDeviceD2D.dll");
            Plugins.link("SteamDrawDevice.dll");
            w.drawDevice = new DrawDeviceD2D();
            w.drawDevice.recreate();
            w.drawDevice = new DualDrawDevice_Z();
            Plugins.link("win32ole.dll");
            var voice = new WIN32OLE("SAPI.SpVoice");
            var voices;
            try { voices = voice.invoke("GetVoices"); } catch(e) { voices = "failed"; }
            if (voices !== "failed") throw "ole transport invented";
            voice.addEvent("Word", w);
            var x = new ActiveX("Shell.Explorer", w, 1, 2, 30, 40);
            if (x.width !== 30 || x.visible !== 0) throw "activex geometry";
            Debug.message("compat passed");
        "#
            .to_vec(),
        )],
        json!({"locale": "ja-JP"}),
        "compat passed",
    );
}

#[test]
fn http_request_reaches_only_authorized_origins() {
    use std::io::{Read as _, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(2) {
            let mut stream = stream.unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = "こんにちは";
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nX-Test: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("game");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::write(
        assets.join("startup.tjs"),
        format!(
            r#"
            var w = new Window();
            Plugins.link("httprequest.dll");
            global.states = [];
            global.ok = new HttpRequest(w);
            var ok = global.ok;
            ok.onReadyStateChange = function(s) {{
                var ok = global.ok;
                global.states.add(s);
                if (s != HttpRequest.LOADED) return;
                if (ok.status !== 200 || ok.statusText !== "OK") throw "status " + ok.status + " " + ok.statusText;
                if (ok.response !== "こんにちは") throw "body " + ok.response;
                if (ok.getResponseHeader("x-test") !== "yes") throw "header";
                if (ok.contentTypeEncoding !== "utf-8") throw "charset";
                global.denied = new HttpRequest(w);
                global.denied.onReadyStateChange = function(s) {{
                    if (s != HttpRequest.LOADED) return;
                    if (global.denied.status !== 0) throw "unauthorized origin reached";
                    Debug.message("http passed " + global.states.join(","));
                }};
                global.denied.open("GET", "http://127.0.0.2:{port}/");
                global.denied.send();
            }};
            ok.open("GET", "http://127.0.0.1:{port}/hello");
            ok.send();
        "#
        ),
    )
    .unwrap();
    let project = serde_json::from_value(json!({"schema":2,"name":"http",
        "content_revision":"1","assets":"game","canvas":[64,64],
        "clients":[{"name":"game","game":"authored","adapter":"krkr","config":{"canvas":[64,64]}}]}))
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut rt = Runtime::with_asset_root(project, &assets, &temp.path().join("state"), registry).unwrap();
    rt.host.grant_http_origin(crysirror_core::state::ClientId(1), &format!("http://127.0.0.1:{port}")).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        rt.tick(1, InputFrame::default()).unwrap();
        let log = rt.inspect()["sessions"][0]["native"]["log_tail"].to_string();
        if log.contains("http passed 1,2,3,4") {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "{}", rt.inspect());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A 4x2 RGB document: folder "Chara" (id 9) holding "Face" (id 7, 2x2 at
/// x=1, multiply, opacity 200) and its closing divider, one vertical guide.
fn psd_fixture() -> Vec<u8> {
    fn be16(v: &mut Vec<u8>, x: u16) {
        v.extend(x.to_be_bytes());
    }
    fn be32(v: &mut Vec<u8>, x: u32) {
        v.extend(x.to_be_bytes());
    }
    fn record(info: &mut Vec<u8>, rect: [u32; 4], channels: &[(i16, usize)], blend: &[u8; 4], opacity: u8, name: &str, extra: &[u8]) {
        for v in rect {
            be32(info, v);
        }
        be16(info, channels.len() as u16);
        for (id, len) in channels {
            be16(info, *id as u16);
            be32(info, *len as u32);
        }
        info.extend(b"8BIM");
        info.extend(blend);
        info.extend([opacity, 0, 0, 0]);
        let mut e = Vec::new();
        be32(&mut e, 0);
        be32(&mut e, 0);
        let mut pascal = vec![name.len() as u8];
        pascal.extend(name.as_bytes());
        while pascal.len() % 4 != 0 {
            pascal.push(0);
        }
        e.extend(pascal);
        e.extend(extra);
        be32(info, e.len() as u32);
        info.extend(e);
    }
    let block = |key: &[u8; 4], body: &[u8]| {
        let mut v = b"8BIM".to_vec();
        v.extend(key);
        v.extend((body.len() as u32).to_be_bytes());
        v.extend(body);
        v
    };
    let mut f = b"8BPS".to_vec();
    be16(&mut f, 1);
    f.extend([0; 6]);
    be16(&mut f, 3);
    be32(&mut f, 2);
    be32(&mut f, 4);
    be16(&mut f, 8);
    be16(&mut f, 3);
    be32(&mut f, 0);
    let mut res = b"8BIM".to_vec();
    be16(&mut res, 1032);
    res.extend([0, 0]);
    let mut guide = Vec::new();
    for v in [1u32, 576, 576, 1, 64] {
        be32(&mut guide, v);
    }
    guide.extend([0, 0]);
    be32(&mut res, 21);
    res.extend(&guide);
    be32(&mut f, res.len() as u32);
    f.extend(res);
    let raw = |plane: [u8; 4]| {
        let mut v = vec![0, 0];
        v.extend(plane);
        v
    };
    let face: Vec<Vec<u8>> = vec![raw([255, 255, 255, 128]), raw([10, 20, 30, 40]), raw([50, 60, 70, 80]), raw([90, 100, 110, 120])];
    let empty = vec![0u8, 0];
    let mut info = Vec::new();
    be16(&mut info, 3);
    let mut divider = Vec::new();
    divider.extend(block(b"lsct", &3u32.to_be_bytes()));
    record(&mut info, [0; 4], &[(-1, 2), (0, 2), (1, 2), (2, 2)], b"norm", 255, "</Layer group>", &divider);
    let mut face_extra = block(b"lyid", &7u32.to_be_bytes());
    face_extra.extend(block(b"luni", &[0, 0, 0, 4, 0, b'F', 0, b'a', 0, b'c', 0, b'e']));
    let ids: Vec<(i16, usize)> = [-1i16, 0, 1, 2].iter().zip(&face).map(|(id, d)| (*id, d.len())).collect();
    record(&mut info, [0, 1, 2, 3], &ids, b"mul ", 200, "face", &face_extra);
    let mut folder = block(b"lsct", &1u32.to_be_bytes());
    folder.extend(block(b"lyid", &9u32.to_be_bytes()));
    record(&mut info, [0; 4], &[(-1, 2), (0, 2), (1, 2), (2, 2)], b"norm", 255, "Chara", &folder);
    for _ in 0..4 {
        info.extend(&empty);
    }
    for d in &face {
        info.extend(d);
    }
    for _ in 0..4 {
        info.extend(&empty);
    }
    let mut section = Vec::new();
    be32(&mut section, info.len() as u32);
    section.extend(info);
    be32(&mut section, 0);
    be32(&mut f, section.len() as u32);
    f.extend(section);
    be16(&mut f, 0);
    for c in 0..3u8 {
        f.extend([c * 10 + 1; 8]);
    }
    f
}

#[test]
fn psd_class_reads_layers_into_krkr_layers_and_serves_psd_media() {
    run(
        &[
            ("art.psd", psd_fixture()),
            (
                "startup.tjs",
                br#"
                Plugins.link("psd.dll");
                var p = new PSD();
                if (p.width !== -1) throw "facts before load";
                var nodata = false; try { p.getLayerType(0); } catch(e) { nodata = (e.message == "no data"); }
                if (!nodata) throw "no data";
                if (!p.load("art.psd")) throw "load";
                if (p.width !== 4 || p.height !== 2 || p.layer_count !== 3 || p.color_mode !== PSD.color_mode_rgb) throw "facts";
                if (p.getLayerType(2) !== PSD.layer_type_folder || p.getLayerType(0) !== PSD.layer_type_hidden) throw "types";
                var info = p.getLayerInfo(1);
                if (info.name !== "Face" || info.layer_id !== 7 || info.group_layer_id !== 9) throw "info " + info.name;
                if (info.blend_mode !== PSD.blend_mode_multiply || info.type !== ltPsMultiplicative || info.opacity !== 200) throw "blend";
                var range = false; try { p.getLayerName(3); } catch(e) { range = (e.message == "not such layer"); }
                if (!range) throw "range";
                var bad = false; try { p.getLayerData(p, 1); } catch(e) { bad = (e.message == "not layer"); }
                if (!bad) throw "not layer";
                var w = new Window();
                var base = new Layer(w, null);
                var l = new Layer(w, base);
                p.getLayerData(l, 1);
                if (l.left !== 1 || l.top !== 0 || l.width !== 2 || l.imageWidth !== 2 || l.opacity !== 200 || l.name !== "Face") throw "assign";
                if (l.type !== ltPsMultiplicative) throw "type";
                var folder = false; try { p.getLayerData(l, 2); } catch(e) { folder = (e.message == "invalid layer type"); }
                if (!folder) throw "folder data";
                var g = p.getGuides();
                if (g.vertical[0] !== 64 || g.horizontal.count !== 0) throw "guides";
                if (p.getSlices() !== void || p.getLayerComp() !== void) throw "absent resources";
                var merged = new Layer(w, base);
                if (!p.getBlend(merged) || merged.imageWidth !== 4) throw "blend image";
                if (!Storages.isExistentStorage("psd://art.psd/root/chara/face.bmp")) throw "media path";
                if (!Storages.isExistentStorage("psd://ART.PSD/id/7.bmp")) throw "media id";
                if (Storages.isExistentStorage("psd://art.psd/root/face.bmp")) throw "media folder";
                var t = new Layer(w, base);
                t.loadImages("psd://art.psd/root/chara/face.bmp");
                if (t.imageWidth !== 2 || t.imageHeight !== 2) throw "media image";
                var timer = new Timer(function() {
                    if ((l.getMainPixel(0, 0) & 0xffffff) != 0x0a325a || l.getMaskPixel(0, 0) != 255) throw "pixel " + l.getMainPixel(0, 0);
                    if (l.getMaskPixel(1, 1) != 128) throw "alpha " + l.getMaskPixel(1, 1);
                    timer.enabled = false;
                    Debug.message("psd passed");
                }, "");
                timer.interval = 1;
                timer.enabled = true;
            "#
                .to_vec(),
            ),
        ],
        json!({}),
        "psd passed",
    );
}

#[test]
fn wave_filter_plugins_prepare_filtered_resources() {
    let samples: Vec<i16> = (0..44100).map(|i| ((i % 100) as i16 - 50) * 200).collect();
    run(
        &[
            ("voice.wav", wav(&samples, 44100)),
            (
                "startup.tjs",
                br#"
                Plugins.link("wfBasicEffect.dll");
                Plugins.link("wfTypicalDSP.dll");
                var d = new WaveSoundBuffer.DelayEffect(100, 0.5, 0.8, 500);
                var v = new WaveSoundBuffer.StkFreeVerb();
                if (v.effectMix != 0.75 || v.roomSize != 0.75 || v.damping != 0.25 || v.extend != 1000) throw "verb defaults";
                v.effectMix = 2;
                if (v.effectMix != 1) throw "effectMix clamp";
                var e = new WaveSoundBuffer.GraphicEqualizer();
                if (e.getGain(3) != 1 || e.getGain(10) != 0) throw "eq gains";
                e.setGain(3, 0.5);
                e.setGain(12, 0.5);
                if (e.getGain(3) != 0.5) throw "setGain";
                var f = new WaveSoundBuffer.WaveDSPFilter("BandPass", "Butterworth");
                if (f.setParams(void, 2, 1900, 2900) != 4) throw "param count";
                var i = f.getParamInfo(2);
                if (i.label != "Fc" || i.name != "Center Frequency" || i.currentValue != 1900 || i.defaultValue != 2000) throw "param info";
                if (f.getParamInfo(1).max != 50 || f.getParamInfo(0).currentValue != 44100) throw "order/rate info";
                if (f.getParamInfo(4) !== void) throw "info range";
                if (typeof f.interface != "Integer") throw "interface";
                var bad = false;
                try { new WaveSoundBuffer.WaveDSPFilter("LowShelf", "Elliptic"); } catch (x) { bad = true; }
                if (!bad) throw "unsupported design accepted";
                var w = new WaveSoundBuffer(null);
                w.filters.add(f);
                w.filters.add(d);
                w.open("voice.wav");
                if (w.totalSamples != 44100 + 22050) throw "delay tail " + w.totalSamples;
                w.play();
                var r = new WaveSoundBuffer(null);
                r.filters.add(v);
                r.open("voice.wav");
                if (r.totalSamples != 44100 + 44100) throw "verb tail " + r.totalSamples;
                r.filters.clear();
                bad = false;
                try { r.play(); } catch (x) { bad = true; }
                if (!bad) throw "filter change after open accepted";
                var pv = new WaveSoundBuffer.PhaseVocoder();
                if (pv.window != 4096 || pv.overlap != 0) throw "vocoder defaults";
                pv.window = 1024;
                if (pv.window != 1024) throw "vocoder window";
                Debug.message("wave filters passed");
            "#
                .to_vec(),
            ),
        ],
        json!({}),
        "wave filters passed",
    );
}

#[test]
fn layer_ex_btoa_and_shrink_copy_members() {
    run(
        &[(
            "startup.tjs",
            br#"
            var w = new Window();
            var base = new Layer(w, null);
            var a = new Layer(w, base);
            a.setImageSize(4, 4);
            a.fillRect(0, 0, 4, 4, 0xff000000);
            a.fillRect(0, 2, 4, 2, 0xff000080);
            a.copyBottomBlueToTopAlpha();
            if (a.getMaskPixel(1, 1) != 0x80) throw "btoa " + a.getMaskPixel(1, 1);
            var s = new Layer(w, base);
            s.setImageSize(4, 4);
            s.fillRect(0, 0, 2, 4, 0xff000000);
            s.fillRect(2, 0, 2, 4, 0xff0000c8);
            var d = new Layer(w, base);
            d.setImageSize(2, 2);
            d.fillRect(0, 0, 2, 2, 0);
            d.shrinkCopy(0, 0, 2, 2, s, 0, 0, 4, 4);
            if ((d.getMainPixel(1, 0) & 0xff) != 0xc8 || (d.getMainPixel(0, 0) & 0xff) != 0) throw "shrink " + d.getMainPixel(1, 0);
            var f = new Layer(w, base);
            f.setImageSize(1, 1);
            f.shrinkCopyFast(s, 4, 2);
            if (f.imageWidth != 1 || f.imageHeight != 2 || (f.getMainPixel(0, 0) & 0xff) != 0x64) throw "fast " + f.imageWidth + " " + f.getMainPixel(0, 0);
            var o = new Layer(w, base);
            o.setImageSize(2, 1);
            o.fillRect(0, 0, 2, 1, 0xff102030);
            var m = new Layer(w, base);
            m.setImageSize(2, 1);
            m.fillRect(0, 0, 1, 1, 0x00ffffff);
            m.fillRect(1, 0, 1, 1, 0xff405060);
            o.overwrapRect(0, 0, m, 0, 0, 2, 1);
            if ((o.getMainPixel(0, 0) & 0xffffff) != 0x102030 || (o.getMainPixel(1, 0) & 0xffffff) != 0x405060) throw "overwrap";
            Debug.message("layerex passed");
        "#
            .to_vec(),
        )],
        json!({}),
        "layerex passed",
    );
}

#[test]
fn batch_plugin_members_smoke() {
    let samples: Vec<i16> = (0..22050).map(|i| ((i % 64) as i16 - 32) * 400).collect();
    run(
        &[
            ("voice.wav", wav(&samples, 44100)),
            (
                "startup.tjs",
                br#"
                Plugins.link("layerStwCopy.dll");
                Plugins.link("GlitchEffect.dll");
                Plugins.link("gamepad.dll");
                Plugins.link("wumultitrack.dll");
                Plugins.link("wfBasicEffect.dll");
                var w = new Window();
                var base = new Layer(w, null);
                var s = new Layer(w, base);
                s.setImageSize(4, 2);
                s.fillRect(0, 0, 4, 2, 0xff000000);
                s.fillRect(0, 0, 1, 2, 0xff0000ff);
                var d = new Layer(w, base);
                d.setImageSize(4, 2);
                d.fillRect(0, 0, 4, 2, 0);
                // Row 0 starts at source column 1, row 1 at column 0.
                d.stitchWrappedCopy(0, 0, 4, 2, s, 0, 0, 4, 2, [1, 0]);
                if ((d.getMainPixel(3, 0) & 0xff) != 0xff || (d.getMainPixel(0, 0) & 0xff) != 0) throw "stitch row0";
                if ((d.getMainPixel(0, 1) & 0xff) != 0xff) throw "stitch row1";
                var g = new Layer(w, base);
                g.setImageSize(4, 2);
                g.fillRect(0, 0, 4, 2, 0);
                g.doGlitch(s, %[seed: 7, noise: 0, per_x: 0, per_y: 0, per_col: 0]);
                if ((g.getMainPixel(0, 0) & 0xff) != 0xff || g.getMaskPixel(2, 1) != 255) throw "glitch identity";
                g.glitchCopy(0, 0, s, 0, 0, 4, 2, %[seed: 3]);
                var p = new Layer(w, base);
                p.setImageSize(2, 1);
                p.fillRect(0, 0, 1, 1, 0xff112233);
                p.fillRect(1, 0, 1, 1, 0x10112233);
                p.copyAlphaToProvince(128, 5, 0);
                if (p.getProvincePixel(0, 0) != 5 || p.getProvincePixel(1, 0) != 0) throw "alpha to province";
                var hit = p.fillToProvince(0x112233, 9, 1);
                if (hit[2] != 2 || p.getProvincePixel(1, 0) != 9) throw "fill to province";
                p.fillByProvince(9, 0xff010203);
                if ((p.getMainPixel(0, 0) & 0xffffff) != 0x010203) throw "fill by province";
                var port = new GamepadPort();
                port.initialize(0);
                if (port.count != 0 || port.getController(0) !== null) throw "gamepad";
                if (typeof WaveSoundBuffer.multiTrackOpenCallbacks != "Object") throw "multitrack callbacks";
                WaveSoundBuffer.setMultiTrackMask("bgm", void, 1);
                if (System.waitForAppLock("x", 0) != 1) throw "app lock";
                var W = WaveSoundBuffer;
                var ring = new W.RingModulation(new W.OscFilter(new W.OscNoise(0.8), new W.MathAbs(), new W.MathConstAdd(0.2)));
                var voc = new W.Vocoder(new W.OscMix(new W.OscNoise(4), new W.OscBlit(24, 48), new W.OscBlitSaw(32, 96), new W.OscBlitSquare(8, 120), new W.OscSineWave(1, 440)), 1.5);
                if (voc.bands != 24 || voc.filtersPerBand != 6) throw "vocoder defaults";
                var b1 = new WaveSoundBuffer(null);
                b1.filters.add(new W.GainLimit(6, 0.5, 4));
                b1.filters.add(ring);
                b1.open("voice.wav");
                if (b1.totalSamples != 22050) throw "ring length " + b1.totalSamples;
                var b2 = new WaveSoundBuffer(null);
                b2.filters.add(voc);
                b2.filters.add(new W.GainLimit(0, 0.9, -1));
                b2.open("voice.wav");
                if (b2.totalSamples != 22050) throw "vocoder length";
                Plugins.link("wvdecoder.dll");
                Plugins.link("win32dialog.dll");
                var data = new WaveSoundData.F32("voice.wav");
                if (data.channels != 1 || data.frequency != 44100 || data.samples != 22050) throw "wave data " + data.samples;
                if (Math.abs(data.totalTime - 500) > 0.001 || data.format != 0x01010020) throw "wave format";
                if (Math.abs(data.getSample(1) - (-31 * 400 / 32768)) > 1e-6 || data.getSample(22050) !== void) throw "getSample";
                if (Math.abs(data.setSample(1, 0.25) - (-31 * 400 / 32768)) > 1e-6 || data.getSample(1) != 0.25) throw "setSample";
                var loop = new W.OscLoop(data, 1);
                invalidate data;
                var b3 = new WaveSoundBuffer(null);
                var pv = new W.PhaseVocoder();
                pv.pitch = 1.3; pv.window = 256;
                b3.filters.add(new W.RingModulation(loop));
                b3.filters.add(pv);
                b3.open("voice.wav");
                if (b3.totalSamples <= 20000 || b3.totalSamples > 22050) throw "phase vocoder length " + b3.totalSamples;
                Debug.message("batch passed");
            "#
                .to_vec(),
            ),
        ],
        json!({}),
        "batch passed",
    );
}

/// Minimal RGBA PNG (stored deflate blocks).
fn png(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for b in data {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
        out.extend((body.len() as u32).to_be_bytes());
        let mut both = kind.to_vec();
        both.extend(body);
        out.extend(&both);
        out.extend(crc(&both).to_be_bytes());
    }
    let mut raw = vec![];
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend(pixel(x, y));
        }
    }
    let (mut a, mut b) = (1u32, 0u32);
    for v in &raw {
        a = (a + u32::from(*v)) % 65521;
        b = (b + a) % 65521;
    }
    let mut z = vec![0x78, 0x01, 1];
    z.extend((raw.len() as u16).to_le_bytes());
    z.extend((!(raw.len() as u16)).to_le_bytes());
    z.extend(&raw);
    z.extend(((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = vec![];
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn slice_layer_color_tables_and_wave_data_extras() {
    let samples: Vec<i16> = (0..4410).map(|i| if i < 2205 { 16384 } else { -8192 }).collect();
    run(
        &[
            ("voice.wav", wav(&samples, 44100)),
            ("pic.tlg", png(2, 5, |_, y| [y as u8 * 10, 0, 0, 255])),
            (
                "startup.tjs",
                br#"
                Plugins.link("PackinOne.dll");
                var entries = Storages.dirlist("file://./");
                if (entries.find("pic.tlg") < 0) throw "PackinOne directory listing";
                Plugins.link("msdfrender.dll");
                Plugins.link("wvdecoder.dll");
                Plugins.link("win32dialog.dll");
                var size = Layer.fetchImageSize("pic.tlg");
                if (size[0] != 2 || size[1] != 5) throw "fetchImageSize " + size[0];
                var w = new Window();
                var base = new Layer(w, null);
                SliceLayer.Config.height = 2;
                SliceLayer.Config.minimal = 1;
                var sl = new SliceLayer(w, base);
                sl.loadImages("pic");
                if (sl.width != 2 || sl.height != 5) throw "slice size";
                var rows = [];
                sl.foreachSlice(function(lay) { this.add(lay.imageHeight); } incontextof rows);
                if (rows.count != 3 || rows[0] != 2 || rows[2] != 1) throw "slices " + rows.count + " " + rows[2];
                var third;
                sl.foreachSlice(function(lay) { global.__last = lay; });
                if (((__last.getMainPixel(0, 0) >> 16) & 0xff) != 40) throw "slice rows " + __last.getMainPixel(0, 0);
                var standalone = new Layer(w, base);
                standalone.setImageSize(2, 5);
                var copySlices = SliceLayer.SliceLoader.loadSlicedImage incontextof global;
                copySlices("pic.tlg", 5, [standalone]);
                if (((standalone.getMainPixel(0, 4) >> 16) & 0xff) != 40) throw "standalone slice rows";
                var assembled = new Layer(w, base);
                assembled.setImageSize(2, 5);
                assembled.holdAlpha = true;
                assembled.operateRect(0, 0, standalone, 0, 0, 2, 5, omAuto);
                if (assembled.getMaskPixel(0, 4) != 255) throw "alpha part over an empty pixel";
                sl.setSliceProp("opacity", 128);
                if (__last.opacity != 128) throw "setSliceProp";
                var table = [];
                table[0] = 0xff000000; table[7] = 0x80112233;
                var ctab = Scripts.makeColorTable(table);
                if (ctab.length != 1028) throw "color table";
                var g = new Layer(w, base);
                g.setImageSize(2, 1);
                g.fillRect(0, 0, 2, 1, 0);
                g.copyGlyphByColorTable(ctab, 0, 0, 2, 1, <% 00 07 %>);
                if ((g.getMainPixel(1, 0) & 0xffffff) != 0x112233 || g.getMaskPixel(1, 0) != 0x80 || g.getMaskPixel(0, 0) != 255) throw "color table copy";
                var d = new WaveSoundData.S16("voice.wav");
                var peaks = [];
                var n = d.getLipSync(function(p, i) { this[i] = p; } incontextof peaks, 2205);
                if (n != 2 || peaks[0] != 16384 * 16384 || peaks[1] != 8192 * 8192) throw "lip sync " + n + " " + peaks[0];
                d.foreach(function(i, v, ch) { return i == 0 ? 100 : void; }, 0, 2);
                if (d.getSample(0) != 100 || d.getSample(1) != 16384) throw "foreach";
                var thumb = new Layer(w, base);
                thumb.setImageSize(4, 8);
                thumb.fillRect(0, 0, 4, 8, 0);
                d.makeThumbnail(thumb, 0xffff0000);
                if (thumb.getMaskPixel(0, 5) != 255 || thumb.getMaskPixel(0, 0) != 0 || thumb.getMaskPixel(3, 3) != 255) throw "thumbnail";
                var r = System.commandExecute("x", "y", 10);
                if (r.status != "failed") throw "commandExecute";
                var dlg = new WIN32Dialog();
                if (dlg.open(w) != 0) throw "dialog open";
                Debug.message("extras passed");
            "#
                .to_vec(),
            ),
        ],
        json!({}),
        "extras passed",
    );
}

fn wav_channels(frames: usize, channels: u16, rate: u32, f: impl Fn(usize, u16) -> i16) -> Vec<u8> {
    let data: Vec<u8> = (0..frames)
        .flat_map(|i| (0..channels).map(move |c| (i, c)))
        .flat_map(|(i, c)| f(i, c).to_le_bytes())
        .collect();
    let mut out = Vec::new();
    out.extend(b"RIFF");
    out.extend((36 + data.len() as u32).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(channels.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend((rate * 2 * u32::from(channels)).to_le_bytes());
    out.extend((2 * channels).to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}

#[test]
fn multitrack_descriptions_open_register_and_mask() {
    let quad = wav_channels(8000, 4, 8000, |i, c| ((i % 50) as i16) * 100 * (c as i16 + 1));
    let stereo = wav_channels(8000, 2, 8000, |i, _| (i % 40) as i16 * 50);
    run(
        &[
            ("bgm/quad.wav", quad),
            ("bgm/a.wav", stereo.clone()),
            ("bgm/b.wav", stereo),
            ("bgm/song.mchx", b"# gather\nchannel\t2\nsource\tquad.wav\ntrack\tdrums\t0\t1\ntrack\tbass\t2\t3\nvolume\tbass\t0.5\n".to_vec()),
            (
                "startup.tjs",
                br#"
                Plugins.link("wumultitrack.dll");
                global.opened = [];
                WaveSoundBuffer.multiTrackOpenCallbacks.add(function(id, storage, count, names, aliases) {
                    global.opened.add(id + ":" + count + ":" + names["drums"] + names["bass"] + names["a"] + names["b"]);
                });
                var w = new Window();
                var g = new WaveSoundBuffer(w);
                g.open("bgm/song.mchx");
                var m = new WaveSoundBuffer(w);
                m.open("bgm/a|b|mix.mtrk");
                if (opened.count != 2 || opened[0] != "quad.wav:2:01" || opened[1] != "mix.mtrk:2:01")
                    throw "open callbacks " + opened[0] + " / " + opened[1];
                if (WaveSoundBuffer.setMultiTrackMask("nothing", void, 0) != -1) throw "missing instance";
                WaveSoundBuffer.setMultiTrackMask("quad.wav", "bass", 1.0, 500);
                WaveSoundBuffer.setMultiTrackMask("mix.mtrk", ["a"], 0.0);
                WaveSoundBuffer.setDelayMultiTrackMask("mix.mtrk", [4000], ["b", 0.25, void, 1.0], 100);
                if (WaveSoundBuffer.rebindMultiTrackID("mix.mtrk", "renamed") != 1) throw "rebind";
                if (WaveSoundBuffer.setMultiTrackMask("mix.mtrk", void, 1) != -1) throw "old id still bound";
                WaveSoundBuffer.setMultiTrackMask("renamed", void, 1);
                g.play();
                m.play();
                var bad = false;
                try { var x = new WaveSoundBuffer(w); x.open("bgm/missing.mchx"); } catch(e) { bad = true; }
                if (!bad) throw "missing description opened";
                Debug.message("multitrack passed");
            "#
                .to_vec(),
            ),
        ],
        json!({}),
        "multitrack passed",
    );
}
