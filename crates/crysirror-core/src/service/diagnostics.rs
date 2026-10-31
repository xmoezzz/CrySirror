// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Bounded owner-committed, client-scoped engine logs. No VM/device/file sink.
use crate::{state::ClientId, Error, Result};
use alloc::{
    collections::{BTreeMap, VecDeque},
    string::String,
    vec::Vec,
};
pub const MAX_LOG_LINES: usize = 1024;
pub const MAX_LOG_LINE_BYTES: usize = 4096;
const CLIENT_BYTES: usize = 1024 * 1024;
const TOTAL_BYTES: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRecord {
    pub sequence: u64,
    pub time_ms: u64,
    pub message: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogReceipt {
    pub client: ClientId,
    pub sequence: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogSnapshot {
    pub client: ClientId,
    pub dropped: u64,
    pub records: Vec<LogRecord>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct LogViewStamp {
    first: u64,
    last: u64,
}
#[derive(Default)]
struct Ring {
    records: VecDeque<LogRecord>,
    sequence: u64,
    dropped: u64,
    bytes: usize,
}
#[derive(Default)]
pub struct LogBook {
    clients: BTreeMap<ClientId, Ring>,
    bytes: usize,
}
impl LogBook {
    /// Complete at the exclusive owner boundary, or fail with no mutation.
    pub fn append(&mut self, client: ClientId, time_ms: u64, message: &str) -> Result<LogReceipt> {
        if message.len() > MAX_LOG_LINE_BYTES {
            return Err(Error::Limit("engine log line bytes"));
        }
        if !self.clients.contains_key(&client) && self.clients.len() >= 64 {
            return Err(Error::Limit("engine log clients"));
        }
        let old = self.clients.get(&client);
        let sequence = old
            .map_or(0, |r| r.sequence)
            .checked_add(1)
            .ok_or(Error::Limit("engine log sequence"))?;
        let mut evict = 0;
        let mut freed = 0;
        if let Some(r) = old {
            for record in &r.records {
                if r.records.len() - evict < MAX_LOG_LINES
                    && r.bytes - freed + message.len() <= CLIENT_BYTES
                {
                    break;
                }
                freed += record.message.len();
                evict += 1;
            }
        }
        if self.bytes - freed + message.len() > TOTAL_BYTES {
            return Err(Error::Limit("engine log total bytes"));
        }
        let r = self.clients.entry(client).or_default();
        for _ in 0..evict {
            let record = r.records.pop_front().unwrap();
            r.bytes -= record.message.len();
            r.dropped += 1;
        }
        r.sequence = sequence;
        r.bytes += message.len();
        self.bytes = self.bytes - freed + message.len();
        r.records.push_back(LogRecord {
            sequence,
            time_ms,
            message: message.into(),
        });
        Ok(LogReceipt { client, sequence })
    }
    pub fn snapshot(&self, client: ClientId) -> LogSnapshot {
        let ring = self.clients.get(&client);
        LogSnapshot {
            client,
            dropped: ring.map_or(0, |r| r.dropped),
            records: ring
                .map(|r| r.records.iter().cloned().collect())
                .unwrap_or_default(),
        }
    }
    /// Maintain the pre-existing public App log view from the canonical store.
    /// App code and native adapters share one ring; this is a derived cache.
    pub fn sync_view(
        &self,
        client: ClientId,
        stamp: &mut LogViewStamp,
        view: &mut VecDeque<String>,
    ) {
        let Some(r) = self.clients.get(&client) else {
            view.clear();
            *stamp = LogViewStamp::default();
            return;
        };
        let first = r.records.front().map_or(0, |v| v.sequence);
        let last = r.records.back().map_or(0, |v| v.sequence);
        if stamp.first == 0
            || stamp.first > first
            || stamp.last > last
            || stamp.last < first.saturating_sub(1)
        {
            view.clear();
            view.extend(r.records.iter().map(|v| v.message.clone()));
        } else {
            let removed = (first - stamp.first).min(view.len() as u64) as usize;
            for _ in 0..removed {
                view.pop_front();
            }
            let n = (last - stamp.last).min(r.records.len() as u64) as usize;
            view.extend(
                r.records
                    .iter()
                    .skip(r.records.len() - n)
                    .map(|v| v.message.clone()),
            );
        }
        *stamp = LogViewStamp { first, last };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logs_have_owner_receipts_scope_eviction_and_atomic_limits() {
        let a = ClientId(1);
        let b = ClientId(2);
        let mut book = LogBook::default();
        let mut view = VecDeque::new();
        let mut stamp = LogViewStamp::default();
        for n in 0..1025 {
            let receipt = book.append(a, n, "x").unwrap();
            assert_eq!(receipt.sequence, n + 1);
            book.sync_view(a, &mut stamp, &mut view);
        }
        assert_eq!(view.len(), 1024);
        assert_eq!(book.snapshot(a).dropped, 1);
        assert!(book.snapshot(b).records.is_empty());
        let before = book.snapshot(a);
        assert!(book.append(a, 0, &"x".repeat(4097)).is_err());
        assert_eq!(book.snapshot(a), before);
        book.append(b, 7, "native").unwrap();
        book.sync_view(a, &mut stamp, &mut view);
        assert!(!view.iter().any(|v| v == "native"));
        assert_eq!(book.snapshot(b).records[0].time_ms, 7);
    }
}
