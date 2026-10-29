// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded owner-thread notifications. Producers post logical targets; the
//! embedding invokes callbacks. A captured fence excludes recursive new posts.
use crate::{Error, Result};
use alloc::collections::VecDeque;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchPhase {
    Exclusive,
    Normal,
    Idle,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Event {
    client: u32,
    target: u64,
    serial: u64,
    phase: DispatchPhase,
}
#[derive(Clone, Debug)]
pub struct DeferredDispatch {
    capacity: usize,
    serial: u64,
    queue: VecDeque<Event>,
}
impl DeferredDispatch {
    pub fn new(capacity: usize) -> Result<Self> {
        if capacity == 0 || capacity > 4096 {
            return Err(Error::Invalid("deferred dispatch capacity"));
        }
        Ok(Self {
            capacity,
            serial: 0,
            queue: VecDeque::new(),
        })
    }
    pub fn len(&self) -> usize {
        self.queue.len()
    }
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    /// Captures one owner turn. Callback-generated posts await a later fence.
    pub fn fence(&self) -> u64 {
        self.serial
    }
    /// A cached notification replaces pending notifications for this target.
    /// Admission failure leaves existing notifications intact.
    pub fn post(
        &mut self,
        client: u32,
        target: u64,
        phase: DispatchPhase,
        cached: bool,
    ) -> Result<()> {
        if client == 0 || target == 0 {
            return Err(Error::Invalid("deferred dispatch identity"));
        }
        let replace = cached
            && self
                .queue
                .iter()
                .any(|e| e.client == client && e.target == target);
        if self.queue.len() >= self.capacity && !replace {
            return Err(Error::Limit("deferred dispatch full"));
        }
        let serial = self
            .serial
            .checked_add(1)
            .ok_or(Error::Limit("deferred dispatch exhausted"))?;
        if cached {
            self.cancel(client, target);
        }
        self.queue.push_back(Event {
            client,
            target,
            serial,
            phase,
        });
        self.serial = serial;
        Ok(())
    }
    /// Targets still queued for `client` (owners use them as liveness roots).
    pub fn targets(&self, client: u32) -> impl Iterator<Item = u64> + '_ {
        self.queue.iter().filter(move |e| e.client == client).map(|e| e.target)
    }
    pub fn cancel(&mut self, client: u32, target: u64) -> usize {
        let n = self.queue.len();
        self.queue
            .retain(|e| e.client != client || e.target != target);
        n - self.queue.len()
    }
    pub fn pop(&mut self, client: u32, fence: u64, phase: DispatchPhase) -> Option<u64> {
        let i = self
            .queue
            .iter()
            .position(|e| e.client == client && e.serial <= fence && e.phase == phase)?;
        self.queue.remove(i).map(|e| e.target)
    }
}
