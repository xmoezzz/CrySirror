// SPDX-License-Identifier: MPL-2.0
//! Original native task-number policy. Reserved roots denote CrySirror-owned
//! input/audio services, not private implementations of either subsystem.
use anyhow::{Context, Result};
use std::collections::BTreeSet;
#[derive(Debug)]
pub struct NativeTaskPolicy {
    slots: BTreeSet<u32>,
    current: u32,
}
impl NativeTaskPolicy {
    pub fn bootstrap() -> Result<Self> {
        // Reviewed ABI: 5DBE40 constructs BASE=0; 5DD3C0 creates INPUT then
        // SOUND; 5DD580 allocates the first unused number, searching root+tree.
        let mut policy = Self {
            slots: BTreeSet::new(),
            current: 0,
        };
        let root = policy.allocate()?;
        debug_assert_eq!(root, 0);
        policy.allocate()?;
        policy.allocate()?;
        policy.current = policy.allocate()?;
        Ok(policy)
    }
    fn allocate(&mut self) -> Result<u32> {
        let id = (0..65536)
            .find(|id| !self.slots.contains(id))
            .context("CatSystem2 native task number budget")?;
        self.slots.insert(id);
        Ok(id)
    }
    pub fn current(&self) -> u32 {
        self.current
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn root_input_audio_and_script_have_native_logical_numbers() {
        let tasks = NativeTaskPolicy::bootstrap().unwrap();
        assert_eq!(tasks.current(), 3);
        assert_eq!(tasks.slots.len(), 4);
    }
}
