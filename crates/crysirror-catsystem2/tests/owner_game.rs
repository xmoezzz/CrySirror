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
        // Validate every actual shared mounted namespace, not only raw tables.
        let mounted = crysirror_resource::ArchiveMount::new(archive, cx.clone())?;
        entry_count += mounted.entries().len();
        for entry in mounted.entries() {
            if name == "bgm.int" && entry.path.to_ascii_lowercase().contains("looppoint") {
                let bytes = crysirror_resource::read_all(mounted.open_entry(&entry.path)?.as_ref(), 1024 * 1024)?;
                println!("owner loop metadata {} bytes={} lines={} prefix={:?}",entry.path,bytes.len(),bytes.split(|b|*b==b'\n').count(),String::from_utf8_lossy(&bytes[..bytes.len().min(600)]));
            }
            if name == "kcs.int" && entry.path.to_ascii_lowercase().ends_with(".kcs") {
                let bytes = crysirror_resource::read_all(
                    mounted.open_entry(&entry.path)?.as_ref(),
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
                    mounted.open_entry(&entry.path)?.as_ref(),
                    16 * 1024 * 1024,
                )?;
                Scene::parse(&bytes).with_context(|| format!("{name}/{}", entry.path))?;
                cst_count += 1;
            }
        }
        println!(
            "archive={name}; mounted_entries={}",
            mounted.entries().len()
        );
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
    save:PathBuf,
    kcs: crysirror_resource::ArchiveMount,
    config: crysirror_resource::ArchiveMount,
    bgm: crysirror_resource::ArchiveMount,
}
impl crysirror_core::service::ClientStorage for GameStorage {
    fn read(&self, path: &crysirror_core::path::VfsPath) -> crysirror_core::Result<Vec<u8>> {
        if path.mount()==crysirror_core::path::Mount::Save {
            return std::fs::read(self.save.join(path.relative())).map_err(|e|if e.kind()==std::io::ErrorKind::NotFound {crysirror_core::Error::NotFound}else{crysirror_core::Error::Io(e.to_string())});
        }
        crysirror_resource::read_all(self.open_asset(path)?.as_ref(), 64 * 1024 * 1024)
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
    fn open_read(&self,path:&crysirror_core::path::VfsPath)->crysirror_core::Result<Arc<dyn AssetReader>> {
        if path.mount()==crysirror_core::path::Mount::Asset {return self.open_asset(path);}
        if path.mount()!=crysirror_core::path::Mount::Save {return Err(crysirror_core::Error::ReadOnly);}
        let file=std::fs::File::open(self.save.join(path.relative())).map_err(|e|if e.kind()==std::io::ErrorKind::NotFound {crysirror_core::Error::NotFound}else{crysirror_core::Error::Io(e.to_string())})?;
        Ok(Arc::new(Reader(Mutex::new(file))))
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
        if let Some(member) = path.relative().strip_prefix("bgm.int/") { return self.bgm.open(member); }
        if let Some(member) = path.relative().strip_prefix("config.int/") {
            return self.config.open(member);
        }
        let file = std::fs::File::open(self.root.join(path.relative()))
            .map_err(|e|if e.kind()==std::io::ErrorKind::NotFound{crysirror_core::Error::NotFound}else{crysirror_core::Error::Io(e.to_string())})?;
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
    loops: crysirror_audio::loop_catalog::AudioLoopCatalog,
    logs: crysirror_core::service::LogBook,
    layers: crysirror_scene::layer::LayerSystem,
    routes:crysirror_scene::input_routes::InputRouting,
    shared_images:crysirror_runtime::Runtime,
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
    fn image_metadata(&mut self,client:ClientId)->crysirror_core::Result<Arc<dyn crysirror_core::service::ImageMetadata>> {self.shared_images.host.image_metadata(client)}
    fn publish_image_resource(&mut self,client:ClientId,id:u32,image:Arc<dyn crysirror_core::image_processing::ImageResource>)->crysirror_core::Result<crysirror_core::service::UploadedImage> {self.shared_images.host.publish_image_resource(client,id,image)}
    fn release_image_resource(&mut self,client:ClientId,path:&str)->crysirror_core::Result<()> {self.shared_images.host.release_image_resource(client,path)}
    fn append_log(&mut self,client:ClientId,message:&str)->crysirror_core::Result<crysirror_core::service::LogReceipt> {if client!=ClientId(1){return Err(crysirror_core::Error::NotFound);} println!("original native debug log: {message}");self.logs.append(client,0,message)}
    fn log_snapshot(&self,client:ClientId)->crysirror_core::Result<crysirror_core::service::LogSnapshot> {if client!=ClientId(1){return Err(crysirror_core::Error::NotFound);}Ok(self.logs.snapshot(client))}
    fn publish_audio_loop_metadata(&mut self,client:ClientId,entries:&[crysirror_core::service::AudioLoopMetadata])->crysirror_core::Result<crysirror_core::service::AudioLoopCatalogReceipt> { if client!=ClientId(1) {return Err(crysirror_core::Error::NotFound);} self.loops.publish(client,entries) }
    fn audio_loop_metadata(&self,client:ClientId,asset:&str)->crysirror_core::Result<Option<crysirror_core::service::AudioLoopMetadata>> {if client!=ClientId(1) {return Err(crysirror_core::Error::NotFound);} self.loops.get(client,asset)}

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
        if let Some(ui)=v.get("ui") {
            let commands: Vec<crysirror_scene::ui::UiCommand> = serde_json::from_value(ui.clone())
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))?;
            self.ui.apply_batch(client, &commands).map_err(|e| crysirror_core::Error::Io(e.to_string()))?;
        }
        if let Some(routes)=v.get("input_routes") {
            let commands:Vec<crysirror_scene::input_routes::InputRouteCommand>=serde_json::from_value(routes.clone()).map_err(|e|crysirror_core::Error::Io(e.to_string()))?;
            self.routes.apply_batch(client,&commands).map_err(|e|crysirror_core::Error::Io(e.to_string()))?;
        }
        if let Some(layers)=v.get("layers") {
            let commands:Vec<crysirror_scene::layer::LayerCommand>=serde_json::from_value(layers.clone()).map_err(|e|crysirror_core::Error::Io(e.to_string()))?;
            self.layers.apply_batch(client,&commands).map_err(|e|crysirror_core::Error::Io(e.to_string()))?;
        }
        Ok(())
    }
    fn query_engine(
        &mut self,
        client: ClientId,
        query: crysirror_core::service::EngineQuery,
    ) -> crysirror_core::Result<Vec<u8>> {
        if client != ClientId(1) {return Err(crysirror_core::Error::NotFound);}
        match query {
            crysirror_core::service::EngineQuery::InputRouteDelivery=>serde_json::to_vec(&self.routes.poll(client).map_err(|e|crysirror_core::Error::Io(e.to_string()))?),
            crysirror_core::service::EngineQuery::InputRouteReceiver(id)=>serde_json::to_vec(&self.routes.receiver(client,id)),
            crysirror_core::service::EngineQuery::InputRouteStats=>serde_json::to_vec(&self.routes.stats(client)),
            crysirror_core::service::EngineQuery::MenuEvents=>serde_json::to_vec(&self.ui.events),
            crysirror_core::service::EngineQuery::Layer(id)=>serde_json::to_vec(&self.layers.get(client,id)),
            _=>return Err(crysirror_core::Error::Unsupported("test engine query")),
        }.map_err(|e|crysirror_core::Error::Io(e.to_string()))
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
    let mut config_cx = cx.clone();
    config_cx.source_name = Some("config.int".into());
    let config_archive = format.open(resolver.open("config.int")?, &config_cx)?;
    let mut bgm_cx=cx.clone();bgm_cx.source_name=Some("bgm.int".into());let bgm_archive=format.open(resolver.open("bgm.int")?,&bgm_cx)?;
    let startup = parse_startup(&std::fs::read(root.join("config/startup.xml"))?)?;
    let config = crysirror_catsystem2::Config {
        script: startup.script,
        font: "default".into(),
        heap_bytes: startup.heap_bytes,
        executable: Some("cs2.exe".into()),
        canvas: startup.canvas,
    };
    let mut vm = crysirror_catsystem2::CatSystem2Vm::new(ClientId(1), config)?;
    let save=tempfile::tempdir()?;
    // Use the real engine image worker and publication service. The helper's
    // scoped assets are empty; original game I/O still uses GameStorage above.
    let image_root=tempfile::tempdir()?;std::fs::create_dir(image_root.path().join("assets"))?;
    std::fs::write(image_root.path().join("project.json"),serde_json::to_vec(&json!({"schema":2,"name":"owner shared images","content_revision":"1","assets":"assets","canvas":[1280,720],"clients":[{"name":"images","adapter":"crysirror-script-v1","game":"images"}]}))?)?;
    let shared_images=crysirror_runtime::Runtime::load(&image_root.path().join("project.json"),&image_root.path().join("state"))?;
    let mut host = MenuHost {
        storage: Arc::new(GameStorage {
            root,
            save:save.path().to_owned(),
            kcs: crysirror_resource::ArchiveMount::new(archive, cx)?,
            config: crysirror_resource::ArchiveMount::new(config_archive, config_cx)?,
            bgm: crysirror_resource::ArchiveMount::new(bgm_archive,bgm_cx)?,
        }),
        ui: Default::default(),
        loops: Default::default(),
        logs: Default::default(),
        layers: Default::default(),
        routes:Default::default(),
        shared_images,
    };
    let start = std::time::Instant::now();
    while vm.preparation_pending() {
        vm.step(&mut host, Default::default(), 1000)?;
        ensure!(start.elapsed()<std::time::Duration::from_secs(10),"native bootstrap preparation did not complete");
        std::thread::yield_now();
    }
    let prepared:serde_json::Value=serde_json::from_slice(&vm.diagnostics())?;
    ensure!(prepared["instructions"]==0&&prepared["pc"]==0,"native bootstrap polling executed original instructions");
    println!("original bootstrap prepared without executing KCS instructions");
    let error = loop {
        host.routes.admit_frame(1,Default::default())?;
        match vm.step(&mut host, Default::default(), 1000) {
            Ok(_) => {
                host.layers.frame([1280,720])?;
                host.layers.commit_frame_order()?;
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
    let diagnostics:serde_json::Value=serde_json::from_slice(&vm.diagnostics())?;
    ensure!(diagnostics["native_calls"]["400"].as_u64().unwrap_or(0)>=1,"native LoopPoint import did not complete");
    ensure!(diagnostics["loop_catalog_entries"]==17,"actual BGM loop catalog was not committed");
    ensure!(diagnostics["native_calls"]["359"].as_u64().unwrap_or(0)>=1,"native settings buffers were not committed");
    ensure!(diagnostics["native_calls"]["209"]==1&&diagnostics["interpreter"]["flags"]==2048&&diagnostics["interpreter"]["backup_initialised"]==false,"actual native interpreter was not constructed");
    ensure!(diagnostics["settings_source"]["gamedef_loaded"]==true&&diagnostics["settings_source"]["setup_loaded"]==false&&diagnostics["settings_source"]["flag_count"]==2048,"actual source settings were not preserved");

    if diagnostics["native_calls"]["256"].as_u64().unwrap_or(0)>0 {
        let list=&diagnostics["tasks"]["image_lists"][0];let pool=&list["manager"];
        ensure!(pool["size"]==json!([3072,4096])&&pool["bytes"]==50331648u64&&pool["page_size"]==json!([256,256])&&pool["page_count"]==192&&pool["owner"]==1,"actual shared image pool declaration/resources changed");
        let id=list["base_plane"].as_u64().context("actual native base plane missing")? as u32;
        let group=host.layers.get(ClientId(1),0x43000000+id*2).context("shared base plane group missing")?;
        let payload=host.layers.get(ClientId(1),0x43000000+id*2+1).context("shared base image payload missing")?;
        ensure!(!group.visible&&payload.rect==[0.,0.,1280.,720.]&&payload.transform.z==1000.,"native base plane geometry/enable/Z changed");
        ensure!(matches!(&payload.paint,crysirror_scene::layer::Paint::Image{asset,..} if asset.starts_with("__image_resource/")),"base plane is not bound to a real engine image");
    }
    use crysirror_core::service::HostServices;
    let loop_point=host.audio_loop_metadata(ClientId(1),"bgm.int/bgm_01.ogg")?.context("actual engine-owned BGM metadata")?;
    ensure!(loop_point.range.start_frame==1911584&&loop_point.range.end_frame==Some(3819008),"original loop coordinates changed");

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
    // Real original script reached a genuine shared receiver. Until original
    // key-slot/71 callback resume is mapped, physical input must stop at the
    // same source PC with its engine lease intact, never disappear or replay.
    if diagnostics["native_calls"]["848"].as_u64().unwrap_or(0)>0 {
        let receiver=host.routes.receiver(ClientId(1),5).context("actual shared click controller")?;
        ensure!(receiver.priority==10000&&receiver.enabled&&!receiver.hover_enabled&&receiver.rect==Some([-100000,-100000,200000,200000]),"original receiver binding/hover projection changed");
        if diagnostics["native_calls"]["849"].as_u64().unwrap_or(0)>0{ensure!(!receiver.drag_enabled,"native drag flag did not reach shared receiver");}
        use crysirror_core::service::{buttons,InputFrame};
        for (name,frame) in [
            ("click-down",InputFrame{buttons_down:buttons::MOUSE_LEFT,buttons_pressed:buttons::MOUSE_LEFT,pointer_x:640,pointer_y:360}),
            ("click-up",InputFrame{pointer_x:640,pointer_y:360,..Default::default()}),
            ("ctrl-down",InputFrame{buttons_down:buttons::CTRL,buttons_pressed:buttons::CTRL,pointer_x:640,pointer_y:360}),
            ("ctrl-up",InputFrame{pointer_x:640,pointer_y:360,..Default::default()}),
        ] {
            host.routes.admit_frame(1,frame)?;
            let failed=vm.step(&mut host,Default::default(),1000).unwrap_err().to_string();
            ensure!(failed.contains("unsupported native input callback"),"{name} was lost before the native callback boundary: {failed}");
            let first=host.routes.poll(ClientId(1))?.context("unacknowledged native delivery disappeared")?;
            let repeated=vm.step(&mut host,Default::default(),1000).unwrap_err().to_string();
            ensure!(repeated==failed&&host.routes.poll(ClientId(1))?==Some(first.clone()),"{name} callback replayed or consumed withoutACK");
            let after:serde_json::Value=serde_json::from_slice(&vm.diagnostics())?;
            ensure!(after["pc"]==diagnostics["pc"]&&after["instructions"]==diagnostics["instructions"]&&after["pending_native"]==diagnostics["pending_native"],"{name} advanced unsupported original script");
            println!("actual input boundary {name}: sourcePC={:?}, phase={:?}, retained receipt={:?}",after["pc"],first.event.phase,first.receipt);
            host.routes.apply_batch(ClientId(1),&[crysirror_scene::input_routes::InputRouteCommand::Cancel])?;
        }
    }
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

/// Complements the focused MenuHost fixture above: archives, source workers,
/// images, layers, input journal and the VM all belong to one actual Runtime.
#[cfg(feature = "runtime")]
#[test]
#[ignore = "requires explicitly authorized CRYSIRROR_CATSYSTEM2_GAME; no game data shipped"]
fn owner_actual_runtime_keeps_native_frontier_and_shared_input_receipts() -> Result<()> {
    use crysirror_core::{service::{buttons, EngineQuery, HostServices, InputFrame}, state::ClientId};
    use crysirror_runtime::{AdapterRegistry, Project, Runtime};
    use crysirror_scene::input_routes::{InputDelivery, InputRoutePhase};
    let root=PathBuf::from(std::env::var_os("CRYSIRROR_CATSYSTEM2_GAME").context("game data directory")?);
    let startup=parse_startup(&std::fs::read(root.join("config/startup.xml"))?)?;
    let project:Project=serde_json::from_value(json!({"schema":2,"name":"authorized original Cat runtime","content_revision":"1","assets":"game","canvas":startup.canvas,"clients":[{"name":"native","adapter":"catsystem2","game":"native","config":{"script":startup.script,"font":"default","heap_bytes":startup.heap_bytes,"executable":"cs2.exe","canvas":startup.canvas}}]}))?;
    let mut registry=AdapterRegistry::default();crysirror_catsystem2::register(&mut registry)?;
    let state=tempfile::tempdir()?;
    let mut rt=Runtime::with_asset_root(project,&root,state.path(),registry)?;
    let resolver=Arc::new(Root(root.clone()));
    let mut cx=DecodeContext::new(ClientId(1));cx.resolver=Some(resolver.clone());cx.limits.max_entry_size=1024*1024*1024;
    cx.parameters.insert("cat-system/int".into(),json!({"executable":"cs2.exe"}));
    let format=crysirror_formats_arc::legacy_archive("cat-system/int").unwrap();
    let mut names=std::fs::read_dir(&root)?.collect::<std::io::Result<Vec<_>>>()?.into_iter().map(|e|e.file_name().to_string_lossy().into_owned()).filter(|n|n.to_ascii_lowercase().ends_with(".int")).collect::<Vec<_>>();names.sort();
    for name in &names {
        cx.source_name=Some(name.clone());
        let archive=format.open(resolver.open(name)?,&cx)?;
        rt.mount_provider(ClientId(1),name,Arc::new(crysirror_resource::ArchiveMount::new(archive,cx.clone())?))?;
    }
    // The real desktop native OS bar is outside the game pointer surface.
    rt.host.engine.ui.menus.set_native_bars(true);
    let started=std::time::Instant::now();
    while rt.poll_vm_preparations()? {
        ensure!(started.elapsed()<std::time::Duration::from_secs(20),"original Runtime preparation timeout");
        std::thread::yield_now();
    }
    let ready=rt.inspect();
    ensure!(ready["time_ms"]==0&&ready["frame"]==0&&ready["sessions"][0]["native"]["instructions"]==0,"original Runtime preparation executed/advanced a game frame");
    let error=loop {
        match rt.tick(16,InputFrame::default()) {
            Ok(())=>{ensure!(started.elapsed()<std::time::Duration::from_secs(20),"original Runtime native progress timeout");std::thread::yield_now();}
            Err(e)=>break format!("{e:#}"),
        }
    };
    let inspect=rt.inspect();let native=&inspect["sessions"][0]["native"];
    println!("actual canonical Runtime archives={} frontier={error}",names.len());
    println!("{native}");
    ensure!(native["pending_native"]["id"]==447&&native["pending_native"]["arguments"]==json!([8,406])&&native["pc"]==0x1926,"original Runtime frontier changed");
    let fes=&native["tasks"]["fes_tasks"][0];
    ensure!(fes["id"]==8&&fes["image_list"]==6&&fes["native_rect"]==json!([0,0,3072,4096])&&fes["upload_position"]==json!([0,0]),"canonical native FES binding did not preserve actual manager geometry/position");
    ensure!(fes["string_bank_owner"]==3&&fes["first_tick"]==true&&fes["low_button"].is_null()&&fes["saved_cursor"].is_null()&&fes["player_loaded"]==false,"canonical native FES constructor/tick/unknown cursor state changed");
    ensure!(native["tasks"]["buttons"][0]["result"]==0,"native child tick did not reset existing controller result");
    let pending=native["pending_native"].clone();let instructions=native["instructions"].clone();
    let list=&native["tasks"]["image_lists"][0];let path=list["manager"]["path"].as_str().context("canonical engine atlas missing")?;
    let atlas=rt.assets.surface(ClientId(1),path).context("canonical published atlas path did not resolve")?;
    ensure!([atlas.width,atlas.height]==[3072,4096]&&atlas.rgba.len()==50331648,"canonical shared atlas geometry changed");
    ensure!(rt.assets.surface(ClientId(2),path).is_none(),"canonical shared atlas leaked to another client");
    let plane=list["base_plane"].as_u64().context("canonical base plane missing")? as u32;
    let group=rt.host.engine.layers.get(ClientId(1),0x43000000+plane*2).context("canonical base group missing")?;
    let image=rt.host.engine.layers.get(ClientId(1),0x43000000+plane*2+1).context("canonical base image layer missing")?;
    ensure!(!group.visible&&image.rect==[0.,0.,1280.,720.]&&image.transform.z==1000.,"canonical native base plane state changed");
    for (name,input,phase) in [
        ("click-down",InputFrame{buttons_down:buttons::MOUSE_LEFT,buttons_pressed:buttons::MOUSE_LEFT,pointer_x:640,pointer_y:360},InputRoutePhase::Press),
        ("click-up",InputFrame{pointer_x:640,pointer_y:360,..Default::default()},InputRoutePhase::Release),
        ("ctrl-down",InputFrame{buttons_down:buttons::CTRL,buttons_pressed:buttons::CTRL,pointer_x:640,pointer_y:360},InputRoutePhase::Press),
        ("ctrl-up",InputFrame{pointer_x:640,pointer_y:360,..Default::default()},InputRoutePhase::Release),
    ] {
        let e=rt.tick(16,input).unwrap_err();ensure!(format!("{e:#}").contains("unsupported native input callback"),"actual Runtime did not reach original callback boundary");
        let delivery:Option<InputDelivery>=serde_json::from_slice(&rt.host.query_engine(ClientId(1),EngineQuery::InputRouteDelivery)?)?;
        let delivery=delivery.context("actual Runtime callback lost delivery")?;ensure!(delivery.event.phase==phase,"actual Runtime edge phase changed");
        let again:Option<InputDelivery>=serde_json::from_slice(&rt.host.query_engine(ClientId(1),EngineQuery::InputRouteDelivery)?)?;
        ensure!(again.as_ref().map(|d|d.receipt)==Some(delivery.receipt),"actual Runtime repeated poll redelivered an edge");
        let current=rt.inspect();ensure!(current["sessions"][0]["native"]["pending_native"]==pending&&current["sessions"][0]["native"]["instructions"]==instructions,"actual Runtime callback changed original continuation");
        println!("actual canonical Runtime input {name}: {delivery:?}");
        rt.host.submit_engine(ClientId(1),1,&serde_json::to_vec(&json!({"input_routes":[{"op":"cancel"}]}))?)?;
    }
    Ok(())
}
