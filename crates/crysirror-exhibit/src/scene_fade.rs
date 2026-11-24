// SPDX-License-Identifier: MPL-2.0
//! The admitted v1 START sceneFadeOut, after the title panel is retired.
use crate::sound::Audio;
use anyhow::Result;
use crysirror_core::{service::HostServices, state::ClientId};
use serde_json::json;

const SLOT: i32 = 83;
const LAYER: u32 = 256 + SLOT as u32;
const VISUAL_MS: u64 = 1200;
const STEP_MS: u64 = 100;
const AUDIO_STEPS: u64 = (VISUAL_MS + 400) / STEP_MS + 1;

pub(super) struct SceneFade {
    start: u64,
    music: Option<[u32; 2]>,
    visual_level: u8,
    music_step: u64,
}

fn visual_level(elapsed: u64) -> u8 {
    (elapsed.min(VISUAL_MS) * 16 / VISUAL_MS) as u8
}

fn music_level(initial: [u32; 2], step: u64) -> [u32; 2] {
    initial.map(|from| {
        let delta = -(i64::from(from));
        (i64::from(from) + step.min(AUDIO_STEPS) as i64 * delta / AUDIO_STEPS as i64) as u32
    })
}

impl SceneFade {
    pub fn begin(
        h: &mut dyn HostServices,
        c: ClientId,
        [w, height]: [u32; 2],
        audio: &Audio,
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
            start: h.now_ms(),
            music,
            visual_level: 0,
            music_step: 0,
        })
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
        let new_level = visual_level(elapsed);
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
        if let Some(initial) = self.music {
            let step = (elapsed / STEP_MS).min(AUDIO_STEPS);
            if step != self.music_step {
                audio.set_scene_music_level(h, c, music_level(initial, step))?;
                self.music_step = step;
            }
        }
        if self.visual_level == 16 && (self.music.is_none() || self.music_step == AUDIO_STEPS) {
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
            [0, 74, 75, 600, 1199, 1200].map(visual_level),
            [0, 0, 1, 8, 15, 16]
        );
        assert_eq!(music_level([140, 111], 1), [132, 105]);
        assert_eq!(music_level([140, 111], 16), [9, 7]);
        assert_eq!(music_level([140, 111], 17), [0, 0]);
        assert_eq!(AUDIO_STEPS, 17);
    }
}
