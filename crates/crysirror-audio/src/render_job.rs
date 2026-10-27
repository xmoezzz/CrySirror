// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Exclusive DSP ownership, bounded worker frames and explicit owner publication.
use crate::{decoder::StereoFrame, mixer::Mixer};
use anyhow::{Context, Result, ensure};
use crysirror_tasks::{Task, TaskPool, table::next_operation_id};
struct Product {
    mixers: Vec<Mixer>,
    pcm: Vec<StereoFrame>,
    error: Option<String>,
}
/// One frame's DSP state is moved, never borrowed or locked across threads.
/// Poll/cancel must receive the same ordered owner mixer slots.
#[must_use = "poll or cancel with the owner mixer slots before dropping"]
pub struct MixJob {
    task: Option<Task<Product>>,
    leases: Vec<u64>,
}
impl MixJob {
    pub fn begin(
        pool: &TaskPool,
        mixers: &mut [&mut Mixer],
        frames: usize,
        extra: Vec<StereoFrame>,
    ) -> Result<Self> {
        ensure!(
            (1..=8).contains(&mixers.len()),
            "audio frame mixer count must be 1..8"
        );
        let counts = vec![frames; mixers.len()];
        Self::begin_reserved(pool.reserve()?, mixers, &counts, extra)
    }
    fn begin_reserved(
        permit: crysirror_tasks::TaskPermit,
        mixers: &mut [&mut Mixer],
        counts: &[usize],
        extra: Vec<StereoFrame>,
    ) -> Result<Self> {
        ensure!(
            (1..=8).contains(&mixers.len()) && counts.len() == mixers.len(),
            "audio frame mixer/count mismatch"
        );
        let rate = mixers[0].sample_rate();
        let frames = counts.iter().copied().max().unwrap_or(0).max(extra.len());
        ensure!(
            frames <= rate as usize * 2,
            "audio worker frame exceeds two seconds"
        );
        ensure!(
            extra.iter().flatten().all(|x| x.is_finite()),
            "nonfinite audio frame input"
        );
        for m in mixers.iter() {
            m.idle()?;
            ensure!(m.sample_rate() == rate, "audio frame sample rate mismatch");
        }
        let counts = counts.to_vec();
        let leases = (0..mixers.len())
            .map(|_| next_operation_id())
            .collect::<Result<Vec<_>>>()?;
        let mut owned = Vec::with_capacity(mixers.len());
        for (m, &id) in mixers.iter_mut().zip(&leases) {
            match m.lease_render(id) {
                Ok(state) => owned.push(state),
                Err(error) => {
                    for ((slot, &old), state) in mixers.iter_mut().zip(&leases).zip(owned) {
                        slot.finish_render(old, state);
                    }
                    return Err(error);
                }
            }
        }
        let task = permit.spawn(move |cancel| {
            let mut pcm = vec![[0.; 2]; frames];
            let result = (|| -> Result<()> {
                let mut block = vec![[0.; 2]; 4096];
                for (mixer, &count) in owned.iter_mut().zip(&counts) {
                    for out in pcm[..count].chunks_mut(4096) {
                        cancel.check()?;
                        let data = &mut block[..out.len()];
                        mixer.render(data)?;
                        for (a, b) in out.iter_mut().zip(data) {
                            a[0] += b[0];
                            a[1] += b[1];
                        }
                    }
                }
                for (a, b) in pcm.iter_mut().zip(extra) {
                    a[0] += b[0];
                    a[1] += b[1];
                }
                ensure!(
                    pcm.iter().flatten().all(|x| x.is_finite()),
                    "nonfinite worker mix output"
                );
                Ok(())
            })();
            Ok(Product {
                mixers: owned,
                pcm,
                error: result.err().map(|e| format!("{e:#}")),
            })
        });
        let task = match task {
            Ok(task) => task,
            Err(error) => {
                for (slot, &id) in mixers.iter_mut().zip(&leases) {
                    slot.abort_render(id);
                }
                return Err(error);
            }
        };
        Ok(Self {
            task: Some(task),
            leases,
        })
    }
    fn valid(&self, mixers: &[&mut Mixer]) -> bool {
        mixers.len() == self.leases.len()
            && mixers
                .iter()
                .zip(&self.leases)
                .all(|(m, id)| m.lease_id() == Some(*id))
    }
    /// None means still running; only completion publishes state and PCM.
    pub fn poll(&mut self, mixers: &mut [&mut Mixer]) -> Result<Option<Vec<StereoFrame>>> {
        if !self.valid(mixers) {
            self.cancel(mixers);
            anyhow::bail!("audio frame lease invalidated; discard the obsolete job");
        }
        let result = match self
            .task
            .as_mut()
            .context("audio frame already collected")?
            .try_take()
        {
            Ok(None) => return Ok(None),
            Ok(Some(p)) => p,
            Err(e) => {
                self.task = None;
                for (m, &id) in mixers.iter_mut().zip(&self.leases) {
                    m.abort_render(id);
                }
                return Err(e);
            }
        };
        self.task = None;
        let failed = result.error;
        for ((slot, &id), mut state) in mixers.iter_mut().zip(&self.leases).zip(result.mixers) {
            if failed.is_some() {
                state.clear();
            }
            ensure!(
                slot.finish_render(id, state),
                "audio frame lease changed at publication"
            );
        }
        if let Some(error) = failed {
            anyhow::bail!("audio worker frame: {error}");
        }
        Ok(Some(result.pcm))
    }
    /// Invalidate matching owner slots immediately. Late worker data cannot
    /// replace reset/restored voices, and actual source leases drop on workers.
    pub fn cancel(&mut self, mixers: &mut [&mut Mixer]) {
        self.task.take();
        for (m, &id) in mixers.iter_mut().zip(&self.leases) {
            m.abort_render(id);
        }
    }
}

