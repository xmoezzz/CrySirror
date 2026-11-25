// SPDX-License-Identifier: MPL-2.0
//! Game progress saves. The PS3 save data format is not reproduced (no
//! original save was examined); CrySirror keeps the progress a checkpoint
//! restores as JSON in the client's save mount: the current task subnode,
//! every subnode's completion record word (subnode +0x1c, 3 complete),
//! the player's weapons, held weapon, difficulty, statistics and the game
//! state the natives write (SATD-nd-retail-runtime).
use crate::ss::Runtime;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub const PATH: &str = "naughty-dog/uc2-progress.json";
const VERSION: u32 = 1;
const MAX_SAVE: usize = 1 << 20;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub version: u32,
    /// `(node id, subnode id)` of the current task.
    pub task: Option<(u32, u32)>,
    /// Completion record word 0 by subnode id.
    pub completion: Vec<(u32, u32)>,
    pub weapons: Vec<(u32, i32)>,
    pub current_weapon: Option<u32>,
    pub difficulty: u32,
    pub stats: Vec<((u32, u32), i32)>,
    pub journal_page: i32,
}

impl Progress {
    pub fn capture(rt: &Runtime) -> Self {
        let tasks = &rt.inner.tasks;
        let task = rt.inner.task.map(|(i, j)| (tasks.nodes[i].id, tasks.nodes[i].subnodes[j].id));
        let mut completion = Vec::new();
        for node in &tasks.nodes {
            for s in &node.subnodes {
                let word = rt.dc.mem.u32(s.address + 0x1c).ok().filter(|r| *r != 0).and_then(|r| rt.dc.mem.u32(r).ok());
                if let Some(w) = word.filter(|w| *w != 0) {
                    completion.push((s.id, w));
                }
            }
        }
        let mut stats: Vec<_> = rt.inner.stats_snapshot();
        stats.sort_unstable();
        Self {
            version: VERSION,
            task,
            completion,
            weapons: rt.inner.weapons.iter().map(|(k, v)| (*k, *v)).collect(),
            current_weapon: rt.inner.current_weapon,
            difficulty: rt.inner.difficulty,
            stats,
            journal_page: rt.inner.game.journal_page,
        }
    }

    /// Write the progress into a runtime; returns the task to start.
    pub fn restore(&self, rt: &mut Runtime) -> Result<Option<(usize, usize)>> {
        ensure!(self.version == VERSION, "save version {}", self.version);
        for node in rt.inner.tasks.nodes.clone() {
            for s in &node.subnodes {
                let Some(&(_, w)) = self.completion.iter().find(|(id, _)| *id == s.id) else { continue };
                let record = rt.dc.mem.u32(s.address + 0x1c)?;
                if record != 0 {
                    rt.dc.mem.write_u32(record, w)?;
                }
            }
        }
        rt.inner.weapons = self.weapons.iter().copied().collect();
        rt.inner.current_weapon = self.current_weapon;
        rt.inner.difficulty = self.difficulty;
        rt.inner.restore_stats(&self.stats);
        rt.inner.game.journal_page = self.journal_page;
        Ok(self.task.and_then(|(_, sub)| rt.inner.tasks.find(sub)))
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_SAVE, "save size");
        serde_json::from_slice(bytes).context("save data")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_round_trips() {
        let p = Progress { version: VERSION, task: Some((1, 2)), completion: vec![(5, 3)], weapons: vec![(7, -1)], current_weapon: Some(7), difficulty: 2, stats: vec![((1, 0), 4)], journal_page: -1 };
        assert_eq!(Progress::from_bytes(&p.to_bytes().unwrap()).unwrap(), p);
        assert!(Progress::from_bytes(b"{\"version\":1,\"bogus\":0}").is_err());
    }
}
