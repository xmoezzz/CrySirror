// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Analytic PCM and worker gates; no device or game assets.
use anyhow::{Result, ensure};
use crysirror_audio::{
    buffered::{BufferedSource, StreamBufferSpec},
    decoder::{AudioSource, Clip, SourceRead, StereoFrame},
    mixer::{LoopPoints, Mixer, MixerConfig, VoiceHandle, VoiceSpec},
};
use crysirror_tasks::TaskPool;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, ThreadId},
    time::{Duration, Instant},
};
#[derive(Default)]
struct Probe {
    calls: Mutex<Vec<ThreadId>>,
    seeks: Mutex<Vec<u64>>,
    block: AtomicBool,
    gate: (Mutex<bool>, Condvar),
    entered: AtomicBool,
}
impl Probe {
    fn open(&self) {
        *self.gate.0.lock().unwrap() = true;
        self.gate.1.notify_all();
    }
}
struct Release(Arc<Probe>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.open();
    }
}
struct Ramp {
    at: usize,
    total: usize,
    probe: Arc<Probe>,
}
fn sample(i: usize) -> StereoFrame {
    [i as f32 / 10000.; 2]
}
impl AudioSource for Ramp {
    fn sample_rate(&self) -> u32 {
        8000
    }
    fn frame_count(&self) -> Option<u64> {
        Some(self.total as u64)
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        self.probe
            .calls
            .lock()
            .unwrap()
            .push(thread::current().id());
        if self.at >= 512 && self.probe.block.load(Ordering::Acquire) {
            self.probe.entered.store(true, Ordering::Release);
            let (released, _) = self
                .probe
                .gate
                .1
                .wait_timeout_while(
                    self.probe.gate.0.lock().unwrap(),
                    Duration::from_secs(5),
                    |v| !*v,
                )
                .unwrap();
            ensure!(*released, "fixture gate timeout");
        }
        // Short positive reads do not mean EOF.
        let n = out.len().min(17).min(self.total.saturating_sub(self.at));
        for (i, v) in out[..n].iter_mut().enumerate() {
            *v = sample(self.at + i);
        }
        self.at += n;
        Ok(n)
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        self.probe
            .calls
            .lock()
            .unwrap()
            .push(thread::current().id());
        self.probe.seeks.lock().unwrap().push(frame);
        ensure!(frame <= self.total as u64, "outside source");
        self.at = frame as usize;
        Ok(())
    }
}
fn prepared(
    pool: &TaskPool,
    probe: Arc<Probe>,
    total: usize,
    loop_start: Option<u64>,
) -> BufferedSource {
    let p = pool.clone();
    pool.spawn(move |_| {
        BufferedSource::prepare(
            Box::new(Ramp {
                at: 0,
                total,
                probe,
            }),
            p,
            StreamBufferSpec {
                chunk_frames: 256,
                prefetch_chunks: 2,
            },
            loop_start,
        )
    })
    .unwrap()
    .wait()
    .unwrap()
}
fn mixer() -> Mixer {
    Mixer::new(MixerConfig {
        sample_rate: 8000,
        max_voices: 4,
        seed: 1,
    })
    .unwrap()
}
fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "worker timeout");
        thread::sleep(Duration::from_millis(1));
    }
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 2e-6, "{a} != {b}");
}
fn ready(m: &mut Mixer, h: VoiceHandle, n: usize) {
    until(|| {
        m.service_sources().unwrap();
        let s = m.source_status(1, h).unwrap().unwrap();
        assert!(s.error.is_none(), "{:?}", s.error);
        s.ready_frames >= n
    });
}
#[test]
fn blocked_decoder_does_not_block_other_voices_or_advance_starved_clock() {
    let pool = TaskPool::new("audio-gate", 1, 2).unwrap();
    let probe = Arc::new(Probe::default());
    let release = Release(probe.clone());
    let source = prepared(&pool, probe.clone(), 4096, None);
    probe.block.store(true, Ordering::Release);
    let mut m = mixer();
    let h = m.play(1, Box::new(source), VoiceSpec::default()).unwrap();
    m.play(
        2,
        Clip::new(8000, vec![[0.25; 2]; 4096]).unwrap().source(),
        VoiceSpec::default(),
    )
    .unwrap();
    let mut out = vec![[0.; 2]; 512];
    m.render(&mut out).unwrap();
    for (i, s) in out.iter().enumerate() {
        near(s[0], 0.25 + sample(i)[0]);
    }
    until(|| probe.entered.load(Ordering::Acquire));
    let before = m.position(1, h).unwrap();
    let mut stalled = vec![[0.; 2]; 128];
    m.render(&mut stalled).unwrap();
    assert!(stalled.iter().all(|s| *s == [0.25; 2]));
    assert_eq!(m.position(1, h).unwrap(), before);
    assert_eq!(m.stats().underrun_frames, 128);
    drop(release);
    ready(&mut m, h, 256);
    let mut out = [[0.; 2]; 1];
    m.render(&mut out).unwrap();
    near(out[0][0], 0.25 + sample(512)[0]);
    assert!(
        probe
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|id| *id != thread::current().id())
    );
}
#[test]
fn interpolation_across_chunks_matches_resident_pcm_without_backward_io() {
    let pool = TaskPool::new("audio-rate", 1, 4).unwrap();
    let probe = Arc::new(Probe::default());
    let source = prepared(&pool, probe.clone(), 8192, None);
    let mut m = mixer();
    let spec = VoiceSpec {
        rate: 0.75,
        ..VoiceSpec::default()
    };
    let h = m.play(1, Box::new(source), spec.clone()).unwrap();
    let mut oracle = mixer();
    oracle
        .play(
            1,
            Clip::new(8000, (0..8192).map(sample).collect())
                .unwrap()
                .source(),
            spec,
        )
        .unwrap();
    for _ in 0..40 {
        ready(&mut m, h, 256);
        let mut a = [[0.; 2]; 64];
        let mut b = a;
        m.render(&mut a).unwrap();
        oracle.render(&mut b).unwrap();
        for (a, b) in a.iter().zip(b) {
            near(a[0], b[0]);
        }
    }
    assert_eq!(*probe.seeks.lock().unwrap(), vec![0]);
    assert_eq!(m.stats().underrun_frames, 0);
}
#[test]
fn custom_loop_cache_and_fractional_rate_match_resident_pcm() {
    let pool = TaskPool::new("audio-loop", 1, 4).unwrap();
    let probe = Arc::new(Probe::default());
    let source = prepared(&pool, probe.clone(), 700, Some(530));
    let mut m = mixer();
    let spec = VoiceSpec {
        rate: 0.75,
        looping: true,
        loop_points: Some(LoopPoints {
            start: 530,
            end: 670,
        }),
        ..VoiceSpec::default()
    };
    let h = m.play(1, Box::new(source), spec.clone()).unwrap();
    let mut oracle = mixer();
    oracle
        .play(
            1,
            Clip::new(8000, (0..700).map(sample).collect())
                .unwrap()
                .source(),
            spec,
        )
        .unwrap();
    for block in 0..40 {
        if block < 10 {
            ready(&mut m, h, 1);
        }
        m.service_sources().unwrap();
        let mut a = [[0.; 2]; 64];
        let mut b = a;
        m.render(&mut a).unwrap();
        oracle.render(&mut b).unwrap();
        for (a, b) in a.iter().zip(b) {
            near(a[0], b[0]);
        }
    }
    assert_eq!(m.stats().underrun_frames, 0);
}
#[test]
fn backpressure_retains_decoder_and_uncached_seek_supersedes_old_chunks() {
    let pool = TaskPool::new("audio-seek", 1, 1).unwrap();
    let probe = Arc::new(Probe::default());
    let mut source = prepared(&pool, probe, 4096, None);
    let permit = pool.reserve().unwrap();
    let mut out = vec![[0.; 2]; 512];
    assert_eq!(source.poll_read(&mut out).unwrap(), SourceRead::Frames(256));
    assert_eq!(source.poll_read(&mut out).unwrap(), SourceRead::Frames(256));
    source.service().unwrap();
    assert_eq!(source.poll_read(&mut out).unwrap(), SourceRead::Pending);
    assert!(!source.poll_seek(1200).unwrap());
    source.service().unwrap();
    assert!(!source.poll_seek(2400).unwrap());
    drop(permit);
    until(|| {
        source.service().unwrap();
        source.poll_seek(2400).unwrap()
    });
    assert_eq!(
        source.poll_read(&mut out[..1]).unwrap(),
        SourceRead::Frames(1)
    );
    near(out[0][0], sample(2400)[0]);
    assert!(source.seek(0).is_err()); // No fake synchronous completion.
}
#[test]
fn pending_seek_freezes_and_cancel_restores_fractional_clock() {
    let pool = TaskPool::new("audio-cancel", 1, 2).unwrap();
    let probe = Arc::new(Probe::default());
    let release = Release(probe.clone());
    let source = prepared(&pool, probe.clone(), 4096, None);
    let mut m = mixer();
    let h = m
        .play(
            1,
            Box::new(source),
            VoiceSpec {
                rate: 0.75,
                ..VoiceSpec::default()
            },
        )
        .unwrap();
    let mut out = [[0.; 2]; 3];
    m.render(&mut out).unwrap();
    let old = m.position(1, h).unwrap();
    probe.block.store(true, Ordering::Release);
    let seek = m.begin_seek(1, h, 2000).unwrap();
    m.service_sources().unwrap();
    until(|| probe.entered.load(Ordering::Acquire));
    m.render(&mut out).unwrap();
    assert!(out.iter().flatten().all(|s| *s == 0.));
    assert_eq!(m.position(1, h).unwrap(), old);
    m.cancel_seek(1, h, seek).unwrap();
    drop(release);
    until(|| {
        m.service_sources().unwrap();
        m.source_status(1, h).unwrap().unwrap().worker_pending == false
    });
    m.render(&mut out[..1]).unwrap();
    near(out[0][0], 2.25 / 10000.);
    assert!(m.seek_progress(1, h, seek).is_err());
}

