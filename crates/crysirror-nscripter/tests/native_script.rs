// SPDX-License-Identifier: GPL-2.0-or-later
//! Self-authored native source/PNG/WAVE resources; no game bytes or fonts.
use crysirror_core::{
    service::{buttons, InputFrame},
    state::ClientId,
};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
fn fixture(script: &str, encrypted: bool) -> (tempfile::TempDir, Runtime) {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("assets/game");
    std::fs::create_dir_all(&assets).unwrap();
    let mut bytes = script.as_bytes().to_vec();
    if encrypted {
        for b in &mut bytes {
            *b ^= 0x84;
        }
    }
    let name = if encrypted { "nscript.dat" } else { "0.txt" };
    std::fs::write(assets.join(name), bytes).unwrap();
    let png = crysirror_image::encode_png(&crysirror_image::ImageSurface {
        width: 2,
        height: 2,
        rgba: [50, 100, 200, 128].repeat(4),
    })
    .unwrap();
    std::fs::write(assets.join("pixel.png"), png).unwrap();
    let project=serde_json::from_value(json!({"schema":2,"name":"native-nscript","content_revision":"1","canvas":[640,480],"assets":"assets","clients":[{"name":"native","adapter":"nscripter","game":"authored","asset_prefix":"game","config":{"script":name,"encoding":"utf8","font":"fixture-font"}}]})).unwrap();
    let path = temp.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_nscripter::register(&mut registry).unwrap();
    let runtime =
        Runtime::with_registry(project, &path, &temp.path().join("state"), registry).unwrap();
    (temp, runtime)
}
fn diag(r: &Runtime) -> Value {
    r.inspect()["sessions"][0]["native"].clone()
}
fn pump(r: &mut Runtime, ready: impl Fn(&Runtime) -> bool) {
    let until = Instant::now() + Duration::from_secs(3);
    while !ready(r) {
        r.tick(16, InputFrame::default()).unwrap();
        assert!(Instant::now() < until, "{}", diag(r));
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn press(r: &mut Runtime) {
    r.tick(
        16,
        InputFrame {
            buttons_pressed: buttons::ENTER,
            ..Default::default()
        },
    )
    .unwrap();
}
#[test]
fn native_branch_call_loop_text_and_image_use_engine_state_and_distinct_input_edges() {
    let source="*define\ncaption \"Native source\"\nnumalias limit,3\ndefsub twice\ngame\n*start\nmov %0,0\nfor %1=1 to limit\nadd %0,%1\nnext\ngosub *twice,4\nif %0=14 goto *ready : mov %0,-999\nmov %0,-998\n*ready\nbg black,1\nlsp 1,\":a;pixel.png\",0,0\ncsp 1\ncsp 999\nprint 1\nlsp 0,\":a;pixel.png\",10,20\nprint 1\ntextspeed 0\n`初回\\\nmov %2,77\nclick\nend\n*twice\ngetparam %9\nmul %9,2\nadd %0,%9\nreturn\n";
    let (_temp, mut r) = fixture(source, false);
    pump(&mut r, |r| diag(r)["wait"].get("Click").is_some());
    assert_eq!(diag(&r)["integers"]["0"], 14);
    assert!(diag(&r)["integers"]["2"].is_null());
    let inspect = r.inspect();
    let layers = &inspect["engine"]["layers"]["clients"]["1"];
    let prepared = layers["4096"]["paint"]["asset"].as_str().unwrap();
    assert!(prepared.starts_with("__surface/"));
    assert_eq!(diag(&r)["native"]["image_sources"]["4096"], "pixel.png");
    let pixels = r.assets.surface(ClientId(1), prepared).unwrap();
    assert_eq!(pixels.rgba.as_ref(), [50, 100, 200, 128].repeat(4));
    assert_eq!(layers["4096"]["rect"], json!([10.0, 20.0, 2.0, 2.0]));
    assert_eq!(layers["1610612736"]["paint"]["kind"], "glyph_run");
    assert_eq!(
        layers["1610612736"]["paint"]["glyphs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(inspect["engine"]["ui"]["title"][1], "Native source");
    press(&mut r);
    assert_eq!(diag(&r)["integers"]["2"], 77);
    assert_eq!(diag(&r)["halted"], false);
    assert!(r.inspect()["engine"]["layers"]["clients"]["1"]["1610612736"].is_null());
    press(&mut r);
    assert!(r.vm_sessions_halted());
}
#[test]
fn xor_script_and_framework_restore_preserve_native_control_without_faking_original_saves() {
    let (_temp, mut r) = fixture("*start\nmov %0,5\nclick\nadd %0,2\nclick\nend\n", true);
    pump(&mut r, |r| diag(r)["wait"].get("Click").is_some());
    let saved = r.snapshot().unwrap();
    assert!(r.export_original_save(ClientId(1)).is_err());
    press(&mut r);
    assert_eq!(diag(&r)["integers"]["0"], 7);
    r.restore(&saved).unwrap();
    assert_eq!(diag(&r)["integers"]["0"], 5);
    let before = r.snapshot().unwrap();
    assert!(r.restore(b"not a snapshot").is_err());
    assert_eq!(before, r.snapshot().unwrap());
    press(&mut r);
    assert_eq!(diag(&r)["integers"]["0"], 7);
    press(&mut r);
    assert!(r.vm_sessions_halted());
}
#[test]
fn unsupported_native_command_fails_at_its_pc_and_does_not_acknowledge_the_next_instruction() {
    let (_temp, mut r) = fixture("*start\nmov %0,3\nsavegame 1\nmov %0,999\nend\n", false);
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match r.tick(16, InputFrame::default()) {
            Ok(()) => {
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => {
                assert!(format!("{error:#}").contains("savegame"));
                break;
            }
        }
    }
    assert_eq!(diag(&r)["integers"]["0"], 3);
    assert_eq!(diag(&r)["pc"], 1);
}

#[test]
fn native_selection_uses_shared_ui_and_keeps_hidden_choice_ordinals() {
    let (_temp, mut r) = fixture(
        "*start\ntextspeed 0\n`前文/\nselnum %3,\"\",\"Visible second choice\"\nclick\nend\n",
        false,
    );
    pump(&mut r, |r| diag(r)["wait"].get("Choices").is_some());
    let inspect = r.inspect();
    let widgets = &inspect["engine"]["ui"]["clients"]["1"];
    assert!(widgets["1644167169"].is_null());
    assert_eq!(
        widgets["1644167170"]["spec"]["text"],
        "Visible second choice"
    );
    press(&mut r);
    assert_eq!(diag(&r)["integers"]["3"], 1);
    assert_eq!(diag(&r)["text"], "");
    assert!(r.inspect()["engine"]["layers"]["clients"]["1"]["1610612736"].is_null());
    assert_eq!(diag(&r)["halted"], false);
    press(&mut r);
    assert!(r.vm_sessions_halted());
}

#[test]
fn global_metadata_commits_on_shared_worker_and_cold_instance_reads_it() {
    let (temp, mut runtime) = fixture(
        "*define\nglobalon\ngame\n*start\nadd %200,1\nclick\nend\n",
        false,
    );
    pump(&mut runtime, |r| diag(r)["wait"].get("Click").is_some());
    pump(&mut runtime, |r| r.snapshot().is_ok());
    assert_eq!(diag(&runtime)["integers"]["200"], 1);
    let mut registry = AdapterRegistry::default();
    crysirror_nscripter::register(&mut registry).unwrap();
    let mut cold = Runtime::with_registry(
        runtime.project.clone(),
        &temp.path().join("project.json"),
        &temp.path().join("state"),
        registry,
    )
    .unwrap();
    pump(&mut cold, |r| diag(r)["wait"].get("Click").is_some());
    assert_eq!(diag(&cold)["integers"]["200"], 2);
    assert!(cold.export_original_save(ClientId(1)).is_err());
}

#[test]
fn native_background_clears_all_character_slots_and_staging_but_keeps_ordinary_sprites() {
    let (_temp,mut runtime)=fixture("*start\nld l,\":c;pixel.png\",1\nld c,\":c;pixel.png\",1\nld r,\":c;pixel.png\",1\nlsp 7,\":a;pixel.png\",10,20\nprint 1\nclick\nbg white,0\nprint 1\nclick\nld r,\":c;pixel.png\",0\nbg black,1\nprint 1\nclick\nend\n",false);
    pump(&mut runtime, |r| diag(r)["wait"].get("Click").is_some());
    let before = runtime.inspect();
    for id in ["5096", "5097", "5098"] {
        assert!(!before["engine"]["layers"]["clients"]["1"][id].is_null());
    }
    assert!(!before["engine"]["layers"]["clients"]["1"]["4103"].is_null());
    press(&mut runtime);
    let cleared = runtime.inspect();
    for id in ["5096", "5097", "5098"] {
        assert!(cleared["engine"]["layers"]["clients"]["1"][id].is_null());
    }
    assert!(!cleared["engine"]["layers"]["clients"]["1"]["4103"].is_null());
    press(&mut runtime);
    pump(&mut runtime, |r| diag(r)["wait"].get("Click").is_some());
    let after = runtime.inspect();
    for id in ["5096", "5097", "5098"] {
        assert!(after["engine"]["layers"]["clients"]["1"][id].is_null());
        assert!(diag(&runtime)["native"]["image_sources"][id].is_null());
    }
    assert!(!after["engine"]["layers"]["clients"]["1"]["4103"].is_null());
    press(&mut runtime);
    assert!(runtime.vm_sessions_halted());
}
