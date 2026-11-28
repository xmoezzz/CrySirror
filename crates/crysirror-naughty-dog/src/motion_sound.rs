// SPDX-License-Identifier: MPL-2.0
//! Player EFF cues from the ARTGROUP_DATA of proto-eff.pak. BCAS20097 1.00:
//! 0x7C624 looks up the animation StringId; 0x7C5E8–0x7C600 read its cues.
//! 0x30CBF8 dispatches explicit, contact and gear sounds. 0x30B0E8 uses the
//! mesh audio prefix/suffix and the contacted triangle's low six PAT bits.
use crate::{
    dc::{Dc, sid},
    mesh::Loaded,
};
use anyhow::{Context, Result, ensure};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Sound,
    Foot,
    Hand,
    Gear,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Cue {
    frame: f32,
    kind: Kind,
    name: u32,
    pub joint: u32,
    pub gain: f32,
    pub pitch: f32,
    water: bool,
}

#[derive(Clone, Default)]
pub(crate) struct MeshAudio {
    prefix: String,
    suffix: String,
    gear: String,
}

pub(crate) fn mesh_profiles(bytes: &[u8]) -> Result<HashMap<u32, MeshAudio>> {
    let mut dc = Dc::default();
    dc.load(sid("mesh-audio-config"), bytes)?;
    let at = *dc
        .symbols
        .get(&0x1b40_5df2)
        .context("mesh audio configuration table")?;
    ensure!(dc.mem.type_of(at)? == 0x1609_4dd2, "mesh audio table type");
    let n = dc.mem.u32(at)?;
    ensure!(n <= 4096, "mesh audio profile count");
    let entries = dc.mem.u32(at + 4)?;
    let mut meshes = HashMap::new();
    for i in 0..n {
        let key = dc.mem.u32(entries + 8 * i)?;
        let row = dc.mem.u32(entries + 8 * i + 4)?;
        ensure!(
            dc.mem.type_of(row)? == 0x918e_6e5f,
            "mesh audio profile type"
        );
        let string = |offset| dc.mem.cstr(dc.mem.u32(row + offset)?);
        meshes.insert(
            key,
            MeshAudio {
                prefix: string(4)?,
                suffix: string(8)?,
                gear: string(12)?,
            },
        );
    }
    Ok(meshes)
}

const CONTACTS: &[&str] = &[
    "vine-grab",
    "heel",
    "toe",
    "heel-stealth",
    "toe-stealth",
    "drag",
    "knee",
    "plant",
    "scuff",
    "settle",
    "slide",
    "tap",
    "turn",
    "pat",
    "brush",
    "grab",
    "roll",
    "body-impact",
    "head-impact",
    "foot-impact",
    "hand-impact",
    "pipe-grab",
];

impl Cue {
    pub fn hand(self) -> bool {
        self.kind == Kind::Hand
    }

    pub fn needs_contact(self) -> bool {
        matches!(self.kind, Kind::Foot | Kind::Hand)
    }

