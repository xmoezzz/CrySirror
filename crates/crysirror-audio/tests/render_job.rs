// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Offline sample oracles and explicit worker ownership; no VM or devices.
use crysirror_audio::{
    decoder::{AudioSource, Clip, StereoFrame},
    mixer::{Mixer, MixerConfig, VoiceSpec},
    render_job::{MixFrame, MixJob},
};
use crysirror_tasks::TaskPool;
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
fn mixer() -> Mixer {
    Mixer::new(MixerConfig {
        sample_rate: 8000,
        ..Default::default()
    })
    .unwrap()
}
fn source() -> Box<dyn AudioSource> {
    Clip::new(
        8000,
        (0..32000)
            .map(|i| [(i as f32 * 0.07).sin() * 0.25; 2])
            .collect(),
    )
    .unwrap()
    .source()
}
fn collect(job: &mut MixJob, mixers: &mut [&mut Mixer]) -> anyhow::Result<Vec<StereoFrame>> {
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(p) = job.poll(mixers)? {
            return Ok(p);
        }
        assert!(Instant::now() < limit, "worker completion timeout");
        thread::yield_now();
    }
}
fn frame(frame: &mut MixFrame, mixers: &mut [&mut Mixer]) -> Vec<StereoFrame> {
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(p) = frame.poll(mixers).unwrap() {
            return p;
        }
        assert!(Instant::now() < limit);
        thread::yield_now();
    }
}
#[test]
fn paired_worker_pcm_and_published_transport_match_offline_samples() {
    let pool = TaskPool::new("mix-test", 1, 4).unwrap();
    let mut a = mixer();
    let mut b = mixer();
    let mut reference = mixer();
    let spec = VoiceSpec {
        gain: 0.4,
        fade_in_ms: 20.,
        ..Default::default()
    };
    let h = a.play(1, source(), spec.clone()).unwrap();
    reference.play(1, source(), spec).unwrap();
    b.play(
        1,
        source(),
        VoiceSpec {
            gain: 0.6,
            fade_in_ms: 20.,
            ..Default::default()
        },
    )
    .unwrap();
    let mut expected = vec![[0.; 2]; 1024];
    reference.render(&mut expected).unwrap();
    for p in &mut expected {
        p[0] = p[0] / 0.4 + 0.125;
        p[1] = p[1] / 0.4 - 0.25;
    }
    let mut job = MixJob::begin(
        &pool,
        &mut [&mut a, &mut b],
        1024,
        vec![[0.125, -0.25]; 1024],
    )
    .unwrap();
    assert!(a.is_rendering());
    assert_eq!(a.playback_info(1, h).unwrap().age_frames, 0);
    assert!(a.set_gain(1, h, 1.).is_err());
    assert!(a.render(&mut [[0.; 2]; 1]).is_err());
    assert_eq!(a.stats().active, 1);
    assert!(!a.is_playing(2, h));
    let pcm = collect(&mut job, &mut [&mut a, &mut b]).unwrap();
    for (actual, expected) in pcm.iter().flatten().zip(expected.iter().flatten()) {
        assert!((actual - expected).abs() < 2e-7);
    }
    assert!(!a.is_rendering());
    assert_eq!(a.playback_info(1, h).unwrap().age_frames, 1024);
    assert!(job.poll(&mut [&mut a, &mut b]).is_err());
}
#[test]
fn prepared_frame_retries_capacity_without_losing_pcm_or_advancing_dsp() {
    let pool = TaskPool::new("mix-full", 1, 1).unwrap();
    let permit = pool.reserve().unwrap();
    let mut a = mixer();
    let h = a.play(1, source(), VoiceSpec::default()).unwrap();
    let mut f = MixFrame::new(pool.clone(), vec![32], vec![[0.125; 2]; 40]).unwrap();
    for _ in 0..3 {
        assert!(f.poll(&mut [&mut a]).unwrap().is_none());
        assert!(!a.is_rendering());
        assert_eq!(a.playback_info(1, h).unwrap().age_frames, 0);
    }
    drop(permit);
    let pcm = frame(&mut f, &mut [&mut a]);
    assert_eq!(pcm.len(), 40);
    assert_eq!(&pcm[32..], &[[0.125; 2]; 8]);
    assert_eq!(a.playback_info(1, h).unwrap().age_frames, 32);
}
#[test]
fn per_mixer_counts_and_deferred_client_controls_have_one_boundary() {
    let pool = TaskPool::new("mix-counts", 1, 2).unwrap();
    let (tx, rx) = mpsc::channel();
    let (started, ready) = mpsc::channel();
    let gate = pool
        .spawn(move |_| {
            started.send(()).unwrap();
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        })
        .unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut a = mixer();
    let mut b = mixer();
    let h = a.play(1, source(), VoiceSpec::default()).unwrap();
    let k = b.play(2, source(), VoiceSpec::default()).unwrap();
    let mut f = MixFrame::new(pool, vec![16, 8], vec![]).unwrap();
    assert!(f.poll(&mut [&mut a, &mut b]).unwrap().is_none());
    a.pause_client(1, true);
    b.stop_client(2);
    tx.send(()).unwrap();
    gate.wait().unwrap();
    frame(&mut f, &mut [&mut a, &mut b]);
    assert_eq!(a.playback_info(1, h).unwrap().age_frames, 16);
    assert!(a.playback_info(1, h).unwrap().paused);
    assert!(!b.is_playing(2, k));
}
#[test]
fn reset_and_late_completion_cannot_resurrect_a_voice_or_replace_new_generation() {
    let pool = TaskPool::new("mix-reset", 1, 2).unwrap();
    let mut a = mixer();
    let old = a.play(1, source(), VoiceSpec::default()).unwrap();
    let mut job = MixJob::begin(&pool, &mut [&mut a], 128, vec![]).unwrap();
    a.clear();
    let new = a.play(1, source(), VoiceSpec::default()).unwrap();
    assert_ne!(old, new);
    assert!(job.poll(&mut [&mut a]).is_err());
    assert!(a.is_playing(1, new));
    assert!(!a.is_playing(1, old));
    assert_eq!(a.playback_info(1, new).unwrap().age_frames, 0);
}
struct Probe {
    thread: Arc<Mutex<Option<thread::ThreadId>>>,
    fail: bool,
    panic: bool,
}
impl AudioSource for Probe {
    fn sample_rate(&self) -> u32 {
        8000
    }
    fn frame_count(&self) -> Option<u64> {
        None
    }
    fn seek(&mut self, _: u64) -> anyhow::Result<()> {
        Ok(())
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> anyhow::Result<usize> {
        *self.thread.lock().unwrap() = Some(thread::current().id());
        assert!(!self.panic, "authored DSP panic");
        anyhow::ensure!(!self.fail, "authored source error");
        out.fill([0.25; 2]);
        Ok(out.len())
    }
}
#[test]
fn actual_source_reads_run_on_worker_and_failure_or_panic_releases_slots() {
    for (fail, panic) in [(false, false), (true, false), (false, true)] {
        let pool = TaskPool::new("mix-probe", 1, 2).unwrap();
        let mut a = mixer();
        let observed = Arc::new(Mutex::new(None));
        a.play(
            1,
            Box::new(Probe {
                thread: observed.clone(),
                fail,
                panic,
            }),
            VoiceSpec::default(),
        )
        .unwrap();
        assert!(observed.lock().unwrap().is_none());
        let mut job = MixJob::begin(&pool, &mut [&mut a], 64, vec![]).unwrap();
        let result = collect(&mut job, &mut [&mut a]);
        assert_ne!(observed.lock().unwrap().unwrap(), thread::current().id());
        assert!(!a.is_rendering());
        assert_eq!(result.is_err(), fail || panic);
        if fail || panic {
            assert_eq!(a.stats().active, 0);
        }
    }
}
#[test]
fn cancelling_a_queued_job_discards_state_and_never_reads_source() {
    let pool = TaskPool::new("mix-cancel", 1, 2).unwrap();
    let (tx, rx) = mpsc::channel();
    let (started, ready) = mpsc::channel();
    let gate = pool
        .spawn(move |_| {
            started.send(()).unwrap();
            rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    ready.recv().unwrap();
    let mut a = mixer();
    let observed = Arc::new(Mutex::new(None));
    a.play(
        1,
        Box::new(Probe {
            thread: observed.clone(),
            fail: false,
            panic: false,
        }),
        VoiceSpec::default(),
    )
    .unwrap();
    let mut job = MixJob::begin(&pool, &mut [&mut a], 32, vec![]).unwrap();
    assert!(job.poll(&mut [&mut a]).unwrap().is_none());
    job.cancel(&mut [&mut a]);
    assert!(!a.is_rendering());
    assert_eq!(a.stats().active, 0);
    tx.send(()).unwrap();
    gate.wait().unwrap();
    drop(pool);
    assert!(observed.lock().unwrap().is_none());
}
#[test]
fn invalid_frames_reject_before_transferring_any_dsp_state() {
    let pool = TaskPool::new("mix-invalid", 1, 2).unwrap();
    let mut a = mixer();
    assert!(MixJob::begin(&pool, &mut [&mut a], 16001, vec![]).is_err());
    assert!(!a.is_rendering());
    assert!(MixJob::begin(&pool, &mut [&mut a], 1, vec![[f32::NAN; 2]]).is_err());
    assert!(!a.is_rendering());
    assert_eq!(pool.stats().outstanding, 0);
}

#[test]
fn reverb_history_moves_between_frames_without_resetting_or_cloning_its_tail() {
    use crysirror_audio::reverb::{PreparedReverb, ReverbSettings};
    fn room() -> Mixer {
        let mut m = mixer();
        let mut settings = ReverbSettings::default();
        settings.room.room_size_metres = 10.;
        settings.room.decay_seconds = 0.3;
        m.set_reverb(1, Some(PreparedReverb::prepare(8000, settings).unwrap()))
            .unwrap();
        m.play(
            1,
            Clip::new(8000, vec![[0.5; 2]]).unwrap().source(),
            VoiceSpec::default(),
        )
        .unwrap();
        m
    }
    let mut reference = room();
    let mut expected = vec![[0.; 2]; 16000];
    reference.render(&mut expected).unwrap();
    let mut owned = room();
    let pool = TaskPool::new("mix-reverb", 1, 2).unwrap();
    let mut actual = Vec::new();
    while actual.len() < expected.len() {
        let count = (expected.len() - actual.len()).min(512);
        let mut job = MixJob::begin(&pool, &mut [&mut owned], count, vec![]).unwrap();
        assert!(owned.reverb(1).is_some());
        actual.extend(collect(&mut job, &mut [&mut owned]).unwrap());
    }
    assert_eq!(actual, expected);
    assert!(actual[800..2400].iter().flatten().any(|s| s.abs() > 1e-5));
    assert_eq!(owned.stats().active, 0);
}