/// Bounded prepared frame with deferred worker admission. Polling retries pool
/// capacity without rerunning simulation or losing the prepared PCM input.
#[must_use = "poll or cancel with the owner mixer slots before dropping"]
pub struct MixFrame {
    pool: TaskPool,
    counts: Vec<usize>,
    extra: Option<Vec<StereoFrame>>,
    job: Option<MixJob>,
}
impl MixFrame {
    pub fn new(pool: TaskPool, counts: Vec<usize>, extra: Vec<StereoFrame>) -> Result<Self> {
        ensure!(
            (1..=8).contains(&counts.len())
                && counts.iter().copied().max().unwrap_or(0) <= 384000
                && extra.len() <= 384000,
            "audio prepared frame extent"
        );
        ensure!(
            extra.iter().flatten().all(|x| x.is_finite()),
            "nonfinite prepared PCM"
        );
        Ok(Self {
            pool,
            counts,
            extra: Some(extra),
            job: None,
        })
    }
    pub fn poll(&mut self, mixers: &mut [&mut Mixer]) -> Result<Option<Vec<StereoFrame>>> {
        if self.job.is_none() {
            ensure!(self.extra.is_some(), "audio frame consumed or cancelled");
            ensure!(
                mixers.len() == self.counts.len(),
                "audio frame mixer/count mismatch"
            );
            let permit = match self.pool.reserve() {
                Ok(p) => p,
                Err(e) if e.is::<crysirror_tasks::CapacityExceeded>() => return Ok(None),
                Err(e) => return Err(e),
            };
            self.job = Some(MixJob::begin_reserved(
                permit,
                mixers,
                &self.counts,
                self.extra.take().context("audio frame consumed")?,
            )?);
        }
        self.job.as_mut().unwrap().poll(mixers)
    }
    pub fn cancel(&mut self, mixers: &mut [&mut Mixer]) {
        if let Some(job) = &mut self.job {
            job.cancel(mixers);
        }
        self.extra = None;
    }
}
