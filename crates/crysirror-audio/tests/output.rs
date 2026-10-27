// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Real asynchronous ownership and callback sample oracles without a device.
use crysirror_audio::pcm_output::*;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
struct Stream;
impl OutputStream for Stream {}
fn format(rate: u32, channels: u16) -> OutputFormat {
    OutputFormat {
        sample_rate: rate,
        channels,
        sample_format: "f32".into(),
    }
}
fn wait(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "output completion timeout");
        thread::yield_now();
    }
}
fn pair(rate: u32, channels: u16) -> (PcmOutput, DeviceConsumer, OutputSignals) {
    let (tx, rx) = mpsc::channel();
    let mut output =
        PcmOutput::with_backend(move |consumer: OutputConsumer, signals: OutputSignals| {
            tx.send((consumer.configure(rate, channels)?, signals))?;
            OpenedOutput::new(format(rate, channels), Stream)
        });
    output.submit(&[[0.01; 2]]).unwrap();
    let (consumer, signals) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    wait(|| output.status().phase == OutputPhase::Ready);
    output.reset();
    (output, consumer, signals)
}
#[derive(Clone)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);
impl Gate {
    fn new() -> Self {
        Self(Arc::new((Mutex::new(false), Condvar::new())))
    }
    fn block(&self) {
        let state = self
            .0
            .1
            .wait_timeout_while(self.0.0.lock().unwrap(), Duration::from_secs(5), |v| !*v)
            .unwrap();
        assert!(*state.0, "authored gate timeout");
    }
    fn release(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}
struct Release(Gate);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}
#[test]
fn lazy_open_does_not_touch_the_backend_for_silence_or_invalid_extents() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut output = PcmOutput::with_backend(move |_: OutputConsumer, _: OutputSignals| {
        observed.fetch_add(1, Ordering::SeqCst);
        OpenedOutput::new(format(48000, 2), Stream)
    });
    output.submit(&[]).unwrap();
    output.submit(&[[0.; 2]; 16]).unwrap();
    output.submit(&[[f32::NAN, f32::INFINITY]]).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(output.status().phase, OutputPhase::Idle);
    assert!(output.submit(&vec![[1.; 2]; 96001]).is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let receipt = output.begin_close();
    assert!(receipt.is_closed());
    assert!(output.submit(&[[1.; 2]]).is_err());
}
#[test]
fn opening_is_off_owner_and_drop_does_not_wait_for_a_blocked_device_open() {
    let gate = Gate::new();
    let release = Release(gate.clone());
    let (tx, rx) = mpsc::channel();
    let mut output = PcmOutput::with_backend(move |_: OutputConsumer, _: OutputSignals| {
        tx.send(thread::current().id())?;
        gate.block();
        OpenedOutput::new(format(48000, 2), Stream)
    });
    output.submit(&[[0.25; 2]]).unwrap();
    let owner = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(owner, thread::current().id());
    let receipt = output.watch();
    assert_eq!(receipt.status().phase, OutputPhase::Opening);
    output.pause();
    output.reset();
    assert!(output.poll().unwrap().paused);
    drop(output);
    assert!(!receipt.is_closed());
    assert_eq!(receipt.status().phase, OutputPhase::Closing);
    drop(release);
    wait(|| receipt.is_closed());
    assert_eq!(receipt.status().phase, OutputPhase::Closed);
}
struct GatedDrop {
    thread: thread::ThreadId,
    gate: Gate,
    tx: mpsc::Sender<thread::ThreadId>,
}
impl OutputStream for GatedDrop {}
impl Drop for GatedDrop {
    fn drop(&mut self) {
        assert_eq!(thread::current().id(), self.thread);
        self.tx.send(thread::current().id()).unwrap();
        self.gate.block();
    }
}
#[test]
fn close_receipt_waits_for_actual_worker_resource_destruction() {
    let gate = Gate::new();
    let release = Release(gate.clone());
    let (tx, rx) = mpsc::channel();
    let mut output = PcmOutput::with_backend(move |_: OutputConsumer, _: OutputSignals| {
        OpenedOutput::new(
            format(48000, 2),
            GatedDrop {
                thread: thread::current().id(),
                gate: gate.clone(),
                tx: tx.clone(),
            },
        )
    });
    output.submit(&[[0.25; 2]]).unwrap();
    wait(|| output.status().phase == OutputPhase::Ready);
    let receipt = output.begin_close();
    assert_ne!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        thread::current().id()
    );
    assert!(!receipt.is_closed());
    assert_eq!(receipt.status().phase, OutputPhase::Closing);
    drop(output);
    assert!(!receipt.is_closed());
    drop(release);
    wait(|| receipt.is_closed());
}
#[test]
fn stereo_overflow_clamping_pause_and_underrun_keep_whole_frame_order() {
    let (mut output, mut consumer, _) = pair(48000, 2);
    let data = (0..OUTPUT_CAPACITY + 8)
        .map(|i| {
            let s = i as f32 / (OUTPUT_CAPACITY + 8) as f32;
            [s, -s]
        })
        .collect::<Vec<_>>();
    output.submit(&data).unwrap();
    assert_eq!(output.status().queued_frames, OUTPUT_CAPACITY);
    assert_eq!(output.status().dropped_frames, 8);
    let mut out = vec![0.; OUTPUT_CAPACITY * 2];
    consumer.fill(&mut out, |s| s).unwrap();
    for (a, b) in out.chunks_exact(2).zip(&data[8..]) {
        assert_eq!(a, b);
    }
    output
        .submit(&[[2., -2.], [f32::NAN, f32::INFINITY]])
        .unwrap();
    let mut out = [0.; 4];
    consumer.fill(&mut out, |s| s).unwrap();
    assert_eq!(out, [1., -1., 0., 0.]);
    output.submit(&[[0.25, -0.5]]).unwrap();
    output.pause();
    consumer.fill(&mut out, |s| s).unwrap();
    assert_eq!(out, [0.; 4]);
    assert_eq!(output.status().queued_frames, 1);
    assert_eq!(output.status().underrun_frames, 0);
    output.resume();
    consumer.fill(&mut out, |s| s).unwrap();
    assert_eq!(out, [0.25, -0.5, 0., 0.]);
    assert_eq!(output.status().underrun_frames, 1);
}
#[test]
fn reset_discards_cached_resampling_frames_and_retains_device_channel_alignment() {
    let (mut output, mut consumer, _) = pair(96000, 2);
    output.submit(&[[1., -1.]; 8]).unwrap();
    let mut out = [0.; 2];
    consumer.fill(&mut out, |s| s).unwrap();
    assert_eq!(out, [1., -1.]);
    output.reset();
    output.submit(&[[0.25, -0.5]; 8]).unwrap();
    consumer.fill(&mut out, |s| s).unwrap();
    assert_eq!(out, [0.25, -0.5]);
    assert!(output.status().reset_discarded_frames > 1);
    let mut invalid = [1.; 3];
    assert!(consumer.fill(&mut invalid, |s| s).is_err());
    assert_eq!(invalid, [0.; 3]);
    consumer.fill(&mut out, |s| s).unwrap();
    assert_eq!(out, [0.25, -0.5]);
}
#[test]
fn device_rate_conversion_matches_a_continuous_linear_signal_and_callback_partitions() {
    for rate in [24000, 44100, 96000] {
        let (mut output, mut consumer, _) = pair(rate, 2);
        let input = (0..481)
            .map(|i| [i as f32 / 480., -(i as f32) / 480.])
            .collect::<Vec<_>>();
        output.submit(&input).unwrap();
        let frames = rate as usize / 100;
        let mut out = vec![0.; frames * 2];
        for block in out.chunks_mut(14) {
            consumer.fill(block, |s| s).unwrap();
        }
        for (i, v) in out.chunks_exact(2).enumerate() {
            let expected = i as f32 * 48000. / rate as f32 / 480.;
            assert!((v[0] - expected).abs() < 2e-6);
            assert!((v[1] + expected).abs() < 2e-6);
        }
        assert_eq!(output.status().underrun_frames, 0);
    }
}
#[test]
fn mono_and_surround_mapping_use_explicit_channels_without_replication() {
    for channels in [1, 6] {
        let (mut output, mut consumer, _) = pair(48000, channels);
        output.submit(&[[0.25, -0.5]]).unwrap();
        let mut out = vec![1.; channels as usize];
        consumer.fill(&mut out, |s| s).unwrap();
        if channels == 1 {
            assert_eq!(out, [-0.125]);
        } else {
            assert_eq!(out, [0.25, -0.5, 0., 0., 0., 0.]);
        }
    }
}
#[test]
fn opening_failure_is_polled_and_explicit_retry_uses_a_new_session_without_old_pcm() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let (tx, rx) = mpsc::channel();
    let mut output = PcmOutput::with_backend(move |consumer: OutputConsumer, _: OutputSignals| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            anyhow::bail!("authored missing device")
        };
        tx.send(consumer.configure(48000, 2)?)?;
        OpenedOutput::new(format(48000, 2), Stream)
    });
    output.submit(&[[0.75; 2]]).unwrap();
    let previous = output.watch();
    wait(|| previous.is_closed());
    assert!(
        output
            .poll()
            .unwrap_err()
            .to_string()
            .contains("authored missing device")
    );
    assert!(output.submit(&[]).is_err());
    output.pause();
    output.retry().unwrap();
    assert_eq!(output.status().session, 2);
    assert!(output.status().paused);
    assert_eq!(output.status().queued_frames, 0);
    assert_eq!(previous.status().phase, OutputPhase::Failed);
    output.submit(&[[0.125; 2]]).unwrap();
    let mut consumer = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    wait(|| output.status().phase == OutputPhase::Ready);
    output.resume();
    let mut out = [0.; 2];
    consumer.fill(&mut out, |s| s).unwrap();
    assert_eq!(out, [0.125; 2]);
    assert!(output.retry().is_err());
}
#[test]
fn callback_errors_and_open_panics_stay_visible_and_release_the_worker() {
    let (mut output, mut consumer, signals) = pair(48000, 2);
    output.submit(&[[1.; 2]]).unwrap();
    let message = "デバイス故障".repeat(200);
    signals.fail(&message);
    assert!(output.poll().is_err());
    let mut out = [1.; 2];
    consumer.fill(&mut out, |s| s).unwrap();
    assert_eq!(out, [0.; 2]);
    let receipt = output.watch();
    wait(|| receipt.is_closed());
    let status = receipt.status();
    assert_eq!(status.phase, OutputPhase::Failed);
    assert!(status.error.unwrap().len() <= 512);
    let mut failed = PcmOutput::with_backend(|_: OutputConsumer, _: OutputSignals| {
        panic!("authored opening panic")
    });
    failed.submit(&[[0.125; 2]]).unwrap();
    wait(|| failed.watch().is_closed());
    assert!(failed.poll().unwrap_err().to_string().contains("panicked"));
}
#[test]
fn concurrent_overflow_and_resets_never_tear_a_stereo_frame() {
    let (mut output, mut consumer, _) = pair(48000, 2);
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let finished = done.clone();
    let callback = thread::spawn(move || {
        let mut out = [0.; 128];
        while !finished.load(Ordering::Acquire) {
            consumer.fill(&mut out, |s| s).unwrap();
            for frame in out.chunks_exact(2) {
                assert_eq!(frame[0], -frame[1]);
            }
        }
    });
    for i in 0..10000 {
        let s = (i % 100) as f32 / 100.;
        output.submit(&[[s, -s]; 7]).unwrap();
        if i % 71 == 0 {
            output.reset();
        }
        assert!(output.status().queued_frames <= OUTPUT_CAPACITY);
    }
    done.store(true, Ordering::Release);
    callback.join().unwrap();
    output.begin_close();
}

#[test]
fn closing_during_stream_failure_preserves_the_callback_error_after_cleanup() {
    let gate = Gate::new();
    let release = Release(gate.clone());
    let (tx, rx) = mpsc::channel();
    let (sig, signals) = mpsc::channel();
    let mut output = PcmOutput::with_backend(move |_: OutputConsumer, signal: OutputSignals| {
        sig.send(signal)?;
        OpenedOutput::new(
            format(48000, 2),
            GatedDrop {
                thread: thread::current().id(),
                gate: gate.clone(),
                tx: tx.clone(),
            },
        )
    });
    output.submit(&[[0.25; 2]]).unwrap();
    wait(|| output.status().phase == OutputPhase::Ready);
    signals
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .fail("authored callback failure during close");
    let receipt = output.begin_close();
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!receipt.is_closed());
    drop(release);
    wait(|| receipt.is_closed());
    assert_eq!(receipt.status().phase, OutputPhase::Failed);
    assert_eq!(
        receipt.status().error.as_deref(),
        Some("authored callback failure during close")
    );
}
