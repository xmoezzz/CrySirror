// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Authored PCM over canonical logical declarations, without any VM or device.
use crysirror_audio::{
    decoder::Clip,
    mixer::{Bus, Mixer, MixerConfig},
    scene::{SceneAudio, SceneVoicePhase},
};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
    state::{AudioVoice, ClientId, Command, Limits, ObjectId, SceneState},
};
use crysirror_tasks::{Task, TaskPool};
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{self, ThreadId},
    time::{Duration, Instant},
};
fn wav(samples: &[i16]) -> Vec<u8> {
    let size = samples.len() as u32 * 2;
    let mut b = b"RIFF".to_vec();
    b.extend((36 + size).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(48000u32.to_le_bytes());
    b.extend(96000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(size.to_le_bytes());
    for s in samples {
        b.extend(s.to_le_bytes());
    }
    b
}
struct Storage {
    value: i16,
    calls: Mutex<Vec<ThreadId>>,
}
impl ClientStorage for Storage {
    fn read(&self, p: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        self.calls.lock().unwrap().push(thread::current().id());
        match p.relative() {
            "tone.wav" => Ok(wav(&(0..16000)
                .map(|i| if i < 8000 { self.value } else { self.value / 2 })
                .collect::<Vec<_>>())),
            "intro.wav" => Ok(wav(&vec![4096; 1000])),
            _ => Err(crysirror_core::Error::NotFound),
        }
    }
    fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
    fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
    fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
        Ok(vec![])
    }
}
struct Gate {
    gate: Arc<(Mutex<bool>, Condvar)>,
    task: Option<Task<()>>,
}
impl Gate {
    fn block(p: &TaskPool) -> Self {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let g = gate.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let task = p
            .spawn(move |_| {
                tx.send(())?;
                let (v, _) =
                    g.1.wait_timeout_while(g.0.lock().unwrap(), Duration::from_secs(5), |v| !*v)
                        .unwrap();
                anyhow::ensure!(*v, "gate timeout");
                Ok(())
            })
            .unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            gate,
            task: Some(task),
        }
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
        if let Some(t) = self.task.take() {
            t.wait().unwrap();
        }
    }
}
fn id(client: u32, channel: u32) -> ObjectId {
    ObjectId {
        client: ClientId(client),
        local: channel,
    }
}
fn play(scene: &mut SceneState, client: u32, channel: u32, name: &str, looping: bool) {
    scene
        .apply_batch(
            ClientId(client),
            &[Command::PlayAudio {
                channel,
                voice: AudioVoice {
                    asset: VfsPath::new(Mount::Asset, name).unwrap(),
                    looping,
                    gain_milli: 1000,
                },
            }],
            Limits::default(),
        )
        .unwrap();
}
fn storage() -> Arc<Storage> {
    Arc::new(Storage {
        value: 16384,
        calls: Mutex::new(vec![]),
    })
}
fn mixer() -> Mixer {
    Mixer::new(MixerConfig {
        sample_rate: 48000,
        max_voices: 128,
        ..Default::default()
    })
    .unwrap()
}
fn sync(
    a: &mut SceneAudio,
    m: &mut Mixer,
    s: &SceneState,
    p: &TaskPool,
    io: &Arc<Storage>,
    restored: u64,
    live: bool,
) -> anyhow::Result<()> {
    a.sync(m, s, restored, p, live, |_| Ok(io.clone()))
}
fn ready(
    a: &mut SceneAudio,
    m: &mut Mixer,
    s: &SceneState,
    p: &TaskPool,
    io: &Arc<Storage>,
    key: ObjectId,
) {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        sync(a, m, s, p, io, 0, true).unwrap();
        if a.status(m, s, key).phase == SceneVoicePhase::Playing {
            return;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn pending_and_full_pool_do_not_block_other_voices_or_publish_after_stop() {
    let p = TaskPool::new("scene", 1, 64).unwrap();
    let mut a = SceneAudio::default();
    let mut m = mixer();
    let mut s = SceneState::default();
    let io = storage();
    let permits = (0..64).map(|_| p.reserve().unwrap()).collect::<Vec<_>>();
    let other = m
        .play(
            2,
            Clip::new(48000, vec![[0.25; 2]; 1024]).unwrap().source(),
            Default::default(),
        )
        .unwrap();
    play(&mut s, 1, 3, "tone.wav", true);
    sync(&mut a, &mut m, &s, &p, &io, 0, true).unwrap();
    assert_eq!(a.status(&m, &s, id(1, 3)).phase, SceneVoicePhase::Loading);
    assert!(io.calls.lock().unwrap().is_empty());
    let mut out = [[0.; 2]; 48];
    m.render(&mut out).unwrap();
    assert!(out.iter().all(|v| *v == [0.25; 2]));
    s.apply_batch(
        ClientId(1),
        &[Command::StopAudio { channel: 3 }],
        Limits::default(),
    )
    .unwrap();
    sync(&mut a, &mut m, &s, &p, &io, 0, true).unwrap();
    drop(permits);
    sync(&mut a, &mut m, &s, &p, &io, 0, true).unwrap();
    assert_eq!(a.status(&m, &s, id(1, 3)).phase, SceneVoicePhase::Stopped);
    assert!(io.calls.lock().unwrap().is_empty());
    assert!(m.is_playing(2, other));
}
#[test]
fn replay_same_asset_stale_completion_gain_routing_and_restore_use_logical_epochs() {
    let p = TaskPool::new("scene", 1, 64).unwrap();
    let mut a = SceneAudio::default();
    let mut m = mixer();
    let mut s = SceneState::default();
    let io = storage();
    play(&mut s, 1, 3, "tone.wav", true);
    ready(&mut a, &mut m, &s, &p, &io, id(1, 3));
    let mut out = [[0.; 2]; 48];
    m.render(&mut out).unwrap();
    assert_eq!(out[0], [0.5; 2]);
    let first = s.voice_epoch(id(1, 3));
    let gate = Gate::block(&p);
    play(&mut s, 1, 3, "tone.wav", true);
    sync(&mut a, &mut m, &s, &p, &io, 0, true).unwrap();
    assert!(s.voice_epoch(id(1, 3)) > first);
    play(&mut s, 1, 3, "intro.wav", true);
    sync(&mut a, &mut m, &s, &p, &io, 0, true).unwrap();
    drop(gate);
    ready(&mut a, &mut m, &s, &p, &io, id(1, 3));
    s.apply_batch(
        ClientId(1),
        &[Command::SetAudioGain {
            channel: 3,
            gain_milli: 500,
        }],
        Limits::default(),
    )
    .unwrap();
    let epoch = s.voice_epoch(id(1, 3));
    a.set_route(id(1, 3), Bus::Music).unwrap();
    m.set_bus(
        1,
        Bus::Music,
        crysirror_audio::mixer::BusSettings {
            gain: 0.5,
            muted: false,
        },
    )
    .unwrap();
    sync(&mut a, &mut m, &s, &p, &io, 0, true).unwrap();
    m.render(&mut out).unwrap();
    assert_eq!(out[0], [0.03125; 2]);
    assert_eq!(s.voice_epoch(id(1, 3)), epoch);
    sync(&mut a, &mut m, &s, &p, &io, 1, true).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while a.status(&m, &s, id(1, 3)).phase != SceneVoicePhase::Playing {
        sync(&mut a, &mut m, &s, &p, &io, 1, true).unwrap();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(a.status(&m, &s, id(1, 3)).position_seconds, Some(0.));
    assert!(
        io.calls
            .lock()
            .unwrap()
            .iter()
            .all(|t| *t != thread::current().id())
    );
}
#[test]
fn ended_declarations_do_not_restart_and_failure_is_visible_without_discarding_old_voice() {
    let p = TaskPool::new("scene", 1, 64).unwrap();
    let mut a = SceneAudio::default();
    let mut m = mixer();
    let mut s = SceneState::default();
    let io = storage();
    play(&mut s, 1, 3, "intro.wav", false);
    sync(&mut a, &mut m, &s, &p, &io, 0, false).unwrap();
    let mut out = vec![[0.; 2]; 1024];
    m.render(&mut out).unwrap();
    assert_eq!(a.status(&m, &s, id(1, 3)).phase, SceneVoicePhase::Ended);
    let calls = io.calls.lock().unwrap().len();
    sync(&mut a, &mut m, &s, &p, &io, 0, false).unwrap();
    assert_eq!(calls, io.calls.lock().unwrap().len());
    play(&mut s, 1, 3, "intro.wav", true);
    sync(&mut a, &mut m, &s, &p, &io, 0, false).unwrap();
    play(&mut s, 1, 3, "missing.wav", true);
    assert!(sync(&mut a, &mut m, &s, &p, &io, 0, false).is_err());
    assert_eq!(a.status(&m, &s, id(1, 3)).phase, SceneVoicePhase::Failed);
    m.render(&mut out[..1]).unwrap();
    assert_eq!(out[0], [0.125; 2]);
}
#[test]
fn offline_full_canonical_limit_is_immediate_and_pause_includes_prepared_voices() {
    let p = TaskPool::new("scene", 1, 64).unwrap();
    let mut a = SceneAudio::default();
    let mut m = mixer();
    let mut s = SceneState::default();
    let io = storage();
    for ch in 0..128 {
        play(&mut s, 1, ch, "intro.wav", true);
    }
    sync(&mut a, &mut m, &s, &p, &io, 0, false).unwrap();
    assert_eq!(m.stats().active, 128);
    a.set_paused(&mut m, true).unwrap();
    let mut out = [[0.; 2]; 16];
    m.render(&mut out).unwrap();
    assert!(out.iter().flatten().all(|v| *v == 0.));
    assert_eq!(a.status(&m, &s, id(1, 100)).phase, SceneVoicePhase::Paused);
    assert_eq!(a.status(&m, &s, id(1, 100)).position_seconds, Some(0.));
    a.set_paused(&mut m, false).unwrap();
    m.render(&mut out).unwrap();
    assert_eq!(out[0], [16.; 2]);
}
#[test]
fn excess_realtime_declarations_queue_and_eventually_publish_all_without_spurious_failures() {
    let p = TaskPool::new("scene", 1, 64).unwrap();
    let mut a = SceneAudio::default();
    let mut m = mixer();
    let mut s = SceneState::default();
    let io = storage();
    let gate = Gate::block(&p);
    for ch in 0..20 {
        play(&mut s, 1, ch, "intro.wav", true);
    }
    sync(&mut a, &mut m, &s, &p, &io, 0, true).unwrap();
    assert_eq!(a.status(&m, &s, id(1, 19)).phase, SceneVoicePhase::Queued);
    drop(gate);
    let end = Instant::now() + Duration::from_secs(5);
    while m.stats().active < 20 {
        sync(&mut a, &mut m, &s, &p, &io, 0, true).unwrap();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn streaming_fault_is_visible_in_status_while_valid_predecoded_prefix_is_retained() {
    use crysirror_core::service::AssetReader;
    struct BrokenReader(Vec<u8>);
    impl AssetReader for BrokenReader {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(self.0.len() as u64)
        }
        fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
            if at >= 44 + 8192 * 2 {
                return Err(crysirror_core::Error::Io("authored stream fault".into()));
            }
            let from = (at as usize).min(self.0.len());
            let n = out.len().min(self.0.len() - from);
            out[..n].copy_from_slice(&self.0[from..from + n]);
            Ok(n)
        }
    }
    struct BrokenStorage;
    impl ClientStorage for BrokenStorage {
        fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            Err(crysirror_core::Error::Unsupported(
                "must use capability reader",
            ))
        }
        fn open_asset(&self, _: &VfsPath) -> crysirror_core::Result<Arc<dyn AssetReader>> {
            let frames = 65536u32;
            let mut b = vec![0; 44];
            b[..2].copy_from_slice(&1u16.to_le_bytes());
            b[2..4].copy_from_slice(&16u16.to_le_bytes());
            for (at, n) in [
                (4, 48000u32),
                (8, u32::MAX),
                (20, frames * 2),
                (24, 44 + frames * 2),
                (28, frames),
            ] {
                b[at..at + 4].copy_from_slice(&n.to_le_bytes());
            }
            for _ in 0..frames {
                b.extend(16384i16.to_le_bytes());
            }
            Ok(Arc::new(BrokenReader(b)))
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            Err(crysirror_core::Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            Err(crysirror_core::Error::ReadOnly)
        }
        fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
            Ok(vec![])
        }
    }
    let p = TaskPool::new("scene-fault", 1, 64).unwrap();
    let mut a = SceneAudio::default();
    let mut m = mixer();
    let mut s = SceneState::default();
    play(&mut s, 1, 0, "authored.nwa", false);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        a.sync(&mut m, &s, 0, &p, true, |_| Ok(Arc::new(BrokenStorage)))
            .unwrap();
        if a.status(&m, &s, id(1, 0)).phase == SceneVoicePhase::Playing {
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    loop {
        m.service_sources().unwrap();
        if a.status(&m, &s, id(1, 0)).phase == SceneVoicePhase::Failed {
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    let mut prefix = [[0.; 2]; 1];
    m.render(&mut prefix).unwrap();
    assert_eq!(prefix[0], [0.5; 2]);
    assert!(
        a.status(&m, &s, id(1, 0))
            .error
            .unwrap()
            .contains("authored stream fault")
    );
}
