//! Explicit local owner input; never ships game bytes, fonts or archive keys.
use anyhow::{ensure, Context, Result};
use crysirror_catsystem2::{
    kcs::{Event, Machine, Program},
    parse_startup,
    scene::Scene,
};
use crysirror_core::{service::AssetReader, state::ClientId};
use crysirror_resource::{DecodeContext, DependencyResolver};
use serde_json::json;
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
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> crysirror_core::Result<usize> {
        let mut f = self.0.lock().unwrap();
        f.seek(SeekFrom::Start(offset))
            .and_then(|_| f.read(buf))
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
struct Root(PathBuf);
impl DependencyResolver for Root {
    fn open(&self, path: &str) -> Result<Arc<dyn AssetReader>> {
        ensure!(!path.split('/').any(|p| p == ".."), "escaped owner root");
        Ok(Arc::new(Reader(Mutex::new(std::fs::File::open(
            self.0.join(path),
        )?))))
    }
}
#[test]
#[ignore = "requires explicitly authorized CRYSIRROR_CATSYSTEM2_GAME; no game data shipped"]
fn owner_original_archives_scripts_and_kcs_entry_frontier() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("CRYSIRROR_CATSYSTEM2_GAME").context("game data directory")?,
    );
    let resolver = Arc::new(Root(root.clone()));
    let mut cx = DecodeContext::new(ClientId(1));
    // Index admission is independent of the bounded script read below. Movies
    // may be large; this test never materializes those payloads.
    cx.limits.max_entry_size = 1024 * 1024 * 1024;
    cx.resolver = Some(resolver.clone());
    cx.parameters
        .insert("cat-system/int".into(), json!({"executable":"cs2.exe"}));
    let format = crysirror_formats_arc::legacy_archive("cat-system/int").unwrap();
    let startup = parse_startup(&std::fs::read(root.join("config/startup.xml"))?)?;
    let mut paths = std::fs::read_dir(&root)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|p| p.file_name().to_string_lossy().into_owned())
        .filter(|p| p.to_ascii_lowercase().ends_with(".int"))
        .collect::<Vec<_>>();
    paths.sort();
    let mut archive_count = 0;
    let mut entry_count = 0;
    let mut kcs_count = 0;
    let mut cst_count = 0;
    let mut main = None;
    for name in paths {
        cx.source_name = Some(name.clone());
        let archive = format
            .open(resolver.open(&name)?, &cx)
            .with_context(|| name.clone())?;
        archive_count += 1;
        entry_count += archive.entries().len();
        for entry in archive.entries() {
            if name == "kcs.int" && entry.path.to_ascii_lowercase().ends_with(".kcs") {
                let bytes = crysirror_resource::read_all(
                    archive.open(entry.id, &cx)?.as_ref(),
                    16 * 1024 * 1024,
                )?;
                let program =
                    Program::parse(&bytes).with_context(|| format!("{name}/{}", entry.path))?;
                kcs_count += 1;
                if format!("{name}/{}", entry.path) == startup.script {
                    main = Some(program);
                }
            } else if name == "scene.int" && entry.path.to_ascii_lowercase().ends_with(".cst") {
                let bytes = crysirror_resource::read_all(
                    archive.open(entry.id, &cx)?.as_ref(),
                    16 * 1024 * 1024,
                )?;
                Scene::parse(&bytes).with_context(|| format!("{name}/{}", entry.path))?;
                cst_count += 1;
            }
        }
        println!("archive={name}; entries={}", archive.entries().len());
    }
    let program = main.context("original configured KCS entry was not found")?;
    let mut machine = Machine::new(&program, startup.heap_bytes)?;
    let event = machine.step(&program, 1000)?;
    let Event::Native(call) = event else {
        anyhow::bail!("original entry did not reach a native call")
    };
    // Report the actual continuation, rather than acknowledging it as a no-op.
    println!("original entry frontier={}; native_id={}; pc={:#x}; args={:?}; instructions={}; archives={archive_count}; entries={entry_count}; kcs={kcs_count}; cst={cst_count}",startup.script,call.id,call.pc,call.arguments,machine.instructions);
    ensure!(
        machine.pending.as_ref() == Some(&call) && machine.pc == call.pc,
        "native call was prematurely acknowledged"
    );
    Ok(())
}

