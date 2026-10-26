// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use crysirror_audio::channels::ChannelAudio;
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, ClientStorage},
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
fn input(name: &str) -> AudioAsset {
    AudioAsset {
        path: VfsPath::new(Mount::Asset, name).unwrap(),
        frame_limit: None,
    }
}
fn play(channel: u32, name: &str) -> AudioRequest {
    AudioRequest::PlayAsset {
        channel,
        source: input(name),
        intro: None,
        parameters: AudioParameters::default(),
    }
}
fn wait(audio: &mut ChannelAudio, owner: u32, t: u64) -> crysirror_core::service::AudioPreparation {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let s = audio.preparation(owner, t).unwrap();
        if s.phase != AudioPreparePhase::Pending {
            return s;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
}
fn setup() -> (ChannelAudio, TaskPool, Arc<Storage>) {
    let p = TaskPool::new("channel-test", 1, 64).unwrap();
    let mut a = ChannelAudio::default();
    a.configure_preparation(p.clone(), true);
    (
        a,
        p,
        Arc::new(Storage {
            value: 8192,
            calls: Mutex::new(vec![]),
        }),
    )
}
#[test]
fn capability_reads_are_on_workers_and_foreign_or_canceled_receipts_cannot_publish() {
    let (mut a, p, storage) = setup();
    let gate = Gate::block(&p);
    let t = a
        .begin_preparation(1, play(7, "tone.wav"), storage.clone())
        .unwrap();
    assert_eq!(
        a.preparation(1, t).unwrap().phase,
        AudioPreparePhase::Pending
    );
    assert!(storage.calls.lock().unwrap().is_empty());
    assert!(!a.is_playing(1, 7));
    assert!(a.preparation(2, t).is_err());
    assert!(a.release_preparation(2, t).is_err());
    a.render(16).unwrap();
    assert!(a.take_output().iter().flatten().all(|v| *v == 0.));
    a.release_preparation(1, t).unwrap();
    drop(gate);
    let t = a
        .begin_preparation(1, play(7, "tone.wav"), storage.clone())
        .unwrap();
    assert_eq!(wait(&mut a, 1, t).phase, AudioPreparePhase::Ready);
    a.release_preparation(1, t).unwrap();
    a.render(1).unwrap();
    assert!(a.take_output().iter().all(|v| *v == [0.25; 2]));
    assert!(
        storage
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|id| *id != thread::current().id())
    );
}
#[test]
fn intro_tail_declared_limits_start_offset_gain_and_loop_are_preserved() {
    let (mut a, _p, storage) = setup();
    let t = a
        .begin_preparation(
            1,
            AudioRequest::PlayAsset {
                channel: 8,
                source: AudioAsset {
                    frame_limit: Some(14000),
                    ..input("tone.wav")
                },
                intro: Some(AudioAsset {
                    frame_limit: Some(500),
                    ..input("intro.wav")
                }),
                parameters: AudioParameters {
                    start_ms: 250,
                    looping: true,
                    loop_start: 200,
                    decode_gain: 2.,
                    gain: 0.5,
                    ..Default::default()
                },
            },
            storage,
        )
        .unwrap();
    assert_eq!(wait(&mut a, 1, t).phase, AudioPreparePhase::Ready);
    a.render(200).unwrap();
    let pcm = a.take_output();
    assert_eq!(pcm.len(), 9600);
    assert!(pcm[..2500].iter().all(|v| *v == [0.125; 2]));
    assert!(pcm[2500..].iter().all(|v| *v == [0.25; 2]));
}
#[test]
fn failed_replacement_stop_supersession_reset_and_ready_receipts_are_distinct() {
    let (mut a, p, storage) = setup();
    let t = a
        .begin_preparation(1, play(3, "tone.wav"), storage.clone())
        .unwrap();
    assert_eq!(wait(&mut a, 1, t).phase, AudioPreparePhase::Ready);
    a.release_preparation(1, t).unwrap();
    let bad = a
        .begin_preparation(1, play(3, "missing.wav"), storage.clone())
        .unwrap();
    assert_eq!(wait(&mut a, 1, bad).phase, AudioPreparePhase::Failed);
    assert!(a.is_playing(1, 3));
    a.release_preparation(1, bad).unwrap();
    let gate = Gate::block(&p);
    let t = a
        .begin_preparation(1, play(3, "tone.wav"), storage.clone())
        .unwrap();
    a.submit(
        1,
        AudioRequest::Stop {
            channel: 3,
            fade_ms: 0.,
        },
    )
    .unwrap();
    drop(gate);
    assert_eq!(wait(&mut a, 1, t).phase, AudioPreparePhase::Failed);
    assert!(!a.is_playing(1, 3));
    a.release_preparation(1, t).unwrap();
    let t = a
        .begin_preparation(1, play(3, "tone.wav"), storage)
        .unwrap();
    assert_eq!(wait(&mut a, 1, t).phase, AudioPreparePhase::Ready);
    for _ in 0..100 {
        a.render(16).unwrap();
        if !a.is_playing(1, 3) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(!a.is_playing(1, 3));
    assert_eq!(a.preparation(1, t).unwrap().phase, AudioPreparePhase::Ready);
    a.reset();
    assert!(a.preparation(1, t).is_err());
    assert!(a.take_output().is_empty());
}
#[test]
fn retained_receipts_bound_admission_and_restart_false_does_not_reopen_assets() {
    let (mut a, p, storage) = setup();
    let t = a
        .begin_preparation(1, play(9, "tone.wav"), storage.clone())
        .unwrap();
    wait(&mut a, 1, t);
    a.release_preparation(1, t).unwrap();
    let calls = storage.calls.lock().unwrap().len();
    let t = a
        .begin_preparation(
            1,
            AudioRequest::PlayAsset {
                channel: 9,
                source: input("missing.wav"),
                intro: None,
                parameters: AudioParameters {
                    restart: false,
                    ..Default::default()
                },
            },
            storage.clone(),
        )
        .unwrap();
    assert_eq!(wait(&mut a, 1, t).phase, AudioPreparePhase::Ready);
    assert_eq!(storage.calls.lock().unwrap().len(), calls);
    a.release_preparation(1, t).unwrap();
    let gate = Gate::block(&p);
    let mut tickets = vec![];
    for n in 0..16 {
        tickets.push(
            a.begin_preparation(1, play(100 + n, "tone.wav"), storage.clone())
                .unwrap(),
        );
    }
    assert!(
        a.begin_preparation(1, play(200, "tone.wav"), storage)
            .is_err()
    );
    for t in tickets {
        a.release_preparation(1, t).unwrap();
    }
    drop(gate);
}

#[test]
fn newer_preparation_wins_and_canceling_unobserved_completion_keeps_live_voice() {
    let (mut a, p, storage) = setup();
    let gate = Gate::block(&p);
    let old = a
        .begin_preparation(1, play(4, "tone.wav"), storage.clone())
        .unwrap();
    let loud = Arc::new(Storage {
        value: 16384,
        calls: Mutex::new(vec![]),
    });
    let newer = a.begin_preparation(1, play(4, "tone.wav"), loud).unwrap();
    drop(gate);
    assert_eq!(wait(&mut a, 1, old).phase, AudioPreparePhase::Failed);
    assert_eq!(wait(&mut a, 1, newer).phase, AudioPreparePhase::Ready);
    a.release_preparation(1, old).unwrap();
    a.release_preparation(1, newer).unwrap();
    let pending = a
        .begin_preparation(1, play(4, "tone.wav"), storage)
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while p.stats().outstanding > 0 {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    a.release_preparation(1, pending).unwrap();
    a.render(1).unwrap();
    assert!(a.take_output().iter().all(|s| *s == [0.5; 2]));
}

#[test]
fn saturated_shared_pool_keeps_preparation_pending_and_retryable_without_io() {
    let (mut a, p, storage) = setup();
    let permits = (0..p.stats().capacity)
        .map(|_| p.reserve().unwrap())
        .collect::<Vec<_>>();
    let canceled = a
        .begin_preparation(1, play(1, "tone.wav"), storage.clone())
        .unwrap();
    let live = a
        .begin_preparation(1, play(2, "tone.wav"), storage.clone())
        .unwrap();
    for _ in 0..3 {
        assert_eq!(
            a.preparation(1, live).unwrap().phase,
            AudioPreparePhase::Pending
        );
    }
    assert!(storage.calls.lock().unwrap().is_empty());
    a.release_preparation(1, canceled).unwrap();
    drop(permits);
    assert_eq!(wait(&mut a, 1, live).phase, AudioPreparePhase::Ready);
    assert!(!a.is_playing(1, 1));
    assert!(a.is_playing(1, 2));
}
