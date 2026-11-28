// SPDX-License-Identifier: MPL-2.0
//! `SoundMgr`: BGM, environment, SE and system sounds from the game archives
//! through shared channel audio. Default volumes follow `SoundMgr.Init`
//! (`SetVolumeAll(50)`, BGM 30, SE 60, System 60, Env 30, Voice 100), as
//! `AudioListener.volume × AudioSource.volume`.
use crate::files::GameFiles;
use anyhow::{bail, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, HostServices},
    state::ClientId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bgm,
    Env,
    Se,
    System,
    Voice,
}

impl Kind {
    pub(crate) fn channel(self) -> u32 {
        match self {
            Kind::Bgm => 0,
            Kind::Env => 1,
            Kind::Se => 2,
            Kind::System => 3,
            Kind::Voice => 4,
        }
    }
    /// `SoundMgr.Init` per-type volume, percent.
    fn volume(self) -> f32 {
        match self {
            Kind::Bgm | Kind::Env => 30.,
            Kind::Se | Kind::System => 60.,
            Kind::Voice => 100.,
        }
    }
}

/// `SoundMgr.SeType` in enum order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeType {
    Click,
    IClick,
    Cancel,
    Hover,
}

/// `UIButton.PlaySoundClick`: `SeType` value or the `Self` file.
pub fn button_click(se: i64, own_file: &str) -> Option<String> {
    Some(match se {
        0 => system_file(SeType::Click).into(),
        1 => system_file(SeType::IClick).into(),
        2 => system_file(SeType::Cancel).into(),
        3 => system_file(SeType::Hover).into(),
        4 if !own_file.is_empty() => own_file.into(),
        _ => return None,
    })
}

/// `SoundMgr.m_arySeFileName`, indexed by `SeType`.
pub fn system_file(se: SeType) -> &'static str {
    ["SE001.ogg", "SE002.ogg", "SE003.ogg", "SE004.ogg"][se as usize]
}

#[derive(Default)]
pub struct Sound {
    pending: Vec<(Kind, String, u64)>,
    pub all_volume: f32,
    /// `SoundMgr` per-type volumes (percent), indexed by `Kind`.
    pub volumes: [f32; 5],
    pub current_bgm: Option<String>,
    bgm_legacy: bool,
    pub dance_volume: f32,
    pub special_se: bool,
    spatial: [bool; 5],
    voice_position: [f32; 3],
}

impl Sound {
    pub fn new() -> Self {
        let volumes = [Kind::Bgm, Kind::Env, Kind::Se, Kind::System, Kind::Voice].map(Kind::volume);
        Self { pending: Vec::new(), all_volume: 50., volumes, current_bgm: None, bgm_legacy:false, dance_volume: 70., special_se: true, spatial: [false; 5], voice_position: [0.; 3] }
    }

    pub fn standalone() -> Self {
        let mut sound = Self::new();
        sound.all_volume = 80.;
        sound.volumes = [35., 50., 60., 50., 80.];
        sound.spatial = [false, true, true, false, true];
        sound
    }

    pub fn spatial_enabled(&self, kind: Kind) -> bool {
        self.spatial[kind as usize]
    }

    fn spatial_position(&self, kind: Kind) -> Option<[f32; 3]> {
        self.spatial_enabled(kind).then_some(if kind == Kind::Voice { self.voice_position } else { [0.; 3] })
    }

    fn apply_spatial(&self, host: &mut dyn HostServices, client: ClientId, kind: Kind) -> Result<()> {
        host.submit_audio(client, AudioRequest::SetChannelSpatial {
            channel: kind.channel(), position: self.spatial_position(kind),
        })?;
        Ok(())
    }

    pub fn set_spatial_mode(&mut self, host: &mut dyn HostServices, client: ClientId, kind: Kind, enabled: bool) -> Result<()> {
        self.spatial[kind as usize] = enabled;
        if !self.pending.iter().any(|p| p.0 == kind) {
            self.apply_spatial(host, client, kind)?;
        }
        Ok(())
    }

    pub fn set_voice_position(&mut self, host: &mut dyn HostServices, client: ClientId, position: [f32; 3]) -> Result<()> {
        if position != self.voice_position {
            self.voice_position = position;
            if self.spatial_enabled(Kind::Voice) && !self.pending.iter().any(|p| p.0 == Kind::Voice) {
                self.apply_spatial(host, client, Kind::Voice)?;
            }
        }
        Ok(())
    }

    pub fn voice_busy(&self, host: &dyn HostServices, client: ClientId) -> Result<bool> {
        Ok(self.pending.iter().any(|p| p.0 == Kind::Voice) || host.audio_playing(client, Kind::Voice.channel())?)
    }

    pub fn set_dance_volume(&mut self, host: &mut dyn HostServices, client: ClientId, volume: f32) -> Result<()> {
        host.submit_audio(client, AudioRequest::SetMixGroupGain {
            name: "com3d2-dance".into(),
            gain: volume / 100.,
            fade_ms: 0.,
        })?;
        self.dance_volume = volume;
        Ok(())
    }