#[test]
fn pinned_loop_head_is_ready_during_pool_backpressure_without_queue_overflow() {
    let pool = TaskPool::new("audio-loop-cache", 1, 1).unwrap();
    let probe = Arc::new(Probe::default());
    let mut source = prepared(&pool, probe, 4096, Some(1500));
    let permit = pool.reserve().unwrap();
    assert!(source.poll_seek(1503).unwrap());
    source.service().unwrap();
    assert!(!source.buffer_status().unwrap().worker_pending);
    let mut out = [[0.; 2]; 1];
    assert_eq!(source.poll_read(&mut out).unwrap(), SourceRead::Frames(1));
    near(out[0][0], sample(1503)[0]);
    drop(permit);
    source.service().unwrap();
    assert!(!source.buffer_status().unwrap().worker_pending); // Two pinned chunks already fill the configured queue.
}

#[test]
fn worker_decode_failure_is_visible_after_queued_pcm_without_false_eof() {
    struct Broken {
        at: usize,
    }
    impl AudioSource for Broken {
        fn sample_rate(&self) -> u32 {
            8000
        }
        fn frame_count(&self) -> Option<u64> {
            Some(2048)
        }
        fn seek(&mut self, frame: u64) -> Result<()> {
            self.at = frame as usize;
            Ok(())
        }
        fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
            ensure!(self.at < 512, "authored packet failure");
            let n = out.len().min(512 - self.at);
            out[..n].fill([0.5; 2]);
            self.at += n;
            Ok(n)
        }
    }
    let pool = TaskPool::new("audio-failure", 1, 2).unwrap();
    let p = pool.clone();
    let mut source = pool
        .spawn(move |_| {
            BufferedSource::prepare(
                Box::new(Broken { at: 0 }),
                p,
                StreamBufferSpec {
                    chunk_frames: 256,
                    prefetch_chunks: 3,
                },
                None,
            )
        })
        .unwrap()
        .wait()
        .unwrap();
    until(|| {
        source.service().unwrap();
        source.buffer_status().unwrap().error.is_some()
    });
    let mut out = [[0.; 2]; 256];
    assert_eq!(source.poll_read(&mut out).unwrap(), SourceRead::Frames(256));
    assert_eq!(source.poll_read(&mut out).unwrap(), SourceRead::Frames(256));
    assert!(out.iter().all(|s| *s == [0.5; 2]));
    assert!(
        format!("{:#}", source.poll_read(&mut out).unwrap_err())
            .contains("authored packet failure")
    );
}
