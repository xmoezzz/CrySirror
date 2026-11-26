// SPDX-License-Identifier: MPL-2.0
//! Player melee combos from `player-combos.bin`.
//!
//! A combo record (`player-default-combo`, type `0xb1419e72`) names its
//! stage arrays by `StringId`: +0x00 intro, +0x04 intro-close, +0x08
//! intro-reverse, +0x10 sync, +0x14 hard-hit, +0x18 hard-finish. Each stage
//! is a module entry `{count, entries*}` (`array-combo-entry`) of 108-byte
//! entries: +0x00 the hero clip, +0x04 the hero's following idle (finishers
//! only), +0x0c the NPC's paired clip.
//!
//! The press windows, the entry choice and the stage order beyond
//! intro → sync → hard-hit → hard-finish are not decoded (the native combo
//! controller reads the entries' timing words); the host advances one
//! stage per press (SATD-nd-retail-runtime).
use crate::dc::{Dc, sid};
use anyhow::{Context, Result, ensure};

const ENTRY: u32 = 108;

/// A paired strike: the player's clip and the NPC's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strike {
    pub hero: u32,
    pub npc: u32,
}

/// The stages in press order: intro, sync, hard-hit, hard-finish.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Combo {
    pub stages: Vec<Vec<Strike>>,
}

/// Seconds after a strike before the next press counts, and after which
/// the combo lapses (not native).
pub const PRESS_AFTER: f64 = 0.35;
pub const LAPSE: f64 = 1.6;

impl Combo {
    /// Read `player-default-combo`.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut dc = Dc::default();
        dc.load(sid("player-combos"), bytes)?;
        let m = dc.module(sid("player-combos")).context("player-combos module")?;
        let entry = |name: u32| m.entries.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
        let record = entry(sid("player-default-combo")).context("player-default-combo")?;
        let mut stages = Vec::new();
        for field in [0x00, 0x10, 0x14, 0x18] {
            let Some(array) = entry(dc.mem.u32(record + field)?) else { continue };
            let (count, items) = (dc.mem.u32(array)?, dc.mem.u32(array + 4)?);
            ensure!(count < 64, "combo stage size");
            let strikes = (0..count).map(|k| Ok(Strike { hero: dc.mem.u32(items + ENTRY * k)?, npc: dc.mem.u32(items + ENTRY * k + 0xc)? })).collect::<Result<Vec<_>>>()?;
            if !strikes.is_empty() {
                stages.push(strikes);
            }
        }
        ensure!(!stages.is_empty(), "player-default-combo has no stages");
        Ok(Self { stages })
    }

    /// The stage a press at `now` starts against `target`, given the
    /// running combo: the next stage within the window, else the intro;
    /// `None` when pressed too soon after the last strike.
    pub fn next_stage(&self, running: Option<&ComboState>, target: u32, now: f64) -> Option<usize> {
        match running {
            Some(s) if s.target == target && now - s.at < PRESS_AFTER => None,
            Some(s) if s.target == target && now - s.at <= LAPSE && s.stage + 1 < self.stages.len() => Some(s.stage + 1),
            _ => Some(0),
        }
    }

    pub fn is_finish(&self, stage: usize) -> bool {
        stage + 1 == self.stages.len()
    }
}

/// The running combo: its target, stage and the time of the last strike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComboState {
    pub target: u32,
    pub stage: usize,
    pub at: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_advance_and_lapse() {
        let s = Strike { hero: 1, npc: 2 };
        let c = Combo { stages: vec![vec![s]; 4] };
        assert_eq!(c.next_stage(None, 7, 0.), Some(0));
        let run = ComboState { target: 7, stage: 0, at: 0. };
        assert_eq!(c.next_stage(Some(&run), 7, 0.1), None);
        assert_eq!(c.next_stage(Some(&run), 7, 0.5), Some(1));
        assert_eq!(c.next_stage(Some(&run), 8, 0.5), Some(0));
        assert_eq!(c.next_stage(Some(&run), 7, 2.), Some(0));
        let last = ComboState { target: 7, stage: 3, at: 0. };
        assert_eq!(c.next_stage(Some(&last), 7, 0.5), Some(0));
        assert!(c.is_finish(3));
    }
}