    pub fn play(&mut self, host: &mut dyn HostServices, client: ClientId, files: &GameFiles, kind: Kind, name: &str, fade_s: f32, looping: bool) -> Result<()> {
        self.play_profile(host,client,files,kind,name,fade_s,looping,false)
    }

    pub(crate) fn play_profile(&mut self, host:&mut dyn HostServices, client:ClientId,files:&GameFiles,kind:Kind,name:&str,fade_s:f32,looping:bool,legacy:bool)->Result<()>{
        let path = files.path(name).with_context(|| format!("SoundMgr: {name} is not in the archives"))?;
        if kind == Kind::Bgm {
            if self.bgm_legacy==legacy && self.current_bgm.as_deref().is_some_and(|b| b.eq_ignore_ascii_case(name)) {
                return Ok(());
            }
            self.bgm_legacy=legacy;
            self.current_bgm = Some(name.to_owned());
        }
        let parameters = AudioParameters {
            gain: self.gain(kind),
            spatial_position: self.spatial_position(kind),
            decode_gain: if legacy { match kind { Kind::Bgm=>0.3,Kind::Se=>0.8,Kind::System=>0.5,Kind::Env=>0.15,Kind::Voice=>2.0 } } else {1.0},
            looping,
            ..Default::default()
        };
        let _ = fade_s;
        let ticket = host.begin_audio(
            client,
            AudioRequest::PlayAsset {
                channel: kind.channel(),
                source: AudioAsset { path: VfsPath::new(Mount::Asset, path)?, frame_limit: None },
                intro: None,
                parameters,
            },
        )?;
        self.clear_pending(host, client, kind)?;
        self.pending.push((kind, name.to_owned(), ticket));
        Ok(())
    }

    fn gain(&self, kind: Kind) -> f32 {
        self.all_volume / 100. * self.volumes[kind as usize] / 100.
    }

    /// `SoundMgr.SetVolumeAll` (`kind` None) / `SetVolume`; playing
    /// channels follow at once.
    pub fn set_volume(&mut self, host: &mut dyn HostServices, client: ClientId, kind: Option<Kind>, value: f32) -> Result<()> {
        let value = value.clamp(0., 100.);
        match kind {
            None => self.all_volume = value,
            Some(k) => self.volumes[k as usize] = value,
        }
        for k in [Kind::Bgm, Kind::Env, Kind::Se, Kind::System, Kind::Voice] {
            if kind.is_none_or(|x| x == k) && !self.pending.iter().any(|p| p.0 == k) {
                host.submit_audio(client, AudioRequest::SetGain { channel: k.channel(), gain: self.gain(k), fade_ms: 0. })?;
            }
        }
        Ok(())
    }

    fn clear_pending(&mut self, host: &mut dyn HostServices, client: ClientId, kind: Kind) -> Result<()> {
        if let Some(index) = self.pending.iter().position(|p| p.0 == kind) {
            host.release_audio(client, self.pending[index].2)?;
            self.pending.swap_remove(index);
        }
        Ok(())
    }

    pub fn stop(&mut self, host: &mut dyn HostServices, client: ClientId, kind: Kind, fade_s: f32) -> Result<()> {
        if kind == Kind::Bgm {
            self.current_bgm = None;
        }
        self.clear_pending(host, client, kind)?;
        host.submit_audio(client, AudioRequest::Stop { channel: kind.channel(), fade_ms: fade_s.max(0.) * 1000. })?;
        Ok(())
    }

    pub(crate) fn stop_all(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        for kind in [Kind::Bgm, Kind::System, Kind::Se, Kind::Voice, Kind::Env] {
            self.stop(host, client, kind, 0.)?;
        }
        while let Some(&(_, _, ticket)) = self.pending.last() {
            host.release_audio(client, ticket)?;
            self.pending.pop();
        }
        Ok(())
    }

    /// Consume preparation receipts; a failed decode is a visible error.
    pub fn poll(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        let mut keep = Vec::new();
        for (kind, name, ticket) in std::mem::take(&mut self.pending) {
            let p = host.audio_preparation(client, ticket)?;
            match p.phase {
                AudioPreparePhase::Pending => keep.push((kind, name, ticket)),
                AudioPreparePhase::Ready => {
                    self.apply_spatial(host, client, kind)?;
                    host.submit_audio(client, AudioRequest::SetGain {
                        channel: kind.channel(), gain: self.gain(kind), fade_ms: 0.,
                    })?;
                    host.release_audio(client, ticket)?;
                }
                AudioPreparePhase::Failed => bail!("SoundMgr {kind:?} {name}: {}", p.error.unwrap_or_default()),
            }
        }
        self.pending = keep;
        Ok(())
    }
}

/// A `SoundMgr` call made by scene or KAG logic, executed by the adapter.
#[derive(Clone, Debug, PartialEq)]
pub enum SoundOp {
    Play { kind: Kind, file: String, fade_s: f32, looping: bool },
    Stop { kind: Kind, fade_s: f32 },
    /// `SoundMgr.SetVolumeAll` (`kind` None) / `SetVolume`, percent.
    Volume { kind: Option<Kind>, value: f32 },
}

/// `Path.ChangeExtension(name, ".ogg")`.
pub fn ogg(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => format!("{stem}.ogg"),
        _ => format!("{name}.ogg"),
    }
}
