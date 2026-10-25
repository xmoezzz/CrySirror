// SPDX-License-Identifier: MPL-2.0
//! Nonblocking dependency graph over immutable results and bounded workers.
use crate::{Cancellation, CapacityExceeded, Task, TaskPool, profile::Profiler};
use anyhow::{Context, Result, ensure};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
static NEXT_GRAPH: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId {
    graph: u64,
    index: usize,
}
type Work<T> = Box<dyn FnOnce(Cancellation, Vec<Arc<T>>) -> Result<T> + Send>;
struct Node<T> {
    name: String,
    deps: Vec<NodeId>,
    work: Arc<Mutex<Option<Work<T>>>>,
    task: Option<Task<T>>,
    value: Option<Arc<T>>,
}
pub struct FrameGraph<T: Send + Sync + 'static> {
    id: u64,
    nodes: Vec<Node<T>>,
    failure: Option<String>,
    profiler: Profiler,
    frame: u64,
}
impl<T: Send + Sync + 'static> FrameGraph<T> {
    pub fn new(profiler: Profiler, frame: u64) -> Self {
        Self {
            id: NEXT_GRAPH.fetch_add(1, Ordering::Relaxed),
            nodes: vec![],
            failure: None,
            profiler,
            frame,
        }
    }
    /// Dependencies must already exist in this graph, making cycles impossible.
    pub fn add(
        &mut self,
        name: &str,
        deps: &[NodeId],
        work: impl FnOnce(Cancellation, Vec<Arc<T>>) -> Result<T> + Send + 'static,
    ) -> Result<NodeId> {
        ensure!(self.nodes.len() < 256, "frame graph node budget exceeded");
        ensure!(
            deps.iter()
                .all(|d| d.graph == self.id && d.index < self.nodes.len()),
            "foreign or forward graph dependency"
        );
        ensure!(
            self.nodes
                .iter()
                .all(|n| n.task.is_none() && n.value.is_none()),
            "cannot extend a running graph"
        );
        let id = NodeId {
            graph: self.id,
            index: self.nodes.len(),
        };
        self.nodes.push(Node {
            name: name.into(),
            deps: deps.to_vec(),
            work: Arc::new(Mutex::new(Some(Box::new(work)))),
            task: None,
            value: None,
        });
        Ok(id)
    }
    fn fail(&mut self, e: anyhow::Error) -> anyhow::Error {
        for n in &mut self.nodes {
            if let Some(t) = n.task.take() {
                t.cancel();
            }
        }
        self.failure = Some(format!("{e:#}"));
        e
    }
    /// Poll completion, then admit ready nodes. Never waits for a worker.
    /// Capacity pressure keeps the original closure for retry, without dropping
    /// or running it on the owner thread.
    pub fn poll(&mut self, pool: &TaskPool) -> Result<bool> {
        ensure!(
            self.failure.is_none(),
            "frame graph failed: {}",
            self.failure.as_deref().unwrap_or_default()
        );
        for i in 0..self.nodes.len() {
            let result = self.nodes[i].task.as_mut().map(Task::try_take);
            match result {
                Some(Ok(Some(v))) => {
                    self.nodes[i].task = None;
                    self.nodes[i].value = Some(Arc::new(v));
                }
                Some(Err(e)) => {
                    let name = self.nodes[i].name.clone();
                    return Err(self.fail(e.context(name)));
                }
                _ => {}
            }
        }
        for i in 0..self.nodes.len() {
            if self.nodes[i].value.is_some() || self.nodes[i].task.is_some() {
                continue;
            }
            let Some(inputs) = self.nodes[i]
                .deps
                .iter()
                .map(|d| self.nodes[d.index].value.clone())
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let slot = self.nodes[i].work.clone();
            let profiler = self.profiler.clone();
            let frame = self.frame;
            let name = self.nodes[i].name.clone();
            match pool.spawn(move |cancel| {
                let _span = profiler.span(frame, name);
                let work = slot
                    .lock()
                    .unwrap()
                    .take()
                    .context("graph job already taken")?;
                work(cancel, inputs)
            }) {
                Ok(t) => self.nodes[i].task = Some(t),
                Err(e) if e.is::<CapacityExceeded>() => break,
                Err(e) => return Err(self.fail(e)),
            }
        }
        Ok(self.nodes.iter().all(|n| n.value.is_some()))
    }
    pub fn get(&self, id: NodeId) -> Result<Option<Arc<T>>> {
        ensure!(id.graph == self.id, "foreign graph result");
        Ok(self
            .nodes
            .get(id.index)
            .context("graph node missing")?
            .value
            .clone())
    }
    /// Explicit offline join. Interactive hosts must use poll and a retained
    /// prior snapshot; workers cannot block on their own executor.
    pub fn wait(&mut self, pool: &TaskPool) -> Result<()> {
        while !self.poll(pool)? {
            let Some(i) = self.nodes.iter().position(|n| n.task.is_some()) else {
                anyhow::bail!("frame graph cannot make progress: pool capacity exhausted");
            };
            let task = self.nodes[i].task.take().unwrap();
            match task.wait() {
                Ok(v) => self.nodes[i].value = Some(Arc::new(v)),
                Err(e) => return Err(self.fail(e.context(self.nodes[i].name.clone()))),
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_jobs_overlap_and_join_runs_after_both() {
        let pool = TaskPool::new("dag", 2, 4).unwrap();
        let mut g = FrameGraph::new(Profiler::default(), 1);
        let b = Arc::new(std::sync::Barrier::new(2));
        let b1 = b.clone();
        let a = g
            .add("a", &[], move |_, _| {
                b1.wait();
                Ok(2)
            })
            .unwrap();
        let c = g
            .add("b", &[], move |_, _| {
                b.wait();
                Ok(3)
            })
            .unwrap();
        let join = g.add("join", &[a, c], |_, v| Ok(*v[0] + *v[1])).unwrap();
        g.wait(&pool).unwrap();
        assert_eq!(*g.get(join).unwrap().unwrap(), 5);
    }
    #[test]
    fn backpressure_retries_and_failed_dependency_never_runs() {
        let pool = TaskPool::new("dag", 1, 1).unwrap();
        let mut g = FrameGraph::new(Profiler::default(), 1);
        let a = g.add("a", &[], |_, _| Ok(1)).unwrap();
        let b = g.add("b", &[], |_, _| Ok(2)).unwrap();
        let j = g.add("join", &[a, b], |_, v| Ok(*v[0] + *v[1])).unwrap();
        g.wait(&pool).unwrap();
        assert_eq!(*g.get(j).unwrap().unwrap(), 3);
        let mut bad = FrameGraph::<u32>::new(Profiler::default(), 2);
        assert!(bad.add("foreign", &[a], |_, _| Ok(0)).is_err());
        let f = bad
            .add("fail", &[], |_, _| anyhow::bail!("fixture"))
            .unwrap();
        bad.add("dependent", &[f], |_, _| panic!("must not run"))
            .unwrap();
        assert!(bad.wait(&pool).unwrap_err().to_string().contains("fail"));
    }
}
