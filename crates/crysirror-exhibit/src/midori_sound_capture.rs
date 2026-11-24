// SPDX-License-Identifier: MPL-2.0
//! Midori 2011 native sound-scene register image, without checkpoint mutation.
use crate::{midori_ambient::Ambient, midori_music::Music};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Lane {
    pub id: i32,
    levels: [i32; 2],
    flags: u32,
}

impl Default for Lane {
    fn default() -> Self {
        // SoundInfo lane constructor (0x69210), not the explicit stop shape.
        Self {
            id: -1,
            levels: [214; 2],
            flags: 0,
        }
    }
}

impl Lane {
    pub fn music(id: i32, levels: [i32; 2]) -> Self {
        Self {
            id,
            levels,
            flags: 0x41,
        }
    }

    pub fn stop_music() -> Self {
        Self {
            id: -1,
            levels: [0; 2],
            flags: 0x40,
        }
    }

    pub fn ambience(id: i32, levels: [i32; 2]) -> Self {
        Self {
            id,
            levels,
            flags: 0x200,
        }
    }

    pub fn stop_ambience() -> Self {
        Self::ambience(-1, [-1; 2])
    }

    fn append_to(self, out: &mut String) {
        let [left, right] = self.levels;
        // 0x68350 emits record version 0, then the id, the run of two
        // dot-separated authored levels, flags, and a zero extra-list count.
        // The admitted BGM/ambient methods do not install an extra list.
        out.push_str(&format!(";0,{},2.{left}.{right},{},0", self.id, self.flags));
    }
}

/// The audio portion of native blockEntry's register image. No caller applies
/// it until every other native capture owner is also available.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SoundCapture {
    pub normal: (u32, i32),
    pub text: (u32, String),
}

impl SoundCapture {
    pub fn from_owners(music: &Music, ambience: &Ambient) -> Option<Self> {
        // Native initScenario (0x143470) chooses R61. The writer (0x68750)
        // stores five SoundInfo lanes at T204 in their fixed layout. Loop SE,
        // environment voice and auxiliary lane are still unadmitted and
        // retain their actual initial descriptor, not synthesized silence.
        let lanes = [
            music.capture_lane()?,
            Lane::default(),
            ambience.capture_lane()?,
            Lane::default(),
            Lane::default(),
        ];
        let mut value = "5".to_owned();
        for lane in lanes {
            lane.append_to(&mut value);
        }
        Some(Self {
            normal: (61, 0x80_0000),
            text: (204, value),
        })
    }
}
