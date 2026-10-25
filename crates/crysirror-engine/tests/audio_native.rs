// SPDX-License-Identifier: MPL-2.0
//! Standalone public engine/data-VM audio routes, no RealLive VM or devices.
use crysirror_core::service::InputFrame;
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
fn nwa() -> Vec<u8> {
    let mut bytes = vec![0; 44];
    bytes[..2].copy_from_slice(&1u16.to_le_bytes());
    bytes[2..4].copy_from_slice(&16u16.to_le_bytes());
    for (at, n) in [
        (4, 48000u32),
        (8, u32::MAX),
        (20, 8192),
        (24, 8236),
        (28, 4096),
    ] {
        bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
    }
    for _ in 0..4096 {
        bytes.extend_from_slice(&16384i16.to_le_bytes());
    }
    bytes
}
fn runtime(program: serde_json::Value) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/tone.nwa"), nwa()).unwrap();
    let project = json!({"schema":2,"name":"native resource without native VM","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"game","adapter":"crysirror-script-v1","game":"fixture","program":program}]});
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let runtime = Runtime::load(&path, &dir.path().join("state")).unwrap();
    (dir, runtime)
}
fn call(app: &mut App, method: &str, args: &[Value]) -> Value {
    let cx = app.context();
    let audio = cx.singleton("Audio").unwrap();
    cx.call(audio, method, args).unwrap()
}
fn output(app: &mut App) -> Vec<[f32; 2]> {
    app.frame(16, InputFrame::default()).unwrap();
    app.context().state().unwrap().mixer_audio.take_output()
}
#[test]
fn public_audio_sfx_and_streaming_music_load_nwa_without_a_native_vm() {
    let (_dir, runtime) = runtime(json!([]));
    let mut app = App::new(runtime, "game").unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let h = call(
        &mut app,
        "playSfx",
        &[Value::from_json(
            json!({"asset":"tone.nwa","gain":0.5,"looping":true}),
        )],
    );
    let samples = output(&mut app);
    assert_eq!(samples.len(), 768);
    assert!(samples.iter().flatten().all(|x| (*x - 0.25).abs() < 1e-6));
    call(&mut app, "stopVoice", &[h]);
    call(
        &mut app,
        "playMusic",
        &[
            Value::Str("song".into()),
            Value::from_json(json!({"asset":"tone.nwa","gain":0.25})),
            Value::Int(0),
        ],
    );
    let samples = output(&mut app);
    assert!(samples.iter().flatten().all(|x| (*x - 0.125).abs() < 1e-6));
}
#[test]
fn ordinary_data_vm_play_reaches_the_same_native_resource_decoder() {
    let (_dir, mut runtime) = runtime(
        json!([{"op":"play","channel":3,"asset":"tone.nwa","looping":true,"gain_milli":500},{"op":"halt"}]),
    );
    runtime.tick(0, InputFrame::default()).unwrap();
    let mut audio =
        crysirror_audio::AudioSystem::new(crysirror_audio::legacy_pcm::MixerDevice::default());
    audio
        .sync(&runtime.host.scene, 0, |id, voice| {
            runtime.assets.read_for(id.client, voice.asset.relative())
        })
        .unwrap();
    let mut out = [[0.; 2]; 48];
    audio.render_pcm(&mut out).unwrap();
    assert!(out.iter().flatten().all(|x| (*x - 0.25).abs() < 1e-6));
}
