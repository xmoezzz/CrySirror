// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native and ordinary clients share preparation receipts and one channel mixer.
use crate::{
    buffered::{BufferedSource, StreamBufferSpec},
    channels::{ChannelAudio, PreparedChannel, asset, prepare, source},
    mixer::Ducking,
};
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::Mount,
    service::{AudioPreparation, AudioPreparePhase, AudioRequest, ClientStorage},
};
use crysirror_tasks::{
    Cancellation, TaskPool,
    table::{OperationState, OperationTable},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
enum Product {
    Prepared(PreparedChannel),
    Published,
}
pub(super) struct ChannelJobs {
    jobs: OperationTable<Product>,
    channels: BTreeMap<u64, u32>,
    intent: BTreeMap<(u32, u32), u64>,
    pool: Option<TaskPool>,
    nonblocking: bool,
    budget: Arc<AtomicUsize>,
}
impl Default for ChannelJobs {
    fn default() -> Self {
        Self {
            jobs: OperationTable::new(16, 64).unwrap(),
            channels: BTreeMap::new(),
            intent: BTreeMap::new(),
            pool: None,
            nonblocking: false,
            budget: Arc::new(AtomicUsize::new(0)),
        }
    }
}
struct Charge {
    budget: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
fn channel(request: &AudioRequest) -> Result<u32> {
    match request {
        AudioRequest::Play { channel, .. } | AudioRequest::PlayAsset { channel, .. } => {
            Ok(*channel)
        }
        _ => anyhow::bail!("preparation requires a play request"),
    }
}
impl ChannelAudio {
    /// Restore/shutdown boundary. Receipts cancel without waiting; reservations
    /// stay alive while any worker or stream still owns the corresponding data.
    pub fn reset(&mut self) {
        self.jobs.jobs.clear();
        self.jobs.intent.clear();
        self.jobs.channels.clear();
        self.mixer.clear();
        self.voices.clear();
        self.output.clear();
        self.remainder = 0;
    }
    /// Configure the shared bounded pool. Realtime preparation never waits;
    /// explicit offline preparation keeps immediate receipt behavior.
    pub fn configure_preparation(&mut self, pool: TaskPool, nonblocking: bool) {
        self.jobs.pool = Some(pool);
        self.jobs.nonblocking = nonblocking;
    }
    /// Native callers retain their current event until preparation reports Ready.
    pub fn begin_preparation(
        &mut self,
        client: u32,
        request: AudioRequest,
        storage: Arc<dyn ClientStorage>,
    ) -> Result<u64> {
        let channel = channel(&request)?;
        self.jobs.jobs.admit(client)?;
        let p = match &request {
            AudioRequest::Play { parameters, .. } | AudioRequest::PlayAsset { parameters, .. } => {
                parameters
            }
            _ => unreachable!(),
        };
        ensure!(
            p.decode_gain.is_finite()
                && (0.0..=16.).contains(&p.decode_gain)
                && p.pan.is_finite()
                && (0.0..=1.).contains(&p.pan),
            "channel audio gain/pan"
        );
        ensure!(
            p.replace_fade_ms.is_finite() && (0. ..=600000.).contains(&p.replace_fade_ms),
            "channel replacement fade"
        );
        crate::mixer::VoiceSpec {
            gain: p.gain,
            rate: p.rate,
            fade_in_ms: p.fade_ms,
            ..Default::default()
        }
        .validate()?;
        let skip = !p.restart && self.is_playing(client, channel);
        let bytes = match &request {
            AudioRequest::Play { source, intro, .. } => {
                ensure!(
                    source.bytes.len() <= 64 * 1024 * 1024
                        && intro
                            .as_ref()
                            .is_none_or(|v| v.bytes.len() <= 64 * 1024 * 1024),
                    "encoded channel memory limit"
                );
                source.bytes.len() + intro.as_ref().map_or(0, |v| v.bytes.len())
            }
            AudioRequest::PlayAsset { source, intro, .. } => {
                ensure!(
                    source.path.mount() == Mount::Asset
                        && intro
                            .as_ref()
                            .is_none_or(|v| v.path.mount() == Mount::Asset),
                    "channel audio asset mount"
                );
                0
            }
            _ => unreachable!(),
        } + 4 * 1024 * 1024;
        let budget = self.jobs.budget.clone();
        ensure!(
            budget
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n
                    .checked_add(bytes)
                    .filter(|n| *n <= 512 * 1024 * 1024))
                .is_ok(),
            "channel audio reservation exceeded"
        );
        let charge = Arc::new(Charge { budget, bytes });
        let realtime = self.jobs.nonblocking;
        let pool = self.jobs.pool.clone();
        let workers = pool.clone();
        ensure!(
            !realtime || workers.is_some(),
            "configure audio workers before realtime preparation"
        );
        let work = move |cancel: Cancellation| -> Result<Product> {
            cancel.check()?;
            let (tail, intro, p) = match request {
                AudioRequest::Play {
                    source: encoded,
                    intro,
                    parameters: p,
                    ..
                } => {
                    let gain = p.decode_gain;
                    (
                        source(encoded, gain)?,
                        intro.map(|v| source(v, gain)).transpose()?,
                        p,
                    )
                }
                AudioRequest::PlayAsset {
                    source: input,
                    intro,
                    parameters: p,
                    ..
                } => {
                    let gain = p.decode_gain;
                    (
                        asset(input, gain, storage.as_ref())?,
                        intro
                            .map(|v| asset(v, gain, storage.as_ref()))
                            .transpose()?,
                        p,
                    )
                }
                _ => unreachable!(),
            };
            cancel.check()?;
            let mut prepared = prepare(tail, intro, p)?;
            if realtime {
                let mut source = BufferedSource::prepare_at_cancellable(
                    prepared.source,
                    workers.context("audio worker pool")?,
                    StreamBufferSpec::default(),
                    prepared.spec.loop_points.map(|p| p.start),
                    prepared.start,
                    &cancel,
                )?;
                source.retain_storage(charge)?;
                prepared.source = Box::new(source);
            }
            cancel.check()?;
            Ok(Product::Prepared(prepared))
        };
        let ticket = if skip {
            self.jobs
                .jobs
                .insert_ready(client, 0, "channel_audio", Product::Published)?
        } else if realtime {
            self.jobs.jobs.submit_deferred(
                client,
                0,
                "channel_audio",
                pool.as_ref().unwrap(),
                work,
            )?
        } else {
            let value = work(Cancellation::default())?;
            self.jobs
                .jobs
                .insert_ready(client, 0, "channel_audio", value)?
        };
        self.jobs.channels.insert(ticket, channel);
        if !skip {
            self.jobs.intent.insert((client, channel), ticket);
        }
        // Offline callers can acknowledge in the original frame. Realtime
        // callers observe the distinct worker/publication boundary.
        if !realtime {
            self.poll_preparations()?;
        }
        Ok(ticket)
    }
    pub fn poll_preparations(&mut self) -> Result<()> {
        let mut jobs = std::mem::take(&mut self.jobs);
        let result = (|| {
            for (owner, _, ticket) in jobs.jobs.tickets() {
                if jobs.jobs.status(owner, 0, ticket)?.state != OperationState::Ready {
                    continue;
                }
                let channel = jobs.channels[&ticket];
                let _ = jobs.jobs.publish(owner, 0, ticket, |product| {
                    let Product::Prepared(prepared) = product else {
                        return Ok(product);
                    };
                    ensure!(
                        jobs.intent.get(&(owner, channel)) == Some(&ticket),
                        "channel preparation superseded or stopped"
                    );
                    if self.clients.insert(owner) {
                        self.mixer.set_ducking(
                            owner,
                            Ducking {
                                gain: 1.,
                                ..Default::default()
                            },
                        )?;
                    }
                    self.publish(owner, channel, prepared)?;
                    Ok(Product::Published)
                });
            }
            self.mixer.service_sources()?;
            Ok(())
        })();
        self.jobs = jobs;
        result
    }
    pub fn preparation(&mut self, client: u32, ticket: u64) -> Result<AudioPreparation> {
        self.poll_preparations()?;
        let status = self.jobs.jobs.status(client, 0, ticket)?;
        Ok(AudioPreparation {
            channel: self.jobs.channels[&ticket],
            phase: match status.state {
                OperationState::Pending => AudioPreparePhase::Pending,
                OperationState::Ready => AudioPreparePhase::Ready,
                OperationState::Failed => AudioPreparePhase::Failed,
            },
            error: status.error,
        })
    }
    pub fn release_preparation(&mut self, client: u32, ticket: u64) -> Result<()> {
        // Do not poll/publicize an unobserved completed job during cancellation.
        self.jobs.jobs.cancel(client, ticket)?;
        self.jobs.channels.remove(&ticket);
        self.jobs.intent.retain(|_, v| *v != ticket);
        Ok(())
    }
    pub(super) fn cancel_pending_control(
        &mut self,
        client: u32,
        request: &AudioRequest,
    ) -> Result<()> {
        let (channel, stop) = match request {
            AudioRequest::Play { channel, .. }
            | AudioRequest::PlayAsset { channel, .. }
            | AudioRequest::PlayPcm { channel, .. } => (*channel, false),
            AudioRequest::Stop { channel, fade_ms } => {
                ensure!(
                    fade_ms.is_finite() && (0.0..=600000.).contains(fade_ms),
                    "channel stop fade"
                );
                (*channel, true)
            }
            AudioRequest::Pause { channel, .. }
            | AudioRequest::Resume { channel, .. }
            | AudioRequest::SetGain { channel, .. }
            | AudioRequest::SetPan { channel, .. }
            | AudioRequest::SetLooped { channel, .. } => (*channel, false),
        };
        if !stop {
            self.poll_preparations()?;
        }
        if let Some(&ticket) = self.jobs.intent.get(&(client, channel)) {
            if stop {
                self.jobs.intent.remove(&(client, channel));
            } else {
                let status = self.jobs.jobs.status(client, 0, ticket)?;
                ensure!(
                    status.state != OperationState::Pending,
                    "await channel preparation before synchronous controls"
                );
            }
        }
        Ok(())
    }
}
