// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Allocation-bounded logical input delivery, independent of devices or VMs.
use crate::{Error, Result, service::InputFrame};
use alloc::{collections::VecDeque, vec::Vec};
pub const MAX_INPUT_SAMPLES: usize = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputSample {
    pub sequence: u64,
    pub time_ms: u64,
    /// pressed includes observed up->down transitions and explicit pulses.
    pub frame: InputFrame,
    /// End-of-frame releases, including an explicit press pulse not held down.
    pub released: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputReceipt {
    pub client: u32,
    pub epoch: u64,
    pub sequence: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputBatch {
    pub samples: Vec<InputSample>,
    pub receipt: Option<InputReceipt>,
}
/// Portable logical input, not hardware state. Owner supplies a fresh epoch
/// when reconstructing it so old acknowledgement receipts cannot be reused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputBufferSnapshot {
    pub schema: u32,
    pub capacity: usize,
    pub next_sequence: u64,
    pub consumed_sequence: u64,
    pub base_down: u64,
    pub observed: Option<InputFrame>,
    pub observed_time: u64,
    pub leased_through: u64,
    pub samples: Vec<InputSample>,
}
impl InputBufferSnapshot {
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 {
            return Err(Error::Invalid("input buffer schema"));
        }
        if self.capacity == 0
            || self.capacity > MAX_INPUT_SAMPLES
            || self.samples.len() > self.capacity
        {
            return Err(Error::Invalid("input buffer capacity"));
        }
        if self.consumed_sequence > self.next_sequence
            || self.leased_through < self.consumed_sequence
            || self.leased_through > self.next_sequence
            || self.next_sequence - self.consumed_sequence != self.samples.len() as u64
        {
            return Err(Error::Invalid("input buffer sequence"));
        }
        let mut down = self.base_down;
        let mut at = self.consumed_sequence;
        let mut time = 0;
        for sample in &self.samples {
            at = at
                .checked_add(1)
                .ok_or(Error::Invalid("input sequence exhausted"))?;
            let f = sample.frame;
            let expected_release = (down & !f.buttons_down) | (f.buttons_pressed & !f.buttons_down);
            if sample.sequence != at
                || sample.time_ms < time
                || sample.time_ms > self.observed_time
                || sample.released != expected_release
                || (f.buttons_down & !down & !f.buttons_pressed) != 0
            {
                return Err(Error::Invalid("input sample order/edges"));
            }
            down = f.buttons_down;
            time = sample.time_ms;
        }
        match self.observed {
            Some(f) if f.buttons_pressed == 0 && f.buttons_down == down => (),
            None if self.next_sequence == 0 && self.samples.is_empty() && self.base_down == 0 => (),
            _ => return Err(Error::Invalid("input observation snapshot")),
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputBuffer {
    client: u32,
    epoch: u64,
    state: InputBufferSnapshot,
    queue: VecDeque<InputSample>,
}
impl InputBuffer {
    pub fn new(client: u32, epoch: u64, capacity: usize) -> Result<Self> {
        Self::restore(
            client,
            epoch,
            InputBufferSnapshot {
                schema: 1,
                capacity,
                next_sequence: 0,
                consumed_sequence: 0,
                base_down: 0,
                observed: None,
                observed_time: 0,
                leased_through: 0,
                samples: Vec::new(),
            },
        )
    }
    pub fn restore(client: u32, epoch: u64, mut state: InputBufferSnapshot) -> Result<Self> {
        if client == 0 || epoch == 0 {
            return Err(Error::Invalid("input owner/epoch"));
        }
        state.validate()?;
        let queue = core::mem::take(&mut state.samples).into();
        Ok(Self {
            client,
            epoch,
            state,
            queue,
        })
    }
    fn owner(&self, client: u32) -> Result<()> {
        if client != self.client {
            Err(Error::Invalid("foreign input buffer"))
        } else {
            Ok(())
        }
    }
    pub fn client(&self) -> u32 {
        self.client
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn len(&self) -> usize {
        self.queue.len()
    }
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    pub fn capacity(&self) -> usize {
        self.state.capacity
    }
    pub fn observed_time(&self) -> u64 {
        self.state.observed_time
    }
    pub fn snapshot(&self) -> InputBufferSnapshot {
        let mut s = self.state.clone();
        s.samples = self.queue.iter().copied().collect();
        s
    }
    pub fn observed(&self) -> Option<InputFrame> {
        self.state.observed
    }
    fn plan(&self, client: u32, time_ms: u64, mut f: InputFrame) -> Result<(InputFrame, u64, u8)> {
        self.owner(client)?;
        if self.state.observed.is_some() && time_ms < self.state.observed_time {
            return Err(Error::Invalid("input clock moved backwards"));
        }
        let previous = self.state.observed.unwrap_or_default();
        f.buttons_pressed |= f.buttons_down & !previous.buttons_down;
        let released =
            (previous.buttons_down & !f.buttons_down) | (f.buttons_pressed & !f.buttons_down);
        let changed = self.state.observed.is_none()
            || f.buttons_pressed != 0
            || released != 0
            || f.pointer_x != previous.pointer_x
            || f.pointer_y != previous.pointer_y;
        let action = if !changed {
            0
        } else if self.queue.back().is_some_and(|last| {
            f.buttons_pressed == 0
                && released == 0
                && last.frame.buttons_pressed == 0
                && last.released == 0
                && last.sequence > self.state.leased_through
        }) {
            1
        } else {
            if self.queue.len() == self.state.capacity {
                return Err(Error::Limit(
                    "input buffer full; consume input before retrying",
                ));
            }
            self.state
                .next_sequence
                .checked_add(1)
                .ok_or(Error::Limit("input sequence exhausted"))?;
            2
        };
        Ok((f, released, action))
    }
    /// Constant-time, non-mutating admission for an atomic owner batch.
    pub fn preflight_push(&self, client: u32, time_ms: u64, f: InputFrame) -> Result<()> {
        self.plan(client, time_ms, f).map(|_| ())
    }
    /// Normalize atomically. Only unleased, edge-free trailing motion coalesces.
    pub fn push(&mut self, client: u32, time_ms: u64, f: InputFrame) -> Result<()> {
        let (mut frame, released, action) = self.plan(client, time_ms, f)?;
        if action == 1 {
            let last = self.queue.back_mut().unwrap();
            last.time_ms = time_ms;
            last.frame = frame;
        } else if action == 2 {
            self.state.next_sequence += 1;
            self.queue.push_back(InputSample {
                sequence: self.state.next_sequence,
                time_ms,
                frame,
                released,
            });
        }
        frame.buttons_pressed = 0;
        self.state.observed = Some(frame);
        self.state.observed_time = time_ms;
        Ok(())
    }
    /// Publishing a batch pins its samples until acknowledged. Repeated reads
    /// return the same prefix; producer motion cannot rewrite leased records.
    pub fn read(&mut self, client: u32, maximum: usize) -> Result<InputBatch> {
        self.owner(client)?;
        if maximum == 0 || maximum > MAX_INPUT_SAMPLES {
            return Err(Error::Invalid("input read count"));
        }
        let samples: Vec<_> = self.queue.iter().take(maximum).copied().collect();
        let receipt = samples.last().map(|s| InputReceipt {
            client,
            epoch: self.epoch,
            sequence: s.sequence,
        });
        if let Some(r) = receipt {
            self.state.leased_through = self.state.leased_through.max(r.sequence);
        }
        Ok(InputBatch { samples, receipt })
    }
    pub fn acknowledge(&mut self, client: u32, receipt: InputReceipt) -> Result<()> {
        self.owner(client)?;
        if receipt.client != client
            || receipt.epoch != self.epoch
            || receipt.sequence <= self.state.consumed_sequence
            || receipt.sequence > self.state.leased_through
            || !self.queue.iter().any(|s| s.sequence == receipt.sequence)
        {
            return Err(Error::Invalid("stale/foreign input receipt"));
        }
        while self
            .queue
            .front()
            .is_some_and(|s| s.sequence <= receipt.sequence)
        {
            let s = self.queue.pop_front().unwrap();
            self.state.base_down = s.frame.buttons_down;
            self.state.consumed_sequence = s.sequence;
        }
        Ok(())
    }
    /// Discard queued logical intents but keep the observed held baseline;
    /// loading a native file must not replay pre-load clicks into the new scene.
    pub fn discard(&mut self, client: u32) -> Result<()> {
        self.owner(client)?;
        self.queue.clear();
        self.state.consumed_sequence = self.state.next_sequence;
        self.state.leased_through = self.state.next_sequence;
        self.state.base_down = self.state.observed.map_or(0, |f| f.buttons_down);
        Ok(())
    }
}
