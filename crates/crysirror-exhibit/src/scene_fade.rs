// SPDX-License-Identifier: MPL-2.0
//! V1 black scene fades with an independently stepped music lane.
use crate::sound::Audio;
use anyhow::Result;
use crysirror_core::{service::HostServices, state::ClientId};
use serde_json::json;

const SLOT: i32 = 83;
const LAYER: u32 = 256 + SLOT as u32;
const STEP_MS: u64 = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    Title,
    ClosePrinter,
}

impl Kind {
    fn duration(self) -> u64 {
        match self {
            Self::Title => 1200,
            Self::ClosePrinter => 800,
        }
    }

    fn audio_steps(self) -> u64 {
        (self.duration() + 400) / STEP_MS + 1
    }
}

pub(super) struct SceneFade {
    kind: Kind,
    start: u64,
    music: Option<[u32; 2]>,
    visual_level: u8,
    music_step: u64,
}

fn visual_level(elapsed: u64, duration: u64) -> u8 {
    (elapsed.min(duration) * 16 / duration) as u8
}

fn music_level(initial: [u32; 2], step: u64, steps: u64) -> [u32; 2] {
    initial.map(|from| {
        let delta = -(i64::from(from));
        (i64::from(from) + step.min(steps) as i64 * delta / steps as i64) as u32
    })
}

impl SceneFade {
    pub fn begin(
        h: &mut dyn HostServices,
        c: ClientId,
        [w, height]: [u32; 2],
        audio: &Audio,
        kind: Kind,
    ) -> Result<Self> {
        let music = audio.scene_music_level(h, c)?;
        h.submit_engine(
            c,
            1,
            &serde_json::to_vec(&json!({"layers":[{"op":"put","layer":{
                "id":LAYER,"order":[2,SLOT],"rect":[0,0,w,height],
                "paint":{"kind":"solid","color":[0,0,0,255]},"opacity":0
            }}]}))?,
        )?;
        Ok(Self {
            kind,
            start: h.now_ms(),
            music,
            visual_level: 0,
            music_step: 0,
        })
    }

    pub fn closes_printer(&self, now: u64) -> bool {
        self.kind == Kind::ClosePrinter
            && self.visual_level < 16
            && now.saturating_sub(self.start) >= self.kind.duration()
    }

    pub fn present(
        &mut self,
        h: &mut dyn HostServices,
        c: ClientId,
        [w, height]: [u32; 2],
        audio: &mut Audio,
        draw: &mut [i32; 19],
    ) -> Result<bool> {
        let elapsed = h.now_ms().saturating_sub(self.start);
        let new_level = visual_level(elapsed, self.kind.duration());
        if new_level != self.visual_level {
            if new_level == 16 {
                // The native fade-in is immediate: replace the scene draw
                // with a black solid first, then remove the temporary cover.
                h.submit_engine(
                    c,
                    1,
                    &serde_json::to_vec(&json!({"layers":[
                        {"op":"put","layer":{"id":1,"order":[0,0],"rect":[0,0,w,height],
                            "paint":{"kind":"solid","color":[0,0,0,255]}}},
                        {"op":"remove","id":LAYER}
                    ]}))?,
                )?;
                draw.fill(0);
                draw[0] = i32::MIN; // high-bit solid color with zero RGB
                draw[8] = -1;
                draw[9] = -1;
                draw[11] = (w | (height << 16)) as i32;
            } else {
                h.submit_engine(
                    c,
                    1,
                    &serde_json::to_vec(&json!({"layers":[
                        {"op":"set_opacity","id":LAYER,"opacity":16 * new_level}
                    ]}))?,
                )?;
            }
            self.visual_level = new_level;
        }
        let steps = self.kind.audio_steps();
        if let Some(initial) = self.music {
            let step = (elapsed / STEP_MS).min(steps);
            if step != self.music_step {
                audio.set_scene_music_level(h, c, music_level(initial, step, steps))?;
                self.music_step = step;
            }
        }
        if self.visual_level == 16 && (self.music.is_none() || self.music_step == steps) {
            audio.stop_scene_music(h, c)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_shade_and_audio_steps_are_independent() {
        assert_eq!(
            [0, 74, 75, 600, 1199, 1200].map(|time| visual_level(time, Kind::Title.duration())),
            [0, 0, 1, 8, 15, 16]
        );
        let steps = Kind::Title.audio_steps();
        assert_eq!(music_level([140, 111], 1, steps), [132, 105]);
        assert_eq!(music_level([140, 111], 16, steps), [9, 7]);
        assert_eq!(music_level([140, 111], 17, steps), [0, 0]);
        assert_eq!(steps, 17);
        let prologue = SceneFade {
            kind: Kind::ClosePrinter,
            start: 40,
            music: Some([140, 111]),
            visual_level: 15,
            music_step: 7,
        };
        assert_eq!(visual_level(799, Kind::ClosePrinter.duration()), 15);
        assert!(!prologue.closes_printer(839));
        assert!(prologue.closes_printer(840));
        assert_eq!(Kind::ClosePrinter.audio_steps(), 13);
        assert_eq!(music_level([140, 111], 8, 13), [54, 43]);
    }
}
