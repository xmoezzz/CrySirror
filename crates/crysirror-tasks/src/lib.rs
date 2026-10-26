// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded CPU workers and awaitable, cooperatively cancellable jobs. No devices.
#![forbid(unsafe_code)]
pub mod graph;
pub mod profile;
pub mod table;
use anyhow::{Result, ensure};
use crossbeam_channel::{Sender, bounded};
use futures::channel::oneshot;
use serde::Serialize;
use std::{
    cell::Cell,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    thread::{self, JoinHandle},
};
/// Backpressure: retry submission after queued/running work completes.
#[derive(Debug)]
pub struct CapacityExceeded;
impl std::fmt::Display for CapacityExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("task pool capacity exceeded; retry later")
    }
}
impl std::error::Error for CapacityExceeded {}
static NEXT_POOL: AtomicU64 = AtomicU64::new(1);
thread_local! {static WORKER:Cell<u64>=const {Cell::new(0)};}
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> Result<()> {
        ensure!(!self.is_cancelled(), "task cancelled");
        Ok(())
    }
}
#[derive(Default)]
struct Counters {
    outstanding: AtomicUsize,
    reserved: AtomicUsize,
    running: AtomicUsize,
    completed: AtomicU64,
    cancelled: AtomicU64,
    panicked: AtomicU64,
    rejected: AtomicU64,
}
#[derive(Clone, Debug, Serialize)]
pub struct TaskStats {
    pub workers: usize,
    pub capacity: usize,
    pub outstanding: usize,
    /// Admitted slots not yet submitted; dropping a permit releases its slot.
    pub reserved: usize,
    pub running: usize,
    pub completed: u64,
    pub cancelled: u64,
    pub panicked: u64,
    pub rejected: u64,
}
type Job = Box<dyn FnOnce() + Send + 'static>;
struct Owner {
    id: u64,
    sender: Option<Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
    counts: Arc<Counters>,
    capacity: usize,
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.sender.take();
        let current = thread::current().id();
        for worker in self.workers.drain(..) {
            if worker.thread().id() != current {
                let _ = worker.join();
            }
        }
    }
}
/// Cloneable pool. Capacity bounds reserved + queued + running work.
#[derive(Clone)]
pub struct TaskPool(Arc<Owner>);
impl TaskPool {
    pub fn new(name: &str, workers: usize, capacity: usize) -> Result<Self> {
        ensure!(
            (1..=64).contains(&workers) && capacity >= workers && capacity <= 65536,
            "invalid task pool limits"
        );
        let id = NEXT_POOL.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = bounded::<Job>(capacity);
        let counts = Arc::new(Counters::default());
        let mut handles = vec![];
        for index in 0..workers {
            let worker_rx = rx.clone();
            match thread::Builder::new()
                .name(format!("crysirror-{name}-{index}"))
                .spawn(move || {
                    WORKER.with(|v| v.set(id));
                    while let Ok(job) = worker_rx.recv() {
                        job();
                    }
                }) {
                Ok(h) => handles.push(h),
                Err(e) => {
                    drop(tx);
                    drop(rx);
                    for h in handles {
                        let _ = h.join();
                    }
                    return Err(e.into());
                }
            }
        }
        Ok(Self(Arc::new(Owner {
            id,
            sender: Some(tx),
            workers: handles,
            counts,
            capacity,
        })))
    }
    /// Reserve admission before moving a resource into a job. A rejected
    /// reservation leaves the caller's decoder/buffer ownership intact.
    pub fn reserve(&self) -> Result<TaskPermit> {
        let c = &self.0.counts;
        if c.outstanding
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.0.capacity).then_some(n + 1)
            })
            .is_err()
        {
            c.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(CapacityExceeded.into());
        }
        c.reserved.fetch_add(1, Ordering::AcqRel);
        Ok(TaskPermit {
            pool: self.clone(),
            submitted: false,
        })
    }
    pub fn spawn<T: Send + 'static>(
        &self,
        work: impl FnOnce(Cancellation) -> Result<T> + Send + 'static,
    ) -> Result<Task<T>> {
        self.reserve()?.spawn(work)
    }
    pub fn stats(&self) -> TaskStats {
        let c = &self.0.counts;
        TaskStats {
            workers: self.0.workers.len(),
            capacity: self.0.capacity,
            outstanding: c.outstanding.load(Ordering::Acquire),
            reserved: c.reserved.load(Ordering::Acquire),
            running: c.running.load(Ordering::Acquire),
            completed: c.completed.load(Ordering::Relaxed),
            cancelled: c.cancelled.load(Ordering::Relaxed),
            panicked: c.panicked.load(Ordering::Relaxed),
            rejected: c.rejected.load(Ordering::Relaxed),
        }
    }
}
/// One admitted slot, owned by the submitter until consumed or dropped.
/// It keeps the pool alive. An unused permit releases admission; final pool
/// shutdown still follows the pool owner's join contract.
pub struct TaskPermit {
    pool: TaskPool,
    submitted: bool,
}
impl TaskPermit {
    pub fn spawn<T: Send + 'static>(
        mut self,
        work: impl FnOnce(Cancellation) -> Result<T> + Send + 'static,
    ) -> Result<Task<T>> {
        let c = self.pool.0.counts.clone();
        let cancel = Cancellation::default();
        let token = cancel.clone();
        let (tx, rx) = oneshot::channel();
        let counts = c.clone();
        let job: Job = Box::new(move || {
            counts.running.fetch_add(1, Ordering::AcqRel);
            let result = if token.is_cancelled() {
                Err(anyhow::anyhow!("task cancelled"))
            } else {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(token.clone())))
                {
                    Ok(v) => v,
                    Err(_) => {
                        counts.panicked.fetch_add(1, Ordering::Relaxed);
                        Err(anyhow::anyhow!("worker task panicked"))
                    }
                }
            };
            let result = if token.is_cancelled() {
                counts.cancelled.fetch_add(1, Ordering::Relaxed);
                Err(anyhow::anyhow!("task cancelled"))
            } else {
                result
            };
            counts.running.fetch_sub(1, Ordering::AcqRel);
            counts.outstanding.fetch_sub(1, Ordering::AcqRel);
            counts.completed.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(result);
        });
        if self.pool.0.sender.as_ref().unwrap().try_send(job).is_err() {
            c.rejected.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("task pool is closed");
        }
        self.submitted = true;
        c.reserved.fetch_sub(1, Ordering::AcqRel);
        Ok(Task {
            receiver: rx,
            cancel,
            pool: self.pool.0.id,
            finished: false,
        })
    }
}
impl Drop for TaskPermit {
    fn drop(&mut self) {
        if !self.submitted {
            self.pool.0.counts.reserved.fetch_sub(1, Ordering::AcqRel);
            self.pool
                .0
                .counts
                .outstanding
                .fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// Real Future: polling never blocks. Dropping it requests cancellation.
/// `wait` is an explicit offline/frame-barrier operation, forbidden inside its own pool.
pub struct Task<T> {
    receiver: oneshot::Receiver<Result<T>>,
    cancel: Cancellation,
    pool: u64,
    finished: bool,
}
impl<T> Task<T> {
    pub fn cancel(&self) {
        self.cancel.cancel()
    }
    pub fn try_take(&mut self) -> Result<Option<T>> {
        ensure!(!self.finished, "task result already consumed");
        match self.receiver.try_recv() {
            Ok(None) => Ok(None),
            Ok(Some(result)) => {
                self.finished = true;
                result.map(Some)
            }
            Err(_) => {
                self.finished = true;
                anyhow::bail!("task worker disconnected")
            }
        }
    }
    pub fn wait(self) -> Result<T> {
        ensure!(
            !WORKER.with(|v| v.get() == self.pool),
            "cannot block a worker on its own task pool"
        );
        futures::executor::block_on(self)
    }
}
impl<T> Future for Task<T> {
    type Output = Result<T>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.finished {
            return Poll::Ready(Err(anyhow::anyhow!("task result already consumed")));
        }
        match Pin::new(&mut self.receiver).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(v) => {
                self.finished = true;
                Poll::Ready(v.unwrap_or_else(|_| Err(anyhow::anyhow!("task worker disconnected"))))
            }
        }
    }
}
impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        if !self.finished {
            self.cancel.cancel()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    #[test]
    fn reservations_preserve_caller_ownership_on_backpressure() {
        let pool = TaskPool::new("permit", 1, 1).unwrap();
        let permit = pool.reserve().unwrap();
        assert_eq!(pool.stats().reserved, 1);
        let mut owned = Some(String::from("decoder"));
        match pool.reserve() {
            Err(e) => assert!(e.is::<CapacityExceeded>()),
            Ok(p) => {
                let value = owned.take().unwrap();
                let _ = p.spawn(move |_| Ok(value));
                panic!("must reject before capture");
            }
        }
        assert_eq!(owned.as_deref(), Some("decoder"));
        drop(permit);
        assert_eq!(pool.stats().outstanding, 0);
        assert_eq!(pool.stats().reserved, 0);
        let value = owned.take().unwrap();
        assert_eq!(
            pool.reserve()
                .unwrap()
                .spawn(move |_| Ok(value))
                .unwrap()
                .wait()
                .unwrap(),
            "decoder"
        );
        assert_eq!(pool.stats().outstanding, 0);
    }
    #[test]
    fn two_jobs_run_concurrently_and_futures_wake() {
        let pool = TaskPool::new("test", 2, 4).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let jobs: Vec<_> = (0..2)
            .map(|_| {
                let b = barrier.clone();
                pool.spawn(move |_| {
                    b.wait();
                    Ok(thread::current().id())
                })
                .unwrap()
            })
            .collect();
        barrier.wait();
        let mut ids = jobs
            .into_iter()
            .map(|j| futures::executor::block_on(j).unwrap());
        assert_ne!(ids.next(), ids.next());
        assert_eq!(pool.stats().completed, 2);
    }
    #[test]
    fn bounded_cancel_and_panic_do_not_kill_workers() {
        let pool = TaskPool::new("test", 1, 2).unwrap();
        let (ready, started) = bounded(1);
        let (gate, release) = bounded(1);
        let a = pool
            .spawn(move |_| {
                ready.send(()).unwrap();
                release.recv().unwrap();
                Ok(1)
            })
            .unwrap();
        started.recv().unwrap();
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let b = pool
            .spawn(move |_| {
                flag.store(true, Ordering::Relaxed);
                Ok(2)
            })
            .unwrap();
        assert!(pool.spawn(|_| Ok(3)).is_err());
        b.cancel();
        gate.send(()).unwrap();
        assert_eq!(a.wait().unwrap(), 1);
        assert!(b.wait().is_err());
        assert!(!ran.load(Ordering::Relaxed));
        assert!(
            pool.spawn::<()>(|_| panic!("fixture"))
                .unwrap()
                .wait()
                .is_err()
        );
        assert_eq!(pool.spawn(|_| Ok(4)).unwrap().wait().unwrap(), 4);
        assert_eq!(pool.stats().outstanding, 0);
    }
    #[test]
    fn nested_wait_is_rejected_instead_of_deadlocking() {
        let pool = TaskPool::new("test", 1, 4).unwrap();
        let nested = pool.clone();
        assert!(
            pool.spawn(move |_| nested.spawn(|_| Ok(1))?.wait())
                .unwrap()
                .wait()
                .is_err()
        );
    }
}
