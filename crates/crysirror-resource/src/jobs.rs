// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Owner-polled resource work. Workers receive immutable bytes, registries and
//! client capabilities; only successful completions become owner-visible state.
//! Tickets/results are derived runtime state and are intentionally not saved.
use crate::{Archive, DecodeContext, DecodedFile, FileRegistry, FormatRegistry};
use anyhow::{ensure, Context, Result};
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_tasks::{Task, TaskPool};
use std::{collections::BTreeMap, sync::Arc};

pub const MAX_RESOURCE_JOBS_PER_CLIENT: usize = 64;
pub const MAX_RESOURCE_JOBS: usize = 256;
#[derive(Clone)]
pub enum ResourceResult {
    File {
        format: &'static str,
        decoded: DecodedFile,
        context: DecodeContext,
    },
    Archive {
        format: &'static str,
        archive: Arc<dyn Archive>,
        context: DecodeContext,
    },
}
struct Entry {
    client: ClientId,
    kind: &'static str,
    pending: Option<Task<ResourceResult>>,
    result: Option<ResourceResult>,
    error: Option<String>,
}
/// Bounded owner-side resource task table. Polling never joins a worker.
/// Cancellation is checked before/after codecs: a codec already running may
/// finish CPU work, but its cancelled output is dropped rather than published.
#[derive(Default)]
pub struct ResourceTasks {
    jobs: BTreeMap<u64, Entry>,
}
impl ResourceTasks {
    fn admit(&self, client: ClientId, context: &DecodeContext) -> Result<()> {
        ensure!(
            context.client == client,
            "resource task context belongs to another client"
        );
        ensure!(
            self.jobs.len() < MAX_RESOURCE_JOBS,
            "resource task table capacity exceeded"
        );
        ensure!(
            self.jobs.values().filter(|j| j.client == client).count()
                < MAX_RESOURCE_JOBS_PER_CLIENT,
            "client resource task capacity exceeded"
        );
        Ok(())
    }
    fn submit(
        &mut self,
        client: ClientId,
        kind: &'static str,
        pool: &TaskPool,
        work: impl FnOnce(crysirror_tasks::Cancellation) -> Result<ResourceResult> + Send + 'static,
    ) -> Result<u64> {
        // The global ID may have a gap after pool rejection; no rejected job is kept.
        let ticket = crysirror_tasks::table::next_operation_id()?;
        let task = pool.spawn(work)?;
        self.jobs.insert(
            ticket,
            Entry {
                client,
                kind,
                pending: Some(task),
                result: None,
                error: None,
            },
        );
        Ok(ticket)
    }
    pub fn begin_file(
        &mut self,
        client: ClientId,
        registry: FileRegistry,
        path: String,
        source: Arc<dyn AssetReader>,
        format: Option<String>,
        context: DecodeContext,
        pool: &TaskPool,
    ) -> Result<u64> {
        self.begin_file_with_source(
            client,
            registry,
            path,
            move || Ok(source),
            format,
            context,
            pool,
        )
    }
    /// Open the source on the worker too: a VFS provider may itself need to
    /// decompress an archive entry while opening a logical resource.
    pub fn begin_file_with_source(
        &mut self,
        client: ClientId,
        registry: FileRegistry,
        path: String,
        source: impl FnOnce() -> Result<Arc<dyn AssetReader>> + Send + 'static,
        format: Option<String>,
        context: DecodeContext,
        pool: &TaskPool,
    ) -> Result<u64> {
        self.admit(client, &context)?;
        self.submit(client, "file", pool, move |cancel| {
            let context = context.with_cancellation(cancel.clone());
            cancel.check()?;
            let source = source()?;
            cancel.check()?;
            let (format, decoded) = registry.decode(&path, source, format.as_deref(), &context)?;
            cancel.check()?;
            Ok(ResourceResult::File {
                format,
                decoded,
                context: context.published(),
            })
        })
    }
    pub fn begin_archive(
        &mut self,
        client: ClientId,
        registry: FormatRegistry,
        source: Arc<dyn AssetReader>,
        format: Option<String>,
        context: DecodeContext,
        pool: &TaskPool,
    ) -> Result<u64> {
        self.begin_archive_with_source(client, registry, move || Ok(source), format, context, pool)
    }
    /// Schedule both provider materialization and archive indexing on workers.
    pub fn begin_archive_with_source(
        &mut self,
        client: ClientId,
        registry: FormatRegistry,
        source: impl FnOnce() -> Result<Arc<dyn AssetReader>> + Send + 'static,
        format: Option<String>,
        context: DecodeContext,
        pool: &TaskPool,
    ) -> Result<u64> {
        self.admit(client, &context)?;
        self.submit(client, "archive", pool, move |cancel| {
            let context = context.with_cancellation(cancel.clone());
            cancel.check()?;
            let source = source()?;
            cancel.check()?;
            let (format, archive) = registry.open(source, format.as_deref(), &context)?;
            cancel.check()?;
            Ok(ResourceResult::Archive {
                format,
                archive: Arc::from(archive),
                context: context.published(),
            })
        })
    }
    fn entry(&mut self, client: ClientId, ticket: u64) -> Result<&mut Entry> {
        let job = self
            .jobs
            .get_mut(&ticket)
            .context("resource task ticket not found")?;
        ensure!(
            job.client == client,
            "resource task ticket belongs to another client"
        );
        Ok(job)
    }
    fn refresh(job: &mut Entry) {
        let Some(task) = job.pending.as_mut() else {
            return;
        };
        match task.try_take() {
            Ok(None) => {}
            Ok(Some(value)) => {
                job.pending = None;
                job.result = Some(value);
            }
            Err(error) => {
                job.pending = None;
                job.error = Some(format!("{error:#}"));
            }
        }
    }
    pub fn poll(&mut self, client: ClientId, ticket: u64) -> Result<Option<&ResourceResult>> {
        let job = self.entry(client, ticket)?;
        Self::refresh(job);
        if let Some(error) = &job.error {
            anyhow::bail!("resource task failed: {error}");
        }
        Ok(job.result.as_ref())
    }
    pub fn status(&mut self, client: ClientId, ticket: u64) -> Result<serde_json::Value> {
        let job = self.entry(client, ticket)?;
        Self::refresh(job);
        let state = if job.error.is_some() {
            "failed"
        } else if job.result.is_some() {
            "ready"
        } else {
            "pending"
        };
        Ok(serde_json::json!({"ticket":ticket,"state":state,"kind":job.kind,"error":job.error}))
    }
    pub fn cancel(&mut self, client: ClientId, ticket: u64) -> Result<()> {
        self.entry(client, ticket)?;
        let job = self.jobs.remove(&ticket).expect("ticket checked above");
        if let Some(task) = &job.pending {
            task.cancel();
        }
        // Dropping a ready result releases its decoded-memory reservations. Dropping
        // a pending receiver discards late completions even if decoding cannot stop.
        drop(job);
        Ok(())
    }
}
