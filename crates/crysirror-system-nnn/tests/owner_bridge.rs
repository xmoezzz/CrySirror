//! Owner data stays under Downloads. Heavy MSI/SPT/PE work is a cancellable
//! resource worker dependency; only the engine owner submits presentation calls.
use crysirror_core::{
    service::{AssetReader, HostServices},
    state::ClientId,
};
use crysirror_resource::{installer::InstallerCatalog, read_all, DecodeContext};
use crysirror_runtime::Runtime;
use crysirror_system_nnn::{
    bridge::{Bridge, CallbackProfile, SceneBindings},
    config,
    expression::VariableProfile,
    machine::{Machine, Step},
    Program, XorProfile,
};
use serde_json::json;
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
        let mut f = self.0.lock().unwrap();
        f.seek(SeekFrom::Start(at))
            .and_then(|_| f.read(out))
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
#[test]
#[ignore = "reads original owner MSI data on a scoped CPU worker; no original executable runs"]
fn owner_original_mugen2_calls_commit_shared_engine_state_until_next_actual_effect() {
    let path = std::env::var("CRYSIRROR_TEST_MSI").unwrap();
    let pool = crysirror_tasks::TaskPool::new("nnn-original-bootstrap", 1, 1).unwrap();
    let mut task = pool
        .spawn(move |cancel| {
            let f = std::fs::File::open(path)?;
            let len = f.metadata()?.len();
            let mut cx = DecodeContext::new(ClientId(1)).with_cancellation(cancel.clone());
            cx.limits.max_entry_size = 1024 * 1024 * 1024;
            let catalog = InstallerCatalog::open(Arc::new(Reader(Mutex::new(f), len)), &cx)?;
            let read = |path: &str, limit: u64| -> anyhow::Result<Vec<u8>> {
                let (id, _) = catalog
                    .files()
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.path == path)
                    .ok_or_else(|| anyhow::anyhow!("original resource absent: {path}"))?;
                let r = catalog.read(id, &cx)?;
                Ok(read_all(r.as_ref(), limit)?)
            };
            let exe = read("mugen2/mugen2.exe", 32 << 20)?;
            let sha = crysirror_resource::pe::PeData::parse(&exe)?.sha256();
            let hash = sha.iter().map(|b| format!("{b:02x}")).collect::<String>();
            anyhow::ensure!(
                hash == "e85e92ec538d50eae08d27286177149972d443a96d1415277c1ae1f9ee108bdb",
                "unreviewed native callback module"
            );
            let p = Program::parse(
                &read("mugen2/spt/preprologue.spt", 16 << 20)?,
                XorProfile::default(),
            )?;
            let names = config::Names::parse(&read("mugen2/spt/var.fxf", 1 << 20)?, true)?;
            let game = config::Names::parse(&read("mugen2/init2/game.fxf", 1 << 20)?, true)?;
            let initial =
                config::Names::parse(&read("mugen2/spt/varinitdata.xtx", 1 << 20)?, false)?;
            anyhow::ensure!(
                names.tokens.len() == 1000,
                "original legacy variable profile"
            );
            let mut variables =
                config::configured_variables(VariableProfile::Legacy1000, &names, &game)?;
            config::initialize_new_game(
                &mut variables,
                VariableProfile::Legacy1000,
                (game.integer(b"varInitFlag")?.unwrap_or(0) != 0).then_some(&initial),
            )?;
            cancel.check()?;
            Ok((p, variables, hash))
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    let (p, vars, hash) = loop {
        if let Some(v) = task.try_take().unwrap() {
            break v;
        }
        assert!(Instant::now() < until);
        std::thread::yield_now();
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"schema":2,"name":"NNN original owner bridge","content_revision":"1","assets":"assets","canvas":[800,600],"clients":[{"name":"native","adapter":"crysirror-script-v1","game":"owner"}]})).unwrap()).unwrap();
    let mut rt = Runtime::load(&path, &dir.path().join("state")).unwrap();
    // This is explicitly an existing opaque caller backdrop, not a claimed
    // reconstructed original title screen or a hosted legacy framebuffer.
    rt.host.submit_engine(ClientId(1),1,&serde_json::to_vec(&json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,800,600],"paint":{"kind":"solid","color":[0,0,0,255]}}}]})).unwrap()).unwrap();
    let port_vars = vars.clone();
    let mut machine = Machine::with_variables(vars);
    let mut bridge = Bridge::new(
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
    let mut reached = None;
    for _ in 0..100 {
        let Step::Native(call) = machine.poll(&p, 100).unwrap() else {
            panic!("expected native boundary")
        };
        match bridge.commit(&mut machine, &call, &mut rt.host) {
            Ok(()) => println!(
                "committed pc={:#x} code={:#x} {:?}",
                call.pc, call.code, call.payload
            ),
            Err(e) => {
                println!("actual next boundary: {e:#} payload={:?}", call.payload);
                assert_eq!(machine.poll(&p, 10).unwrap(), Step::Native(call.clone()));
                reached = Some(call);
                break;
            }
        }
    }
    let call = reached.expect("must retain unimplemented original effect");
    assert_eq!(call.pc, 0x1a1);
    assert_eq!(call.code, 0x33330013);
    assert!(machine.executed() > 16);
    assert_eq!(bridge.overlap_captures, 1);
    assert_eq!(bridge.previous_effect(0), Some(54));
    let layer = rt
        .host
        .engine
        .layers
        .get(ClientId(1), 100)
        .expect("original EFFECT_FILL owns shared layer");
    println!("module_sha256={hash} original_instructions={} pc={:#x} engine_api_calls={} captures={} shared_layer={layer:?}",machine.executed(),machine.pc(),bridge.api_calls,bridge.overlap_captures);

    // Run the same actual source through the Core VmAdapter port. A fresh
    // Runtime gives it a fresh owner scope instead of inheriting helper state.
    use crysirror_core::vm::VmAdapter;
    let mut port_rt = Runtime::load(&path, &dir.path().join("port-state")).unwrap();
    port_rt.host.submit_engine(ClientId(1),1,&serde_json::to_vec(&json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,800,600],"paint":{"kind":"solid","color":[0,0,0,255]}}}]})).unwrap()).unwrap();
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
    let mut vm = crysirror_system_nnn::adapter::NativeAdapter::from_prepared(
        Arc::new(p),
        Machine::with_variables(port_vars),
        bridge,
    )
    .unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Err(e) = vm.step(&mut port_rt.host, Default::default(), 1000) {
            println!("actual Core VmAdapter frontier={e}");
            assert!(e.to_string().contains("pc=0x1a1"));
            break;
        }
        port_rt.tick(1, Default::default()).unwrap();
        assert!(
            Instant::now() < until,
            "VM port failed to reach original boundary"
        );
        std::thread::yield_now();
    }
    let diagnostics: serde_json::Value = serde_json::from_slice(&vm.diagnostics()).unwrap();
    assert_eq!(diagnostics["instructions"], 25);
    assert_eq!(diagnostics["worker_pending"], false);
    assert_eq!(diagnostics["engine_api_calls"], 6);
    assert_eq!(diagnostics["overlap_captures"], 1);
    assert!(port_rt.host.engine.layers.get(ClientId(1), 100).is_some());
    assert!(vm.snapshot().is_err());
    assert!(vm.restore(&[]).is_err());
    let before = port_rt.inspect();
    assert!(vm.step(&mut port_rt.host, Default::default(), 100).is_err());
    assert_eq!(port_rt.inspect(), before);
    println!("core_port={diagnostics}");
}
