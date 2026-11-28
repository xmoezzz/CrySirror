// SPDX-License-Identifier: MPL-2.0
//! Owner-side playback of the native runtime's audio commands (schema in
//! `rt/audio.rs`) over the host's channel audio service.
//!
//! renpysound numbers its channels; each gets a client-local host channel
//! on first use, and status reports name the playing item by its id. Upstream's backend keeps one
//! playing and one queued sample per channel: the queued sample starts
//! when the host reports the playing one finished, one owner step later
//! (the host has no gapless queue). Volumes combine the channel volume
//! with the secondary (mixer/fade) volume and the sample's relative
//! volume; pan maps Ren'Py's -1..1 onto the host's 0..1 balance. Audio
//! filters are not available and are reported once.
use crate::files::ScopedFiles;
use crate::rt::link::AudioStatus;
use crysirror_core::{
    Error, Result,
    service::{AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, HostServices, PcmBus},
    state::ClientId,
};
use serde_json::Value as J;
use std::collections::BTreeMap;

#[derive(Clone)]
struct Sample {
    name: String,
    file: String,
    fadein: f64,
    start: f64,
    relative_volume: f64,
    paused: bool,
}

#[derive(Default)]
struct Channel {
    id: u32,
    playing: Option<Sample>,
    queued: Option<Sample>,
    /// Preparation ticket of the playing sample until it is published.
    ticket: Option<u64>,
    volume: f64,
    secondary: f64,
    pan: f64,
    paused: bool,
}

pub struct HostAudio {
    channels: BTreeMap<String, Channel>,
    global_pause: bool,
    pub diagnostics: Vec<String>,
    filter_reported: bool,
}

impl Default for HostAudio {
    fn default() -> Self {
        HostAudio {
            channels: BTreeMap::new(),
            global_pause: false,
            diagnostics: Vec::new(),
            filter_reported: false,
        }
    }
}

fn f(v: &J, k: &str, d: f64) -> f64 {
    v.get(k).and_then(J::as_f64).unwrap_or(d)
}

fn s(v: &J, k: &str) -> String {
    v.get(k).and_then(J::as_str).unwrap_or("").to_owned()
}

/// Channels are renpysound channel numbers; status reports name them as
/// the same number.
fn channel_key(v: &J) -> String {
    match v.get("channel") {
        Some(J::Number(n)) => n.to_string(),
        Some(J::String(s)) => s.clone(),
        _ => String::new(),
    }
}

impl HostAudio {
    fn channel(&mut self, name: &str) -> &mut Channel {
        let next = self.channels.len() as u32 + 1;
        self.channels
            .entry(name.to_owned())
            .or_insert_with(|| Channel {
                id: next,
                volume: 1.0,
                secondary: 1.0,
                ..Channel::default()
            })
    }

    fn gain(c: &Channel, sample: &Sample) -> f32 {
        (c.volume * c.secondary * sample.relative_volume).clamp(0.0, 4.0) as f32
    }

    fn start(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        files: &ScopedFiles,
        name: &str,
        sample: Sample,
    ) -> Result<()> {
        let paused = self.global_pause;
        let c = self.channel(name);
        let Some(path) = files.asset_path(&sample.file).cloned() else {
            return Err(Error::Io(format!(
                "Ren'Py audio file missing: {}",
                sample.file
            )));
        };
        let parameters = AudioParameters {
            gain: Self::gain(c, &sample),
            pan: ((c.pan + 1.0) / 2.0).clamp(0.0, 1.0) as f32,
            fade_ms: (sample.fadein * 1000.0) as f32,
            start_ms: (sample.start.max(0.0) * 1000.0) as u64,
            paused: sample.paused || c.paused || paused,
            bus: PcmBus::Sfx,
            ..AudioParameters::default()
        };
        let ticket = host.begin_audio(
            client,
            AudioRequest::PlayAsset {
                channel: c.id,
                source: AudioAsset {
                    path,
                    frame_limit: None,
                },
                intro: None,
                parameters,
            },
        )?;
        if let Some(old) = c.ticket.replace(ticket) {
            host.release_audio(client, old)?;
        }
        c.playing = Some(sample);
        Ok(())
    }

    fn stop(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        name: &str,
        fade_ms: f32,
    ) -> Result<()> {
        let c = self.channel(name);
        if let Some(t) = c.ticket.take() {
            host.release_audio(client, t)?;
        }
        c.playing = None;
        let id = c.id;
        host.submit_audio(
            client,
            AudioRequest::Stop {
                channel: id,
                fade_ms,
            },
        )
    }

