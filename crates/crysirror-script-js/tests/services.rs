// SPDX-License-Identifier: MPL-2.0
//! New service globals use the real Oxc/Boa bindings, without devices or fonts.
use crysirror_core::service::{buttons, InputFrame};
use crysirror_engine::App;
use crysirror_runtime::Runtime;
use crysirror_script_js::JsHost;

#[test]
fn typescript_service_globals_drive_real_pcm_rebinding_and_localization() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("assets")).unwrap();
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend(3236u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(3200u32.to_le_bytes());
    for _ in 0..1600 {
        wav.extend(3200i16.to_le_bytes());
    }
    std::fs::write(tmp.path().join("assets/tone.wav"), wav).unwrap();
    let project = serde_json::json!({"schema":2,"name":"services","content_revision":"1","assets":"assets","canvas":[64,64],"clients":[{"name":"game","adapter":"crysirror-script-v1","game":"services"}]});
    std::fs::write(
        tmp.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("state")).unwrap(),
        "game",
    )
    .unwrap();
    let source = r#"
      let voice: any;
      function onStart(): void {
        localization.define("en", {strings:{coins:{forms:{one:"{count} coin",other:"{count} coins"}}}});
        localization.setLocale("en-US");
        engine.log(localization.translate("coins",{count:2}));
        input.rebind("confirm",{bindings:[{kind:"key",key:"enter"}]});
        audio.setBus("sfx",{gain:0.5,muted:false});
        voice=audio.playSfx({asset:"tone.wav",bus:"sfx",gain:1});
        try {text.state("missing");}catch(error){engine.log("text-bound");}
      }
      function onUpdate(): void {
        if(input.pressed("confirm")){audio.stopVoice(voice,0);engine.log("confirm");}
      }
    "#;
    app.add_script_host(Box::new(
        JsHost::with_source("services", "services.ts", source).unwrap(),
    ))
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    let first = app.context().state().unwrap().mixer_audio.take_output();
    assert_eq!(first.len(), 768);
    assert!(first.iter().any(|p| p[0] > 0.01));
    app.frame(
        16,
        InputFrame {
            buttons_down: buttons::ENTER,
            buttons_pressed: buttons::ENTER,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(app
        .context()
        .state()
        .unwrap()
        .mixer_audio
        .take_output()
        .iter()
        .all(|p| p == &[0., 0.]));
    assert_eq!(
        app.engine()
            .unwrap()
            .log
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["2 coins", "text-bound", "confirm"]
    );
}
