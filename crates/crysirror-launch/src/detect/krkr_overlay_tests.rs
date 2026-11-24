// SPDX-License-Identifier: MPL-2.0
//! Installed-game launch with self-authored XP3 files and script data.
use super::*;
use crysirror_core::{service::InputFrame, state::ClientId};
use std::{fs, io::Cursor};

fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = tag.to_vec();
    chunk.extend((data.len() as u64).to_le_bytes());
    chunk.extend(data);
    chunk
}

fn xp3(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut result = b"XP3\r\n \n\x1a\x8bg\x01".to_vec();
    result.extend(0u64.to_le_bytes());
    let mut index = Vec::new();
    for (path, payload) in entries {
        let size = payload.len() as u64;
        let start = result.len() as u64;
        result.extend(*payload);
        let path: Vec<_> = path.encode_utf16().collect();
        let mut info = 0u32.to_le_bytes().to_vec();
        info.extend(size.to_le_bytes());
        info.extend(size.to_le_bytes());
        info.extend((path.len() as u16).to_le_bytes());
        info.extend(path.into_iter().flat_map(u16::to_le_bytes));
        let mut segment = 0u32.to_le_bytes().to_vec();
        segment.extend(start.to_le_bytes());
        segment.extend(size.to_le_bytes());
        segment.extend(size.to_le_bytes());
        let mut entry = chunk(b"info", &info);
        entry.extend(chunk(b"segm", &segment));
        entry.extend(chunk(b"adlr", &0u32.to_le_bytes()));
        index.extend(chunk(b"File", &entry));
    }
    let start = result.len() as u64;
    result[11..19].copy_from_slice(&start.to_le_bytes());
    result.push(0);
    result.extend((index.len() as u64).to_le_bytes());
    result.extend(index);
    result
}

fn run(plan: &Plan, game: &Path) -> (String, [u32; 2]) {
    let saves = tempfile::tempdir().unwrap();
    let runtime = crysirror_runtime::Runtime::with_asset_root(
        serde_json::from_value(plan.project.clone()).unwrap(),
        game,
        saves.path(),
        crate::core::adapter_registry().unwrap(),
    )
    .unwrap();
    let mut player = crate::core::bootstrap(runtime, plan).unwrap();
    let runtime = player.rt();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        runtime.tick(16, InputFrame::default()).unwrap();
        let state = runtime.inspect();
        let native = &state["sessions"][0]["native"];
        assert!(native["error"].is_null(), "{native}");
        let log = native["log_tail"].to_string();
        if log.contains("fixture-complete") {
            let layer = runtime.host.engine.canvases.surface(ClientId(1), 2);
            let dimensions = layer.map_or([0, 0], |s| [s.width, s.height]);
            return (log, dimensions);
        }
        assert!(std::time::Instant::now() < deadline, "{log}");
        std::thread::yield_now();
    }
}

#[test]
fn selected_launcher_loads_plain_xp3_members_before_the_base() {
    let game = tempfile::tempdir().unwrap();
    let path = game.path();
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(13, 7)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    fs::write(path.join("launcher.exe"), b"MZKrkrPatch.json").unwrap();
    fs::write(path.join("original.exe"), b"MZoriginal").unwrap();
    fs::write(
        path.join("data.xp3"),
        xp3(&[
            (
                "startup.tjs",
                br#"
            var scWidth = 640, scHeight = 480;
            Scripts.execStorage("message.tjs");
            Scripts.execStorage("file://./data.xp3>message.tjs");
            Scripts.execStorage("base-only.tjs");
            Debug.message("binary-member", Storages.loadOctet("image.dref")[0]);
            if (Storages.isExistentStorage("ARC://./inside/scheme.tjs")) {
                Scripts.execStorage("ARC://./inside/scheme.tjs");
                Scripts.execStorage("archive://./inside/second.tjs");
                Debug.message("psb-exists", Storages.isExistentStorage("psb://Panel/tile.png"));
                var window = new Window(), root = new Layer(window, null);
                var picture = new Layer(window, root);
                picture.loadImages("psb://Panel/tile.png");
            }
            Debug.message("fixture-complete");
        "#,
            ),
            ("message.tjs", b"Debug.message('base-message');"),
            ("base-only.tjs", b"Debug.message('base-fallback');"),
            ("image.dref", &[10]),
        ]),
    )
    .unwrap();
    fs::write(
        path.join("Edition-A.xp3"),
        xp3(&[
            ("Message.tjs", b"Debug.message('overlay-message');"),
            ("inside/scheme.tjs", b"Debug.message('overlay-arc');"),
            ("Panel/tile.png", png.get_ref()),
            ("image.dref", &[20]),
        ]),
    )
    .unwrap();
    fs::write(
        path.join("extra.xp3"),
        xp3(&[
            ("message.tjs", b"Debug.message('wrong-priority');"),
            ("inside/second.tjs", b"Debug.message('overlay-archive');"),
        ]),
    )
    .unwrap();
    let manifest = path.join("KrkrPatch.json");
    fs::write(
        &manifest,
        r#"{
        "gameExecutableFile":"original.exe",
        "patchArchives":["edition-a.xp3","extra.xp3"],
        "patchProtocols":["arc://","archive://","psb://"],
        "patchNoProtocol":true
    }"#,
    )
    .unwrap();

    let dir = SourcePath::local(path);
    let base = detect_source_with_entry(&dir, Some("original.exe")).unwrap();
    let (base_log, _) = run(&base, path);
    assert_eq!(base_log.matches("base-message").count(), 2, "{base_log}");
    assert!(base_log.contains("base-fallback"), "{base_log}");
    assert!(base_log.contains("binary-member 10"), "{base_log}");
    assert!(!base_log.contains("overlay-message"), "{base_log}");

    let patched = detect_source_with_entry(&dir, Some("launcher.exe")).unwrap();
    let (patched_log, size) = run(&patched, path);
    for marker in [
        "overlay-message",
        "base-message",
        "base-fallback",
        "overlay-arc",
        "overlay-archive",
        "psb-exists 1",
        "binary-member 20",
    ] {
        assert!(patched_log.contains(marker), "{patched_log}");
    }
    assert_eq!(
        patched_log.matches("base-message").count(),
        1,
        "{patched_log}"
    );
    assert!(!patched_log.contains("wrong-priority"), "{patched_log}");
    assert_eq!(size, [13, 7]);

    fs::write(
        &manifest,
        r#"{"gameExecutableFile":"original.exe","patchArchives":["missing.xp3"]}"#,
    )
    .unwrap();
    let error = detect_source_with_entry(&dir, Some("launcher.exe")).unwrap_err();
    assert!(error.to_string().contains("missing.xp3"), "{error:#}");
}
