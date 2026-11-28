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
pub(super) enum Product {
    Prepared(PreparedChannel),
    Published,
}
pub(super) struct ChannelJobs {
    pub(super) jobs: OperationTable<Product>,
    channels: BTreeMap<u64, u32>,
    intent: BTreeMap<(u32, u32), u64>,
    pool: Option<TaskPool>,
    pub(super) nonblocking: bool,
    pub(super) budget: Arc<AtomicUsize>,
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
impl ChannelJobs {
    pub(super) fn exchange_reserved(&self,client:u32,left:u32,right:u32)->bool{
        self.intent.contains_key(&(client,left))||self.intent.contains_key(&(client,right))
    }
    pub(super) fn clear_owner(&mut self, owner: u32) {
        let ids = self
            .jobs
            .tickets()
            .into_iter()
            .filter_map(|(client, _, ticket)| if client == owner { Some(ticket) } else { None })
            .collect::<std::collections::BTreeSet<_>>();
        self.jobs.clear_owner(owner);
        self.channels.retain(|ticket, _| !ids.contains(ticket));
        self.intent.retain(|(client, _), _| *client != owner);
    }
}
pub(super) struct Charge {
    budget: Arc<AtomicUsize>,
    bytes: AtomicUsize,
}
impl Charge {
    pub(super) fn reserve(budget: Arc<AtomicUsize>, bytes: usize) -> Result<Self> {
        ensure!(
            budget
                .fetch_update(Ordering::Relaxed, Ordering::Acquire, |n| n
                    .checked_add(bytes)
                    .filter(|n| *n <= 512 * 1024 * 1024))
                .is_ok(),
            "channel audio reservation exceeded"
        );
        Ok(Self { budget, bytes: AtomicUsize::new(bytes) })
    }
    pub(super) fn shrink_to(&self,bytes:usize)->Result<()> {
        let previous=self.bytes.load(Ordering::Relaxed);
        ensure!(bytes<=previous,"audio preparation exceeded its reservation");
        self.bytes.store(bytes,Ordering::Relaxed);
        self.budget.fetch_sub(previous-bytes,Ordering::AcqRel);Ok(())
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}
fn channel(request: &AudioRequest) -> Result<u32> {
    match request {
        AudioRequest::Play { channel, .. }
        | AudioRequest::PlayPlaylist {channel,..}
        | AudioRequest::PlayResident {channel,..}
        | AudioRequest::PlayAsset { channel, .. }
        | AudioRequest::PlayResource { channel, .. } => Ok(*channel),
        _ => anyhow::bail!("preparation requires a play request"),
    }
}
impl ChannelAudio {
    /// Restore/shutdown boundary. Receipts cancel without waiting; reservations
    /// stay alive while any worker or stream still owns the corresponding data.
    pub fn reset(&mut self) {
        self.replacement_epoch = self.replacement_epoch.saturating_add(1);
        self.replacement_versions.clear();
        self.sequences.clear();
        self.seeks.clear();
        self.clear_releases();
        self.jobs.jobs.clear();
        self.jobs.intent.clear();
        self.jobs.channels.clear();
        self.mixer.clear();
        self.resources.clear();
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
    pub fn begin_resource_preparation(
        &mut self,
        client: u32,
        asset: crysirror_core::service::AudioAsset,
        measure_frames: bool,
        storage: Arc<dyn ClientStorage>,
    ) -> Result<u64> {
        self.resources.begin(
            client,
            asset,
            measure_frames,
            storage,
            self.jobs.pool.clone(),
            self.jobs.nonblocking,
        )
    }
    pub fn begin_transformed_resource_preparation(
        &mut self,
        client: u32,
        asset: crysirror_core::service::AudioAsset,
        transform: crysirror_core::service::AudioResourceTransform,
        storage: Arc<dyn ClientStorage>,
    ) -> Result<u64> {
        self.resources.begin_transformed(
            client,
            asset,
            true,
            Some(transform),
            storage,
            self.jobs.pool.clone(),
            self.jobs.nonblocking,
        )
    }
    /// Native callers retain their current event until preparation reports Ready.
    pub fn begin_preparation(
        &mut self,
        client: u32,
        request: AudioRequest,
        storage: Arc<dyn ClientStorage>,
    ) -> Result<u64> {
        self.begin_preparation_with_meter(client, request, storage, None)
    }

    pub fn begin_preparation_with_meter(
        &mut self,
        client: u32,
        request: AudioRequest,
        storage: Arc<dyn ClientStorage>,
        meter: Option<crysirror_core::service::AudioSourceMeterSpec>,
    ) -> Result<u64> {
        let meter = meter
            .map(|spec| crate::mixer::source_meter::SourceMeter::new(spec, self.mixer.sample_rate()))
            .transpose()?;
        self.poll_releases()?;
        let channel = channel(&request)?;
        self.admit_channel_operation(client)?;
        let p = match &request {
            AudioRequest::Play { parameters, .. }
            | AudioRequest::PlayPlaylist {parameters,..}
            | AudioRequest::PlayResident {parameters,..}
            | AudioRequest::PlayAsset { parameters, .. }
            | AudioRequest::PlayResource { parameters, .. } => parameters,
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
            gain_envelope: p.gain_envelope,
            mix_group: p.mix_group.clone(),
            spatial: p.spatial_position.map(|position| crate::mixer::Spatial { position, ..Default::default() }),
            gain: p.gain,
            rate: p.rate,
            fade_in_ms: p.fade_ms,
            ..Default::default()
        }
        .validate()?;
        let skip = !p.restart && self.is_playing(client, channel);
        let bytes = match &request {
            AudioRequest::Play { source, intro, .. } | AudioRequest::PlayResident {source,intro,..} => {
                ensure!(
                    source.bytes.len() <= 64 * 1024 * 1024
                        && intro
                            .as_ref()
                            .is_none_or(|v| v.bytes.len() <= 64 * 1024 * 1024),
                    "encoded channel memory limit"
                );
                source.bytes.len() + intro.as_ref().map_or(0, |v| v.bytes.len())
            }
            AudioRequest::PlayPlaylist { source, .. } => source.encoded_bytes()?,
            AudioRequest::PlayResource { .. } => 0,
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
        } + if matches!(&request,AudioRequest::PlayResident{..}){(128*1024*1024)*(if matches!(&request,AudioRequest::PlayResident{intro:Some(_),..}){3}else{2})}else{4*1024*1024};
        let resident=matches!(&request,AudioRequest::PlayResident{..});
        let resource = if let AudioRequest::PlayResource { resource, .. } = &request {
            Some(self.resources.capture(client, *resource)?)
        } else {
            None
        };
        let charge = Arc::new(Charge::reserve(self.jobs.budget.clone(), bytes)?);
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
                } | AudioRequest::PlayResident {source:encoded,intro,parameters:p,..} => {
                    let gain=p.decode_gain;
                    let tail=source(encoded,gain)?;
                    let tail=if resident{crate::resident::decode(tail,&cancel,Some(charge.clone()))?}else{tail};
                    let intro=intro.map(|v|->Result<_>{let source=source(v,gain)?;if resident{crate::resident::decode(source,&cancel,Some(charge.clone()))}else{Ok(source)}}).transpose()?;
                    (tail,intro,p)
                }
                AudioRequest::PlayPlaylist { source, parameters: p, .. } => {
                    (crate::playlist::source(source, p.decode_gain, &cancel)?, None, p)
                }
                AudioRequest::PlayResource { parameters: p, .. } => {
                    let opened = resource.context("prepared resource missing")?.open()?;
                    (
                        crate::channels::wrap_source(opened, None, p.decode_gain)?,
                        None,
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
            prepared.source_meter = meter;
            if resident {
                let frames=prepared.source.frame_count().context("resident PCM extent unavailable")?;
                let retained=usize::try_from(frames)?.checked_mul(8).and_then(|bytes|bytes.checked_add(4096)).context("resident reservation overflow")?;
                charge.shrink_to(retained)?;
            }
            if realtime && !resident && prepared.source.track_count() > 0 {
                // Each stem gets its own worker buffer; the mixer mixes them.
                let (loop_start, start) = (prepared.spec.loop_points.map(|p| p.start), prepared.start);
                crate::multitrack::buffer_tracks(
                    &mut prepared.source,
                    workers.context("audio worker pool")?,
                    loop_start,
                    start,
                    &cancel,
                    charge,
                )?;
            } else if realtime && !resident {
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
            self.cancel_release_channel(client, channel);
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
        if self.mixer.is_rendering() {
            return Ok(());
        }
        self.poll_releases()?;
        let mut jobs = std::mem::take(&mut self.jobs);
        let result = (|| {
            for (owner, _, ticket) in jobs.jobs.tickets() {
                if jobs.jobs.status(owner, 0, ticket)?.state != OperationState::Ready {
                    continue;
                }
                let channel = jobs.channels[&ticket];
                let published = jobs.jobs.publish(owner, 0, ticket, |product| {
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
                //The intent protects publication, not the lifetime of a ready
                //receipt. Once installed it must not pin its old channel name.
                if published.is_ok()&&jobs.intent.get(&(owner,channel))==Some(&ticket){
                    jobs.intent.remove(&(owner,channel));
                }
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
                OperationState::Ready => {
                    if matches!(
                        self.jobs.jobs.poll(client, 0, ticket)?,
                        Some(Product::Published)
                    ) {
                        AudioPreparePhase::Ready
                    } else {
                        AudioPreparePhase::Pending
                    }
                }
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
            AudioRequest::ExchangeChannels { .. } => anyhow::bail!("channel exchange must preflight both channels atomically"),
            AudioRequest::SetMixGroup { .. } | AudioRequest::SetMixGroupGain { .. } => {
                return Ok(());
            }
            AudioRequest::Play { channel, .. }
            | AudioRequest::PlayPlaylist {channel,..}
        | AudioRequest::PlayResident {channel,..}
            | AudioRequest::PlayAsset { channel, .. }
            | AudioRequest::PlayPcm { channel, .. }
            | AudioRequest::PlayResource { channel, .. } => (*channel, false),
            AudioRequest::Stop { channel, fade_ms } => {
                ensure!(
                    fade_ms.is_finite() && (0.0..=600000.).contains(fade_ms),
                    "channel stop fade"
                );
                (*channel, true)
            }
            AudioRequest::SetStereoGain {
                channel,
                gains,
                fade_ms,
            } => {
                crate::levels::validate_stereo_gains(*gains)?;
                ensure!(
                    fade_ms.is_finite() && (0.0..=600000.).contains(fade_ms),
                    "channel stereo gain fade"
                );
                (*channel, false)
            }
            AudioRequest::FadeGain {
                channel,
                gain,
                fade_ms,
                ..
            } => {
                ensure!(
                    gain.is_finite() && (0.0..=4.).contains(gain),
                    "channel voice gain"
                );
                ensure!(
                    fade_ms.is_finite() && (0.0..=600000.).contains(fade_ms),
                    "channel gain fade"
                );
                (*channel, false)
            }
            AudioRequest::SetRate{channel,rate}=>{ensure!(rate.is_finite()&&(1./4096. ..=32.).contains(rate),"live audio rate out of range");(*channel,false)},
            AudioRequest::SetStereoEffect { channel, effect } => { if let Some(spec) = effect { spec.validate().map_err(|e| anyhow::anyhow!("{e}"))?; } (*channel, false) },
            AudioRequest::SetEcho{channel,effect}=>{if let Some(spec)=effect {spec.validate().map_err(|e|anyhow::anyhow!("{e}"))?;}(*channel,false)},
            AudioRequest::Balance{channel,command}=>{crate::balance::validate(*command)?;(*channel,false)},
            AudioRequest::SetLoopSchedule {
                channel,
                start,
                end,
                passes,
            } => {
                crate::mixer::validate_loop_schedule(*start, *end, *passes)?;
                (*channel, false)
            }
            AudioRequest::Pause { channel, .. }
            | AudioRequest::SetTransportActive { channel, .. }
            | AudioRequest::Resume { channel, .. }
            | AudioRequest::ResumeWithOptions { channel, .. }
            | AudioRequest::SetGain { channel, .. }
            | AudioRequest::FinishGainFade { channel, .. }
            | AudioRequest::SetPan { channel, .. }
            | AudioRequest::SetChannelSpatial { channel, .. }
            | AudioRequest::SetLooped { channel, .. }
            | AudioRequest::SetChannelSwap { channel, .. }
            | AudioRequest::SetTrackGains { channel, .. } => (*channel, false),
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
