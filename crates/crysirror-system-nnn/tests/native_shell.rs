//! Native outer-mode fixtures use shared images/Layers, never a VM compositor.
use crysirror_core::{service::{InputFrame,MemoryAssetReader},state::ClientId};
use crysirror_system_nnn::{config::Names,expression::VariableProfile,native_bootstrap::NativeConfiguration,native_resources::ResourceNames,native_shell::{NativeLogoPlayer,NativeLogoSpec,NativeMode,NativeStartupProfile},resource_bridge::NativeResources,XorProfile};
use std::{sync::Arc,time::{Duration,Instant}};
fn names(s:&str)->Names {Names::parse(s.as_bytes(),false).unwrap()}
fn config()->NativeConfiguration {NativeConfiguration{resources:ResourceNames::new("").unwrap(),setup:names("dwqPackLevel 1"),variable_names:names(""),project:names("actual_back"),commands:names(""),functions:names(""),initial:None,module_sha256:"authored-profile".into(),profile:VariableProfile::Legacy1000,scripts:XorProfile::default()}}
#[test]
fn native_startup_profile_preserves_mode_chain_logo_threshold_and_legacy_defaults() {
    let config=config();
    let profile=NativeStartupProfile::from_tables(&config,&names("nextMode logo codecCheckFlag 1"),&names("logoNumber 1 useCustomFileFlag 1 filename1 ta_logo useCustomTimeFlag 1 fadeInTime1 1 keepTime1 350 fadeOutTime1 1 useCustomSkipFlag 1 skipOK1 3 nextMode opening"),&names("backScriptFlag 1 backScriptName actual_back nextMode printmovie"),names("backScriptFlag 1 backScriptName actual_back backScriptLoopFlag 0"),names("movieFileName actual.mpg")).unwrap();
    assert_eq!(profile.codec_limit,3,"reviewed legacy constructor differs from modern codecLimit4");assert!(profile.codec_check);assert!(!profile.startup_jump_script);
    assert_eq!(profile.nothing_next,NativeMode::Logo);assert_eq!(profile.logo_next,NativeMode::Opening);assert_eq!(profile.opening_next,NativeMode::Movie);
    assert_eq!(profile.logos[0].frames,[1,350,1]);assert_eq!(profile.logos[0].skip_after_launch,3);
    assert_eq!(profile.logos[0].source.packed[0].archive,"dwq/sys0.gpk");assert_eq!(profile.logos[0].source.packed[0].member,"ta_logo.dwq");
    assert_eq!(profile.opening_script,b"actual_back");assert!(!profile.opening_loop);assert!(!profile.title_loop);
    assert!(NativeMode::named(b"invented").is_err());
}
fn setup(launch:u32)->(tempfile::TempDir,crysirror_runtime::Runtime,NativeLogoPlayer) {
    let root=tempfile::tempdir().unwrap();std::fs::create_dir(root.path().join("game")).unwrap();
    let project:crysirror_runtime::Project=serde_json::from_value(serde_json::json!({"schema":2,"name":"native logo","content_revision":"1","assets":"game","canvas":[16,16],"clients":[{"name":"native","adapter":"crysirror-script-v1","game":"authored"}]})).unwrap();
    let mut rt=crysirror_runtime::Runtime::with_asset_root(project,root.path(),&root.path().join("state"),Default::default()).unwrap();
    crysirror_formats_blackcyc::register_images(&mut rt.assets.decoders).unwrap();rt.tick(0,Default::default()).unwrap();
    let image=crysirror_image::ImageSurface{width:2,height:1,rgba:vec![200,100,50,7,10,20,30,255]};
    let mut bytes=vec![b' ';64];bytes[..4].copy_from_slice(b"PNG ");bytes[36..40].copy_from_slice(&2u32.to_le_bytes());bytes[40..44].copy_from_slice(&1u32.to_le_bytes());bytes[48..64].copy_from_slice(b"PACKTYPE=8      ");bytes.extend(crysirror_image::encode_png(&image).unwrap());let bytes:Arc<[u8]>=bytes.into();
    let resources=NativeResources::blackcyc(ClientId(1),Arc::new(move|path,cancel|{cancel.check()?;assert_eq!(path,"dwq/sys/logo.dwq");Ok(Some(Arc::new(MemoryAssetReader(bytes.clone()))))})).unwrap();
    let spec=NativeLogoSpec{source:ResourceNames::new("").unwrap().picture(b"sys\\logo",0).unwrap(),frames:[1,2,1],colors:[[0;3];2],skip_after_launch:3};
    let player=NativeLogoPlayer::new(ClientId(1),9000,[16,16],20,launch,vec![spec],resources).unwrap();
    (root,rt,player)
}
fn finish_work(player:&mut NativeLogoPlayer,rt:&mut crysirror_runtime::Runtime) {
    let until=Instant::now()+Duration::from_secs(10);
    loop {player.poll(&mut rt.host,Default::default()).unwrap();rt.tick(0,Default::default()).unwrap();if !player.preparation_pending()&&!player.dependency_pending(){break}assert!(Instant::now()<until);std::thread::yield_now();}
}
fn pixel(rt:&crysirror_runtime::Runtime)->Vec<u8> {let layer=rt.host.engine.layers.get(ClientId(1),9000).unwrap();let crysirror_scene::layer::Paint::Image{asset,..}=&layer.paint else{panic!()};rt.assets.surface(ClientId(1),asset).unwrap().rgba[..4].to_vec()}
#[test]
fn original_logo_sequence_owns_shared_pixels_and_scoped_layer_with_native_clock_and_skip_gate() {
    let (_root,mut rt,mut player)=setup(0);finish_work(&mut player,&mut rt);
    assert_eq!(pixel(&rt),[0,0,0,255]);assert_eq!(player.native_location(),(0,0,0));
    assert!(!player.poll(&mut rt.host,InputFrame{buttons_pressed:1,..Default::default()}).unwrap(),"first launch cannot skip logo whose threshold is3");
    rt.tick(20,Default::default()).unwrap();finish_work(&mut player,&mut rt);
    assert_eq!(pixel(&rt),[200,100,50,255],"logo display ignores native X/mask alpha through shared OverrideAlpha");assert_eq!(player.native_location(),(0,1,0));
    for _ in 0..2 {rt.tick(20,Default::default()).unwrap();finish_work(&mut player,&mut rt);}
    rt.tick(20,Default::default()).unwrap();assert!(player.poll(&mut rt.host,Default::default()).unwrap());assert!(rt.host.engine.layers.get(ClientId(1),9000).is_none());
    let (_root,mut rt,mut player)=setup(2);finish_work(&mut player,&mut rt);assert!(player.poll(&mut rt.host,InputFrame{buttons_pressed:1,..Default::default()}).unwrap());assert!(rt.host.engine.layers.get(ClientId(1),9000).is_none());
}

