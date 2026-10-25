// SPDX-License-Identifier: MPL-2.0
//! Device-only bridge for the engine's stereo mixer. No simulation/DSP here.
use anyhow::{Context, Result};
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const RATE: u32 = 48_000;
const MAX_FRAMES: usize = RATE as usize / 4;
#[derive(Default)]
struct Queue {
    samples: VecDeque<f32>,
    dropped: usize,
}
impl Queue {
    fn push(&mut self, frames: &[[f32; 2]]) {
        let required = self.samples.len() + frames.len().min(MAX_FRAMES) * 2;
        let drop = required.saturating_sub(MAX_FRAMES * 2);
        self.samples.drain(..drop);
        self.dropped += drop / 2 + frames.len().saturating_sub(MAX_FRAMES);
        for frame in frames.iter().skip(frames.len().saturating_sub(MAX_FRAMES)) {
            self.samples.extend(frame.map(|sample| {
                if sample.is_finite() {
                    sample.clamp(-1., 1.)
                } else {
                    0.
                }
            }));
        }
    }
}
struct PcmSource {
    queue: Arc<Mutex<Queue>>,
    epoch: Arc<AtomicU64>,
    seen_epoch: u64,
    block: [f32; 512],
    cursor: usize,
}
impl PcmSource {
    fn new(queue: Arc<Mutex<Queue>>, epoch: Arc<AtomicU64>) -> Self {
        let seen_epoch = epoch.load(Ordering::Acquire);
        Self {
            queue,
            epoch,
            seen_epoch,
            block: [0.; 512],
            cursor: 512,
        }
    }
}
impl Iterator for PcmSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let epoch = self.epoch.load(Ordering::Acquire);
        if epoch != self.seen_epoch {
            self.cursor = 512;
            self.seen_epoch = epoch;
        }
        if self.cursor == self.block.len() {
            let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            for sample in &mut self.block {
                *sample = q.samples.pop_front().unwrap_or(0.);
            }
            self.cursor = 0;
        }
        let value = self.block[self.cursor];
        self.cursor += 1;
        Some(value)
    }
}
impl Source for PcmSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
#[derive(Default)]
pub struct PcmOutput {
    // Sink must drop before the device stream.
    sink: Option<Sink>,
    stream: Option<(OutputStream, OutputStreamHandle)>,
    queue: Arc<Mutex<Queue>>,
    epoch: Arc<AtomicU64>,
    paused: bool,
}
impl PcmOutput {
    pub fn submit(&mut self, frames: &[[f32; 2]]) -> Result<()> {
        if frames.is_empty() {
            return Ok(());
        }
        if self.sink.is_none() {
            // An empty/silent project does not open a device unnecessarily.
            if !frames.iter().flatten().any(|sample| *sample != 0.) {
                return Ok(());
            }
            let (stream, handle) = OutputStream::try_default().context(
                "no mixer audio device; use --no-audio for intentionally silent operation",
            )?;
            let sink = Sink::try_new(&handle)?;
            sink.append(PcmSource::new(self.queue.clone(), self.epoch.clone()));
            if self.paused {
                sink.pause();
            }
            self.sink = Some(sink);
            self.stream = Some((stream, handle));
        }
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(frames);
        Ok(())
    }
    pub fn pause(&mut self) {
        self.paused = true;
        if let Some(sink) = &self.sink {
            sink.pause();
        }
    }
    pub fn resume(&mut self) {
        self.paused = false;
        if let Some(sink) = &self.sink {
            sink.play();
        }
    }
    pub fn reset(&mut self) {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .samples
            .clear();
        self.epoch.fetch_add(1, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_queue_preserves_stereo_order_and_pads_silence_without_device() {
        let q = Arc::new(Mutex::new(Queue::default()));
        q.lock().unwrap().push(&[[0.25, -0.5], [1., -1.]]);
        let epoch = Arc::new(AtomicU64::new(0));
        let mut source = PcmSource::new(q.clone(), epoch.clone());
        assert_eq!(
            source.by_ref().take(6).collect::<Vec<_>>(),
            [0.25, -0.5, 1., -1., 0., 0.]
        );
        let frames = vec![[0.3, 0.6]; MAX_FRAMES + 8];
        q.lock().unwrap().push(&frames);
        assert_eq!(q.lock().unwrap().samples.len(), MAX_FRAMES * 2);
        assert_eq!(q.lock().unwrap().dropped, 8);
        assert_eq!(source.channels(), 2);
        assert_eq!(source.sample_rate(), 48000);
        q.lock().unwrap().samples.clear();
        epoch.fetch_add(1, Ordering::Release);
        assert_eq!(source.next(), Some(0.));
    }
    #[test]
    fn restore_epoch_discards_previously_buffered_samples_without_a_device() {
        let mut output = PcmOutput::default();
        output.queue.lock().unwrap().push(&[[0.75, -0.75]; 512]);
        let mut source = PcmSource::new(output.queue.clone(), output.epoch.clone());
        assert_eq!(source.next(), Some(0.75));
        output.reset();
        output.queue.lock().unwrap().push(&[[0.1, 0.2]]);
        assert_eq!(source.next(), Some(0.1));
        assert_eq!(source.next(), Some(0.2));
    }
    #[test]
    fn final_device_conversion_clamps_after_mix_and_rejects_nonfinite_samples() {
        let mut queue = Queue::default();
        queue.push(&[[2., -2.], [f32::NAN, f32::INFINITY]]);
        assert_eq!(
            queue.samples.into_iter().collect::<Vec<_>>(),
            [1., -1., 0., 0.]
        );
    }
}
