// SPDX-License-Identifier: MPL-2.0
//! Movie companions and the music-settings DC playlist used by the menu.
use crate::dc::Dc;
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AudioAsset, AudioParameters, AudioPreparePhase, AudioRequest, HostServices, PcmBus},
    state::ClientId,
};

const MOVIE: u32 = 0x30;
const MUSIC: u32 = 0x31;
const STINGER: u32 = 0x32;

#[derive(Clone, Debug)]
struct Cue {
    name: String,
    gain: f32,
}

// BCAS20097 music-settings record. The array at +0xa8 has 8-byte entries:
// relocated filename, binary16 dB, two control bytes.
fn cues(dc: &Dc, id: u32) -> Result<(u8, Vec<Cue>)> {
    let &at = dc
        .symbols
        .get(&id)
        .context("unresolved music-settings playlist")?;
    ensure!(
        dc.mem.type_of(at)? == 0x40a4_3830,
        "music-settings playlist type"
    );
    let count = dc.mem.u32(at + 0xa8)?;
    ensure!(count <= 1024, "music playlist size");
    let entries = dc.mem.u32(at + 0xac)?;
    let mode = dc.mem.bytes(at, 1)?[0];
    let cues = (0..count)
        .map(|i| {
            let entry = entries + 8 * i;
            let name = dc.mem.cstr(dc.mem.u32(entry)?)?;
            let db = crate::anim::half((dc.mem.u32(entry + 4)? >> 16) as u16);
            ensure!(db.is_finite(), "invalid music gain");
            Ok(Cue {
                name,
                gain: 10f32.powf(db / 20.).min(4.),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((mode, cues))
}

fn asset(directory: &str, name: &str) -> Result<VfsPath> {
    ensure!(
        !name.is_empty() && !name.contains(['/', '\\']),
        "invalid soundtrack name"
    );
    Ok(VfsPath::new(
        Mount::Asset,
        &format!("{}/sound1/{directory}/{name}.xvag", super::adapter::MAIN),
    )?)
}

struct Voice {
    channel: u32,
    path: VfsPath,
    ticket: Option<u64>,
    peak: f32,
}

impl Voice {
    fn start(
        host: &mut dyn HostServices,
        client: ClientId,
        channel: u32,
        path: VfsPath,
        bus: PcmBus,
        gain: f32,
    ) -> Result<Self> {
        let ticket = host.begin_audio(
            client,
            AudioRequest::PlayAsset {
                channel,
                source: AudioAsset {
                    path: path.clone(),
                    frame_limit: None,
                },
                intro: None,
                parameters: AudioParameters {
                    bus,
                    gain,
                    ..Default::default()
                },
            },
        )?;
        Ok(Self {
            channel,
            path,
            ticket: Some(ticket),
            peak: 0.,
        })
    }

    fn stop(self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        if let Some(ticket) = self.ticket {
            host.release_audio(client, ticket)?;
        }
        host.submit_audio(
            client,
            AudioRequest::Stop {
                channel: self.channel,
                fade_ms: 0.,
            },
        )?;
        Ok(())
    }

    fn finished(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<bool> {
        if let Some(ticket) = self.ticket {
            let prep = host.audio_preparation(client, ticket)?;
            if prep.phase == AudioPreparePhase::Pending {
                return Ok(false);
            }
            host.release_audio(client, ticket)?;
            self.ticket = None;
            ensure!(
                prep.phase == AudioPreparePhase::Ready,
                "audio preparation: {}",
                prep.error.unwrap_or_default()
            );
        }
        let Some(info) = host.audio_info(client, self.channel)? else {
            return Ok(true);
        };
        self.peak = self.peak.max(info.source_peak);
        Ok(!info.source_playing)
    }
}

// Selection branches in sub_319598: 1 walks the list, 2 opens on track
// zero, 3 uses the invocation counter. Random picks exclude recent tracks.
#[derive(Default)]
struct Selection {
    history: std::collections::VecDeque<usize>,
    calls: usize,
}
impl Selection {
    fn pick(&mut self, mode: u8, count: usize, random: &mut u32) -> Option<usize> {
        if count == 0 {
            return None;
        }
        let index = match mode {
            1 => self.history.front().map_or(0, |i| (i + 1) % count),
            2 if self.history.is_empty() => 0,
            3 => self.calls % count,
            _ if count == 1 => 0,
            _ => {
                let avoid = count.saturating_sub(2).min(4);
                let mut candidates: Vec<_> = (0..count).collect();
                for old in self.history.iter().take(avoid) {
                    candidates[*old] = usize::MAX;
                }
                let mut i = 0;
                while i < candidates.len() {
                    if candidates[i] == usize::MAX {
                        candidates.swap_remove(i);
                    } else {
                        i += 1;
                    }
                }
                *random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                // 31978c takes the remainder against remaining - 1 for
                // lists over two entries. Two entries draw against count.
                let range = if count > 2 {
                    candidates.len().saturating_sub(1).max(1)
                } else {
                    candidates.len()
                };
                candidates[*random as usize % range]
            }
        };
        self.calls = self.calls.wrapping_add(1);
        self.history.retain(|&old| old != index);
        self.history.push_front(index);
        self.history.truncate(4);
        Some(index)
    }
}

#[derive(Default)]
pub(crate) struct Soundtrack {
    movie: Option<Voice>,
    preroll_seconds: i32,
    music: Option<Voice>,
    stinger: Option<Voice>,
    menu_list: Option<(u8, Vec<Cue>)>,
    menu_selection: Selection,
    stinger_selections: std::collections::HashMap<u32, Selection>,
    random: u32,
}

impl Soundtrack {
    pub(crate) fn start_movie(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        name: &str,
    ) -> Result<()> {
        // The logo is an unlocalized stream. Story film mixes live beside the
        // dialogue of the active speech language (English on this disc).
        let directory = if name == "sce-ndi-logos" {
            "streams"
        } else {
            "speech/english"
        };
        let path = asset(directory, name)?;
        if self.movie.as_ref().is_some_and(|v| v.path == path) {
            return Ok(());
        }
        self.stop_movie(host, client)?;
        self.movie = Some(Voice::start(host, client, MOVIE, path, PcmBus::Sfx, 1.)?);
        Ok(())
    }

    pub(crate) fn preroll(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        name: &str,
        offset: i32,
    ) -> Result<()> {
        // Native D919C ignores a start while the movie channel is occupied.
        if self.movie.is_none() {
            self.start_movie(host, client, name)?;
            self.preroll_seconds = offset;
        }
        Ok(())
    }

    pub(crate) fn keep_movie(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        next: Option<&str>,
    ) -> Result<()> {
        if self.movie.as_ref().is_some_and(|v| {
            let stem = v
                .path
                .relative()
                .rsplit('/')
                .next()
                .and_then(|s| s.strip_suffix(".xvag"));
            stem != next
        }) {
            self.stop_movie(host, client)?;
        }
        Ok(())
    }

    // DACB4: video presentation follows elapsed audio time minus the
    // signed offset from start-movie-audio. A negative value holds frame 0.
    pub(crate) fn picture_time(&self, host: &dyn HostServices, client: ClientId) -> Result<i64> {
        if self.preroll_seconds == 0 {
            return Ok(0);
        }
        let elapsed = host.audio_info(client, MOVIE)?.map_or(0, |v| {
            v.position_frames.saturating_mul(1000) / u64::from(v.source_rate.max(1))
        });
        Ok(elapsed as i64 - 1000 * i64::from(self.preroll_seconds))
    }

    pub(crate) fn stop_movie(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
    ) -> Result<()> {
        self.preroll_seconds = 0;
        if let Some(voice) = self.movie.take() {
            voice.stop(host, client)?;
        }
        Ok(())
    }

    pub(crate) fn start_menu(&mut self, dc: &Dc, subnode: u32) -> Result<()> {
        let settings = dc.mem.u32(subnode + 0x2c)?;
        if settings != 0 {
            self.menu_list = Some(cues(dc, dc.mem.u32(settings + 0x0c)?)?);
            self.menu_selection = Selection::default();
        }
        Ok(())
    }

    pub(crate) fn stop_menu(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
    ) -> Result<()> {
        self.menu_list = None;
        if let Some(voice) = self.music.take() {
            voice.stop(host, client)?;
        }
        Ok(())
    }

    pub(crate) fn stinger(
        &mut self,
        host: &mut dyn HostServices,
        client: ClientId,
        dc: &Dc,
        id: u32,
    ) -> Result<()> {
        if id == 0 || id == crate::dc::sid("none") {
            return Ok(());
        }
        let (mode, tracks) = cues(dc, id)?;
        let Some(index) = self.stinger_selections.entry(id).or_default().pick(
            mode,
            tracks.len(),
            &mut self.random,
        ) else {
            return Ok(());
        };
        if let Some(old) = self.stinger.take() {
            old.stop(host, client)?;
        }
        let cue = &tracks[index];
        self.stinger = Some(Voice::start(
            host,
            client,
            STINGER,
            asset("music", &cue.name)?,
            PcmBus::Music,
            cue.gain,
        )?);
        Ok(())
    }

    pub(crate) fn diagnostics(&self) -> serde_json::Value {
        let voice = |v: &Option<Voice>| {
            v.as_ref().map(|v| {
                serde_json::json!({
                    "asset": v.path.relative(), "ready": v.ticket.is_none(), "peak": v.peak
                })
            })
        };
        serde_json::json!({"movie":voice(&self.movie), "music":voice(&self.music), "stinger":voice(&self.stinger)})
    }

    pub(crate) fn update(&mut self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        if let Some(movie) = &mut self.movie {
            if movie
                .finished(host, client)
                .with_context(|| movie.path.relative().to_owned())?
            {
                self.movie = None;
            }
        }
        if let Some(stinger) = &mut self.stinger {
            if stinger
                .finished(host, client)
                .with_context(|| stinger.path.relative().to_owned())?
            {
                self.stinger = None;
            }
        }
        if let Some(music) = &mut self.music {
            if !music
                .finished(host, client)
                .with_context(|| music.path.relative().to_owned())?
            {
                return Ok(());
            }
            self.music = None;
        }
        if let Some((mode, list)) = &self.menu_list {
            let Some(index) = self
                .menu_selection
                .pick(*mode, list.len(), &mut self.random)
            else {
                return Ok(());
            };
            let cue = &list[index];
            self.music = Some(Voice::start(
                host,
                client,
                MUSIC,
                asset("music", &cue.name)?,
                PcmBus::Music,
                cue.gain,
            )?);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_opens_with_first_cue_and_avoids_recent_cues() {
        let mut menu = Selection::default();
        let mut rng = 42;
        assert_eq!(menu.pick(2, 11, &mut rng), Some(0));
        for _ in 0..50 {
            let previous = menu.history.clone();
            let cue = menu.pick(2, 11, &mut rng).unwrap();
            assert!(!previous.contains(&cue));
        }
        let mut ordered = Selection::default();
        assert_eq!(
            (0..5)
                .map(|_| ordered.pick(1, 3, &mut rng).unwrap())
                .collect::<Vec<_>>(),
            [0, 1, 2, 0, 1]
        );
    }
}
