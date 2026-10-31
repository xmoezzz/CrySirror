// SPDX-License-Identifier: MPL-2.0
//! Public Audio release API and Runtime safepoints over the actual shared PCM
//! owner. No native VM, mocked host, audio device, or game resources.
use crysirror_core::{
    service::{AudioParameters, AudioRequest, HostServices, PcmAudio},
    state::ClientId,
};
use crysirror_engine::{App, Value};
use crysirror_runtime::Runtime;
use serde_json::json;
use std::time::{Duration, Instant};
fn app() -> (tempfile::TempDir, App) {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&json!({"schema":2,"name":"release","content_revision":"1","canvas":[16,16],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"a"},{"name":"peer","adapter":"crysirror-script-v1","game":"b"}]})).unwrap()).unwrap();
    let mut a = App::new(Runtime::load(&p, &t.path().join("state")).unwrap(), "app").unwrap();
    a.init().unwrap();
    a.start().unwrap();
    (t, a)
}
fn v(j: serde_json::Value) -> Value {
    Value::from_json(j)
}
fn call(a: &mut App, name: &str, args: &[Value]) -> anyhow::Result<Value> {
    let cx = a.context();
    let audio = cx.singleton("Audio")?;
    cx.call(audio, name, args)
}
fn spec() -> Value {
    v(
        json!({"source_channel":0,"tail_channel":1,"threshold_db100":-3000,"target_above_db100":-3000,"target_below_db100":-10000,"steps":30,"freeze_group_gain":true,"release_tail_when_source_missing":true}),
    )
}
fn play(a: &mut App, gain: f32) {
    a.runtime_mut()
        .unwrap()
        .host
        .submit_audio(
            ClientId(1),
            AudioRequest::PlayPcm {
                channel: 0,
                source: PcmAudio::new(48000, (1..=1000).map(|i| [i as f32 / 1000.; 2]).collect())
                    .unwrap(),
                parameters: AudioParameters {
                    gain,
                    ..Default::default()
                },
            },
        )
        .unwrap();
}
#[test]
fn public_methods_transfer_real_pcm_scope_exact_handles_and_block_obsolete_restore() {
    let (_t, mut a) = app();
    let rt = a.runtime().unwrap();
    let saved = rt.snapshot().unwrap();
    let prepared = rt.restore_plan().decode(&saved).unwrap();
    play(&mut a, 0.5);
    let mut pcm = [[0.; 2]; 7];
    a.runtime_mut()
        .unwrap()
        .host
        .channel_audio
        .mixer
        .render(&mut pcm)
        .unwrap();
    assert!((pcm[6][0] - 0.0035).abs() < 1e-7);
    let h = call(&mut a, "beginVoiceRelease", &[spec()]).unwrap();
    let handle = h.to_json();
    assert!(
        handle["generation"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    let i = call(&mut a, "voiceRelease", &[h.clone()])
        .unwrap()
        .to_json();
    assert_eq!(i["phase"], "active");
    assert_eq!(i["source_position_frames"], 7);
    assert_eq!(i["completed_updates"], 0);
    {
        let rt = a.runtime_mut().unwrap();
        assert!(!rt.host.channel_audio.is_playing(1, 0));
        assert!(rt.host.channel_audio.is_playing(1, 1));
        assert!(rt.capture_snapshot().is_err());
        assert!(rt.commit_restore(prepared).is_err());
        let mut next = [[0.; 2]; 1];
        rt.host.channel_audio.mixer.render(&mut next).unwrap();
        assert!((next[0][0] - 0.004).abs() < 1e-7);
    }
    a.context().state().unwrap().client = ClientId(2);
    assert!(call(&mut a, "voiceRelease", &[h.clone()]).is_err());
    assert!(call(&mut a, "advanceVoiceRelease", &[h.clone(), v(json!("1"))]).is_err());
    a.context().state().unwrap().client = ClientId(1);
    for bad in [
        v(json!({"client":1,"generation":9_007_199_254_740_992u64})),
        v(json!({"client":1,"generation":"18446744073709551616"})),
    ] {
        assert!(call(&mut a, "voiceRelease", &[bad]).is_err());
    }
    call(&mut a, "advanceVoiceRelease", &[h.clone(), v(json!("1"))]).unwrap();
    call(&mut a, "advanceVoiceRelease", &[h.clone(), v(json!(1))]).unwrap();
    assert_eq!(
        call(&mut a, "voiceRelease", &[h.clone()])
            .unwrap()
            .to_json()["completed_updates"],
        1
    );
    assert!(call(&mut a, "advanceVoiceRelease", &[h.clone(), v(json!("-1"))]).is_err());
    assert!(call(&mut a, "releaseVoiceRelease", &[h.clone()]).is_err());
    call(
        &mut a,
        "advanceVoiceRelease",
        &[h.clone(), v(json!("1000"))],
    )
    .unwrap();
    let i = call(&mut a, "voiceRelease", &[h.clone()])
        .unwrap()
        .to_json();
    assert_eq!(i["phase"], "complete");
    assert!(i["stopped_at_update"].as_u64().unwrap() <= 30);
    assert!(a.runtime().unwrap().capture_snapshot().is_ok());
    call(&mut a, "releaseVoiceRelease", &[h.clone()]).unwrap();
    assert!(call(&mut a, "voiceRelease", &[h]).is_err());
}
#[test]
fn pending_receipt_waits_for_real_worker_owner_then_publishes_transfer_and_cancellation() {
    let (_t, mut a) = app();
    let saved = a.runtime().unwrap().snapshot().unwrap();
    let prepared = a.runtime().unwrap().restore_plan().decode(&saved).unwrap();
    play(&mut a, 1.);
    let pool = a.runtime().unwrap().assets.cpu_tasks();
    let mut job = {
        let rt = a.runtime_mut().unwrap();
        crysirror_audio::render_job::MixJob::begin(
            &pool,
            &mut [&mut rt.host.channel_audio.mixer],
            5,
            vec![],
        )
        .unwrap()
    };
    let h = call(&mut a, "beginVoiceRelease", &[spec()]).unwrap();
    call(&mut a, "advanceVoiceRelease", &[h.clone(), v(json!("2"))]).unwrap();
    let i = call(&mut a, "voiceRelease", &[h.clone()])
        .unwrap()
        .to_json();
    assert_eq!(i["phase"], "pending");
    assert_eq!(i["submitted_updates"], 2);
    assert_eq!(i["completed_updates"], 0);
    {
        let rt = a.runtime_mut().unwrap();
        assert!(rt.capture_snapshot().is_err());
        assert!(rt.commit_restore(prepared).is_err());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let rt = a.runtime_mut().unwrap();
        if let Some(pcm) = job.poll(&mut [&mut rt.host.channel_audio.mixer]).unwrap() {
            assert_eq!(pcm.len(), 5);
            assert!((pcm[4][0] - 0.005).abs() < 1e-7);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let i = call(&mut a, "voiceRelease", &[h.clone()])
        .unwrap()
        .to_json();
    assert_eq!(i["phase"], "active");
    assert_eq!(i["completed_updates"], 2);
    assert_eq!(i["volume_db100"], -200);
    assert_eq!(i["source_position_frames"], 5);
    assert!(a.runtime().unwrap().capture_snapshot().is_err());
    a.runtime_mut()
        .unwrap()
        .host
        .submit_audio(
            ClientId(1),
            AudioRequest::Stop {
                channel: 1,
                fade_ms: 0.,
            },
        )
        .unwrap();
    assert_eq!(
        call(&mut a, "voiceRelease", &[h.clone()])
            .unwrap()
            .to_json()["phase"],
        "cancelled"
    );
    call(&mut a, "releaseVoiceRelease", &[h]).unwrap();
    assert!(a.runtime().unwrap().capture_snapshot().is_ok());
}