#[test]
fn original_background_context_uses_native_mode_and_next_without_running_sys() {
    use crysirror_core::vm::{VmAdapter,VmYield};
    use crysirror_system_nnn::{Program,native_bootstrap::PreparedBoot,native_control::ScriptSession,machine::Machine,expression::Variables,adapter::NativeAdapter,bridge::{Bridge,CallbackProfile,SceneBindings}};
    let (_root,mut rt,_logo)=setup(0);
    let mut words=vec![0u32;32];words[0]=32;words[1]=0x66660001;words[2]=0x55550001;
    words.extend([8,0x66660005,0,0xa0000000,3,0x800001f4,0x8000001e,0xa000003d,10,0x66660007,0x33330013,6,0,54,20,42,70,130,3,0x66660002,0x55560006]);words[18]=words.len()as u32;
    let program=Arc::new(Program::parse(&words.into_iter().flat_map(|w|(!w).to_le_bytes()).collect::<Vec<_>>(),XorProfile::default()).unwrap());
    let mut vars=Variables::fresh(VariableProfile::Legacy1000);vars.mark_calculated(30,b"mode".to_vec());
    let cfg=config();let boot=PreparedBoot{session:ScriptSession::new(program.clone(),Machine::with_variables(vars)),resources:cfg.resources,setup:cfg.setup,variable_names:cfg.variable_names,project:cfg.project,commands:cfg.commands,functions:cfg.functions,initial:None,module_sha256:cfg.module_sha256,profile:cfg.profile,scripts:cfg.scripts};
    let bridge=Bridge::new(ClientId(1),CallbackProfile::Mugen2Legacy,SceneBindings{root:1,overlap_canvas:7,effect_base:100,size:[16,16],layers:16,music_channel:0}).unwrap();
    use crysirror_core::service::HostServices;
    rt.host.submit_engine(ClientId(1),1,br#"{"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,16,16],"paint":{"kind":"solid","color":[0,0,0,255]}}}]}"#).unwrap();
    let mut vm=NativeAdapter::from_background(boot,Arc::new(|_,_|anyhow::bail!("unexpected script load")),bridge,b"actual_back".to_vec(),program,NativeMode::Opening,false).unwrap();
    let until=Instant::now()+Duration::from_secs(10);
    loop {if vm.step(&mut rt.host,Default::default(),100).unwrap()==VmYield::Halted{break}rt.tick(0,Default::default()).unwrap();assert!(Instant::now()<until);std::thread::yield_now();}
    let diagnostic:serde_json::Value=serde_json::from_slice(&vm.diagnostics()).unwrap();assert_eq!(diagnostic["script"],"actual_back");assert_eq!(diagnostic["background_loop"],false);assert_eq!(diagnostic["script_depth"],0);
    let (machine,bridge)=vm.into_idle_state().unwrap();assert_eq!(machine.variables().unwrap().read_stored(500),3);assert_eq!(machine.variables().unwrap().read_stored(310),0,"background entry must not apply NewGame VarInit");assert_eq!(bridge.api_calls,1);
    assert!(matches!(rt.host.engine.layers.get(ClientId(1),100).unwrap().paint,crysirror_scene::layer::Paint::Solid{color:[42,70,130,255]}));
}

#[test]
fn native_logo_retains_click_edge_across_shared_source_and_image_dependencies() {
    let (_root,mut rt,mut player)=setup(2);
    assert!(!player.poll(&mut rt.host,InputFrame{buttons_pressed:1,..Default::default()}).unwrap());
    let deadline=Instant::now()+Duration::from_secs(10);
    loop{if player.poll(&mut rt.host,Default::default()).unwrap(){break}rt.tick(0,Default::default()).unwrap();assert!(Instant::now()<deadline);std::thread::yield_now();}
    assert!(!player.dependency_pending());assert!(rt.host.engine.layers.get(ClientId(1),9000).is_none());
}
