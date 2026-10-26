// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
use anyhow::Result;
use crysirror_tasks::{
    TaskPool,
    table::{OperationState, OperationTable},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
fn ready<T>(table: &mut OperationTable<T>, owner: u32, epoch: u64, id: u64) -> OperationState {
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let state = table.status(owner, epoch, id).unwrap().state;
        if state != OperationState::Pending {
            return state;
        }
        assert!(Instant::now() < end);
        std::thread::yield_now();
    }
}
#[test]
fn owner_generation_retention_and_failure_are_observable() {
    let pool = TaskPool::new("operation-contract", 1, 4).unwrap();
    let mut jobs = OperationTable::new(1, 2).unwrap();
    let id = jobs.submit(1, 5, "test", &pool, |_| Ok(42)).unwrap();
    assert_eq!(ready(&mut jobs, 1, 5, id), OperationState::Ready);
    assert!(jobs.status(2, 5, id).is_err());
    assert!(jobs.status(1, 6, id).is_err());
    assert!(jobs.submit(1, 5, "test", &pool, |_| Ok(43)).is_err());
    assert_eq!(jobs.take(1, 5, id).unwrap(), Some(42));
    let bad = jobs
        .submit(1, 5, "test", &pool, |_| -> Result<i32> {
            anyhow::bail!("bad codec")
        })
        .unwrap();
    assert_eq!(ready(&mut jobs, 1, 5, bad), OperationState::Failed);
    assert!(
        jobs.status(1, 5, bad)
            .unwrap()
            .error
            .unwrap()
            .contains("bad codec")
    );
    assert!(jobs.poll(1, 5, bad).is_err());
    assert!(jobs.poll(1, 5, bad).is_err());
    assert!(jobs.cancel(2, bad).is_err());
    jobs.cancel(1, bad).unwrap();
}
#[test]
fn phase_admission_retries_without_losing_predecessor_and_commit_failure_is_retained() {
    let cpu = TaskPool::new("phase-cpu", 1, 2).unwrap();
    let io = TaskPool::new("phase-io", 1, 1).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let blocker = io
        .spawn(move |_| {
            rx.recv_timeout(Duration::from_secs(5))?;
            Ok(())
        })
        .unwrap();
    let mut jobs = OperationTable::default();
    let id = jobs
        .submit(1, 0, "phase", &cpu, |_| Ok(Arc::new(vec![1, 2, 3])))
        .unwrap();
    ready(&mut jobs, 1, 0, id);
    assert!(
        jobs.advance(1, 0, id, |data| {
            let data = data.clone();
            io.spawn(move |_| Ok(data))
        })
        .is_err()
    );
    assert_eq!(jobs.poll(1, 0, id).unwrap().unwrap().as_slice(), [1, 2, 3]);
    tx.send(()).unwrap();
    blocker.wait().unwrap();
    jobs.advance(1, 0, id, |data| {
        let data = data.clone();
        io.spawn(move |_| Ok(data))
    })
    .unwrap();
    ready(&mut jobs, 1, 0, id);
    assert!(
        jobs.publish(1, 0, id, |_| anyhow::bail!("publication failed"))
            .is_err()
    );
    assert_eq!(jobs.status(1, 0, id).unwrap().state, OperationState::Failed);
}
#[test]
fn queued_cancellation_and_epoch_drop_do_not_run_or_retain_work() {
    let pool = TaskPool::new("cancel-op", 1, 3).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let blocker = pool
        .spawn(move |_| {
            rx.recv_timeout(Duration::from_secs(5))?;
            Ok(())
        })
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let flag = calls.clone();
    let mut jobs = OperationTable::default();
    let id = jobs
        .submit(1, 3, "test", &pool, move |_| {
            flag.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
    jobs.discard_obsolete(4);
    assert!(jobs.status(1, 3, id).is_err());
    tx.send(()).unwrap();
    blocker.wait().unwrap();
    let fence = pool.spawn(|_| Ok(())).unwrap();
    fence.wait().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn immediate_offline_products_share_owner_generation_and_retention_rules() {
    let mut table = OperationTable::new(1, 2).unwrap();
    let id = table.insert_ready(1, 7, "offline", 42).unwrap();
    assert_eq!(table.status(1, 7, id).unwrap().state, OperationState::Ready);
    assert!(table.status(2, 7, id).is_err());
    assert!(table.status(1, 8, id).is_err());
    assert!(table.admit(1).is_err());
    assert_eq!(table.take(1, 7, id).unwrap(), Some(42));
    assert!(table.status(1, 7, id).is_err());
    assert!(table.admit(1).is_ok());
}
