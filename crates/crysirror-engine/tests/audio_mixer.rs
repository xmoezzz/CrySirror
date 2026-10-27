// SPDX-License-Identifier: MPL-2.0
//! Object API -> VFS/world/native channels -> real CPU decoder/mixer samples.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
fn v(j: serde_json::Value) -> Value {
    Value::from_json(j)
}
fn wav(value: i16) -> Vec<u8> {
    let size = 32000u32;
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + size).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    for _ in 0..16000 {
        b.extend(value.to_le_bytes())
    }
    b
}
fn app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    for (name, amplitude) in [("a", 16384), ("b", 8192)] {
        std::fs::create_dir_all(dir.path().join(format!("assets/{name}"))).unwrap();
        std::fs::write(
            dir.path().join(format!("assets/{name}/tone.wav")),
            wav(amplitude),
        )
        .unwrap();
    }
    let project = json!({"schema":2,"name":"audio","content_revision":"1","assets":"assets","canvas":[64,64],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b","asset_prefix":"b"}]});
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut app = App::new(
        Runtime::load(&path, &dir.path().join("state")).unwrap(),
        "a",
    )
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    (dir, app)
}
fn call(app: &mut App, name: &str, args: &[Value]) -> Value {
    let cx = app.context();
    let audio = cx.singleton("Audio").unwrap();
    cx.call(audio, name, args).unwrap()
}
fn output(app: &mut App, ms: u64) -> Vec<[f32; 2]> {
    app.frame(ms, InputFrame::default()).unwrap();
    app.context().state().unwrap().mixer_audio.take_output()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-4, "{a} != {b}")
}