    pub fn sound(self, profile: Option<&MeshAudio>, pat: Option<u32>) -> Option<u32> {
        match self.kind {
            Kind::Sound => Some(self.name),
            Kind::Gear => {
                let profile = profile.filter(|p| !p.gear.is_empty())?;
                (self.name == sid("gear")).then(|| sid(&format!("sfx-{}gear", profile.gear)))
            }
            Kind::Foot | Kind::Hand => {
                let profile = profile?;
                let contact = CONTACTS.iter().find(|name| sid(name) == self.name)?;
                let surface = if self.water {
                    "water"
                } else {
                    crate::collision::surface_name(pat?)
                };
                Some(sid(&format!(
                    "sfx-{}{contact}-{surface}{}",
                    profile.prefix, profile.suffix
                )))
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct Tracks(HashMap<u32, Vec<Cue>>);

impl Tracks {
    pub fn read(p: &Loaded) -> Result<Self> {
        let item = p
            .directory
            .items
            .iter()
            .find(|i| i.kind == "ARTGROUP_DATA")
            .context("no EFF art group")?;
        let table = p.ptr(item.payload_offset as usize + 0x10)?;
        let n = p.u32(table + 4)? as usize;
        ensure!(n <= 32768, "EFF clip count");
        let base = p.ptr(table + 8)?;
        let mut tracks = HashMap::new();
        let mut total = 0usize;
        for i in 0..n {
            let row = base + 12 * i;
            let count = p.u32(row + 4)? as usize;
            if count == 0 {
                continue;
            }
            total += count;
            ensure!(total <= 1 << 18, "EFF cue count");
            let events = p.ptr(row + 8)?;
            let mut cues = Vec::new();
            for j in 0..count {
                let at = events + 16 * j;
                let kind = match p.u32(at)? {
                    v if v == sid("sound-effect") => Kind::Sound,
                    v if v == sid("foot-effect") => Kind::Foot,
                    0x8667_e9b8 | 0x1012_e99d => Kind::Hand,
                    v if v == sid("gear-effect") => Kind::Gear,
                    _ => continue,
                };
                let frame = p.f32(at + 4)?;
                ensure!(frame.is_finite(), "nonfinite EFF frame");
                let mut cue = Cue {
                    frame,
                    kind,
                    name: 0,
                    joint: 0,
                    gain: 1.,
                    pitch: 0.,
                    water: false,
                };
                // Frame is already in the fixed header. The packed count includes
                // this tag; the remaining tags are {StringId, value} pairs.
                let tags = p.u32(at + 8)? as usize;
                ensure!((1..=64).contains(&tags), "EFF tag count");
                let params = p.ptr(at + 12)?;
                let mut supported = true;
                for k in 0..tags - 1 {
                    let tag = params + 8 * k;
                    let value = p.u32(tag + 4)?;
                    match p.u32(tag)? {
                        v if v == sid("name") => cue.name = value,
                        v if v == sid("joint") => cue.joint = value,
                        v if v == sid("vol") => cue.gain = 10_f32.powf(f32::from_bits(value) / 20.),
                        v if v == sid("pitch") => cue.pitch = f32::from_bits(value),
                        v if v == sid("type") && value == sid("water") => cue.water = true,
                        _ => supported = false,
                    }
                }
                if supported && cue.name != 0 && cue.gain.is_finite() && cue.pitch.is_finite() {
                    cues.push(cue);
                }
            }
            cues.sort_by(|a, b| a.frame.total_cmp(&b.frame));
            if !cues.is_empty() {
                tracks.insert(p.u32(row)?, cues);
            }
        }
        Ok(Self(tracks))
    }
}

#[derive(Default)]
pub(crate) struct Clock(Option<(u32, u32, f32)>);

impl Clock {
    /// The authored EFF timebase is 30 frames/s, independently of the
    /// compressed clip's key rate (native effect-frame range check).
    pub fn advance(
        &mut self,
        tracks: &Tracks,
        state: u32,
        name: &str,
        seconds: f32,
        duration: f32,
        delta: f32,
        water: bool,
        mut fire: impl FnMut(Cue),
    ) {
        let clip = sid(name);
        let now = seconds.clamp(0., duration);
        let previous = self.0.replace((clip, state, now));
        let base = name
            .strip_suffix(crate::anim::MIRROR_SUFFIX)
            .unwrap_or(name);
        let Some(cues) = tracks.0.get(&sid(base)) else {
            return;
        };
        let mut range = |start: f32, end: f32| {
            let from = cues.partition_point(|c| c.frame <= start * 30.);
            for cue in cues[from..]
                .iter()
                .take_while(|c| c.frame <= end * 30.)
                .filter(|c| c.water == water)
            {
                let mut cue = *cue;
                if name != base {
                    cue.joint = mirrored_joint(cue.joint);
                }
                fire(cue);
            }
        };
        match previous {
            Some((c, s, then)) if c == clip && s == state => {
                if now < then {
                    range(then, duration);
                    range(-1., now);
                } else {
                    range(then, now);
                }
            }
            _ => range(
                if now <= delta + 0.0001 {
                    -1.
                } else {
                    now - delta
                },
                now,
            ),
        }
    }
}

fn mirrored_joint(joint: u32) -> u32 {
    for (l, r) in [
        ("l_heel", "r_heel"),
        ("l_ball", "r_ball"),
        ("l_knee", "r_knee"),
        ("l_ankle", "r_ankle"),
        ("l_palm", "r_palm"),
        ("l_wrist", "r_wrist"),
        ("l_elbow", "r_elbow"),
        ("l_shoulder", "r_shoulder"),
    ] {
        if joint == sid(l) {
            return sid(r);
        }
        if joint == sid(r) {
            return sid(l);
        }
    }
    joint
}

#[cfg(test)]
mod tests {
    use super::*;
    use crysirror_core::service::{
        AssetReader, AudioParameters, AudioRequest, EncodedAudio, PcmBus,
    };
    use crysirror_core::{state::ClientId, vfs::VfsProvider};
    use crysirror_resource::{ArchiveMount, DecodeContext, FormatRegistry, read_all};
    use std::{
        io::{Read, Seek, SeekFrom},
        sync::{Arc, Mutex},
    };

    #[test]
    fn cue_clock_fractional_frames_pause_wrap_and_mirror() {
        let cue = Cue {
            frame: 1.5,
            kind: Kind::Foot,
            name: sid("heel"),
            joint: sid("l_heel"),
            gain: 1.,
            pitch: 0.,
            water: false,
        };
        let tracks = Tracks(HashMap::from([(
            sid("run"),
            vec![
                cue,
                Cue {
                    frame: 2.,
                    water: true,
                    ..cue
                },
            ],
        )]));
        let mut clock = Clock::default();
        let mut fired = Vec::new();
        for time in [0., 0.04, 0.05, 0.05, 0.99, 0.01, 0.06] {
            clock.advance(&tracks, 1, "run@mirror", time, 1., 0.04, false, |c| {
                fired.push(c.joint)
            });
        }
        assert_eq!(fired, [sid("r_heel"); 2]);
        clock.advance(&tracks, 1, "run@mirror", 0.1, 1., 0.04, true, |c| {
            fired.push(c.joint)
        });
        assert_eq!(fired.len(), 3);
        // An already phase-locked clip starts at the current phase, without
        // replaying every marker since frame zero.
        clock.advance(&tracks, 2, "run", 0.75, 1., 0.04, false, |c| {
            fired.push(c.joint)
        });
        assert_eq!(fired.len(), 3);
    }

    #[test]
    #[ignore = "requires BCAS20097 01.00 via UNCHARTED2_ROOT"]
    fn installed_action_cues_play_nonzero_pcm() -> Result<()> {
        struct Reader(Mutex<std::fs::File>, u64);
        impl AssetReader for Reader {
            fn len(&self) -> crysirror_core::Result<u64> {
                Ok(self.1)
            }
            fn read_at(&self, pos: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
                let mut f = self.0.lock().unwrap();
                f.seek(SeekFrom::Start(pos))
                    .and_then(|_| f.read(out))
                    .map_err(|e| crysirror_core::Error::Io(e.to_string()))
            }
        }
        let root = std::path::PathBuf::from(
            std::env::var_os("UNCHARTED2_ROOT").context("UNCHARTED2_ROOT")?,
        )
        .join("PS3_GAME/USRDIR/build/main");
        let mut reg = FormatRegistry::default();
        crysirror_formats_arc::register(&mut reg)?;
        let context = DecodeContext::new(ClientId(1));
        let mount = |name: &str| -> Result<_> {
            let file = std::fs::File::open(root.join(name))?;
            let size = file.metadata()?.len();
            let (_, archive) = reg.open(
                Arc::new(Reader(Mutex::new(file), size)),
                Some("sony/psarc"),
                &context,
            )?;
            ArchiveMount::new(archive, context.clone())
        };
        let dc = mount("dc1.psarc")?;
        let profiles = mesh_profiles(&read_all(
            dc.open("mesh-audio-config.bin")?.as_ref(),
            1 << 20,
        )?)?;
        let profile = &profiles[&sid("hero-dirty-beatup")];
        assert_eq!(profile.prefix, "drake-");
        let sounds =
            crate::sound::SoundTable::parse(&read_all(dc.open("audio.bin")?.as_ref(), 2 << 20)?)?;
        let actors = mount("actor27.psarc")?;
        let tracks = Tracks::read(&Loaded::read(
            actors.open("proto-eff.pak")?.as_ref(),
            2 << 20,
        )?)?;
        let archive = mount("sound1/banks.psarc")?;
        let mut banks = crate::sound::Banks::default();
        for n in archive.list()?.into_iter().filter(|n| n.ends_with(".bnk")) {
            banks.add(crate::sound::Bank::read(&n, archive.open(&n)?)?);
        }
        for (clip, frame, effect, name) in [
            ("run-hurt-long", 1., Kind::Foot, "drake-heel-metal-sheet"),
            ("land-hard^idle", 0.5, Kind::Sound, "drake-impact-foot"),
            ("land-soft^idle", 1.5, Kind::Foot, "drake-heel-metal-sheet"),
            (
                "ledge-land-grab-2-hand-contact",
                7.,
                Kind::Hand,
                "drake-grab-metal-sheet",
            ),
            (
                "ledge-land-grab-1-hand-contact",
                4.,
                Kind::Sound,
                "imp-body-fall",
            ),
        ] {
            let mut clock = Clock::default();
            let mut events = Vec::new();
            let dt = 1. / 60.;
            clock.advance(&tracks, 1, clip, frame / 30., 3., dt, false, |c| {
                if c.kind == effect {
                    events.push(c);
                }
            });
            let cue = events
                .iter()
                .find(|c| {
                    c.sound(Some(profile), Some(22))
                        .and_then(|id| sounds.names.get(&id))
                        .is_some_and(|v| v == name)
                })
                .context(clip)?;
            let (bank, sound) = banks.playable(name).context(name)?;
            let (wave, _) = bank.wave(sound.tones[0])?;
            if name == "drake-impact-foot" {
                // Independent extended-VAG float reference (floor arithmetic
                // and unclipped predictor history). CrySirror retains the
                // existing rounded fixed-point path. Probe either side of
                // the first extended predictor, later transients and EOF.
                let pcm = &wave[44..];
                assert_eq!(pcm.len(), 6944 * 2);
                for (i, reference) in [(755, 2942), (756, 355), (757, 3238), (760, 78),
                    (1220, 1811), (1344, -2996), (2048, 7549), (6943, -72)] {
                    let actual = i16::from_le_bytes([pcm[2 * i], pcm[2 * i + 1]]);
                    assert!((i32::from(actual) - reference).abs() <= 40, "sample {i}: {actual} vs {reference}");
                }
            }
            let mut audio = crysirror_audio::channels::ChannelAudio::default();
            audio.submit(
                1,
                AudioRequest::Play {
                    channel: 1,
                    source: EncodedAudio {
                        bytes: wave.into(),
                        frame_limit: None,
                    },
                    intro: None,
                    parameters: AudioParameters {
                        bus: PcmBus::Sfx,
                        gain: cue.gain * f32::from(sound.volume) / 127.,
                        ..Default::default()
                    },
                },
            )?;
            let mut pcm = vec![[0.; 2]; 24000];
            audio.mixer.render(&mut pcm)?;
            let peak = pcm.iter().flatten().map(|v| v.abs()).fold(0., f32::max);
            assert!(peak > 0.0001, "{name} silent");
            eprintln!(
                "{clip} frame {frame} -> {name} in {}, peak {peak:.5}",
                bank.name
            );
        }
        Ok(())
    }
}
