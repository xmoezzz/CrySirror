// SPDX-License-Identifier: GPL-2.0-or-later
//! Opt-in local installed game. Game/script/font bytes never enter the repository.
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{buttons, InputFrame},
    state::ClientId,
};
use crysirror_resource::{ArchiveFormat, ArchiveMount, DecodeContext};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
#[test]
#[ignore = "requires owner-provided CRYSIRROR_NSCRIPT_GAME (Tsukihime) and its licensed font"]
fn installed_game_title_via_shared_services() {
    let root = PathBuf::from(std::env::var_os("CRYSIRROR_NSCRIPT_GAME").expect("game folder"));
    let temp = tempfile::tempdir().unwrap();
    let names = ["arc1.nsa", "arc.nsa", "arc.sar"];
    let prefixes = names.map(|n| format!("__nscript/{n}"));
    let project=serde_json::from_value(json!({"schema":2,"name":"Tsukihime owner verification","content_revision":"1","canvas":[640,480],"assets":"game","fonts":{"native":"default.ttf"},"clients":[{"name":"game","game":"owner-game","adapter":"nscripter","config":{"script":"nscript.dat","font":"native","archives":prefixes}}]})).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_nscripter::register(&mut registry).unwrap();
    let mut runtime = Runtime::with_asset_root(project, &root, temp.path(), registry).unwrap();
    let storage = runtime.host.fs.storage(ClientId(1)).unwrap();
    for (name, prefix) in names.into_iter().zip(prefixes) {
        let format = if name.ends_with("nsa") {
            crysirror_formats_arc::Nscripter::nsa()
        } else {
            crysirror_formats_arc::Nscripter::sar()
        };
        let reader = storage
            .open_asset(&VfsPath::new(Mount::Asset, name).unwrap())
            .unwrap();
        let context = DecodeContext::new(ClientId(1));
        let archive = format.open(reader, &context).unwrap();
        runtime
            .mount_provider(
                ClientId(1),
                &prefix,
                Arc::new(ArchiveMount::new(archive, context).unwrap()),
            )
            .unwrap();
    }
    let maximum = std::env::var("CRYSIRROR_NSCRIPT_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50000u32);
    let story = std::env::var_os("CRYSIRROR_NSCRIPT_STORY").is_some();
    let mut stage = 0u8;
    for tick in 0..maximum {
        let mut override_input = None;
        // Observe before injecting another edge, so the title's first button
        // is not activated by the story-advance track itself.
        if tick % 8 == 0 {
            let inspect = runtime.inspect();
            let d = &inspect["sessions"][0]["native"];
            if d["wait"].get("Buttons").is_some() && stage == 0 {
                assert!(
                    d["native"]["button_atlas"]
                        .as_str()
                        .unwrap_or("")
                        .ends_with("image/title/titlebtn1.png"),
                    "not native title atlas: {d}"
                );
                assert!(
                    d["native"]["image_sources"]["1610612737"]
                        .as_str()
                        .unwrap_or("")
                        .ends_with("image/title/title1.png"),
                    "not native title background: {d}"
                );
                assert!(
                    d["line"]
                        .as_u64()
                        .is_some_and(|n| (12760..12860).contains(&n)),
                    "not title source position: {d}"
                );
                println!(
                    "Reached native title buttons after {tick} ticks: {}",
                    json!({"pc":d["pc"],"line":d["line"],"buttons":d["native"]["buttons"]})
                );
                if !story {
                    return;
                }
                stage = 1;
            } else if d["wait"].get("Buttons").is_some() && stage == 1 {
                assert!(
                    d["native"]["button_atlas"]
                        .as_str()
                        .unwrap_or("")
                        .ends_with("image/gamestart/gamestartbtn.png"),
                    "not native start submenu: {d}"
                );
                override_input = Some(InputFrame {
                    buttons_pressed: buttons::DOWN,
                    ..Default::default()
                });
                stage = 2;
            } else if stage == 2 {
                override_input = Some(InputFrame {
                    buttons_pressed: buttons::ENTER,
                    ..Default::default()
                });
                stage = 3;
            }
            if stage == 3
                && d["text"]
                    .as_str()
                    .unwrap_or("")
                    .contains("When I came to, I was lying on a hospital bed.")
            {
                assert!(
                    d["native"]["image_sources"]["1610612737"]
                        .as_str()
                        .unwrap_or("")
                        .ends_with("image/bg/bg_51b.jpg"),
                    "not first story background: {d}"
                );
                println!(
                    "Reached native s20 hospital story after {tick} ticks: {}",
                    json!({"pc":d["pc"],"line":d["line"],"background":d["native"]["image_sources"]["1610612737"]})
                );
                return;
            }
        }
        let input = override_input.unwrap_or_else(|| {
            if tick % 8 == 0 {
                InputFrame {
                    buttons_pressed: buttons::ENTER,
                    ..Default::default()
                }
            } else {
                InputFrame::default()
            }
        });
        if let Err(error) = runtime.tick(16, input) {
            panic!(
                "tick {tick}: {error:#}\n{}",
                runtime.inspect()["sessions"][0]["native"]
            );
        }
        if tick % 256 == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    panic!(
        "native title not reached: {}",
        runtime.inspect()["sessions"][0]["native"]
    );
}
