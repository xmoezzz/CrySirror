// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Resource ownership and sample oracles without any native VM or device.
use crysirror_audio::{channels::ChannelAudio, resources::AudioResources};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{
        AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, AudioResourceHandle,
        ClientStorage,
    },
};
use crysirror_tasks::TaskPool;
use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
fn wave() -> Vec<u8> {
    let size = 2000u32;
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
    for i in 0..1000i16 {
        b.extend((i * 20).to_le_bytes());
    }
    b
}
#[derive(Default)]
struct Storage {
    data: Mutex<Vec<u8>>,
    calls: Mutex<Vec<thread::ThreadId>>,
}
impl Storage {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            data: Mutex::new(wave()),
            ..Default::default()
        })
    }
}
impl ClientStorage for Storage {
    fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        self.calls.lock().unwrap().push(thread::current().id());
        Ok(self.data.lock().unwrap().clone())
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
fn asset(limit: Option<u64>) -> AudioAsset {
    AudioAsset {
        path: VfsPath::new(Mount::Asset, "tone.wav").unwrap(),
        frame_limit: limit,
    }
}
fn ready(bank: &mut AudioResources, client: u32, t: u64) -> AudioResourceHandle {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let s = bank.preparation(client, t).unwrap();
        if s.phase != AudioPreparePhase::Pending {
            assert_eq!(s.phase, AudioPreparePhase::Ready, "{:?}", s.error);
            return s.resource.unwrap();
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
}
fn prepare(bank: &mut AudioResources, storage: Arc<Storage>) -> AudioResourceHandle {
    let t = bank
        .begin(1, asset(Some(61)), true, storage, None, false)
        .unwrap();
    let h = ready(bank, 1, t);
    bank.release_preparation(1, t).unwrap();
    h
}
#[test]
fn immutable_snapshot_independent_cursors_scopes_and_last_lease_charge() {
    let storage = Storage::new();
    let mut bank = AudioResources::default();
    let h = prepare(&mut bank, storage.clone());
    let resource = bank.capture(1, h).unwrap();
    assert_eq!(resource.info().source_frames, Some(61));
    assert_eq!(resource.info().encoded_bytes, wave().len() as u64);
    *storage.data.lock().unwrap() = b"deleted or replaced".to_vec();
    let mut a = resource.open().unwrap();
    let mut b = resource.open().unwrap();
    let mut out = [[0.; 2]; 2];
    a.seek(7).unwrap();
    assert_eq!(a.read(&mut out).unwrap(), 2);
    assert!((out[0][0] - 140. / 32768.).abs() < 1e-6);
    b.read(&mut out).unwrap();
    assert_eq!(out[0][0], 0.);
    assert!((out[1][0] - 20. / 32768.).abs() < 1e-6);
    assert_eq!(storage.calls.lock().unwrap().len(), 1);
    assert!(bank.capture(2, h).is_err());
    assert!(bank.release(2, h).is_err());
    bank.release(1, h).unwrap();
    assert!(bank.capture(1, h).is_err());
    assert!(bank.resident_bytes() > 0);
    bank.clear();
    assert!(bank.resident_bytes() > 0);
    drop(resource);
    drop(a);
    assert!(bank.resident_bytes() > 0);
    drop(b);
    assert_eq!(bank.resident_bytes(), 0);
}
#[test]
fn worker_reads_and_canceling_an_unobserved_ready_result_do_not_publish() {
    let pool = TaskPool::new("resource-test", 1, 8).unwrap();
    let storage = Storage::new();
    let mut bank = AudioResources::default();
    let (start_tx, start_rx) = std::sync::mpsc::channel();
    let initial_blocker = pool
        .spawn(move |_| {
            start_rx.recv_timeout(Duration::from_secs(5))?;
            Ok(())
        })
        .unwrap();
    let t = bank
        .begin(
            1,
            asset(None),
            true,
            storage.clone(),
            Some(pool.clone()),
            true,
        )
        .unwrap();
    assert_eq!(
        bank.preparation(1, t).unwrap().phase,
        AudioPreparePhase::Pending
    );
    start_tx.send(()).unwrap();
    initial_blocker.wait().unwrap();
    // A FIFO sentinel completes after preparation, without a publication poll.
    pool.spawn(|_| Ok(())).unwrap().wait().unwrap();
    assert_eq!(bank.resource_count(), 0);
    assert!(bank.resident_bytes() > 0);
    assert!(bank.preparation(2, t).is_err());
    assert!(
        storage
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|id| *id != thread::current().id())
    );
    bank.release_preparation(1, t).unwrap();
    assert_eq!(bank.resource_count(), 0);
    assert_eq!(bank.resident_bytes(), 0);
    assert!(bank.preparation(1, t).is_err());
    let (tx, rx) = std::sync::mpsc::channel();
    let blocker = pool
        .spawn(move |_| {
            rx.recv_timeout(Duration::from_secs(5))?;
            Ok(())
        })
        .unwrap();
    let t = bank
        .begin(
            1,
            asset(None),
            true,
            storage.clone(),
            Some(pool.clone()),
            true,
        )
        .unwrap();
    bank.release_preparation(1, t).unwrap();
    tx.send(()).unwrap();
    blocker.wait().unwrap();
    pool.spawn(|_| Ok(())).unwrap().wait().unwrap();
    assert_eq!(storage.calls.lock().unwrap().len(), 1);
    assert_eq!(bank.resident_bytes(), 0);
}
#[test]
fn bounded_handles_failed_probe_cleanup_and_clear_prevent_stale_reuse() {
    let storage = Storage::new();
    let mut bank = AudioResources::default();
    let mut handles = vec![];
    for _ in 0..32 {
        handles.push(prepare(&mut bank, storage.clone()));
    }
    assert!(
        bank.begin(1, asset(None), false, storage.clone(), None, false)
            .is_err()
    );
    let old = handles[0];
    bank.release(1, old).unwrap();
    let new = prepare(&mut bank, storage.clone());
    assert_ne!(old.id, new.id);
    bank.clear();
    assert!(bank.capture(1, new).is_err());
    assert_eq!(bank.resident_bytes(), 0);
    *storage.data.lock().unwrap() = b"broken".to_vec();
    let pool = TaskPool::new("resource-failure", 1, 8).unwrap();
    let t = bank
        .begin(1, asset(None), false, storage, Some(pool), true)
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let s = bank.preparation(1, t).unwrap();
        if s.phase != AudioPreparePhase::Pending {
            assert_eq!(s.phase, AudioPreparePhase::Failed);
            assert!(s.error.is_some());
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(bank.resource_count(), 0);
    assert_eq!(bank.resident_bytes(), 0);
    bank.release_preparation(1, t).unwrap();
}
#[test]
fn native_channel_admission_captures_resource_before_release_and_replays_original_samples() {
    let storage = Storage::new();
    let mut audio = ChannelAudio::default();
    let h = prepare(&mut audio.resources, storage.clone());
    let pool = TaskPool::new("resource-play", 1, 16).unwrap();
    audio.configure_preparation(pool, true);
    let t = audio
        .begin_preparation(
            1,
            AudioRequest::PlayResource {
                channel: 3,
                resource: h,
                parameters: AudioParameters {
                    start_frame: Some(7),
                    paused: true,
                    clamp_range: true,
                    ..Default::default()
                },
            },
            storage.clone(),
        )
        .unwrap();
    audio.resources.release(1, h).unwrap();
    *storage.data.lock().unwrap() = vec![];
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let s = audio.preparation(1, t).unwrap();
        if s.phase != AudioPreparePhase::Pending {
            assert_eq!(s.phase, AudioPreparePhase::Ready, "{:?}", s.error);
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
    audio.release_preparation(1, t).unwrap();
    assert_eq!(
        audio.playback_info(1, 3).unwrap().unwrap().position_frames,
        7
    );
    audio
        .submit(
            1,
            AudioRequest::Resume {
                channel: 3,
                delay_ms: 0.,
                fade_ms: 0.,
            },
        )
        .unwrap();
    audio.render(1).unwrap();
    let out = audio.take_output();
    assert!((out[0][0] - 140. / 32768.).abs() < 1e-6);
    assert_eq!(storage.calls.lock().unwrap().len(), 1);
    audio.reset();
    let end = Instant::now() + Duration::from_secs(5);
    while audio.resources.resident_bytes() > 0 {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn compressed_resources_reopen_independent_decoders_after_worker_metadata_scan() {
    for bytes in [
        include_bytes!("fixtures/tone.ogg").as_slice(),
        include_bytes!("fixtures/tone.mp3").as_slice(),
        include_bytes!("fixtures/tone.flac").as_slice(),
    ] {
        let storage = Storage::new();
        *storage.data.lock().unwrap() = bytes.to_vec();
        let mut bank = AudioResources::default();
        let t = bank
            .begin(1, asset(None), true, storage.clone(), None, false)
            .unwrap();
        let h = ready(&mut bank, 1, t);
        bank.release_preparation(1, t).unwrap();
        let sound = bank.capture(1, h).unwrap();
        assert!(sound.info().source_frames.unwrap() > 0);
        let mut a = sound.open().unwrap();
        let mut b = sound.open().unwrap();
        assert_eq!(a.frame_count(), sound.info().source_frames);
        let mut first = [[0.; 2]; 32];
        let mut second = [[0.; 2]; 32];
        assert_eq!(a.read(&mut first).unwrap(), 32);
        assert_eq!(b.read(&mut second).unwrap(), 32);
        assert_eq!(first, second);
        assert_eq!(storage.calls.lock().unwrap().len(), 1);
    }
}

#[test]
fn transformed_resource_bakes_pcm_on_worker_and_retains_scoped_immutable_audio() {
    use crysirror_core::service::AudioResourceTransform::SiglusJitan;
    let storage = Storage::new();
    let pcm = (0..8000usize)
        .map(|i| (((i * 7919 + 12345) % 60001) as i32 - 30000) as i16)
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();
    *storage.data.lock().unwrap() = crysirror_resource::WaveSpec {
        sample_rate: 8000,
        ..Default::default()
    }
    .encode(&pcm)
    .unwrap();
    let pool = TaskPool::new("transform-resource", 1, 8).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let blocker = pool
        .spawn(move |_| {
            rx.recv_timeout(Duration::from_secs(5))?;
            Ok(())
        })
        .unwrap();
    let mut bank = AudioResources::default();
    let t = bank
        .begin_transformed(
            1,
            asset(None),
            false,
            Some(SiglusJitan { percent: 200 }),
            storage.clone(),
            Some(pool.clone()),
            true,
        )
        .unwrap();
    assert_eq!(
        bank.preparation(1, t).unwrap().phase,
        AudioPreparePhase::Pending
    );
    assert_eq!(bank.resource_count(), 0);
    tx.send(()).unwrap();
    blocker.wait().unwrap();
    let h = ready(&mut bank, 1, t);
    bank.release_preparation(1, t).unwrap();
    assert!(bank.capture(2, h).is_err());
    let sound = bank.capture(1, h).unwrap();
    assert_eq!(sound.info().source_frames, Some(4130));
    assert_eq!(sound.info().source_rate, 8000);
    assert_eq!(sound.info().encoded_bytes, 46 + 4130 * 2);
    *storage.data.lock().unwrap() = vec![];
    let mut a = sound.open().unwrap();
    let mut b = sound.open().unwrap();
    let mut out = vec![[0.; 2]; 4130];
    assert_eq!(a.read(&mut out).unwrap(), 4130);
    let hash = out
        .iter()
        .flat_map(|s| ((s[0] * 32768.) as i16).to_le_bytes())
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
    assert_eq!(hash, 0x7422c527b9b4de3d);
    let mut first = [[0.; 2]; 1];
    b.read(&mut first).unwrap();
    assert_eq!(first[0], out[0]);
    assert!(out.iter().all(|s| s[0] == s[1]));
    assert!(
        storage
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|id| *id != thread::current().id())
    );
    bank.release(1, h).unwrap();
    bank.clear();
    assert!(bank.resident_bytes() > 0);
    drop(a);
    drop(b);
    drop(sound);
    assert_eq!(bank.resident_bytes(), 0);
}

#[test]
fn transform_identity_input_limit_and_canceled_worker_keep_resource_boundaries() {
    use crysirror_core::service::AudioResourceTransform::SiglusJitan;
    let storage = Storage::new();
    let mut bank = AudioResources::default();
    assert!(
        bank.begin_transformed(
            1,
            asset(None),
            true,
            Some(SiglusJitan { percent: 99 }),
            storage.clone(),
            None,
            false
        )
        .is_err()
    );
    assert_eq!(bank.resident_bytes(), 0);
    let t = bank
        .begin_transformed(
            1,
            asset(Some(61)),
            true,
            Some(SiglusJitan { percent: 100 }),
            storage.clone(),
            None,
            false,
        )
        .unwrap();
    let h = ready(&mut bank, 1, t);
    bank.release_preparation(1, t).unwrap();
    let original = bank.capture(1, h).unwrap();
    assert_eq!(original.info().encoded_bytes, wave().len() as u64);
    let mut first = [[0.; 2]; 2];
    original.open().unwrap().read(&mut first).unwrap();
    assert!((first[1][0] - 20. / 32768.).abs() < 1e-6);
    let t = bank
        .begin_transformed(
            1,
            asset(Some(61)),
            false,
            Some(SiglusJitan { percent: 200 }),
            storage.clone(),
            None,
            false,
        )
        .unwrap();
    let h2 = ready(&mut bank, 1, t);
    bank.release_preparation(1, t).unwrap();
    let transformed = bank.capture(1, h2).unwrap();
    assert_eq!(transformed.info().source_frames, Some(61));
    let mut out = [[1.; 2]; 62];
    assert_eq!(transformed.open().unwrap().read(&mut out).unwrap(), 61);
    assert!(out[..61].iter().flatten().all(|s| *s == 0.));
    drop(original);
    drop(transformed);
    bank.clear();
    assert_eq!(bank.resident_bytes(), 0);
    let pool = TaskPool::new("cancel-transform", 1, 8).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let blocker = pool
        .spawn(move |_| {
            rx.recv_timeout(Duration::from_secs(5))?;
            Ok(())
        })
        .unwrap();
    let count = storage.calls.lock().unwrap().len();
    let t = bank
        .begin_transformed(
            1,
            asset(None),
            false,
            Some(SiglusJitan { percent: 200 }),
            storage.clone(),
            Some(pool.clone()),
            true,
        )
        .unwrap();
    bank.release_preparation(1, t).unwrap();
    tx.send(()).unwrap();
    blocker.wait().unwrap();
    pool.spawn(|_| Ok(())).unwrap().wait().unwrap();
    assert_eq!(storage.calls.lock().unwrap().len(), count);
    assert_eq!(bank.resident_bytes(), 0);
    assert_eq!(bank.resource_count(), 0);
}