struct GameStorage {
    root: PathBuf,
    kcs: crysirror_resource::ArchiveMount,
}
impl crysirror_core::service::ClientStorage for GameStorage {
    fn read(&self, path: &crysirror_core::path::VfsPath) -> crysirror_core::Result<Vec<u8>> {
        crysirror_resource::read_all(self.open_asset(path)?.as_ref(), 64 * 1024 * 1024)
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
    fn open_asset(
        &self,
        path: &crysirror_core::path::VfsPath,
    ) -> crysirror_core::Result<Arc<dyn AssetReader>> {
        use crysirror_core::{path::Mount, vfs::VfsProvider};
        if path.mount() != Mount::Asset {
            return Err(crysirror_core::Error::ReadOnly);
        }
        if let Some(member) = path.relative().strip_prefix("kcs.int/") {
            return self.kcs.open(member);
        }
        let file = std::fs::File::open(self.root.join(path.relative()))
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))?;
        Ok(Arc::new(Reader(Mutex::new(file))))
    }
    fn metadata(
        &self,
        mount: crysirror_core::path::Mount,
        path: &str,
    ) -> crysirror_core::Result<Option<crysirror_core::service::StorageMetadata>> {
        if mount != crysirror_core::path::Mount::Asset {
            return Err(crysirror_core::Error::ReadOnly);
        }
        match std::fs::metadata(self.root.join(path)) {
            Ok(m) => Ok(Some(crysirror_core::service::StorageMetadata {
                is_directory: m.is_dir(),
                size: Some(m.len()),
                created_ms: None,
                modified_ms: None,
                accessed_ms: None,
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(crysirror_core::Error::Io(e.to_string())),
        }
    }
    fn write_atomic(
        &self,
        _: &crysirror_core::path::VfsPath,
        _: &[u8],
    ) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
    fn remove(&self, _: &crysirror_core::path::VfsPath) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
    fn list(&self, _: crysirror_core::path::Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
        Err(crysirror_core::Error::Unsupported("test listing"))
    }
}
struct MenuHost {
    storage: Arc<GameStorage>,
    ui: crysirror_scene::ui::UiSystem,
}
impl crysirror_core::service::FileSystem for MenuHost {
    fn read(
        &mut self,
        client: ClientId,
        path: &crysirror_core::path::VfsPath,
    ) -> crysirror_core::Result<Vec<u8>> {
        use crysirror_core::service::ClientStorage;
        if client != ClientId(1) {
            return Err(crysirror_core::Error::NotFound);
        }
        self.storage.read(path)
    }
    fn write_atomic(
        &mut self,
        _: ClientId,
        _: &crysirror_core::path::VfsPath,
        _: &[u8],
    ) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
}
impl crysirror_core::service::HostServices for MenuHost {
    fn submit(
        &mut self,
        _: ClientId,
        _: &[crysirror_core::state::Command],
    ) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::Unsupported("test legacy commands"))
    }
    fn now_ms(&self) -> u64 {
        0
    }
    fn storage(
        &mut self,
        client: ClientId,
    ) -> crysirror_core::Result<Arc<dyn crysirror_core::service::ClientStorage>> {
        if client != ClientId(1) {
            return Err(crysirror_core::Error::NotFound);
        }
        Ok(self.storage.clone())
    }
    fn submit_engine(
        &mut self,
        client: ClientId,
        schema: u32,
        data: &[u8],
    ) -> crysirror_core::Result<()> {
        if client != ClientId(1) || schema != 1 {
            return Err(crysirror_core::Error::Invalid("test scope/schema"));
        }
        let v: serde_json::Value =
            serde_json::from_slice(data).map_err(|e| crysirror_core::Error::Io(e.to_string()))?;
        let commands: Vec<crysirror_scene::ui::UiCommand> = serde_json::from_value(v["ui"].clone())
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))?;
        self.ui
            .apply_batch(client, &commands)
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
    fn query_engine(
        &mut self,
        client: ClientId,
        query: crysirror_core::service::EngineQuery,
    ) -> crysirror_core::Result<Vec<u8>> {
        if client != ClientId(1)
            || !matches!(query, crysirror_core::service::EngineQuery::MenuEvents)
        {
            return Err(crysirror_core::Error::Unsupported("test engine query"));
        }
        serde_json::to_vec(&self.ui.events).map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
#[test]
#[ignore = "requires explicitly authorized CRYSIRROR_CATSYSTEM2_GAME; no game data shipped"]
fn owner_original_adapter_publishes_shared_menu_before_next_native_call() -> Result<()> {
    use crysirror_core::vm::VmAdapter;
    let root = PathBuf::from(
        std::env::var_os("CRYSIRROR_CATSYSTEM2_GAME").context("game data directory")?,
    );
    let resolver = Arc::new(Root(root.clone()));
    let mut cx = DecodeContext::new(ClientId(1));
    cx.resolver = Some(resolver.clone());
    cx.source_name = Some("kcs.int".into());
    cx.parameters
        .insert("cat-system/int".into(), json!({"executable":"cs2.exe"}));
    let format = crysirror_formats_arc::legacy_archive("cat-system/int").unwrap();
    let archive = format.open(resolver.open("kcs.int")?, &cx)?;
    let startup = parse_startup(&std::fs::read(root.join("config/startup.xml"))?)?;
    let config = crysirror_catsystem2::Config {
        script: startup.script,
        font: "default".into(),
        heap_bytes: startup.heap_bytes,
        executable: Some("cs2.exe".into()),
        canvas: startup.canvas,
    };
    let mut vm = crysirror_catsystem2::CatSystem2Vm::new(ClientId(1), config)?;
    let mut host = MenuHost {
        storage: Arc::new(GameStorage {
            root,
            kcs: crysirror_resource::ArchiveMount::new(archive, cx)?,
        }),
        ui: Default::default(),
    };
    let start = std::time::Instant::now();
    let error = loop {
        match vm.step(&mut host, Default::default(), 1000) {
            Ok(_) => {
                ensure!(
                    start.elapsed() < std::time::Duration::from_secs(10),
                    "native source worker did not complete"
                );
                std::thread::yield_now();
            }
            Err(e) => break e.to_string(),
        }
    };
    println!("actual adapter frontier: {error}");
    println!("{}", String::from_utf8(vm.diagnostics())?);
    ensure!(
        error.contains("unsupported native gcall") || error.contains("unsupported KCS opcode"),
        "unexpected native frontier: {error}"
    );
    let menu = host
        .ui
        .menus
        .get(1, 104)
        .context("engine-owned original production menu missing")?;
    let item = crysirror_scene::ui::menu::MenuItemSpec::find(&menu.items, 119)
        .context("engine Return to title item")?;
    ensure!(
        !item.enabled,
        "native gcall393 did not disable engine menu item"
    );
    println!(
        "shared engine menu: bar104, item119 enabled={}, label={}",
        item.enabled, item.label
    );
    host.ui.events.push(crysirror_scene::ui::UiEvent {
        widget: crysirror_scene::ui::WidgetId {
            owner: 1,
            id: 40002,
        },
        kind: "menu_activate".into(),
        value: 0.,
        menu_bar: Some(104),
    });
    let error = vm
        .step(&mut host, Default::default(), 1000)
        .unwrap_err()
        .to_string();
    ensure!(
        error.contains("WM_COMMAND 40002"),
        "unmapped menu click was silently ignored"
    );
    Ok(())
}