#[test]
fn real_samples_pause_seek_partial_parameters_buses_and_scope() {
    let (_dir, mut app) = app();
    let h = call(
        &mut app,
        "playSfx",
        &[v(json!({"asset":"tone.wav","gain":0.5,"looping":true}))],
    );
    let samples = output(&mut app, 16);
    assert_eq!(samples.len(), 768);
    near(samples[0][0], 0.25);
    call(
        &mut app,
        "setVoice",
        &[h.clone(), v(json!({"rate":2,"pitch_semitones":12}))],
    );
    call(&mut app, "setVoice", &[h.clone(), v(json!({"gain":0.75}))]);
    let handle = serde_json::from_value(h.to_json()).unwrap();
    let spec = app
        .context()
        .state()
        .unwrap()
        .mixer_audio
        .mixer
        .spec(1, handle)
        .unwrap();
    assert_eq!(spec.rate, 2.);
    assert_eq!(spec.pitch_semitones, 12.);
    call(&mut app, "pauseVoice", &[h.clone(), Value::Bool(true)]);
    let before = call(&mut app, "voice", &[h.clone()]);
    assert!(output(&mut app, 16).iter().flatten().all(|x| *x == 0.));
    assert_eq!(
        before.to_json(),
        call(&mut app, "voice", &[h.clone()]).to_json()
    );
    call(&mut app, "seekVoice", &[h.clone(), Value::Int(0)]);
    call(&mut app, "pauseVoice", &[h.clone(), Value::Bool(false)]);
    call(
        &mut app,
        "setBus",
        &[
            Value::Str("sfx".into()),
            v(json!({"gain":0.5,"muted":false})),
        ],
    );
    near(output(&mut app, 16)[0][0], 0.1875);
    app.context().state().unwrap().client = ClientId(2);
    let cx = app.context();
    let a = cx.singleton("Audio").unwrap();
    assert!(cx.call(a, "stopVoice", &[h.clone()]).is_err());
    call(&mut app, "playSfx", &[v(json!({"asset":"tone.wav"}))]);
    near(output(&mut app, 16)[0][0], 0.4375); // client b reads different bytes and bus defaults
    let cx = app.context();
    let a = cx.singleton("Audio").unwrap();
    assert!(
        cx.call(a, "playSfx", &[v(json!({"asset":"../tone.wav"}))])
            .is_err()
    );
    assert!(
        cx.call(a, "playSfx", &[v(json!({"asset":"tone.wav","unknown":1}))])
            .is_err()
    );
}
#[test]
fn channel_retrigger_restore_and_pooled_voices_share_one_output() {
    let (_dir, mut app) = app();
    call(
        &mut app,
        "play",
        &[
            Value::Int(3),
            v(json!({"asset":"tone.wav","looping":true,"gain":0.5})),
        ],
    );
    near(output(&mut app, 16)[0][0], 0.25);
    let save = app.runtime().unwrap().snapshot().unwrap();
    call(
        &mut app,
        "playSfx",
        &[v(json!({"asset":"tone.wav","gain":0.5}))],
    );
    near(output(&mut app, 16)[0][0], 0.5);
    app.context()
        .state()
        .unwrap()
        .runtime
        .restore(&save)
        .unwrap();
    near(output(&mut app, 16)[0][0], 0.25);
    assert_eq!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .stats()
            .active,
        1
    );
    let epoch = app
        .runtime()
        .unwrap()
        .host
        .scene
        .voice_epoch(crysirror_core::state::ObjectId {
            client: ClientId(1),
            local: 3,
        });
    call(
        &mut app,
        "play",
        &[Value::Int(3), v(json!({"asset":"tone.wav","gain":0.25}))],
    );
    near(output(&mut app, 16)[0][0], 0.125);
    assert!(
        app.runtime()
            .unwrap()
            .host
            .scene
            .voice_epoch(crysirror_core::state::ObjectId {
                client: ClientId(1),
                local: 3
            })
            > epoch
    );
    call(&mut app, "stop", &[Value::Int(3)]);
    assert!(output(&mut app, 16).iter().flatten().all(|x| *x == 0.));
}
#[test]
fn music_crossfade_and_script_random_container_pool_limit_reach_samples() {
    let (_dir, mut app) = app();
    let old = call(
        &mut app,
        "playMusic",
        &[Value::Str("bgm".into()), v(json!({"asset":"tone.wav"}))],
    );
    near(output(&mut app, 16)[0][0], 0.5);
    let new = call(
        &mut app,
        "playMusic",
        &[
            Value::Str("bgm".into()),
            v(json!({"asset":"tone.wav","gain":0.5})),
            Value::Int(16),
        ],
    );
    let samples = output(&mut app, 16);
    near(samples[0][0], 0.5);
    near(samples[384][0], 0.375);
    near(output(&mut app, 16)[0][0], 0.25);
    assert_eq!(
        call(&mut app, "voice", &[old])
            .field("playing")
            .unwrap()
            .as_bool()
            .unwrap(),
        false
    );
    assert_eq!(
        call(&mut app, "voice", &[new.clone()])
            .field("playing")
            .unwrap()
            .as_bool()
            .unwrap(),
        true
    );
    call(&mut app, "stopVoice", &[new]);
    call(&mut app, "setVoiceLimit", &[Value::Int(1)]);
    call(&mut app, "setSeed", &[Value::Int(42)]);
    let mut previous = None;
    let container = v(
        json!({"variants":[{"asset":"tone.wav"},{"asset":"tone.wav"}],"gain":[0.25,0.25],"pitch_semitones":[0,0],"avoid_repeat":true}),
    );
    for _ in 0..4 {
        let result = call(
            &mut app,
            "playRandom",
            &[Value::Str("steps".into()), container.clone()],
        );
        let index = result.field("variant").unwrap().as_i64().unwrap();
        assert_ne!(previous, Some(index));
        previous = Some(index);
        near(output(&mut app, 16)[0][0], 0.125);
    }
    assert_eq!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .stats()
            .active,
        1
    );
}
#[test]
fn entity_attachment_tracks_camera_and_stale_generation_stops() {
    let (_dir, mut app) = app();
    let cx = app.context();
    let w = cx.singleton("World").unwrap();
    cx.call(
        w,
        "setCamera",
        &[v(json!({"eye":[0,0,0],"target":[0,0,-1]}))],
    )
    .unwrap();
    let emitter = cx
        .call(
            w,
            "spawn",
            &[v(json!({"name":"emitter","position":[5,0,0]}))],
        )
        .unwrap();
    let h = call(
        &mut app,
        "playSfx",
        &[v(
            json!({"asset":"tone.wav","looping":true,"follow":emitter.to_json(),"raycast_occlusion":true,"spatial":{"min_distance":1,"max_distance":20,"attenuation":{"kind":"linear"}}}),
        )],
    );
    let sample = output(&mut app, 16)[0];
    near(sample[0], 0.);
    assert!(sample[1] > 0.3);
    let cx = app.context();
    let w = cx.singleton("World").unwrap();
    let wall=cx.call(w,"spawn",&[v(json!({"name":"wall","position":[2,0,0],"scale":[0.2,4,4],"geometry":{"kind":"cube"},"collider":{"shape":{"kind":"box","half_extents":[0.5,0.5,0.5]}}}))]).unwrap();
    let blocked = output(&mut app, 16);
    assert!(blocked.last().unwrap()[1] < sample[1] * 0.4);
    let handle = serde_json::from_value(h.to_json()).unwrap();
    assert_eq!(
        app.context()
            .state()
            .unwrap()
            .mixer_audio
            .mixer
            .spec(1, handle)
            .unwrap()
            .spatial
            .unwrap()
            .occlusion,
        1.
    );
    app.context()
        .call(wall.as_object().unwrap(), "destroy", &[])
        .unwrap();
    let cx = app.context();
    cx.set(
        emitter.as_object().unwrap(),
        "position",
        Value::vec3([-5., 0., 0.]),
    )
    .unwrap();
    let sample = output(&mut app, 16)[0];
    assert!(sample[0] > 0.3);
    near(sample[1], 0.);
    let cx = app.context();
    cx.call(emitter.as_object().unwrap(), "destroy", &[])
        .unwrap();
    let w = cx.singleton("World").unwrap();
    cx.call(
        w,
        "spawn",
        &[v(json!({"name":"new entity reuses id","position":[5,0,0]}))],
    )
    .unwrap();
    assert!(output(&mut app, 16).iter().flatten().all(|x| *x == 0.));
    assert!(
        !call(&mut app, "voice", &[h])
            .field("playing")
            .unwrap()
            .as_bool()
            .unwrap()
    );
}

