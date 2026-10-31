// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Original legacy audio policy translated into shared audio requests. Plans
//! are immutable candidates: publish audio successfully before adopting `next`.
//! Decoders, envelopes, spatialization and mixing belong to shared services.
use crate::{
    config::Names,
    machine::{NativeCall, Payload},
    native_resources::{MusicLookup, PackedMember, ResourceNames},
};
use anyhow::{Context, Result, bail, ensure};
use crysirror_core::{
    path::VfsPath,
    service::{AudioAsset, AudioParameters, AudioRequest, AudioResourceHandle, PcmBus},
};

#[derive(Clone, Copy, Debug)]
pub struct AudioBindings {
    pub music: u32,
    pub sound_base: u32,
    pub voice_base: u32,
}
impl AudioBindings {
    pub fn validate(self) -> Result<()> {
        let se = self
            .sound_base
            .checked_add(8)
            .context("NNN sound channel overflow")?;
        let vc = self
            .voice_base
            .checked_add(8)
            .context("NNN voice channel overflow")?;
        ensure!(
            !(self.sound_base..se).contains(&self.music)
                && !(self.voice_base..vc).contains(&self.music)
                && (se <= self.voice_base || vc <= self.sound_base),
            "NNN audio channel collision"
        );
        Ok(())
    }
}
/// Explicit caller-provided original preferences, never guessed from defaults.
#[derive(Clone, Copy, Debug)]
pub struct AudioSettings {
    pub music_volume: i32,
    pub sound_volume: i32,
    pub voice_volume: i32,
    pub sound_voice_volume: i32,
    pub music_enabled: bool,
    pub sound_enabled: bool,
    pub voice_enabled: bool,
    pub sound_voice_enabled: bool,
}
impl AudioSettings {
    pub fn validate(self) -> Result<()> {
        ensure!(
            [
                self.music_volume,
                self.sound_volume,
                self.voice_volume,
                self.sound_voice_volume
            ]
            .iter()
            .all(|v| (0..=100).contains(v)),
            "NNN audio preference volume"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Music,
    Sound,
    Voice,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeAudioLookup {
    Music(MusicLookup),
    Sound {
        loose: String,
    },
    /// Original PlayScriptVoice probes the packed entry first and falls back
    /// to the loose file when that lookup fails. Neither is a fake mount hit.
    Voice {
        packed: PackedMember,
        loose: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSource {
    pub kind: SourceKind,
    /// Original encoded logical name (not executable/device state).
    pub name: Vec<u8>,
    /// Relative original lookup candidates; caller applies capability roots.
    pub candidates: Vec<String>,
}
impl NativeSource {
    pub fn lookup(&self, names: &ResourceNames) -> Result<NativeAudioLookup> {
        Ok(match self.kind {
            SourceKind::Music => NativeAudioLookup::Music(names.music_lookup(&self.name)?),
            SourceKind::Sound => NativeAudioLookup::Sound {
                loose: names.system_wave(Some(b"se"), &self.name)?,
            },
            SourceKind::Voice => NativeAudioLookup::Voice {
                packed: names.packed_voice(&self.name)?,
                loose: names.loose_voice(&self.name)?,
            },
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpatialPolicy {
    pub start: [i32; 3],
    pub end: [i32; 3],
    pub move_time: i32,
    pub doppler: i32,
}
impl SpatialPolicy {
    pub fn is_plain(self) -> bool {
        self.start == [0; 3] && self.end == [0; 3] && self.move_time == 0 && self.doppler == 0
    }
}
/// Legacy music ramps interpolate integer centi-decibels every100ms. This is
/// deliberately distinct from AudioParameters' continuous linear-gain fades.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicFade {
    pub ticks_100ms: u32,
    pub target_percent: i32,
    pub fade_in: bool,
}
#[derive(Clone, Debug)]
pub enum AudioOperation {
    None,
    Stop {
        channel: u32,
        fade: Option<MusicFade>,
    },
    Gain {
        channel: u32,
        percent: i32,
    },
    Play {
        channel: u32,
        parameters: AudioParameters,
        repeat_count: i32,
        spatial: SpatialPolicy,
        fade_in: Option<MusicFade>,
        fade_out: Option<MusicFade>,
    },
}
#[derive(Clone, Debug)]
pub struct NativeAudioPolicy {
    pub settings: AudioSettings,
    pub bindings: AudioBindings,
    pub music_number: Option<i32>,
    pub last_music_number: Option<i32>,
    pub music_repeat_count: i32,
    pub music_percent: i32,
    pub sound_work: [[i32; 12]; 8],
    pub voice_work: [[i32; 12]; 8],
    pub voice_names: [Vec<u8>; 8],
    pub default_music_fade_in: u32,
    pub default_music_fade_out: u32,
}
#[derive(Clone, Debug)]
pub struct NativeAudioPlan {
    pub operation: AudioOperation,
    pub source: Option<NativeSource>,
    pub next: NativeAudioPolicy,
}
fn decoded_name(bytes: &[u8]) -> Result<String> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= 128 && !bytes.contains(&0),
        "NNN audio resource name bounds"
    );
    let value = encoding_rs::SHIFT_JIS
        .decode_without_bom_handling_and_without_replacement(bytes)
        .context("NNN audio name is not valid Shift_JIS")?;
    ensure!(
        !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
            && value != "."
            && value != "..",
        "NNN audio resource name is not a basename"
    );
    Ok(value.into_owned())
}
fn table_name(table: &Names, index: i32) -> Result<Vec<u8>> {
    let at = usize::try_from(index)
        .context("NNN negative audio table index")?
        .checked_mul(2)
        .context("NNN audio table index overflow")?;
    let name = table
        .tokens
        .get(at)
        .context("NNN audio table index bounds")?;
    decoded_name(name)?;
    Ok(name.clone())
}
/// Native DirectSound SetVolume argument is percent*100-10000, in centi-dB.
/// This conversion has no percent/100 approximation or zero-volume shortcut.
pub fn legacy_gain(percent: i32) -> Result<f32> {
    ensure!((0..=100).contains(&percent), "NNN audio volume range");
    Ok(10f32.powf((percent as f32 - 100.) / 20.))
}
fn fade(value: i32, default: u32, percent: i32, fade_in: bool) -> Result<Option<MusicFade>> {
    let ticks = if value == -1 {
        default
    } else {
        u32::try_from(value).context("NNN negative native fade")?
    };
    ensure!(ticks <= 864000, "NNN music fade exceeds24hours");
    Ok((ticks > 0).then_some(MusicFade {
        ticks_100ms: ticks,
        target_percent: percent,
        fade_in,
    }))
}
fn spatial(words: &[i32]) -> Result<SpatialPolicy> {
    let coordinate = |at: usize| {
        let x = words.get(at).copied().unwrap_or(0);
        if x >= 128 { x.wrapping_sub(256) } else { x }
    };
    Ok(SpatialPolicy {
        start: [coordinate(4), coordinate(5), coordinate(6)],
        end: [coordinate(8), coordinate(9), coordinate(10)],
        move_time: words.get(7).copied().unwrap_or(0),
        doppler: words.get(11).copied().unwrap_or(0),
    })
}
impl NativeAudioPolicy {
    pub fn new(settings: AudioSettings, bindings: AudioBindings) -> Result<Self> {
        settings.validate()?;
        bindings.validate()?;
        Ok(Self {
            settings,
            bindings,
            music_number: None,
            last_music_number: None,
            music_repeat_count: 0,
            music_percent: settings.music_volume,
            sound_work: [[0; 12]; 8],
            voice_work: [[0; 12]; 8],
            voice_names: std::array::from_fn(|_| Vec::new()),
            default_music_fade_in: 0,
            default_music_fade_out: 5,
        })
    }
    /// Normal native execution profile. Skip/auto, character switches and read
    /// history are caller policy; a caller must not invoke this as a skip stub.
    pub fn plan(
        &self,
        call: &NativeCall,
        music: &Names,
        sound: &Names,
        music_playing: bool,
    ) -> Result<NativeAudioPlan> {
        ensure!(call.identify == 0x66660007, "NNN audio callback identify");
        match (&call.payload, call.code) {
            (Payload::Words { count, words }, 0x33330007) => {
                ensure!(*count as usize == words.len(), "NNN music word count");
                self.music(words, music, music_playing)
            }
            (Payload::Words { count, words }, 0x33330009) => {
                ensure!(*count as usize == words.len(), "NNN sound word count");
                self.clip(words, None, sound, false)
            }
            (
                Payload::Voice {
                    name,
                    parameters,
                    default_voice,
                },
                0x33330008,
            ) => {
                // Legacy callback does not implement the modern default-name
                // rewrite. Its source lookup consumes the pre-received name.
                let _ = *default_voice;
                self.clip(parameters, name.as_deref(), sound, true)
            }
            _ => bail!("NNN audio callback requires exact Music/Sound/Voice payload"),
        }
    }
    pub fn music(&self, words: &[i32], names: &Names, playing: bool) -> Result<NativeAudioPlan> {
        ensure!((1..=5).contains(&words.len()), "NNN music parameter count");
        let number = words[0];
        let mut next = self.clone();
        let mut plan = NativeAudioPlan {
            operation: AudioOperation::None,
            source: None,
            next: next.clone(),
        };
        if number == -1 {
            next.music_number = None;
            next.last_music_number = None;
            plan.operation = AudioOperation::Stop {
                channel: self.bindings.music,
                fade: if playing {
                    fade(
                        words.get(1).copied().unwrap_or(-1),
                        self.default_music_fade_out,
                        self.music_percent,
                        false,
                    )?
                } else {
                    None
                },
            };
        } else if number == -2 {
            if let Some(delta) = words.get(1) {
                next.music_percent = (i64::from(self.settings.music_volume) + i64::from(*delta))
                    .clamp(1, 100) as i32;
                plan.operation = AudioOperation::Gain {
                    channel: self.bindings.music,
                    percent: next.music_percent,
                };
            }
        } else {
            ensure!(number > 0, "NNN music number must be positive, -1 or -2");
            let name = table_name(names, number - 1)?;
            let filename = decoded_name(&name)?;
            let repeat = words.get(1).copied().unwrap_or(0);
            ensure!(repeat >= 0, "NNN negative music repeat count");
            let delta = words.get(2).copied().unwrap_or(-9999);
            let percent = if delta == -9999 {
                self.settings.music_volume
            } else {
                (i64::from(self.settings.music_volume) + i64::from(delta)).clamp(1, 100) as i32
            };
            next.music_number = Some(number - 1);
            next.last_music_number = Some(number - 1);
            next.music_repeat_count = repeat;
            next.music_percent = percent;
            if self.settings.music_enabled {
                plan.source = Some(NativeSource {
                    kind: SourceKind::Music,
                    name,
                    candidates: vec![format!("wgq/{filename}.mml"), format!("wgq/{filename}.wgq")],
                });
                plan.operation = AudioOperation::Play {
                    channel: self.bindings.music,
                    parameters: AudioParameters {
                        gain: legacy_gain(percent)?,
                        looping: repeat == 0,
                        bus: PcmBus::Music,
                        ..AudioParameters::default()
                    },
                    repeat_count: repeat,
                    spatial: spatial(&[])?,
                    fade_in: fade(
                        words.get(3).copied().unwrap_or(-1),
                        self.default_music_fade_in,
                        percent,
                        true,
                    )?,
                    fade_out: if playing {
                        fade(
                            words.get(4).copied().unwrap_or(-1),
                            self.default_music_fade_out,
                            self.music_percent,
                            false,
                        )?
                    } else {
                        None
                    },
                };
            }
        }
        plan.next = next;
        Ok(plan)
    }
    fn clip(
        &self,
        words: &[i32],
        voice_name: Option<&[u8]>,
        names: &Names,
        voice: bool,
    ) -> Result<NativeAudioPlan> {
        ensure!((2..=12).contains(&words.len()), "NNN clip parameter count");
        let channel = usize::try_from(words[1]).context("NNN negative native audio channel")?;
        ensure!(channel < 8, "NNN native audio channel bounds");
        let logical = (if voice {
            self.bindings.voice_base
        } else {
            self.bindings.sound_base
        }) + channel as u32;
        let mut next = self.clone();
        let mut plan = NativeAudioPlan {
            operation: AudioOperation::None,
            source: None,
            next: next.clone(),
        };
        if words[0] == -1 {
            if voice {
                ensure!(voice_name.is_none(), "NNN stopped voice has filename");
                next.voice_work[channel][3] = 0;
            } else {
                next.sound_work[channel][3] = 0;
            }
            plan.operation = AudioOperation::Stop {
                channel: logical,
                fade: None,
            };
        } else {
            ensure!(words[0] >= 0, "NNN negative clip index");
            let mut work = [0; 12];
            work[..words.len()].copy_from_slice(words);
            let s = spatial(words)?;
            work[4..7].copy_from_slice(&s.start);
            work[8..11].copy_from_slice(&s.end);
            let (enabled, volume, bus, name, kind) = if voice {
                let name = voice_name.context("NNN voice lacks pre-received filename")?;
                decoded_name(name)?;
                let stored = name[..name.len().min(62)].to_vec();
                next.voice_names[channel] = stored.clone();
                next.voice_work[channel] = work;
                (
                    if channel < 2 {
                        self.settings.voice_enabled
                    } else {
                        self.settings.sound_voice_enabled
                    },
                    if channel < 2 {
                        self.settings.voice_volume
                    } else {
                        self.settings.sound_voice_volume
                    },
                    PcmBus::Voice,
                    stored,
                    SourceKind::Voice,
                )
            } else {
                next.sound_work[channel] = work;
                (
                    self.settings.sound_enabled,
                    self.settings.sound_volume,
                    PcmBus::Sfx,
                    table_name(names, words[0])?,
                    SourceKind::Sound,
                )
            };
            let filename = decoded_name(&name)?;
            let percent = (i64::from(volume) + i64::from(words.get(2).copied().unwrap_or(0)))
                .clamp(0, 100) as i32;
            if enabled {
                let candidates = if voice {
                    // Packed VPK precedence belongs to the resource mount. A
                    // path list never pretends a missing pack entry was found.
                    ensure!(
                        name.len() >= 2 && name[..2].iter().all(u8::is_ascii),
                        "NNN voice prefix is not two ASCII bytes"
                    );
                    let prefix = std::str::from_utf8(&name[..2])?;
                    vec![
                        format!("cdvaw/{prefix}.vpk"),
                        format!("cdvaw/{prefix}/{filename}.vaw"),
                    ]
                } else {
                    vec![format!("vaw/se/{filename}.vaw")]
                };
                plan.source = Some(NativeSource {
                    kind,
                    name,
                    candidates,
                });
                plan.operation = AudioOperation::Play {
                    channel: logical,
                    parameters: AudioParameters {
                        gain: legacy_gain(percent)?,
                        looping: words.get(3).copied().unwrap_or(0) != 0,
                        bus,
                        ..AudioParameters::default()
                    },
                    repeat_count: 1,
                    spatial: s,
                    fade_in: None,
                    fade_out: None,
                };
            } else if voice {
                // The native voice loader still resolves/decodes for duration
                // before checking the switch. Caller must retain this resource
                // dependency; it is not a successful empty playback.
                bail!("NNN disabled voice requires native duration/resource preparation");
            }
        }
        plan.next = next;
        Ok(plan)
    }
}
impl NativeAudioPlan {
    fn playback(&self) -> Result<(u32, AudioParameters)> {
        let AudioOperation::Play {
            channel,
            parameters,
            repeat_count,
            spatial,
            fade_in,
            fade_out,
        } = &self.operation
        else {
            bail!("NNN audio plan has no playback")
        };
        ensure!(
            spatial.is_plain(),
            "NNN native spatial audio requires shared signed-coordinate/motion profile"
        );
        ensure!(
            *repeat_count <= 1,
            "NNN finite music repeats require shared source sequence completion"
        );
        ensure!(
            fade_in.is_none() && fade_out.is_none(),
            "NNN native100ms stepped decibel envelope requires shared audio capability"
        );
        Ok((*channel, parameters.clone()))
    }
    /// For begin_audio only: shared workers own the actual read/decoder. MML
    /// candidates must be resolved as native playlists before selecting a WGQ.
    pub fn asset_request(&self, path: VfsPath) -> Result<AudioRequest> {
        let (channel, parameters) = self.playback()?;
        Ok(AudioRequest::PlayAsset {
            channel,
            source: AudioAsset {
                path,
                frame_limit: None,
            },
            intro: None,
            parameters,
        })
    }
    pub fn encoded_request(&self,source:crysirror_core::service::EncodedAudio)->Result<AudioRequest>{
        let (channel,parameters)=self.playback()?;
        Ok(AudioRequest::Play{channel,source,intro:None,parameters})
    }
    /// An existing client-scoped shared audio resource already owns its bytes.
    pub fn resource_request(&self, resource: AudioResourceHandle) -> Result<AudioRequest> {
        let (channel, parameters) = self.playback()?;
        Ok(AudioRequest::PlayResource {
            channel,
            resource,
            parameters,
        })
    }
    pub fn control_request(&self) -> Result<Option<AudioRequest>> {
        Ok(match self.operation {
            AudioOperation::None => None,
            AudioOperation::Stop {
                channel,
                fade: None,
            } => Some(AudioRequest::Stop {
                channel,
                fade_ms: 0.,
            }),
            AudioOperation::Stop { fade: Some(_), .. } => {
                bail!("NNN native100ms stepped decibel stop requires shared audio capability")
            }
            AudioOperation::Gain { channel, percent } => Some(AudioRequest::SetGain {
                channel,
                gain: legacy_gain(percent)?,
                fade_ms: 0.,
            }),
            AudioOperation::Play { .. } => bail!("NNN playback requires resource preparation"),
        })
    }
}
