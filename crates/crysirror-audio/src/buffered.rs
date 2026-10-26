// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! A decoder has exactly one owner: idle control state or one bounded worker.
//! PCM consumers poll immutable chunks; they never lock or execute decoder I/O.
use crate::decoder::{AudioSource, SourceBufferStatus, SourceRead, StereoFrame};
use anyhow::{Context, Result, ensure};
use crysirror_tasks::{Cancellation, CapacityExceeded, Task, TaskPool};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, sync::Arc};
/// Read-ahead dimensions, independent of codec packet storage.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StreamBufferSpec {
    /// Frames per decoded chunk, 256..16384.
    pub chunk_frames: u32,
    /// Ready chunks, 2..4, with at most 65536 frames in the regular queue.
    pub prefetch_chunks: u32,
}
impl Default for StreamBufferSpec {
    fn default() -> Self {
        Self {
            chunk_frames: 4096,
            prefetch_chunks: 3,
        }
    }
}
impl StreamBufferSpec {
    pub fn validate(self) -> Result<()> {
        ensure!(
            (256..=16384).contains(&self.chunk_frames)
                && (2..=4).contains(&self.prefetch_chunks)
                && self.chunk_frames * self.prefetch_chunks <= 65536,
            "stream buffer dimensions"
        );
        Ok(())
    }
}
#[derive(Clone)]
struct Block {
    start: u64,
    data: Arc<[StereoFrame]>,
}
impl Block {
    fn end(&self) -> u64 {
        self.start + self.data.len() as u64
    }
}
struct OwnedDecoder {
    inner: Box<dyn AudioSource>,
    _storage: Option<Arc<dyn Send + Sync>>,
}
struct Completion {
    decoder: OwnedDecoder,
    generation: u64,
    block: Block,
    eof: bool,
    seek: Option<u64>,
}
#[derive(Debug)]
pub struct SourcePending;
impl std::fmt::Display for SourcePending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("audio source is pending; poll/service it instead of blocking")
    }
}
impl std::error::Error for SourcePending {}
/// Bounded streaming PCM source. Construct/prewarm on workers, call service on
/// the owner before/after rendering. No shared mutable decoder or per-voice thread.
pub struct BufferedSource {
    rate: u32,
    frames: Option<u64>,
    pool: TaskPool,
    config: StreamBufferSpec,
    decoder: Option<OwnedDecoder>,
    task: Option<Task<Completion>>,
    ready: VecDeque<Block>,
    pins: Vec<Vec<Block>>,
    position: u64,
    decoder_position: u64,
    generation: u64,
    seek: Option<u64>,
    io_seek: Option<u64>,
    eof: bool,
    decoded: u64,
    error: Option<String>,
    _storage: Option<Arc<dyn Send + Sync>>,
}
fn fill(
    decoder: &mut dyn AudioSource,
    start: u64,
    frames: usize,
    cancel: &Cancellation,
) -> Result<(Block, bool)> {
    let mut data = vec![[0.; 2]; frames];
    let mut done = 0;
    let mut eof = false;
    while done < frames {
        cancel.check()?;
        let n = decoder.read(&mut data[done..])?;
        ensure!(n <= frames - done, "invalid asynchronous audio read length");
        if n == 0 {
            eof = true;
            break;
        }
        done += n;
    }
    ensure!(
        data[..done]
            .iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() <= 16.),
        "invalid asynchronous PCM"
    );
    data.truncate(done);
    Ok((
        Block {
            start,
            data: data.into(),
        },
        eof,
    ))
}
impl BufferedSource {
    /// Blocking preparation belongs on a worker or explicit offline tool.
    pub fn prepare(
        source: Box<dyn AudioSource>,
        pool: TaskPool,
        config: StreamBufferSpec,
        loop_start: Option<u64>,
    ) -> Result<Self> {
        Self::prepare_cancellable(source, pool, config, loop_start, &Cancellation::default())
    }
    pub fn prepare_cancellable(
        source: Box<dyn AudioSource>,
        pool: TaskPool,
        config: StreamBufferSpec,
        loop_start: Option<u64>,
        cancel: &Cancellation,
    ) -> Result<Self> {
        Self::prepare_at_cancellable(source, pool, config, loop_start, 0, cancel)
    }
    /// Worker-only preparation with an exact initial decoded-frame prefix.
    pub fn prepare_at_cancellable(
        mut source: Box<dyn AudioSource>,
        pool: TaskPool,
        config: StreamBufferSpec,
        loop_start: Option<u64>,
        initial: u64,
        cancel: &Cancellation,
    ) -> Result<Self> {
        config.validate()?;
        cancel.check()?;
        source.seek(0)?;
        let rate = source.sample_rate();
        let mut frames = source.frame_count();
        ensure!((8000..=192000).contains(&rate), "buffered audio rate");
        let mut queue = VecDeque::with_capacity(config.prefetch_chunks as usize);
        let mut end = 0;
        let mut eof = false;
        let mut decoded = 0;
        for _ in 0..config.prefetch_chunks.min(2) {
            let (block, ended) = fill(source.as_mut(), end, config.chunk_frames as usize, cancel)?;
            decoded += block.data.len() as u64;
            end = block.end();
            if !block.data.is_empty() {
                queue.push_back(block);
            }
            eof = ended;
            if eof {
                break;
            }
        }
        ensure!(!queue.is_empty(), "empty buffered audio source");
        ensure!(
            frames.is_none_or(|n| end <= n),
            "decoded PCM exceeds declared source length"
        );
        if frames.is_none() && eof {
            frames = Some(end);
        }
        let mut pins = vec![queue.iter().cloned().collect::<Vec<_>>()];
        if let Some(start) = loop_start.filter(|v| *v != 0) {
            ensure!(
                frames.is_none_or(|n| start < n),
                "loop cache outside source"
            );
            cancel.check()?;
            source.seek(start)?;
            let mut cache = vec![];
            let mut at = start;
            for _ in 0..config.prefetch_chunks.min(2) {
                let (block, ended) =
                    fill(source.as_mut(), at, config.chunk_frames as usize, cancel)?;
                decoded += block.data.len() as u64;
                at = block.end();
                if !block.data.is_empty() {
                    cache.push(block);
                }
                if ended {
                    break;
                }
            }
            ensure!(!cache.is_empty(), "empty loop cache");
            pins.push(cache);
            source.seek(end)?;
        }
        if initial > 0 {
            ensure!(
                frames.is_none_or(|n| initial <= n),
                "initial audio frame outside source"
            );
            if let Some(cache) = pins.iter().find(|chunks| {
                chunks.first().is_some_and(|b| initial >= b.start)
                    && chunks.last().is_some_and(|b| initial < b.end())
            }) {
                queue = cache
                    .iter()
                    .filter(|b| b.end() > initial)
                    .cloned()
                    .collect();
                end = queue.back().unwrap().end();
                eof = frames == Some(end);
            } else {
                queue.clear();
                source.seek(initial)?;
                end = initial;
                eof = false;
                for _ in 0..config.prefetch_chunks.min(2) {
                    let (block, ended) =
                        fill(source.as_mut(), end, config.chunk_frames as usize, cancel)?;
                    decoded += block.data.len() as u64;
                    end = block.end();
                    if !block.data.is_empty() {
                        queue.push_back(block);
                    }
                    eof = ended;
                    if eof {
                        break;
                    }
                }
                pins.push(queue.iter().cloned().collect());
            }
            cancel.check()?;
            source.seek(end)?;
        }
        Ok(Self {
            rate,
            frames,
            pool,
            config,
            decoder: Some(OwnedDecoder {
                inner: source,
                _storage: None,
            }),
            task: None,
            ready: queue,
            pins,
            position: initial,
            decoder_position: end,
            generation: 1,
            seek: None,
            io_seek: None,
            eof,
            decoded,
            error: None,
            _storage: None,
        })
    }
    /// Attach residency before servicing/publishing this source. Worker-owned
    /// decoder allocations keep the receipt even if playback is canceled.
    pub fn retain_storage(&mut self, storage: Arc<dyn Send + Sync>) -> Result<()> {
        ensure!(
            self.task.is_none(),
            "attach stream residency before dispatch"
        );
        self.decoder
            .as_mut()
            .context("stream decoder missing")?
            ._storage = Some(storage.clone());
        self._storage = Some(storage);
        Ok(())
    }
    fn fail(&self) -> Result<()> {
        if let Some(error) = &self.error {
            anyhow::bail!("audio worker: {error}");
        }
        Ok(())
    }
    fn next_generation(&mut self) -> Result<()> {
        self.generation = self
            .generation
            .checked_add(1)
            .context("audio source generations exhausted")?;
        Ok(())
    }
    fn collect(&mut self) -> Result<()> {
        let Some(task) = &mut self.task else {
            return Ok(());
        };
        match task.try_take() {
            Ok(None) => {}
            Err(error) => {
                self.task = None;
                self.error = Some(format!("{error:#}"));
            }
            Ok(Some(done)) => {
                self.task = None;
                self.decoder_position = done.block.end();
                self.decoded = self.decoded.saturating_add(done.block.data.len() as u64);
                self.decoder = Some(done.decoder);
                if done.generation != self.generation {
                    return Ok(());
                }
                if let Some(goal) = done.seek {
                    ensure!(
                        self.seek.is_none_or(|f| f == goal),
                        "audio seek generation mismatch"
                    );
                    if self.seek.is_some() {
                        self.position = goal;
                        self.seek = None;
                    }
                    self.io_seek = None;
                }
                self.eof = done.eof;
                if self.frames.is_none() && done.eof {
                    self.frames = Some(done.block.end());
                }
                if !done.block.data.is_empty() {
                    ensure!(
                        self.ready.len() < self.config.prefetch_chunks as usize,
                        "audio prefetch overflow"
                    );
                    if let Some(back) = self.ready.back() {
                        ensure!(
                            back.end() == done.block.start,
                            "non-contiguous audio prefetch"
                        );
                    } else {
                        ensure!(
                            self.position == done.block.start,
                            "audio source position mismatch"
                        );
                    }
                    self.ready.push_back(done.block);
                }
            }
        }
        Ok(())
    }
}
impl AudioSource for BufferedSource {
    fn is_nonblocking(&self) -> bool {
        true
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn frame_count(&self) -> Option<u64> {
        self.frames
    }
    fn read(&mut self, out: &mut [StereoFrame]) -> Result<usize> {
        match self.poll_read(out)? {
            SourceRead::Frames(n) => Ok(n),
            SourceRead::End => Ok(0),
            SourceRead::Pending => Err(SourcePending.into()),
        }
    }
    fn seek(&mut self, _: u64) -> Result<()> {
        anyhow::bail!("buffered audio requires poll_seek; synchronous seek is unavailable")
    }
    fn poll_read(&mut self, out: &mut [StereoFrame]) -> Result<SourceRead> {
        if out.is_empty() {
            return Ok(SourceRead::Frames(0));
        }
        if self.seek.is_some() {
            self.fail()?;
            return Ok(SourceRead::Pending);
        }
        let Some(block) = self.ready.front() else {
            self.fail()?;
            return Ok(if self.eof {
                SourceRead::End
            } else {
                SourceRead::Pending
            });
        };
        ensure!(
            self.position >= block.start && self.position < block.end(),
            "audio buffer cursor outside chunk"
        );
        let from = (self.position - block.start) as usize;
        let n = out.len().min(block.data.len() - from);
        out[..n].copy_from_slice(&block.data[from..from + n]);
        self.position += n as u64;
        if self.position == block.end() {
            self.ready.pop_front();
        }
        Ok(SourceRead::Frames(n))
    }
    fn poll_seek(&mut self, frame: u64) -> Result<bool> {
        ensure!(
            self.frames.is_none_or(|n| frame <= n),
            "audio seek outside source"
        );
        self.fail()?;
        if self.seek == Some(frame) {
            return Ok(false);
        }
        if self.seek.is_none() && frame == self.position {
            return Ok(true);
        }
        let cache = self.pins.iter().find(|chunks| {
            chunks.first().is_some_and(|b| frame >= b.start)
                && chunks.last().is_some_and(|b| frame < b.end())
        });
        if let Some(cache) = cache {
            let blocks = cache
                .iter()
                .filter(|b| b.end() > frame)
                .cloned()
                .collect::<VecDeque<_>>();
            let end = blocks.back().unwrap().end();
            self.next_generation()?;
            self.ready = blocks;
            self.position = frame;
            self.seek = None;
            self.io_seek = (self.frames != Some(end)).then_some(end);
            self.eof = self.frames == Some(end);
            return Ok(true);
        }
        self.next_generation()?;
        self.ready.clear();
        self.seek = Some(frame);
        self.io_seek = Some(frame);
        self.eof = false;
        Ok(false)
    }
    fn service(&mut self) -> Result<()> {
        self.collect()?;
        if self.task.is_some() || self.error.is_some() {
            return Ok(());
        }
        if self.ready.len() >= self.config.prefetch_chunks as usize
            || (self.io_seek.is_none() && self.eof)
        {
            return Ok(());
        }
        let permit = match self.pool.reserve() {
            Ok(permit) => permit,
            Err(error) if error.is::<CapacityExceeded>() => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut decoder = self
            .decoder
            .take()
            .context("audio decoder ownership lost")?;
        let generation = self.generation;
        let seek = self.io_seek;
        let start = seek.unwrap_or(self.decoder_position);
        let count = self.config.chunk_frames as usize;
        let rate = self.rate;
        self.task = Some(permit.spawn(move |cancel| {
            cancel.check()?;
            if let Some(frame) = seek {
                decoder.inner.seek(frame)?;
            }
            let (block, eof) = fill(decoder.inner.as_mut(), start, count, &cancel)?;
            ensure!(
                decoder.inner.sample_rate() == rate,
                "audio rate changed mid-stream"
            );
            cancel.check()?;
            Ok(Completion {
                decoder,
                generation,
                block,
                eof,
                seek,
            })
        })?);
        Ok(())
    }
    fn read_position(&self) -> Option<u64> {
        Some(self.position)
    }
    fn buffer_status(&self) -> Option<SourceBufferStatus> {
        Some(SourceBufferStatus {
            ready_frames: self
                .ready
                .iter()
                .map(|b| b.end().saturating_sub(self.position.max(b.start)) as usize)
                .sum(),
            worker_pending: self.task.is_some(),
            seek_pending: self.seek.is_some(),
            generation: self.generation,
            decoded_frames: self.decoded,
            error: self.error.clone(),
        })
    }
}
