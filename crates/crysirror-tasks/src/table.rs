// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Owner-polled, generation-scoped results shared by engine services.
use crate::{Cancellation, Task, TaskPool};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
/// Shared numeric namespace for service-operation tickets.
pub fn next_operation_id() -> Result<u64> {
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| anyhow::anyhow!("operation IDs exhausted"))
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Completion of a bounded service job, observed without blocking the owner.
pub enum OperationState {
    Pending,
    Ready,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
/// A client-scoped ticket belongs to the restore generation that created it.
pub struct OperationStatus {
    /// Opaque operation ID; another client cannot use it.
    pub ticket: u64,
    /// Service-defined operation kind.
    pub kind: &'static str,
    /// Pending, ready or failed; no implicit wait.
    pub state: OperationState,
    /// Failure text, retained until the ticket is released.
    pub error: Option<String>,
}
struct Entry<T> {
    owner: u32,
    generation: u64,
    kind: &'static str,
    pending: Option<Task<T>>,
    result: Option<T>,
    error: Option<String>,
}
pub struct OperationTable<T> {
    entries: BTreeMap<u64, Entry<T>>,
    per_owner: usize,
    total: usize,
}
impl<T> Default for OperationTable<T> {
    fn default() -> Self {
        Self::new(32, 128).expect("constant operation limits")
    }
}
impl<T> OperationTable<T> {
    pub fn new(per_owner: usize, total: usize) -> Result<Self> {
        ensure!(
            per_owner > 0 && total >= per_owner && total <= 65536,
            "operation table limits"
        );
        Ok(Self {
            entries: BTreeMap::new(),
            per_owner,
            total,
        })
    }
    /// Call before capturing expensive input. Ready and failed results retain
    /// capacity until consumed/cancelled; a rejected pool submission inserts none.
    pub fn admit(&self, owner: u32) -> Result<()> {
        ensure!(
            self.entries.len() < self.total
                && self.entries.values().filter(|e| e.owner == owner).count() < self.per_owner,
            "operation result capacity exceeded; consume or cancel retained results"
        );
        Ok(())
    }
    pub fn submit(
        &mut self,
        owner: u32,
        generation: u64,
        kind: &'static str,
        pool: &TaskPool,
        work: impl FnOnce(Cancellation) -> Result<T> + Send + 'static,
    ) -> Result<u64>
    where
        T: Send + 'static,
    {
        self.admit(owner)?;
        let ticket = next_operation_id()?;
        let pending = pool.spawn(move |cancel| {
            cancel.check()?;
            let result = work(cancel.clone())?;
            cancel.check()?;
            Ok(result)
        })?;
        self.entries.insert(
            ticket,
            Entry {
                owner,
                generation,
                kind,
                pending: Some(pending),
                result: None,
                error: None,
            },
        );
        Ok(ticket)
    }
    fn entry(&mut self, owner: u32, generation: u64, ticket: u64) -> Result<&mut Entry<T>> {
        let entry = self
            .entries
            .get_mut(&ticket)
            .context("operation ticket released or unknown")?;
        ensure!(
            entry.owner == owner,
            "operation ticket belongs to another client"
        );
        ensure!(
            entry.generation == generation,
            "operation result belongs to an obsolete generation"
        );
        if let Some(task) = &mut entry.pending {
            match task.try_take() {
                Ok(None) => {}
                Ok(Some(result)) => {
                    entry.pending = None;
                    entry.result = Some(result);
                }
                Err(error) => {
                    entry.pending = None;
                    entry.error = Some(format!("{error:#}"));
                }
            }
        }
        Ok(entry)
    }
    pub fn status(&mut self, owner: u32, generation: u64, ticket: u64) -> Result<OperationStatus> {
        let entry = self.entry(owner, generation, ticket)?;
        Ok(OperationStatus {
            ticket,
            kind: entry.kind,
            state: if entry.pending.is_some() {
                OperationState::Pending
            } else if entry.error.is_some() {
                OperationState::Failed
            } else {
                OperationState::Ready
            },
            error: entry.error.clone(),
        })
    }
    pub fn poll(&mut self, owner: u32, generation: u64, ticket: u64) -> Result<Option<&T>> {
        let entry = self.entry(owner, generation, ticket)?;
        if let Some(error) = &entry.error {
            anyhow::bail!("{error}");
        }
        Ok(entry.result.as_ref())
    }
    /// Pending returns None and retains the job. Failure remains observable.
    pub fn take(&mut self, owner: u32, generation: u64, ticket: u64) -> Result<Option<T>> {
        if self.poll(owner, generation, ticket)?.is_none() {
            return Ok(None);
        }
        Ok(self.entries.remove(&ticket).unwrap().result)
    }
    /// Start a dependent phase only after its predecessor completed. Admission
    /// failure retains the predecessor so the owner can retry with backpressure.
    pub fn advance(
        &mut self,
        owner: u32,
        generation: u64,
        ticket: u64,
        start: impl FnOnce(&T) -> Result<Task<T>>,
    ) -> Result<()> {
        let result = self
            .poll(owner, generation, ticket)?
            .context("operation predecessor is pending")?;
        let task = start(result)?;
        let entry = self.entries.get_mut(&ticket).unwrap();
        entry.result = None;
        entry.pending = Some(task);
        Ok(())
    }
    /// A short owner-only publication step. Failed publication stays failed and
    /// observable, rather than leaking a consumed or apparently ready result.
    pub fn publish(
        &mut self,
        owner: u32,
        generation: u64,
        ticket: u64,
        commit: impl FnOnce(T) -> Result<T>,
    ) -> Result<()> {
        let entry = self.entry(owner, generation, ticket)?;
        if let Some(error) = &entry.error {
            anyhow::bail!("{error}");
        }
        let value = entry.result.take().context("operation is pending")?;
        match commit(value) {
            Ok(value) => {
                entry.result = Some(value);
                Ok(())
            }
            Err(error) => {
                let message = format!("{error:#}");
                entry.error = Some(message.clone());
                anyhow::bail!("{message}")
            }
        }
    }
    pub fn tickets(&self) -> Vec<(u32, u64, u64)> {
        self.entries
            .iter()
            .map(|(id, e)| (e.owner, e.generation, *id))
            .collect()
    }
    /// Cancellation can release an obsolete generation, but never another owner.
    pub fn cancel(&mut self, owner: u32, ticket: u64) -> Result<()> {
        ensure!(
            self.entries
                .get(&ticket)
                .context("operation ticket released or unknown")?
                .owner
                == owner,
            "operation ticket belongs to another client"
        );
        self.entries.remove(&ticket);
        Ok(())
    }
    pub fn discard_obsolete(&mut self, generation: u64) {
        self.entries.retain(|_, e| e.generation == generation);
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn contains(&self, owner: u32, ticket: u64) -> bool {
        self.entries.get(&ticket).is_some_and(|e| e.owner == owner)
    }
}
