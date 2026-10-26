// SPDX-License-Identifier: MPL-2.0
//! Bounded worker/capability/owner-publication behavior, without plugins or VMs.
use anyhow::Result;
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_resource::{
    io::memory, read_all, Archive, ArchiveEntry, ArchiveFormat, DecodeContext, DecodedFile,
    FileFormat, FileKind, FileRegistry, FormatRegistry, Probe, ResourceResult, ResourceTasks,
};
use crysirror_tasks::TaskPool;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
struct Decoder {
    calls: Arc<AtomicUsize>,
    fail: bool,
}
impl FileFormat for Decoder {
    fn id(&self) -> &'static str {
        "test/file"
    }
    fn kind(&self) -> FileKind {
        FileKind::File
    }
    fn probe(&self, _: &str, _: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe::YES)
    }
    fn decode(&self, _: &str, _: Arc<dyn AssetReader>, cx: &DecodeContext) -> Result<DecodedFile> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let reader = cx.decode_with(4, || {
            if self.fail {
                anyhow::bail!("codec rejected fixture");
            }
            Ok(b"done".to_vec())
        })?;
        Ok(DecodedFile {
            reader,
            extension: "dat".into(),
            metadata: serde_json::json!({}),
        })
    }
}
fn registry(fail: bool) -> (FileRegistry, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut r = FileRegistry::default();
    r.register(Arc::new(Decoder {
        calls: calls.clone(),
        fail,
    }))
    .unwrap();
    (r, calls)
}
fn cx(client: u32) -> DecodeContext {
    DecodeContext::new(ClientId(client))
}
fn ready(jobs: &mut ResourceTasks, client: ClientId, ticket: u64) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = jobs.status(client, ticket).unwrap();
        let state = status["state"].as_str().unwrap();
        if state != "pending" {
            return state.into();
        }
        assert!(
            Instant::now() < deadline,
            "resource worker completion timeout"
        );
        thread::yield_now();
    }
}
#[test]
fn blocked_worker_proves_submission_poll_and_cancel_never_join() {
    let pool = TaskPool::new("resource-test", 1, 2).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = pool
        .spawn(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut jobs = ResourceTasks::default();
    let (r, calls) = registry(false);
    let ticket = jobs
        .begin_file(
            ClientId(1),
            r,
            "x".into(),
            memory(vec![]),
            None,
            cx(1),
            &pool,
        )
        .unwrap();
    assert!(jobs.poll(ClientId(1), ticket).unwrap().is_none());
    assert_eq!(
        jobs.status(ClientId(1), ticket).unwrap()["state"],
        "pending"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(jobs.poll(ClientId(2), ticket).is_err());
    assert!(jobs.cancel(ClientId(2), ticket).is_err());
    jobs.cancel(ClientId(1), ticket).unwrap();
    assert!(jobs.status(ClientId(1), ticket).is_err());
    release_tx.send(()).unwrap();
    blocker.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while pool.stats().outstanding > 0 {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[test]
fn source_factories_run_on_workers_and_cancelled_queued_work_never_opens() {
    let pool = TaskPool::new("resource-source-factory", 1, 8).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = pool
        .spawn(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let owner_thread = thread::current().id();
    let opens = Arc::new(AtomicUsize::new(0));
    let cancelled_opens = Arc::new(AtomicUsize::new(0));
    let mut jobs = ResourceTasks::default();
    let (files, _) = registry(false);
    let mut archives = FormatRegistry::default();
    archives.register(Arc::new(Pack)).unwrap();
    let mut keep = vec![];
    for archive in [false, true] {
        for cancel in [false, true] {
            let calls = if cancel {
                cancelled_opens.clone()
            } else {
                opens.clone()
            };
            let factory = move || {
                assert_ne!(thread::current().id(), owner_thread);
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(memory(vec![]))
            };
            let ticket = if archive {
                jobs.begin_archive_with_source(
                    ClientId(1),
                    archives.clone(),
                    factory,
                    Some("test/archive".into()),
                    cx(1),
                    &pool,
                )
                .unwrap()
            } else {
                jobs.begin_file_with_source(
                    ClientId(1),
                    files.clone(),
                    "x".into(),
                    factory,
                    Some("test/file".into()),
                    cx(1),
                    &pool,
                )
                .unwrap()
            };
            assert!(jobs.poll(ClientId(1), ticket).unwrap().is_none());
            if cancel {
                jobs.cancel(ClientId(1), ticket).unwrap();
            } else {
                keep.push(ticket);
            }
        }
    }
    assert_eq!(opens.load(Ordering::SeqCst), 0);
    assert_eq!(cancelled_opens.load(Ordering::SeqCst), 0);
    release_tx.send(()).unwrap();
    blocker.wait().unwrap();
    for ticket in keep {
        assert_eq!(ready(&mut jobs, ClientId(1), ticket), "ready");
    }
    // FIFO barrier includes the last cancelled task, not just retained results.
    pool.spawn(|_| Ok(())).unwrap().wait().unwrap();
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert_eq!(cancelled_opens.load(Ordering::SeqCst), 0);
}
#[test]
fn pool_admission_failure_creates_no_ticket_and_retries_cleanly() {
    let pool = TaskPool::new("resource-admission", 1, 1).unwrap();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = pool
        .spawn(move |_| {
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap();
    let mut jobs = ResourceTasks::default();
    let (r, calls) = registry(false);
    let mut context = cx(3);
    context.limits.max_entry_size = 16;
    context
        .parameters
        .insert("test/file".into(), serde_json::json!({"scope":"original"}));
    assert!(jobs
        .begin_file(
            ClientId(3),
            r.clone(),
            "x".into(),
            memory(vec![]),
            None,
            context.clone(),
            &pool
        )
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(context.resident_bytes(), 0);
    release_tx.send(()).unwrap();
    blocker.wait().unwrap();
    let ticket = jobs
        .begin_file(
            ClientId(3),
            r,
            "x".into(),
            memory(vec![]),
            None,
            context.clone(),
            &pool,
        )
        .unwrap();
    assert_eq!(ready(&mut jobs, ClientId(3), ticket), "ready");
    assert_eq!(context.resident_bytes(), 4);
    match jobs.poll(ClientId(3), ticket).unwrap().unwrap() {
        ResourceResult::File {
            format,
            decoded,
            context: retained,
        } => {
            assert_eq!(retained.limits.max_entry_size, 16);
            assert_eq!(retained.parameters["test/file"]["scope"], "original");
            assert_eq!(*format, "test/file");
            assert_eq!(read_all(decoded.reader.as_ref(), 8).unwrap(), b"done");
        }
        _ => panic!("expected file result"),
    };
    assert!(
        jobs.poll(ClientId(3), ticket).unwrap().is_some(),
        "result remains resident for read/mount"
    );
    jobs.cancel(ClientId(3), ticket).unwrap();
    assert_eq!(context.resident_bytes(), 0);
}
#[test]
fn errors_and_per_client_limits_remain_owned_and_ids_are_global() {
    let pool = TaskPool::new("resource-retention", 1, 128).unwrap();
    let mut jobs = ResourceTasks::default();
    let (r, _) = registry(false);
    assert!(jobs
        .begin_file(
            ClientId(1),
            r.clone(),
            "x".into(),
            memory(vec![]),
            None,
            cx(2),
            &pool
        )
        .is_err());
    let mut tickets = Vec::new();
    for _ in 0..64 {
        tickets.push(
            jobs.begin_file(
                ClientId(1),
                r.clone(),
                "x".into(),
                memory(vec![]),
                None,
                cx(1),
                &pool,
            )
            .unwrap(),
        );
    }
    assert!(jobs
        .begin_file(
            ClientId(1),
            r.clone(),
            "x".into(),
            memory(vec![]),
            None,
            cx(1),
            &pool
        )
        .is_err());
    let other = jobs
        .begin_file(
            ClientId(2),
            r.clone(),
            "x".into(),
            memory(vec![]),
            None,
            cx(2),
            &pool,
        )
        .unwrap();
    assert!(!tickets.contains(&other));
    let mut another = ResourceTasks::default();
    let global = another
        .begin_file(
            ClientId(1),
            r,
            "x".into(),
            memory(vec![]),
            None,
            cx(1),
            &pool,
        )
        .unwrap();
    assert!(!tickets.contains(&global) && global != other);
    for ticket in tickets {
        jobs.cancel(ClientId(1), ticket).unwrap();
    }
    let (failed, _) = registry(true);
    let context = cx(1);
    let ticket = jobs
        .begin_file(
            ClientId(1),
            failed,
            "x".into(),
            memory(vec![]),
            None,
            context.clone(),
            &pool,
        )
        .unwrap();
    assert_eq!(ready(&mut jobs, ClientId(1), ticket), "failed");
    assert!(jobs.status(ClientId(1), ticket).unwrap()["error"]
        .as_str()
        .unwrap()
        .contains("codec rejected"));
    assert!(jobs.poll(ClientId(1), ticket).is_err());
    assert!(jobs.poll(ClientId(1), ticket).is_err());
    assert_eq!(context.resident_bytes(), 0);
    jobs.cancel(ClientId(1), ticket).unwrap();
}
struct Pack;
struct Packed(Vec<ArchiveEntry>);
impl Archive for Packed {
    fn entries(&self) -> &[ArchiveEntry] {
        &self.0
    }
    fn open(&self, id: u32, cx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        anyhow::ensure!(id == 0, "entry id");
        cx.memory(b"entry".to_vec())
    }
}
impl ArchiveFormat for Pack {
    fn id(&self) -> &'static str {
        "test/archive"
    }
    fn probe(&self, _: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe::YES)
    }
    fn open(&self, _: Arc<dyn AssetReader>, _: &DecodeContext) -> Result<Box<dyn Archive>> {
        Ok(Box::new(Packed(vec![ArchiveEntry {
            id: 0,
            path: "item".into(),
            size: 5,
            stored_size: 5,
            metadata: serde_json::Value::Null,
        }])))
    }
}
#[test]
fn archive_completion_retains_its_client_context_for_entry_materialization() {
    let pool = TaskPool::new("resource-archive", 1, 2).unwrap();
    let mut registry = FormatRegistry::default();
    registry.register(Arc::new(Pack)).unwrap();
    let mut jobs = ResourceTasks::default();
    let context = cx(4);
    let ticket = jobs
        .begin_archive(
            ClientId(4),
            registry,
            memory(vec![]),
            Some("test/archive".into()),
            context.clone(),
            &pool,
        )
        .unwrap();
    assert_eq!(ready(&mut jobs, ClientId(4), ticket), "ready");
    assert_eq!(jobs.status(ClientId(4), ticket).unwrap()["kind"], "archive");
    let entry = match jobs.poll(ClientId(4), ticket).unwrap().unwrap() {
        ResourceResult::Archive {
            format,
            archive,
            context,
        } => {
            assert_eq!(*format, "test/archive");
            assert_eq!(context.client, ClientId(4));
            archive.open(0, context).unwrap()
        }
        _ => panic!("expected archive"),
    };
    assert_eq!(context.resident_bytes(), 5);
    jobs.cancel(ClientId(4), ticket).unwrap();
    assert_eq!(read_all(entry.as_ref(), 5).unwrap(), b"entry");
    drop(entry);
    assert_eq!(context.resident_bytes(), 0);
}
struct BlockingDecoder {
    started: std::sync::mpsc::Sender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
impl FileFormat for BlockingDecoder {
    fn id(&self) -> &'static str {
        "test/blocked"
    }
    fn kind(&self) -> FileKind {
        FileKind::File
    }
    fn probe(&self, _: &str, _: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe::YES)
    }
    fn decode(&self, _: &str, _: Arc<dyn AssetReader>, cx: &DecodeContext) -> Result<DecodedFile> {
        let reader = cx.decode_with(4, || {
            self.started.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(vec![1; 4])
        })?;
        Ok(DecodedFile {
            reader,
            extension: "dat".into(),
            metadata: serde_json::Value::Null,
        })
    }
}
#[test]
fn cancelling_running_codec_discards_late_output_and_releases_reservation() {
    let pool = TaskPool::new("resource-running-cancel", 1, 2).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let mut registry = FileRegistry::default();
    registry
        .register(Arc::new(BlockingDecoder {
            started: started_tx,
            release: std::sync::Mutex::new(release_rx),
        }))
        .unwrap();
    let context = cx(1);
    let mut jobs = ResourceTasks::default();
    let ticket = jobs
        .begin_file(
            ClientId(1),
            registry,
            "x".into(),
            memory(vec![]),
            None,
            context.clone(),
            &pool,
        )
        .unwrap();
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(context.resident_bytes(), 4);
    jobs.cancel(ClientId(1), ticket).unwrap();
    assert!(jobs.poll(ClientId(1), ticket).is_err());
    release_tx.send(()).unwrap();
    pool.spawn(|_| Ok(())).unwrap().wait().unwrap();
    assert_eq!(context.resident_bytes(), 0);
    assert!(jobs.status(ClientId(1), ticket).is_err());
}
#[test]
fn global_table_limit_counts_ready_results_and_different_clients() {
    let pool = TaskPool::new("resource-global-limit", 1, 512).unwrap();
    let (r, _) = registry(false);
    let mut jobs = ResourceTasks::default();
    let mut tickets = Vec::new();
    for owner in 1..=4 {
        for _ in 0..64 {
            tickets.push((
                ClientId(owner),
                jobs.begin_file(
                    ClientId(owner),
                    r.clone(),
                    "x".into(),
                    memory(vec![]),
                    None,
                    cx(owner),
                    &pool,
                )
                .unwrap(),
            ));
        }
    }
    assert!(jobs
        .begin_file(
            ClientId(5),
            r.clone(),
            "x".into(),
            memory(vec![]),
            None,
            cx(5),
            &pool
        )
        .is_err());
    let (owner, ticket) = tickets.pop().unwrap();
    jobs.cancel(owner, ticket).unwrap();
    let ticket = jobs
        .begin_file(
            ClientId(5),
            r,
            "x".into(),
            memory(vec![]),
            None,
            cx(5),
            &pool,
        )
        .unwrap();
    jobs.cancel(ClientId(5), ticket).unwrap();
    for (owner, ticket) in tickets {
        jobs.cancel(owner, ticket).unwrap();
    }
}
