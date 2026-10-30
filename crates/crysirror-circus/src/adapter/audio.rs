//! Native audio names/loop metadata mapped onto the shared decoder and mixer.
use super::*;
use crate::machine::{MixerControls, MusicAsset};
const GROUPS: [&str; 3] = ["circus/music", "circus/sfx", "circus/voice"];

pub(super) fn load_music_loops(storage: &dyn ClientStorage) -> Result<BTreeMap<String, [u64; 2]>> {
    // The loop table is native data, not code.43C390 converts these decoded
    // frame indices into byte offsets with channels*bits_per_sample/8.
    const BUILD_SHA: &str = "178ee9a5e34356b11942213e2e32a34afe4933c90d563bf5a3d6402250f54ab7";
    let candidates = storage
        .list(Mount::Asset, "")?
        .into_iter()
        .filter(|n| n.to_ascii_lowercase().ends_with(".exe"))
        .collect::<Vec<_>>();
    ensure!(
        candidates.len() <= 32,
        "Circus native metadata executable candidate limit"
    );
    let mut remaining = crysirror_resource::pe::MAX_PE_BYTES as u64;
    for name in candidates {
        let path = VfsPath::new(Mount::Asset, &name)?;
        let metadata = storage
            .metadata(Mount::Asset, path.relative())?
            .context("Circus native metadata file missing")?;
        if metadata.is_directory {
            continue;
        }
        let size = metadata
            .size
            .context("Circus native metadata file size unavailable")?;
        ensure!(size <= remaining, "Circus native metadata read budget");
        remaining -= size;
        let reader = storage.open_read(&path)?;
        let bytes = crysirror_resource::read_all(reader.as_ref(), size)?;
        let Ok(image) = crysirror_resource::pe::PeData::parse(&bytes) else {
            continue;
        };
        let digest = image
            .sha256()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if digest != BUILD_SHA {
            continue;
        }
        let mut records = BTreeMap::new();
        for index in 0..256u64 {
            let row = image.va_slice(0x4b4ba0 + index * 24, 24)?;
            if row[0] == 0 {
                return Ok(records);
            }
            let end = row[..16]
                .iter()
                .position(|&b| b == 0)
                .context("Circus native music loop name terminator")?;
            let name = std::str::from_utf8(&row[..end])?;
            ensure!(
                name.is_ascii()
                    && !name.contains(['\\', '/', ':'])
                    && !name.chars().any(char::is_control),
                "Circus native loop resource basename"
            );
            let start = u64::from(u32::from_le_bytes(row[16..20].try_into()?));
            let end = u64::from(u32::from_le_bytes(row[20..24].try_into()?));
            ensure!(end == 0 || start < end, "Circus native loop frame interval");
            ensure!(
                records
                    .insert(name.to_ascii_uppercase(), [start, end])
                    .is_none(),
                "Circus native loop resource duplicate"
            );
        }
        bail!("Circus native music loop metadata lacks terminator");
    }
    bail!("Circus native music loop profile requires a recognized original executable as read-only data")
}
impl Bridge<'_> {
    pub(super) fn sync_audio_controls(&mut self, controls: MixerControls) -> Result<()> {
        if self.services.mixer_controls == Some(controls) {
            return Ok(());
        }
        for index in 0..3 {
            if self
                .services
                .mixer_controls
                .is_none_or(|old| old.gain[index] != controls.gain[index])
            {
                self.host.submit_audio(
                    self.client,
                    AudioRequest::SetMixGroupGain {
                        name: GROUPS[index].into(),
                        gain: controls.gain[index],
                        fade_ms: 0.,
                    },
                )?;
            }
            if self
                .services
                .mixer_controls
                .is_some_and(|old| old.enabled[index] && !controls.enabled[index])
                && index != 1
            {
                let channels = if index == 0 { 0..=0 } else { 8..=11 };
                for channel in channels {
                    self.host.submit_audio(
                        self.client,
                        AudioRequest::Stop {
                            channel,
                            fade_ms: 0.,
                        },
                    )?;
                }
            }
        }
        self.services.mixer_controls = Some(controls);
        Ok(())
    }
    pub(super) fn native_music(&mut self, id: u16, looping: bool) -> Result<Option<MusicAsset>> {
        if self.services.names.is_none() {
            let Some(Prepared::Names(names)) = self.services.prepare(Work::Names, self.client)?
            else {
                return Ok(None);
            };
            self.services.names = Some(names);
        }
        let mut name = self
            .services
            .names
            .as_ref()
            .unwrap()
            .music
            .get(usize::from(id))
            .context("Circus native music index")?
            .clone();
        if !name.to_ascii_lowercase().ends_with(".pcm") {
            name.push_str(".PCM");
        }
        let mut asset = MusicAsset {
            name,
            loop_start: 0,
            loop_end: None,
        };
        if looping {
            if self.services.music_loops.is_none() {
                let Some(Prepared::MusicLoops(records)) =
                    self.services.prepare(Work::MusicLoops, self.client)?
                else {
                    return Ok(None);
                };
                self.services.music_loops = Some(records);
            }
            let basename = asset
                .name
                .rsplit(['\\', '/'])
                .next()
                .context("Circus music resource basename")?
                .to_ascii_uppercase();
            if let Some([start, end]) = self.services.music_loops.as_ref().unwrap().get(&basename) {
                asset.loop_start = *start;
                asset.loop_end = (*end != 0).then_some(*end);
            }
        }
        Ok(Some(asset))
    }
    pub(super) fn play_audio(
        &mut self,
        channel: u32,
        name: &str,
        voice: bool,
        looping: bool,
        loop_start: u64,
        loop_end: Option<u64>,
    ) -> Result<bool> {
        let kind = if voice {
            2
        } else if channel == 0 {
            0
        } else {
            1
        };
        if !self
            .services
            .mixer_controls
            .context("Circus native mixer controls missing")?
            .enabled[kind]
        {
            return Ok(true);
        }
        if let Some((pending_channel, pending_name, ticket)) = self.services.audio.as_ref() {
            ensure!(
                *pending_channel == channel && pending_name == name,
                "Circus audio completion ownership"
            );
            let ticket = *ticket;
            let prepared = self.host.audio_preparation(self.client, ticket)?;
            match prepared.phase {
                AudioPreparePhase::Pending => return Ok(false),
                AudioPreparePhase::Failed => bail!(
                    "Circus audio preparation: {}",
                    prepared.error.unwrap_or("unknown error".into())
                ),
                AudioPreparePhase::Ready => {
                    self.host.release_audio(self.client, ticket)?;
                    self.services.audio = None;
                    return Ok(true);
                }
            }
        }
        let Some(Prepared::Audio(bytes, archive)) = self
            .services
            .prepare(Work::Audio(name.into(), voice), self.client)?
        else {
            return Ok(false);
        };
        if let Some(archive) = archive {
            self.services.voice_archive = Some(archive);
        }
        let ticket = self.host.begin_audio(
            self.client,
            AudioRequest::Play {
                channel,
                source: EncodedAudio {
                    bytes,
                    frame_limit: None,
                },
                intro: None,
                parameters: AudioParameters {
                    bus: if voice {
                        PcmBus::Voice
                    } else if channel == 0 {
                        PcmBus::Music
                    } else {
                        PcmBus::Sfx
                    },
                    mix_group: Some(GROUPS[kind].into()),
                    looping,
                    loop_start,
                    loop_end,
                    ..Default::default()
                },
            },
        )?;
        self.services.audio = Some((channel, name.into(), ticket));
        Ok(false)
    }
}