#[test]
fn shared_transport_inspection_scheduled_resume_and_native_channels_are_scoped() {
    let (_dir, mut app) = app();
    let h = call(
        &mut app,
        "playSfx",
        &[v(json!({"asset":"tone.wav","looping":true}))],
    );
    call(
        &mut app,
        "scheduleVoiceResume",
        &[h.clone(), Value::Float(2.), Value::Float(1.)],
    );
    let info = call(&mut app, "playbackInfo", &[h.clone()]).to_json();
    assert_eq!(info["resume_delay_frames"], 96);
    assert_eq!(info["output_rate"], 48000);
    assert!(output(&mut app, 2).iter().flatten().all(|s| *s == 0.));
    let info = call(&mut app, "playbackInfo", &[h.clone()]).to_json();
    assert_eq!(info["position_frames"], 0);
    assert_eq!(info["paused"], false);
    let pcm = output(&mut app, 1);
    near(pcm[0][0], 0.);
    assert!(pcm.last().unwrap()[0] > 0.4);
    app.context()
        .state()
        .unwrap()
        .runtime
        .host
        .channel_audio
        .submit(
            1,
            crysirror_core::service::AudioRequest::PlayPcm {
                channel: 8,
                source: crysirror_core::service::PcmAudio::new(48000, vec![[0.25; 2]; 100])
                    .unwrap(),
                parameters: crysirror_core::service::AudioParameters {
                    start_frame: Some(7),
                    paused: true,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    assert_eq!(
        call(&mut app, "nativeChannelInfo", &[Value::Int(8)]).to_json()["position_frames"],
        7
    );
    app.context().state().unwrap().client = ClientId(2);
    assert!(
        call(&mut app, "nativeChannelInfo", &[Value::Int(8)])
            .to_json()
            .is_null()
    );
    let cx = app.context();
    let audio = cx.singleton("Audio").unwrap();
    assert!(cx.call(audio, "playbackInfo", &[h]).is_err());
}

#[test]
fn reflective_resume_policy_preserves_running_input_and_reports_transport_frames() {
    let (_dir, mut app) = app();
    let h = call(
        &mut app,
        "playSfx",
        &[v(json!({"asset":"tone.wav","looping":true}))],
    );
    call(
        &mut app,
        "scheduleVoiceResumeWithOptions",
        &[
            h.clone(),
            Value::Float(2.),
            Value::Float(0.),
            v(json!({"policy":"preserve"})),
        ],
    );
    let pcm = output(&mut app, 1);
    assert!(pcm.iter().flatten().all(|x| (*x - 0.5).abs() < 1e-6));
    let i = call(&mut app, "playbackInfo", &[h.clone()]).to_json();
    assert_eq!(i["transport_frames"], 48);
    assert_eq!(i["age_frames"], 48);
    assert_eq!(i["resume_delay_frames"], 48);
    assert_eq!(i["stopping"], false);
    assert_eq!(i["pausing"], false);
    let cx = app.context();
    let a = cx.singleton("Audio").unwrap();
    assert!(
        cx.call(
            a,
            "scheduleVoiceResumeWithOptions",
            &[
                h.clone(),
                Value::Float(2.),
                Value::Float(0.),
                v(json!({"policy":"unknown"}))
            ]
        )
        .is_err()
    );
    call(&mut app, "pauseVoice", &[h.clone(), Value::Bool(true)]);
    output(&mut app, 1);
    assert_eq!(
        call(&mut app, "playbackInfo", &[h]).to_json()["transport_frames"],
        48
    );
}
