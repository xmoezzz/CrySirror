// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded PCM transfer and asynchronous device ownership. No simulation/VM DSP.
use crate::decoder::StereoFrame;
use anyhow::{Context, Result, ensure};
use crossbeam_queue::ArrayQueue;
use serde::Serialize;
use std::{
    fmt::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
pub const OUTPUT_RATE: u32 = 48_000;
pub const OUTPUT_CAPACITY: usize = OUTPUT_RATE as usize / 4;
#[derive(Clone, Copy)]
struct Packet {
    epoch: u64,
    frame: StereoFrame,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputPhase {
    Idle,
    Opening,
    Ready,
    Closing,
    Closed,
    Failed,
}
impl OutputPhase {
    fn from_code(n: u8) -> Self {
        match n {
            0 => Self::Idle,
            1 => Self::Opening,
            2 => Self::Ready,
            3 => Self::Closing,
            4 => Self::Closed,
            _ => Self::Failed,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OutputFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
}
impl OutputFormat {
    fn validate(&self) -> Result<()> {
        ensure!(
            (8000..=384000).contains(&self.sample_rate) && (1..=32).contains(&self.channels),
            "output format rate/channels outside supported bounds"
        );
        ensure!(
            !self.sample_format.is_empty() && self.sample_format.len() <= 32,
            "output sample format name"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct OutputStatus {
    pub session: u64,
    pub phase: OutputPhase,
    /// Actual resource destruction has finished, including failed openings.
    pub closed: bool,
    pub paused: bool,
    pub epoch: u64,
    pub queued_frames: usize,
    pub dropped_frames: u64,
    pub reset_discarded_frames: u64,
    pub rendered_frames: u64,
    pub underrun_frames: u64,
    pub format: Option<OutputFormat>,
    pub error: Option<String>,
}
struct Note {
    bytes: [u8; 512],
    len: usize,
}
impl Note {
    fn new() -> Self {
        Self {
            bytes: [0; 512],
            len: 0,
        }
    }
    fn message(&self) -> String {
        String::from_utf8_lossy(&self.bytes[..self.len]).into_owned()
    }
}
impl fmt::Write for Note {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut n = s.len().min(self.bytes.len() - self.len);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.bytes[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}
struct Shared {
    session: u64,
    queue: ArrayQueue<Packet>,
    epoch: AtomicU64,
    paused: AtomicBool,
    close_requested: AtomicBool,
    failed: AtomicBool,
    phase: AtomicU8,
    closed: AtomicBool,
    dropped: AtomicU64,
    discarded: AtomicU64,
    rendered: AtomicU64,
    underrun: AtomicU64,
    format: Mutex<Option<OutputFormat>>,
    error: Mutex<Option<String>>,
    notes: ArrayQueue<Note>,
}
impl Shared {
    fn new(session: u64, epoch: u64, paused: bool) -> Self {
        Self {
            session,
            queue: ArrayQueue::new(OUTPUT_CAPACITY),
            epoch: AtomicU64::new(epoch),
            paused: AtomicBool::new(paused),
            close_requested: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            phase: AtomicU8::new(0),
            closed: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            discarded: AtomicU64::new(0),
            rendered: AtomicU64::new(0),
            underrun: AtomicU64::new(0),
            format: Mutex::new(None),
            error: Mutex::new(None),
            notes: ArrayQueue::new(1),
        }
    }
    fn status(&self) -> OutputStatus {
        let phase = OutputPhase::from_code(self.phase.load(Ordering::Acquire));
        let failed = self.failed.load(Ordering::Acquire);
        OutputStatus {
            session: self.session,
            phase: if failed {
                OutputPhase::Failed
            } else if self.close_requested.load(Ordering::Acquire)
                && !self.closed.load(Ordering::Acquire)
            {
                OutputPhase::Closing
            } else {
                phase
            },
            closed: self.closed.load(Ordering::Acquire),
            paused: self.paused.load(Ordering::Acquire),
            epoch: self.epoch.load(Ordering::Acquire),
            queued_frames: self.queue.len(),
            dropped_frames: self.dropped.load(Ordering::Relaxed),
            reset_discarded_frames: self.discarded.load(Ordering::Relaxed),
            rendered_frames: self.rendered.load(Ordering::Relaxed),
            underrun_frames: self.underrun.load(Ordering::Relaxed),
            format: self
                .format
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            error: self
                .error
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .or_else(|| {
                    failed.then(|| {
                        "audio stream reported an error; device worker is publishing details".into()
                    })
                }),
        }
    }
    fn fail(&self, error: String) {
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
        self.failed.store(true, Ordering::Release);
        self.phase.store(5, Ordering::Release);
    }
    fn reset(&self) {
        if self
            .epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .is_err()
        {
            self.fail("audio output epoch exhausted".into());
        }
        let mut n = 0;
        while self.queue.pop().is_some() {
            n += 1;
        }
        self.discarded.fetch_add(n, Ordering::Relaxed);
    }
    fn push(&self, frames: &[StereoFrame]) {
        let epoch = self.epoch.load(Ordering::Acquire);
        let skipped = frames.len().saturating_sub(OUTPUT_CAPACITY);
        let mut dropped = skipped as u64;
        for frame in &frames[skipped..] {
            let frame = frame.map(|s| if s.is_finite() { s.clamp(-1., 1.) } else { 0. });
            if self.queue.force_push(Packet { epoch, frame }).is_some() {
                dropped += 1;
            }
        }
        self.dropped.fetch_add(dropped, Ordering::Relaxed);
    }
}
/// A completion/diagnostic receipt containing no platform stream or device pointer.
#[derive(Clone)]
pub struct OutputWatch(Arc<Shared>);
impl OutputWatch {
    pub fn status(&self) -> OutputStatus {
        self.0.status()
    }
    pub fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::Acquire)
    }
}
/// Callback-safe error/cancellation lane. No device operations or PCM mutex.
#[derive(Clone)]
pub struct OutputSignals(Arc<Shared>);
impl OutputSignals {
    pub fn is_cancelled(&self) -> bool {
        self.0.close_requested.load(Ordering::Acquire)
    }
    /// Fixed storage; formatting does not allocate a String on this callback.
    pub fn fail(&self, error: impl fmt::Display) {
        let mut note = Note::new();
        let _ = write!(&mut note, "{error}");
        let _ = self.0.notes.push(note);
        self.0.failed.store(true, Ordering::Release);
    }
}
/// A sole consumer capability, moved from the device worker into its callback.
pub struct OutputConsumer(Arc<Shared>);
impl OutputConsumer {
    pub fn configure(self, sample_rate: u32, channels: u16) -> Result<DeviceConsumer> {
        ensure!(
            (8000..=384000).contains(&sample_rate) && (1..=32).contains(&channels),
            "device consumer rate/channels"
        );
        let epoch = self.0.epoch.load(Ordering::Acquire);
        Ok(DeviceConsumer {
            shared: self.0,
            rate: sample_rate,
            channels: channels as usize,
            epoch,
            a: ([0.; 2], false),
            b: ([0.; 2], false),
            fraction: 0.,
            primed: false,
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct OutputBufferError;
impl fmt::Display for OutputBufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("output buffer is not frame-aligned")
    }
}
impl std::error::Error for OutputBufferError {}
/// Callback-owned fixed interpolation state. 48 kHz passes frames directly;
/// other device rates use linear interpolation, with no callback allocation.
pub struct DeviceConsumer {
    shared: Arc<Shared>,
    rate: u32,
    channels: usize,
    epoch: u64,
    a: (StereoFrame, bool),
    b: (StereoFrame, bool),
    fraction: f64,
    primed: bool,
}
impl DeviceConsumer {
    fn pop(&mut self) -> (StereoFrame, bool) {
        // Reset drains on the producer. Bound defensive stale-tag work too.
        for _ in 0..8 {
            match self.shared.queue.pop() {
                Some(p) if p.epoch == self.epoch => return (p.frame, true),
                Some(_) => {}
                None => break,
            }
        }
        ([0.; 2], false)
    }
    fn next(&mut self) -> (StereoFrame, bool) {
        let epoch = self.shared.epoch.load(Ordering::Acquire);
        if epoch != self.epoch {
            self.epoch = epoch;
            self.primed = false;
            self.fraction = 0.;
        }
        if self.shared.close_requested.load(Ordering::Acquire)
            || self.shared.failed.load(Ordering::Acquire)
            || self.shared.paused.load(Ordering::Acquire)
        {
            return ([0.; 2], true);
        }
        let result = if self.rate == OUTPUT_RATE {
            self.pop()
        } else {
            if !self.primed {
                self.a = self.pop();
                self.b = self.pop();
                self.primed = true;
            }
            let t = self.fraction as f32;
            let frame = [
                self.a.0[0] * (1. - t) + self.b.0[0] * t,
                self.a.0[1] * (1. - t) + self.b.0[1] * t,
            ];
            let valid = self.a.1 && (t == 0. || self.b.1);
            self.fraction += OUTPUT_RATE as f64 / self.rate as f64;
            while self.fraction >= 1. {
                self.fraction -= 1.;
                self.a = self.b;
                self.b = self.pop();
            }
            (frame, valid)
        };
        if self.shared.epoch.load(Ordering::Acquire) != self.epoch {
            self.primed = false;
            ([0.; 2], true)
        } else {
            result
        }
    }
    /// Stereo maps to channels 0/1, mono averages L/R, extra channels are silent.
    /// The conversion closure is a host sample conversion, never VM/user DSP.
    pub fn fill<T: Copy>(
        &mut self,
        out: &mut [T],
        mut convert: impl FnMut(f32) -> T,
    ) -> std::result::Result<(), OutputBufferError> {
        if out.len() % self.channels != 0 {
            for sample in out {
                *sample = convert(0.);
            }
            return Err(OutputBufferError);
        }
        let mut rendered = 0u64;
        let mut missing = 0u64;
        for chunk in out.chunks_mut(self.channels) {
            let (frame, ready) = self.next();
            rendered += 1;
            missing += u64::from(!ready);
            for (i, sample) in chunk.iter_mut().enumerate() {
                *sample = convert(if self.channels == 1 {
                    (frame[0] + frame[1]) * 0.5
                } else {
                    match i {
                        0 => frame[0],
                        1 => frame[1],
                        _ => 0.,
                    }
                });
            }
        }
        self.shared.rendered.fetch_add(rendered, Ordering::Relaxed);
        self.shared.underrun.fetch_add(missing, Ordering::Relaxed);
        Ok(())
    }
}
/// Platform resources stay on the opening worker; they need not be Send.
pub trait OutputStream: 'static {}
pub struct OpenedOutput {
    format: OutputFormat,
    _stream: Box<dyn OutputStream>,
}
impl OpenedOutput {
    pub fn new(format: OutputFormat, stream: impl OutputStream) -> Result<Self> {
        format.validate()?;
        Ok(Self {
            format,
            _stream: Box::new(stream),
        })
    }
}
/// Trusted host backend. Opening, play and resource destruction run on a dedicated
/// worker; callback code only receives the consumer/signals capabilities.
pub trait OutputBackend: Send + Sync + 'static {
    fn open(&self, consumer: OutputConsumer, signals: OutputSignals) -> Result<OpenedOutput>;
}
impl<F> OutputBackend for F
where
    F: Fn(OutputConsumer, OutputSignals) -> Result<OpenedOutput> + Send + Sync + 'static,
{
    fn open(&self, c: OutputConsumer, s: OutputSignals) -> Result<OpenedOutput> {
        self(c, s)
    }
}
struct Completion(Arc<Shared>);
impl Drop for Completion {
    fn drop(&mut self) {
        if !self.0.failed.load(Ordering::Acquire) {
            self.0.phase.store(4, Ordering::Release);
        }
        self.0.closed.store(true, Ordering::Release);
    }
}
fn worker(backend: Arc<dyn OutputBackend>, shared: Arc<Shared>) {
    let _completion = Completion(shared.clone());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
        if shared.close_requested.load(Ordering::Acquire) {
            return Ok(());
        }
        let output = backend.open(
            OutputConsumer(shared.clone()),
            OutputSignals(shared.clone()),
        )?;
        if !shared.close_requested.load(Ordering::Acquire) && !shared.failed.load(Ordering::Acquire)
        {
            *shared.format.lock().unwrap_or_else(|e| e.into_inner()) = Some(output.format.clone());
            shared.phase.store(2, Ordering::Release);
        }
        while !shared.close_requested.load(Ordering::Acquire)
            && !shared.failed.load(Ordering::Acquire)
        {
            thread::park_timeout(Duration::from_millis(10));
        }
        // Destruction (including any platform callback join) happens here.
        drop(output);
        if shared.failed.load(Ordering::Acquire) {
            let message = shared
                .notes
                .pop()
                .map(|n| n.message())
                .unwrap_or_else(|| "audio output stream failed".into());
            anyhow::bail!("{message}");
        }
        Ok(())
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error))
            if !shared.close_requested.load(Ordering::Acquire)
                || shared.failed.load(Ordering::Acquire) =>
        {
            shared.fail(format!("{error:#}"))
        }
        Err(_) => shared.fail("audio device worker or stream destruction panicked".into()),
        _ => {}
    }
}
/// Modern PCM output. Submission never opens, plays, closes or joins a device on
/// the caller. poll/status/watch expose opening/failure/actual close completion.
pub struct PcmOutput {
    shared: Arc<Shared>,
    backend: Arc<dyn OutputBackend>,
    worker: Option<JoinHandle<()>>,
}
impl Default for PcmOutput {
    fn default() -> Self {
        #[cfg(feature = "cpal-output")]
        {
            Self::with_backend(crate::cpal_output::CpalBackend)
        }
        #[cfg(not(feature = "cpal-output"))]
        {
            Self::with_backend(|_, _| {
                anyhow::bail!("audio output backend disabled; enable cpal-output or use --no-audio")
            })
        }
    }
}
impl PcmOutput {
    pub fn with_backend(backend: impl OutputBackend) -> Self {
        Self {
            shared: Arc::new(Shared::new(1, 0, false)),
            backend: Arc::new(backend),
            worker: None,
        }
    }
    pub fn watch(&self) -> OutputWatch {
        OutputWatch(self.shared.clone())
    }
    pub fn status(&self) -> OutputStatus {
        self.shared.status()
    }
    /// Fatal device failures remain visible even on empty/silent host frames.
    pub fn poll(&self) -> Result<OutputStatus> {
        let status = self.status();
        ensure!(
            status.phase != OutputPhase::Failed,
            "{}",
            status.error.as_deref().unwrap_or("audio output failed")
        );
        Ok(status)
    }
    pub fn submit(&mut self, frames: &[StereoFrame]) -> Result<()> {
        ensure!(
            !self.shared.close_requested.load(Ordering::Acquire),
            "audio output is closing/closed"
        );
        self.poll()?;
        ensure!(
            frames.len() <= OUTPUT_RATE as usize * 2,
            "PCM submission exceeds two seconds"
        );
        if frames.is_empty() {
            return Ok(());
        }
        if self.worker.is_none() {
            let tail = &frames[frames.len().saturating_sub(OUTPUT_CAPACITY)..];
            if !tail.iter().flatten().any(|s| s.is_finite() && *s != 0.) {
                return Ok(());
            }
        }
        self.shared.push(frames);
        if self.worker.is_none() {
            let shared = self.shared.clone();
            let backend = self.backend.clone();
            self.shared.phase.store(1, Ordering::Release);
            match thread::Builder::new()
                .name("crysirror-audio-device".into())
                .spawn(move || worker(backend, shared))
            {
                Ok(handle) => self.worker = Some(handle),
                Err(error) => {
                    self.shared
                        .fail(format!("spawn audio device worker: {error}"));
                    self.shared.closed.store(true, Ordering::Release);
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }
    pub fn pause(&mut self) {
        self.shared.paused.store(true, Ordering::Release);
    }
    pub fn resume(&mut self) {
        self.shared.paused.store(false, Ordering::Release);
    }
    pub fn reset(&mut self) {
        self.shared.reset();
    }
    /// Nonblocking cancellation. closed becomes true after actual worker resource
    /// destruction. A platform open call can only cancel after returning.
    pub fn begin_close(&mut self) -> OutputWatch {
        self.shared.close_requested.store(true, Ordering::Release);
        self.shared.reset();
        if !self.shared.failed.load(Ordering::Acquire)
            && !self.shared.closed.load(Ordering::Acquire)
        {
            self.shared.phase.store(3, Ordering::Release);
        }
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        } else {
            if !self.shared.failed.load(Ordering::Acquire) {
                self.shared.phase.store(4, Ordering::Release);
            }
            self.shared.closed.store(true, Ordering::Release);
        }
        self.watch()
    }
    /// Explicit recovery after the old device is actually gone. PCM and control
    /// receipts from the previous session cannot enter the new one. Opens lazily.
    pub fn retry(&mut self) -> Result<()> {
        ensure!(
            self.shared.failed.load(Ordering::Acquire)
                && self.shared.closed.load(Ordering::Acquire),
            "retry requires a failed, fully closed output session"
        );
        let session = self
            .shared
            .session
            .checked_add(1)
            .context("audio output session exhausted")?;
        let epoch = self
            .shared
            .epoch
            .load(Ordering::Acquire)
            .checked_add(1)
            .context("audio output epoch exhausted")?;
        let paused = self.shared.paused.load(Ordering::Acquire);
        self.worker.take();
        self.shared = Arc::new(Shared::new(session, epoch, paused));
        Ok(())
    }
}
impl Drop for PcmOutput {
    fn drop(&mut self) {
        self.begin_close();
        self.worker.take();
    }
}

#[cfg(test)]
mod callback_tests {
    use super::*;
    #[test]
    fn callback_transfer_and_error_reporting_do_not_wait_on_control_mutexes() {
        let shared = Arc::new(Shared::new(1, 0, false));
        shared.push(&[[0.25, -0.5]]);
        let mut consumer = OutputConsumer(shared.clone()).configure(48000, 2).unwrap();
        let signals = OutputSignals(shared.clone());
        let format = shared.format.lock().unwrap();
        let error = shared.error.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let callback = thread::spawn(move || {
            let mut out = [0.; 2];
            consumer.fill(&mut out, |s| s).unwrap();
            signals.fail("authored callback failure");
            tx.send(out).unwrap();
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            [0.25, -0.5]
        );
        callback.join().unwrap();
        drop(error);
        drop(format);
    }
}
