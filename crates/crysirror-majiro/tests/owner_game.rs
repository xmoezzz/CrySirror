//! Local original resources only; no game files or native executables run.
use anyhow::{Context, Result};
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_majiro::{
    machine::{Event, Machine, Value},
    program::Program,
};
use crysirror_resource::{read_all, ArchiveFormat, DecodeContext};
use std::{
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Reader(Mutex<std::fs::File>);
impl AssetReader for Reader {
    fn len(&self) -> crysirror_core::Result<u64> {
        self.0
            .lock()
            .unwrap()
            .metadata()
            .map(|m| m.len())
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
    fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let mut f = self.0.lock().unwrap();
        f.seek(SeekFrom::Start(at))
            .and_then(|_| f.read(out))
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
#[test]
#[ignore = "reads explicitly selected local owner Majiro data; never distributes game bytes"]
fn owner_native_mjo_corpus_and_real_start_frontier() -> Result<()> {
    let root = PathBuf::from(std::env::var_os("CRYSIRROR_MAJIRO_GAME").context("owner game root")?);
    let cx = DecodeContext::new(ClientId(1));
    let source = Arc::new(Reader(Mutex::new(std::fs::File::open(
        root.join("scenario1.arc"),
    )?)));
    let archive = crysirror_formats_arc::MajiroArc.open(source, &cx)?;
    let mut parsed = 0;
    let mut instructions = 0;
    for entry in archive.entries() {
        if !entry.path.to_ascii_lowercase().ends_with(".mjo") {
            continue;
        }
        let bytes = read_all(archive.open(entry.id, &cx)?.as_ref(), 16 * 1024 * 1024)?;
        let p = Program::parse(&bytes).with_context(|| format!("native module {}", entry.path))?;
        parsed += 1;
        instructions += p.instructions.len();
        if entry.path.eq_ignore_ascii_case("start.mjo") {
            println!(
                "start entry={:#x} functions={} code={}",
                p.entry,
                p.functions.len(),
                p.code_size
            );
            let mut m = Machine::new(Arc::new(p));
            for _ in 0..1000 {
                if let Some(e) = m.step()? {
                    assert_eq!(
                        e,
                        Event::Native {
                            pc: 0x2056,
                            hash: 0x0ebc053b,
                            arguments: vec![Value::Integer(0)],
                            returns: false
                        }
                    );
                    assert_eq!(m.pc, 0x2056);
                    println!(
                        "actual pending={e:?} executed={} line={}",
                        m.executed, m.line
                    );
                    break;
                }
            }
        }
    }
    println!("parsed={parsed} instructions={instructions}");
    assert_eq!(parsed, 181);
    Ok(())
}
#[cfg(feature = "runtime")]
#[test]
#[ignore = "reads explicitly selected local owner game; no renderer/window/executable runs"]
fn owner_original_start_runs_through_shared_runtime_until_real_native_frontier() -> Result<()> {
    use crysirror_core::{service::InputFrame, vfs::MemoryProvider};
    use crysirror_resource::ArchiveMount;
    use crysirror_runtime::{AdapterRegistry, Project, Runtime};
    use std::time::{Duration, Instant};
    let root = PathBuf::from(std::env::var_os("CRYSIRROR_MAJIRO_GAME").context("owner game root")?);
    let temp = tempfile::tempdir()?;
    std::fs::create_dir(temp.path().join("assets"))?;
    let p = serde_json::json!({"schema":2,"name":"original Majiro resource probe","content_revision":"1","canvas":[800,600],"assets":"assets","clients":[{"name":"native","adapter":"majiro","game":"owner","config":{"script":"scenario/start.mjo","executable":"module/akatsuki.exe","environment":"fastdata/majiro.env","settings_profile":"portable_client","run_mode":1}}]});
    let path = temp.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p)?)?;
    let mut registry = AdapterRegistry::default();
    crysirror_majiro::adapter::register(&mut registry)?;
    let mut rt = Runtime::with_registry(
        Project::load(&path)?,
        &path,
        &temp.path().join("state"),
        registry,
    )?;
    let cx = DecodeContext::new(ClientId(1));
    let archive = crysirror_formats_arc::MajiroArc.open(
        Arc::new(Reader(Mutex::new(std::fs::File::open(
            root.join("scenario1.arc"),
        )?))),
        &cx,
    )?;
    rt.mount_provider(
        ClientId(1),
        "scenario",
        Arc::new(ArchiveMount::new(archive, cx)?),
    )?;
    let cx = DecodeContext::new(ClientId(1));
    let fast = crysirror_formats_arc::MajiroArc.open(
        Arc::new(Reader(Mutex::new(std::fs::File::open(
            root.join("fastdata1.arc"),
        )?))),
        &cx,
    )?;
    rt.mount_provider(
        ClientId(1),
        "fastdata",
        Arc::new(ArchiveMount::new(fast, cx)?),
    )?;
    let mut module = MemoryProvider::new();
    module.insert(
        "akatsuki.exe",
        std::fs::read(root.join("bin/暁の護衛.exe"))?,
    )?;
    rt.mount_provider(ClientId(1), "module", Arc::new(module))?;
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        if let Err(e) = rt.tick(16, InputFrame::default()) {
            println!("actual runtime frontier: {e:#}");
            let inspected = rt.inspect();
            println!("native={}", inspected["sessions"]);
            let native = &inspected["sessions"][0]["native"];
            assert_eq!(native["native_profile_enabled"], true);
            assert!(native["fast_disable_depth"].as_i64().unwrap() > 0);
            assert_ne!(native["pc"], 0x23e);
            assert_eq!(native["keyboard_locks"]["policy"], "engine_logical");
            assert!(!crysirror_core::service::HostServices::keyboard_locks(&rt.host, ClientId(1)).unwrap().logical.caps_lock);
            assert!(native["instructions"].as_u64().unwrap() > 5);
            assert!(format!("{e:#}").contains("Majiro"));
            break;
        }
        assert!(
            Instant::now() < until,
            "original Majiro probe did not reach a visible frontier"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
