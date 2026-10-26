// SPDX-License-Identifier: MPL-2.0
#![cfg(feature = "arc-formats")]
//! File/audio resources reach shared scoped storage and AudioSource without a VM.
use anyhow::Result;
use crysirror_core::service::AssetReader;
use crysirror_engine::{App, Value};
use crysirror_resource::{read_all, DecodeContext, DecodedFile, FileFormat, FileKind, Probe};
use crysirror_runtime::Runtime;
use serde_json::{json, Value as Json};
use std::sync::Arc;
fn app() -> (tempfile::TempDir, App) {
    let tmp = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir_all(tmp.path().join("assets").join(name)).unwrap();
    }
    std::fs::write(tmp.path().join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"file resources","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b","asset_prefix":"b"}]})).unwrap()).unwrap();
    let runtime =
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("state")).unwrap();
    let app = App::new(runtime, "a").unwrap();
    (tmp, app)
}
fn call(app: &mut App, method: &str, args: Json) -> Result<Json> {
    let cx = app.context();
    let resources = cx.singleton("Resources")?;
    let args = args
        .as_array()
        .unwrap()
        .iter()
        .cloned()
        .map(Value::from_json)
        .collect::<Vec<_>>();
    Ok(cx.call(resources, method, &args)?.to_json())
}
fn aff(plain: &[u8]) -> Vec<u8> {
    let mut d = b"AFF\0".to_vec();
    d.resize(16, 0);
    let key = [
        0xc8, 0xbb, 0x8f, 0xb7, 0xed, 0x43, 0x99, 0x4a, 0xa2, 0x7e, 0x5b, 0xb0, 0x68, 0x18, 0xf8,
        0x88,
    ];
    d.extend(
        plain
            .iter()
            .enumerate()
            .map(|(i, b)| if i < 64 { b ^ key[i % 16] } else { *b }),
    );
    d
}
fn bytes(v: &Json) -> Vec<u8> {
    v["bytes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect()
}
fn client(app: &mut App, name: &str) {
    let id = app.runtime().unwrap().client_id(name).unwrap();
    app.context().state().unwrap().client = id;
}
#[test]
fn public_file_decode_detect_and_paging_keep_same_path_client_scoped() {
    let (tmp, mut app) = app();
    std::fs::write(tmp.path().join("assets/a/same.aff"), aff(b"alpha resource")).unwrap();
    std::fs::write(tmp.path().join("assets/b/same.aff"), aff(b"beta resource")).unwrap();
    let formats = call(&mut app, "fileFormats", json!([])).unwrap();
    assert!(formats
        .as_array()
        .unwrap()
        .contains(&json!("alice-soft/aff")));
    let detected = call(&mut app, "detectFile", json!(["same.aff"])).unwrap();
    assert!(detected
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["format"] == "alice-soft/aff" && v["confidence"] == 100));
    let first = call(
        &mut app,
        "readDecoded",
        json!(["same.aff",{"format":"alice-soft/aff","offset":6,"limit":4}]),
    )
    .unwrap();
    assert_eq!(first["format"], "alice-soft/aff");
    assert_eq!(first["extension"], "dat");
    assert_eq!(first["size"], 14);
    assert_eq!(first["offset"], 6);
    assert_eq!(bytes(&first), b"reso");
    client(&mut app, "b");
    let other = call(
        &mut app,
        "readDecoded",
        json!(["same.aff",{"format":"alice-soft/aff"}]),
    )
    .unwrap();
    assert_eq!(bytes(&other), b"beta resource");
    assert!(call(
        &mut app,
        "readDecoded",
        json!(["../a/same.aff",{"format":"alice-soft/aff"}])
    )
    .is_err());
    assert!(call(
        &mut app,
        "readDecoded",
        json!(["same.aff",{"format":"unregistered"}])
    )
    .is_err());
}
#[test]
fn decoded_native_audio_is_a_standard_shared_audio_source() {
    let (tmp, mut app) = app();
    let pcm = [16384i16, -8192, 0, 8192];
    let encoded: Vec<_> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(tmp.path().join("assets/a/tone.P16"), encoded).unwrap();
    let formats = call(&mut app, "audioFormats", json!([])).unwrap();
    assert!(formats.as_array().unwrap().contains(&json!("leaf/p16")));
    let out = call(
        &mut app,
        "readDecoded",
        json!(["tone.P16",{"format":"leaf/p16"}]),
    )
    .unwrap();
    assert_eq!(out["extension"], "wav");
    assert_eq!(out["metadata"]["channels"], 1);
    assert_eq!(out["metadata"]["sample_rate"], 44100);
    let data = bytes(&out);
    assert_eq!(&data[..4], b"RIFF");
    call(
        &mut app,
        "mountDecoded",
        json!(["tone.P16","decoded/tone.wav",{"format":"leaf/p16"}]),
    )
    .unwrap();
    let owner = app.runtime().unwrap().client_id("a").unwrap();
    let other = app.runtime().unwrap().client_id("b").unwrap();
    let mounted = app
        .runtime_mut()
        .unwrap()
        .assets
        .read_for(owner, "decoded/tone.wav")
        .unwrap();
    assert_eq!(mounted.as_ref(), data.as_slice());
    assert!(app
        .runtime_mut()
        .unwrap()
        .assets
        .read_for(other, "decoded/tone.wav")
        .is_err());
    let mut source = crysirror_audio::decoder::open_memory(mounted, Some("wav")).unwrap();
    assert_eq!(source.sample_rate(), 44100);
    let mut frames = [[0.; 2]; 8];
    let count = source.read(&mut frames).unwrap();
    assert_eq!(count, 4);
    for (frame, sample) in frames.iter().zip(pcm) {
        assert_eq!(*frame, [sample as f32 / 32768.; 2]);
    }
}
#[test]
fn decoded_pages_are_bounded_and_parameters_use_immutable_scoped_profiles() {
    let (tmp, mut app) = app();
    let plain = vec![42; 70000];
    std::fs::write(tmp.path().join("assets/a/large.aff"), aff(&plain)).unwrap();
    std::fs::write(tmp.path().join("assets/b/large.aff"), aff(&plain)).unwrap();
    let page = call(
        &mut app,
        "readDecoded",
        json!(["large.aff",{"format":"alice-soft/aff","limit":65536}]),
    )
    .unwrap();
    assert_eq!(bytes(&page).len(), 65536);
    assert_eq!(page["size"], 70000);
    let tail = call(
        &mut app,
        "readDecoded",
        json!(["large.aff",{"format":"alice-soft/aff","offset":65536,"limit":65536}]),
    )
    .unwrap();
    assert_eq!(bytes(&tail), vec![42; 4464]);
    assert!(call(
        &mut app,
        "readDecoded",
        json!(["large.aff",{"format":"alice-soft/aff","limit":65537}])
    )
    .is_err());
    call(
        &mut app,
        "defineProfile",
        json!(["small",{"limits":{"max_entry_size":16},"parameters":{"alice-soft/aff":{}}}]),
    )
    .unwrap();
    assert!(call(
        &mut app,
        "readDecoded",
        json!(["large.aff",{"format":"alice-soft/aff","profile":"small"}])
    )
    .is_err());
    assert!(call(&mut app, "defineProfile", json!(["small", {}])).is_err());
    client(&mut app, "b");
    assert!(call(
        &mut app,
        "readDecoded",
        json!(["large.aff",{"format":"alice-soft/aff","profile":"small"}])
    )
    .is_err());
}
struct Companion;
impl FileFormat for Companion {
    fn id(&self) -> &'static str {
        "test/companion"
    }
    fn kind(&self) -> FileKind {
        FileKind::File
    }
    fn probe(&self, _: &str, _: &dyn AssetReader) -> Result<Probe> {
        Ok(Probe::NO)
    }
    fn decode(&self, _: &str, _: Arc<dyn AssetReader>, cx: &DecodeContext) -> Result<DecodedFile> {
        let name = cx
            .parameters
            .get(self.id())
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("companion.bin");
        let source = cx.open_sibling(name)?;
        let bytes = read_all(source.as_ref(), 128)?;
        Ok(DecodedFile {
            reader: cx.memory(bytes)?,
            extension: "dat".into(),
            metadata: json!({"client":cx.client.0}),
        })
    }
}
#[test]
fn file_companion_resolver_has_client_storage_authority() {
    let (tmp, mut app) = app();
    std::fs::create_dir_all(tmp.path().join("assets/a/sub")).unwrap();
    std::fs::create_dir_all(tmp.path().join("assets/b/sub")).unwrap();
    for who in ["a", "b"] {
        std::fs::write(
            tmp.path().join(format!("assets/{who}/sub/input.bin")),
            b"input",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(format!("assets/{who}/sub/companion.bin")),
            who.as_bytes(),
        )
        .unwrap();
    }
    app.context()
        .state()
        .unwrap()
        .resources
        .files
        .register(Arc::new(Companion))
        .unwrap();
    let a = call(
        &mut app,
        "readDecoded",
        json!(["sub/input.bin",{"format":"test/companion"}]),
    )
    .unwrap();
    assert_eq!(bytes(&a), b"a");
    client(&mut app, "b");
    let b = call(
        &mut app,
        "readDecoded",
        json!(["sub/input.bin",{"format":"test/companion"}]),
    )
    .unwrap();
    assert_eq!(bytes(&b), b"b");
    client(&mut app, "a");
    call(
        &mut app,
        "defineProfile",
        json!(["escape",{"parameters":{"test/companion":{"name":"../../b/sub/companion.bin"}}}]),
    )
    .unwrap();
    assert!(call(
        &mut app,
        "readDecoded",
        json!(["sub/input.bin",{"format":"test/companion","profile":"escape"}])
    )
    .is_err());
}
#[cfg(unix)]
#[test]
fn companion_symlink_cannot_escape_client_capability_root() {
    let (tmp, mut app) = app();
    std::fs::write(tmp.path().join("assets/a/input.bin"), b"input").unwrap();
    std::fs::write(tmp.path().join("assets/b/secret.bin"), b"foreign").unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("assets/b/secret.bin"),
        tmp.path().join("assets/a/companion.bin"),
    )
    .unwrap();
    app.context()
        .state()
        .unwrap()
        .resources
        .files
        .register(Arc::new(Companion))
        .unwrap();
    assert!(call(
        &mut app,
        "readDecoded",
        json!(["input.bin",{"format":"test/companion"}])
    )
    .is_err());
}
fn await_decode(app: &mut App, ticket: u64) -> Json {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let state = call(app, "decodeStatus", json!([ticket])).unwrap();
        if state["state"] != "pending" {
            return state;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "public resource task did not complete"
        );
        std::thread::yield_now();
    }
}
struct Gate(Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
impl Gate {
    fn release(&self) {
        let (lock, wake) = &*self.0;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}
struct MaterializedSource {
    entries: Vec<crysirror_resource::ArchiveEntry>,
    bytes: Vec<u8>,
    opens: Arc<std::sync::atomic::AtomicUsize>,
    owner: std::thread::ThreadId,
}
impl crysirror_resource::Archive for MaterializedSource {
    fn entries(&self) -> &[crysirror_resource::ArchiveEntry] {
        &self.entries
    }
    fn open(&self, id: u32, cx: &DecodeContext) -> Result<Arc<dyn AssetReader>> {
        anyhow::ensure!(id == 0, "source entry id");
        assert_ne!(
            std::thread::current().id(),
            self.owner,
            "source materialized on owner thread"
        );
        self.opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        cx.decode_with(self.bytes.len() as u64, || Ok(self.bytes.clone()))
    }
}
#[test]
fn public_resource_jobs_poll_without_join_and_keep_published_reads_after_cancel() {
    let (_tmp, mut app) = app();
    let owner = app.runtime().unwrap().client_id("a").unwrap();
    let input = aff(b"alpha");
    let opens = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let provider = MaterializedSource {
        entries: vec![crysirror_resource::ArchiveEntry {
            id: 0,
            path: "input.aff".into(),
            size: input.len() as u64,
            stored_size: input.len() as u64,
            metadata: Json::Null,
        }],
        bytes: input,
        opens: opens.clone(),
        owner: std::thread::current().id(),
    };
    let mount =
        crysirror_resource::ArchiveMount::new(Box::new(provider), DecodeContext::new(owner))
            .unwrap();
    app.runtime_mut()
        .unwrap()
        .mount_provider(owner, "source", Arc::new(mount))
        .unwrap();
    app.runtime_mut().unwrap().assets.set_nonblocking(true);
    for (method, args) in [
        ("detectFile", json!(["source/input.aff"])),
        (
            "readDecoded",
            json!(["source/input.aff",{"format":"alice-soft/aff"}]),
        ),
        (
            "mountDecoded",
            json!(["source/input.aff","blocked.aff",{"format":"alice-soft/aff"}]),
        ),
    ] {
        let error = call(&mut app, method, args).unwrap_err().to_string();
        assert!(error.contains("offline-only"), "{method}: {error}");
    }
    assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 0);
    let pool = app.runtime().unwrap().assets.cpu_tasks();
    let gate = Gate(Arc::new((
        std::sync::Mutex::new(false),
        std::sync::Condvar::new(),
    )));
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let mut workers = Vec::new();
    for _ in 0..pool.stats().workers {
        let g = gate.0.clone();
        let started = started_tx.clone();
        workers.push(
            pool.spawn(move |_| {
                started.send(()).unwrap();
                let (lock, wake) = &*g;
                let released = lock.lock().unwrap();
                let (released, timeout) = wake
                    .wait_timeout_while(released, std::time::Duration::from_secs(5), |v| !*v)
                    .unwrap();
                anyhow::ensure!(
                    *released && !timeout.timed_out(),
                    "test worker gate timed out"
                );
                Ok(())
            })
            .unwrap(),
        );
    }
    for _ in 0..workers.len() {
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
    }
    let cancelled = call(
        &mut app,
        "beginDecode",
        json!(["source/input.aff","file",{"format":"alice-soft/aff"}]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    assert_eq!(
        call(&mut app, "decodeStatus", json!([cancelled])).unwrap()["state"],
        "pending"
    );
    assert!(call(&mut app, "readDecode", json!([cancelled])).is_err());
    client(&mut app, "b");
    for method in ["decodeStatus", "readDecode", "cancelDecode"] {
        assert!(call(&mut app, method, json!([cancelled])).is_err());
    }
    assert!(call(&mut app, "mountDecode", json!([cancelled, "foreign.dat"])).is_err());
    client(&mut app, "a");
    call(&mut app, "cancelDecode", json!([cancelled])).unwrap();
    assert!(call(&mut app, "decodeStatus", json!([cancelled])).is_err());
    let ticket = call(
        &mut app,
        "beginDecode",
        json!(["source/input.aff","file",{"format":"alice-soft/aff"}]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    assert_ne!(ticket, cancelled);
    assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 0);
    gate.release();
    for worker in workers {
        worker.wait().unwrap();
    }
    assert_eq!(await_decode(&mut app, ticket)["state"], "ready");
    assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 1);
    let first = call(&mut app, "readDecode", json!([ticket, 1, 3])).unwrap();
    assert_eq!(first["kind"], "file");
    assert_eq!(bytes(&first), b"lph");
    assert!(call(&mut app, "readDecode", json!([ticket, 0, 65537])).is_err());
    assert_eq!(
        bytes(&call(&mut app, "readDecode", json!([ticket])).unwrap()),
        b"alpha"
    );
    call(
        &mut app,
        "mountDecode",
        json!([ticket, "decoded/alpha.dat"]),
    )
    .unwrap();
    call(&mut app, "cancelDecode", json!([ticket])).unwrap();
    assert!(call(&mut app, "readDecode", json!([ticket])).is_err());
    let owner = app.runtime().unwrap().client_id("a").unwrap();
    assert_eq!(
        app.runtime_mut()
            .unwrap()
            .assets
            .read_for(owner, "decoded/alpha.dat")
            .unwrap()
            .as_ref(),
        b"alpha"
    );
    client(&mut app, "b");
    let foreign = app.runtime().unwrap().client_id("b").unwrap();
    assert!(app
        .runtime_mut()
        .unwrap()
        .assets
        .read_for(foreign, "decoded/alpha.dat")
        .is_err());
}
fn sar(name: &str, plain: &[u8]) -> Vec<u8> {
    let start = 6 + name.len() + 1 + 8;
    let mut d = 1u16.to_be_bytes().to_vec();
    d.extend_from_slice(&(start as u32).to_be_bytes());
    d.extend_from_slice(name.as_bytes());
    d.push(0);
    d.extend_from_slice(&0u32.to_be_bytes());
    d.extend_from_slice(&(plain.len() as u32).to_be_bytes());
    d.extend_from_slice(plain);
    d
}
#[test]
fn asynchronous_archive_publication_and_failed_jobs_stay_client_owned() {
    let (tmp, mut app) = app();
    std::fs::write(
        tmp.path().join("assets/a/pack.sar"),
        sar("item.txt", b"shared archive"),
    )
    .unwrap();
    let ticket = call(
        &mut app,
        "beginDecode",
        json!(["pack.sar","archive",{"format":"nscripter/sar"}]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    assert_eq!(await_decode(&mut app, ticket)["state"], "ready");
    let page = call(&mut app, "readDecode", json!([ticket, 0, 1])).unwrap();
    assert_eq!(page["kind"], "archive");
    assert_eq!(page["format"], "nscripter/sar");
    assert_eq!(page["total"], 1);
    assert_eq!(page["entries"][0]["path"], "item.txt");
    assert!(call(&mut app, "readDecode", json!([ticket, 0, 1025])).is_err());
    call(&mut app, "mountDecode", json!([ticket, "pack"])).unwrap();
    call(&mut app, "cancelDecode", json!([ticket])).unwrap();
    let owner = app.runtime().unwrap().client_id("a").unwrap();
    assert_eq!(
        app.runtime_mut()
            .unwrap()
            .assets
            .read_for(owner, "pack/item.txt")
            .unwrap()
            .as_ref(),
        b"shared archive"
    );
    std::fs::write(tmp.path().join("assets/a/bad.aff"), b"AFF\0").unwrap();
    let failed = call(
        &mut app,
        "beginDecode",
        json!(["bad.aff","file",{"format":"alice-soft/aff"}]),
    )
    .unwrap()
    .as_u64()
    .unwrap();
    let state = await_decode(&mut app, failed);
    assert_eq!(state["state"], "failed");
    assert!(state["error"].as_str().is_some_and(|v| !v.is_empty()));
    assert!(call(&mut app, "readDecode", json!([failed])).is_err());
    assert!(call(&mut app, "mountDecode", json!([failed, "bad.dat"])).is_err());
    call(&mut app, "cancelDecode", json!([failed])).unwrap();
}