    /// Applies one tick's commands in issue order.
    pub fn apply(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        files: &ScopedFiles,
        commands: Vec<J>,
    ) -> Result<()> {
        for cmd in commands {
            let op = s(&cmd, "op");
            let name = channel_key(&cmd);
            // The playing item is reported back by its id ("channel:serial").
            let sample = || Sample {
                name: s(&cmd, "id"),
                file: s(&cmd, "file"),
                fadein: f(&cmd, "fadein", 0.0),
                start: f(&cmd, "start", 0.0),
                relative_volume: f(&cmd, "relative_volume", 1.0),
                paused: cmd.get("paused").and_then(J::as_bool).unwrap_or(false),
            };
            match op.as_str() {
                "play" => {
                    self.channel(&name).queued = None;
                    self.start(host, client, files, &name, sample())?;
                }
                "queue" => {
                    let c = self.channel(&name);
                    if c.playing.is_none() {
                        self.start(host, client, files, &name, sample())?;
                    } else {
                        c.queued = Some(sample());
                    }
                }
                "stop" => {
                    self.channel(&name).queued = None;
                    self.stop(host, client, &name, 0.0)?;
                }
                "dequeue" => self.channel(&name).queued = None,
                "fadeout" => {
                    let ms = (f(&cmd, "delay", 0.0) * 1000.0) as f32;
                    self.channel(&name).queued = None;
                    self.stop(host, client, &name, ms)?;
                }
                "pause" | "unpause" => {
                    let paused = op == "pause";
                    let c = self.channel(&name);
                    c.paused = paused;
                    let id = c.id;
                    host.submit_audio(
                        client,
                        AudioRequest::Pause {
                            channel: id,
                            paused,
                            fade_ms: 0.0,
                        },
                    )?;
                }
                "volume" | "secondary_volume" => {
                    let v = f(&cmd, "volume", 1.0);
                    let ms = (f(&cmd, "delay", 0.0) * 1000.0) as f32;
                    let c = self.channel(&name);
                    if op == "volume" {
                        c.volume = v;
                    } else {
                        c.secondary = v;
                    }
                    if let Some(p) = c.playing.clone() {
                        let (id, gain) = (c.id, Self::gain(c, &p));
                        host.submit_audio(
                            client,
                            AudioRequest::SetGain {
                                channel: id,
                                gain,
                                fade_ms: ms,
                            },
                        )?;
                    }
                }
                "pan" => {
                    let pan = f(&cmd, "pan", 0.0);
                    let ms = (f(&cmd, "delay", 0.0) * 1000.0) as f32;
                    let c = self.channel(&name);
                    c.pan = pan;
                    let id = c.id;
                    host.submit_audio(
                        client,
                        AudioRequest::SetPan {
                            channel: id,
                            pan: ((pan + 1.0) / 2.0).clamp(0.0, 1.0) as f32,
                            fade_ms: ms,
                        },
                    )?;
                }
                "global_pause" => {
                    let paused = cmd.get("pause").and_then(J::as_bool).unwrap_or(false);
                    self.global_pause = paused;
                    let ids: Vec<(u32, bool)> =
                        self.channels.values().map(|c| (c.id, c.paused)).collect();
                    for (id, own) in ids {
                        host.submit_audio(
                            client,
                            AudioRequest::Pause {
                                channel: id,
                                paused: paused || own,
                                fade_ms: 0.0,
                            },
                        )?;
                    }
                }
                "filter" => {
                    if !self.filter_reported {
                        self.filter_reported = true;
                        self.diagnostics.push(
                            "Ren'Py audio filters are not supported by the host audio service"
                                .into(),
                        );
                    }
                }
                other => self
                    .diagnostics
                    .push(format!("unknown Ren'Py audio command {other:?}")),
            }
        }
        Ok(())
    }

    /// Advances finished channels to their queued samples and reports the
    /// playing state of every channel for the next tick.
    pub fn status(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        files: &ScopedFiles,
    ) -> Result<Vec<AudioStatus>> {
        let names: Vec<String> = self.channels.keys().cloned().collect();
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            let (id, ticket) = {
                let c = self.channel(&name);
                (c.id, c.ticket)
            };
            let mut preparing = false;
            if let Some(t) = ticket {
                let p = host.audio_preparation(client, t)?;
                match p.phase {
                    AudioPreparePhase::Pending => preparing = true,
                    AudioPreparePhase::Ready => {
                        host.release_audio(client, t)?;
                        self.channel(&name).ticket = None;
                    }
                    AudioPreparePhase::Failed => {
                        host.release_audio(client, t)?;
                        let c = self.channel(&name);
                        c.ticket = None;
                        let file = c.playing.take().map(|s| s.file).unwrap_or_default();
                        self.diagnostics.push(format!(
                            "Ren'Py audio failed: {file}: {}",
                            p.error.unwrap_or_default()
                        ));
                    }
                }
            }
            let playing = preparing
                || (self.channel(&name).playing.is_some() && host.audio_playing(client, id)?);
            if !playing {
                let c = self.channel(&name);
                c.playing = None;
                if let Some(q) = c.queued.take() {
                    self.start(host, client, files, &name, q)?;
                }
            }
            let c = self.channel(&name);
            let (mut position_ms, mut duration_ms) = (None, None);
            if c.playing.is_some()
                && !preparing
                && let Ok(Some(info)) = host.audio_info(client, id)
                && info.source_rate > 0
            {
                position_ms = Some(info.position_frames * 1000 / info.source_rate as u64);
                duration_ms = info
                    .source_frames
                    .map(|f| f * 1000 / info.source_rate as u64);
            }
            let c = self.channel(&name);
            out.push(AudioStatus {
                channel: name.clone(),
                playing: c.playing.as_ref().map(|p| p.name.clone()),
                position_ms,
                duration_ms,
                video_size: None,
            });
        }
        Ok(out)
    }

    /// Stops every channel (session end).
    pub fn stop_all(&mut self, host: &mut dyn HostServices, client: ClientId) {
        let names: Vec<String> = self.channels.keys().cloned().collect();
        for n in names {
            let _ = self.stop(host, client, &n, 0.0);
        }
    }
}
