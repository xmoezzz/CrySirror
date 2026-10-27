// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Ephemeral native operation ordering across audio categories. No mixer/DSP.
use crate::bgm_bridge::Operation;
use anyhow::{Context, Result, ensure};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
#[derive(Default, Clone)]
pub(crate) struct Queue(Vec<(u64, Operation)>);
impl Queue {
    pub(crate) fn push(&mut self, op: Operation) -> Result<()> {
        ensure!(
            self.0.len() < 128,
            "native audio operation batch exceeds 128"
        );
        let order = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .ok()
            .context("native audio order exhausted")?;
        self.0.push((order, op));
        Ok(())
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn entries(self) -> Vec<(u64, Operation)> {
        self.0
    }
}
