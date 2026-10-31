//! Original MSI resources are read in place. No original executable runs and no
//! original script/installation files are written or distributed.
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_resource::{installer::InstallerCatalog, read_all, DecodeContext};
use crysirror_system_nnn::{
    machine::{Machine, Step},
    native_bootstrap::{BootEntry, PreparedBoot},
};
use std::{
    io::{Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
struct Reader(Mutex<std::fs::File>, u64);
impl AssetReader for Reader {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.1)
    }
    fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let mut file = self.0.lock().unwrap();
        file.seek(SeekFrom::Start(at))
            .and_then(|_| file.read(out))
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
#[test]
#[ignore = "reads original owner MSI on resource worker; no original module execution"]
fn original_bootstrap_and_multiple_native_script_boundaries() {
    let path = std::env::var("CRYSIRROR_TEST_MSI").unwrap();
    let pool = crysirror_tasks::TaskPool::new("nnn-native-bootstrap", 1, 1).unwrap();
    let mut task=pool.spawn(move|cancel|->anyhow::Result<()> {
        let file=std::fs::File::open(path)?;let size=file.metadata()?.len();
        let mut cx=DecodeContext::new(ClientId(1)).with_cancellation(cancel.clone());cx.limits.max_entry_size=1<<30;
        let catalog=InstallerCatalog::open(Arc::new(Reader(Mutex::new(file),size)),&cx)?;
        let read=|path:&str,limit:u64|->anyhow::Result<Option<Vec<u8>>> {
            let Some(id)=catalog.files().iter().position(|f|f.path==path)else{return Ok(None)};
            let resource=catalog.read(id,&cx)?;Ok(Some(read_all(resource.as_ref(),limit)?))
        };
        let mut boot=PreparedBoot::mugen2(read,"mugen2","mugen2.exe",BootEntry::NewGame,&cancel)?;
        assert_eq!(boot.session.script_name(),b"sys");
        assert_eq!(boot.session.machine().variables().unwrap().read_stored(310),1);
        println!("prepared original module={} project_scripts={} command_pairs={} function_pairs={}",boot.module_sha256,boot.project.tokens.len(),boot.commands.tokens.len()/2,boot.functions.tokens.len()/2);
        for name in [b"sys".as_slice(),b"preprologue",b"prologue",b"1day"] {
            let program=boot.load_script(name,read,&cancel)?;
            let mut machine=Machine::with_variables(boot.session.machine().variables().unwrap().clone());
            let boundary=crysirror_system_nnn::metadata::Metadata::default().advance(&mut machine,&program,1000)?;
            let Step::Native(call)=boundary else{anyhow::bail!("original script did not reach a native boundary")};
            println!("original script={} words={} instructions={} native_pc={:#x} identify={:#x} code={:#x} payload={:?}",String::from_utf8_lossy(name),program.word_count(),machine.executed(),call.pc,call.identify,call.code,call.payload);
            assert_eq!(machine.poll(&program,1)?,Step::Native(call));
        }
        let program=boot.session.program().clone();
        let Step::Native(call)=crysirror_system_nnn::metadata::Metadata::default().advance(boot.session.machine_mut(),&program,1000)?else{anyhow::bail!("original sys must produce native callback")};
        let crysirror_system_nnn::machine::Payload::Control{script:Some(name),..}=&call.payload else{anyhow::bail!("original controller expected script dispatch")};
        let target=boot.load_script(name,read,&cancel)?;
        boot.session.dispatch_control(&call,|requested|{assert_eq!(requested,name);Ok(target.clone())})?;
        assert_eq!(boot.session.depth(),1);assert_eq!(boot.session.script_name(),b"preprologue");
        let program=boot.session.program().clone();
        let frontier=crysirror_system_nnn::metadata::Metadata::default().advance(boot.session.machine_mut(),&program,1000)?;
        println!("original sys->preprologue dispatch depth={} total_instructions={} frontier={frontier:?}",boot.session.depth(),boot.session.executed());
        Ok(())
    }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(()) = task.try_take().unwrap() {
            break;
        }
        assert!(Instant::now() < deadline, "original bootstrap timeout");
        std::thread::yield_now();
    }
}

#[test]
#[ignore = "actual original sys.spt Core port, MSI capability workers and shared pattern resource; no module execution"]
fn original_sys_core_port_dispatches_preprologue_and_creates_shared_suna() {
    use crysirror_core::{service::HostServices, vm::VmAdapter};
    use crysirror_system_nnn::{
        adapter::{NativeAdapter, ScriptLoader},
        bridge::{Bridge, CallbackProfile, SceneBindings},
        Program,
    };
    let path = std::env::var("CRYSIRROR_TEST_MSI").unwrap();
    let pool = crysirror_tasks::TaskPool::new("nnn-original-core-bootstrap", 1, 1).unwrap();
    let mut task = pool
        .spawn(
            move |cancel| -> anyhow::Result<(PreparedBoot, ScriptLoader)> {
                let file = std::fs::File::open(path)?;
                let size = file.metadata()?.len();
                let source: Arc<dyn AssetReader> = Arc::new(Reader(Mutex::new(file), size));
                let mut cx = DecodeContext::new(ClientId(1)).with_cancellation(cancel.clone());
                cx.limits.max_entry_size = 1 << 30;
                let catalog = InstallerCatalog::open(source.clone(), &cx)?;
                let mut boot = PreparedBoot::mugen2(
                    |path, limit| {
                        let Some(id) = catalog.files().iter().position(|f| f.path == path) else {
                            return Ok(None);
                        };
                        Ok(Some(read_all(catalog.read(id, &cx)?.as_ref(), limit)?))
                    },
                    "mugen2",
                    "mugen2.exe",
                    BootEntry::NewGame,
                    &cancel,
                )?;
                let clock = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs();
                boot.initialize_random_state(Some(u32::try_from(clock)?), None)?;
                let names = boot.resources.clone();
                let project = boot.project.clone();
                let profile = boot.scripts;
                let loader: ScriptLoader = Arc::new(move |name, cancel| {
                    cancel.check()?;
                    anyhow::ensure!(project.index(name).is_some(), "original project membership");
                    let mut cx = DecodeContext::new(ClientId(1)).with_cancellation(cancel.clone());
                    cx.limits.max_entry_size = 1 << 30;
                    let catalog = InstallerCatalog::open(source.clone(), &cx)?;
                    let path = names.script(name)?;
                    let id = catalog
                        .files()
                        .iter()
                        .position(|f| f.path == path)
                        .ok_or_else(|| anyhow::anyhow!("original script absent: {path}"))?;
                    let bytes = read_all(catalog.read(id, &cx)?.as_ref(), 16 << 20)?;
                    let program = Arc::new(Program::parse(&bytes, profile)?);
                    cancel.check()?;
                    Ok(program)
                });
                Ok((boot, loader))
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    let (boot, loader) = loop {
        if let Some(value) = task.try_take().unwrap() {
            break value;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let project_path = dir.path().join("project.json");
    std::fs::write(&project_path,serde_json::to_vec(&serde_json::json!({"schema":2,"name":"NNN original sys Core port","content_revision":"1","assets":"assets","canvas":[800,600],"clients":[{"name":"native","adapter":"crysirror-script-v1","game":"owner"}]})).unwrap()).unwrap();
    let mut runtime =
        crysirror_runtime::Runtime::load(&project_path, &dir.path().join("state")).unwrap();
    runtime.host.submit_engine(ClientId(1),1,&serde_json::to_vec(&serde_json::json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,800,600],"paint":{"kind":"solid","color":[0,0,0,255]}}}]})).unwrap()).unwrap();
    let bridge = Bridge::new(
        ClientId(1),
        CallbackProfile::Mugen2Legacy,
        SceneBindings {
            root: 1,
            overlap_canvas: 7,
            effect_base: 100,
            size: [800, 600],
            layers: 16,
            music_channel: 0,
        },
    )
    .unwrap();
    let mut adapter = NativeAdapter::from_bootstrap(boot, loader, bridge).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let frontier = loop {
        match adapter.step(&mut runtime.host, Default::default(), 1000) {
            Ok(_) => runtime.tick(1, Default::default()).unwrap(),
            Err(error) => break error.to_string(),
        }
        assert!(
            Instant::now() < deadline,
            "original Core port frontier timeout"
        );
        std::thread::yield_now();
    };
    println!("original sys Core frontier={frontier}");
    println!(
        "original sys Core diagnostics={}",
        String::from_utf8_lossy(&adapter.diagnostics())
    );
    assert!(
        adapter.bridge().pattern(12).is_some(),
        "actual original EFFECT29 must create a shared engine pattern before subsequent frontier"
    );
    let diagnostics: serde_json::Value = serde_json::from_slice(&adapter.diagnostics()).unwrap();
    assert_eq!(diagnostics["script"], "preprologue");
    assert!(diagnostics["instructions"].as_u64().unwrap() > 61);
    assert_eq!(diagnostics["owns_framebuffer"], false);
    assert_eq!(diagnostics["factory_registered"], false);
    let before = runtime.inspect();
    assert!(adapter
        .step(&mut runtime.host, Default::default(), 1)
        .is_err());
    assert_eq!(runtime.inspect(), before);
}
