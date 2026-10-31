use crysirror_core::{
    service::{buttons, HostServices, InputFrame, TextExtent, TextMeasurement},
    state::ClientId,
};
use crysirror_system_nnn::{
    config::Names,
    machine::{Machine, NativeCall, Payload, Step},
    native_audio::{
        legacy_gain, AudioBindings, AudioOperation, AudioSettings, NativeAudioLookup,
        NativeAudioPolicy,
    },
    native_resources::ResourceNames,
    native_save::{LegacySaveProfile, NativeSaveFile, SaveBlock},
    native_text::{
        plain_cells, MessageBindings, MessageProfile, NativeMessageState, NativeTextBridge,
    },
    Program, XorProfile,
};
use serde_json::json;
struct Metrics;
impl TextMeasurement for Metrics {
    fn measure_simple(&self, _: &str, _: f32, _: &str) -> crysirror_core::Result<TextExtent> {
        Ok(TextExtent {
            advance: 0.,
            line_height: 24.,
            ascent: 18.,
            descent: -6.,
        })
    }
}
fn sjis(s: &str) -> Vec<u8> {
    let (bytes, _, bad) = encoding_rs::SHIFT_JIS.encode(s);
    assert!(!bad);
    bytes.into_owned()
}
fn profile() -> MessageProfile {
    MessageProfile::from_legacy_setup(
        &Names::parse(
            b"printX,20,printY,40,nextY,30,nameX,10,nameY,10,nameCenterFlag,0",
            false,
        )
        .unwrap(),
        MessageBindings {
            client: ClientId(1),
            root: 1,
            body_layer: 200,
            name_layer: 201,
            order: 30,
            canvas: [800, 600],
            window: None,
        },
        "fixture",
        2,
        Some(&Names { tokens: vec![] }),
        None,
        buttons::MOUSE_LEFT,
    )
    .unwrap()
}
fn message(kind: u32, text: &str) -> NativeCall {
    NativeCall {
        serial: 1,
        pc: 32,
        next: 38,
        identify: 0x66660006,
        code: kind,
        payload: Payload::Message {
            kind,
            serial: 7,
            cutin: 0,
            text: sjis(text),
        },
    }
}
#[test]
fn legacy_message_rows_names_positions_rate_append_and_strict_cells() {
    let p = profile();
    assert_eq!(p.reveal_per_second, 60);
    let first = NativeMessageState::default()
        .prepare(&message(0x22220001, "名前\r\n日本語"), &p, &Metrics)
        .unwrap();
    assert_eq!(
        first.batch["layers"][0]["layer"]["paint"]["glyphs"][0]["baseline"],
        json!([20., 58.])
    );
    assert_eq!(
        first.batch["layers"][0]["layer"]["paint"]["glyphs"][1]["baseline"],
        json!([45., 58.])
    );
    assert_eq!(
        first.batch["layers"][1]["layer"]["paint"]["glyphs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        first.batch["layers"][0]["layer"]["paint"]["reveal"]["rate_per_second"],
        60
    );
    assert!(first.next.window_visible);
    assert_eq!(first.next.body_cells, 3);
    let appended = first
        .next
        .prepare(&message(0x22220003, "次"), &p, &Metrics)
        .unwrap();
    assert_eq!(appended.next.mode, 0x22220001);
    assert!(!appended.next.window_visible);
    assert_eq!(
        appended.batch["layers"][0]["layer"]["paint"]["reveal"]["initial_visible"],
        3
    );
    assert_eq!(
        appended.batch["layers"][0]["layer"]["paint"]["glyphs"][3]["baseline"],
        json!([20., 88.])
    );
    assert!(plain_cells(b"A", None).is_err());
    assert!(plain_cells(b"#", None).is_err());
    assert!(plain_cells(&[0x82], None).is_err());
    let before = first.next.rows.clone();
    assert!(first
        .next
        .prepare(&message(0x22220001, "名前\r日本語"), &p, &Metrics)
        .is_err());
    assert_eq!(first.next.rows, before);
}
#[test]
fn native_audio_uses_db_loop_defaults_explicit_voice_precedence_and_candidate_state() {
    let policy = NativeAudioPolicy::new(
        AudioSettings {
            music_volume: 80,
            sound_volume: 60,
            voice_volume: 70,
            sound_voice_volume: 40,
            music_enabled: true,
            sound_enabled: true,
            voice_enabled: true,
            sound_voice_enabled: true,
        },
        AudioBindings {
            music: 0,
            sound_base: 10,
            voice_base: 20,
        },
    )
    .unwrap();
    let names = Names::parse(b"bg01,title,se02,click", false).unwrap();
    let music = policy.music(&[1], &names, false).unwrap();
    let AudioOperation::Play {
        parameters,
        repeat_count,
        ..
    } = &music.operation
    else {
        panic!()
    };
    assert!(parameters.looping);
    assert_eq!(*repeat_count, 0);
    assert!((parameters.gain - 0.1).abs() < 1e-7);
    assert_eq!(policy.music_number, None);
    assert_eq!(music.next.music_number, Some(0));
    assert!((legacy_gain(0).unwrap() - 0.00001).abs() < 1e-10);
    assert!(policy
        .music(&[1, 2, 0, 3], &names, true)
        .unwrap()
        .asset_request(
            crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Asset, "wgq/bg01.wgq")
                .unwrap()
        )
        .is_err());
    let voice = NativeCall {
        serial: 1,
        pc: 0,
        next: 4,
        identify: 0x66660007,
        code: 0x33330008,
        payload: Payload::Voice {
            name: Some(b"ab0001".to_vec()),
            parameters: vec![0, 2, -10, 1],
            default_voice: 0,
        },
    };
    let plan = policy.plan(&voice, &names, &names, false).unwrap();
    let NativeAudioLookup::Voice { packed, loose } = plan
        .source
        .as_ref()
        .unwrap()
        .lookup(&ResourceNames::new("root").unwrap())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(packed.archive, "root/cdvaw/ab.vpk");
    assert_eq!(packed.member, "ab0001.vaw");
    assert_eq!(loose, "root/cdvaw/ab/ab0001.vaw");
    let AudioOperation::Play {
        channel,
        parameters,
        ..
    } = plan.operation
    else {
        panic!()
    };
    assert_eq!(channel, 22);
    assert!(parameters.looping);
    assert!((parameters.gain - legacy_gain(30).unwrap()).abs() < 1e-12);
}
fn save_fixture() -> (NativeSaveFile, LegacySaveProfile) {
    let profile = LegacySaveProfile {
        header_name: b"authored-save".to_vec(),
        version: 123,
        extension_sizes: vec![],
        omake: false,
        cutin: false,
    };
    let mut info = vec![0; 32];
    info[..4].copy_from_slice(&8u32.to_le_bytes());
    info[4..8].copy_from_slice(&123u32.to_le_bytes());
    let mut mini = vec![0; 20];
    mini[..4].copy_from_slice(&1u32.to_le_bytes());
    mini[4..8].copy_from_slice(&1u32.to_le_bytes());
    let mut vars = vec![0; 4096];
    vars[4000..].fill(0xa5);
    let blocks = vec![
        SaveBlock::new(0, b"authored-save", vec![]).unwrap(),
        SaveBlock::new(1, b"INFO", info).unwrap(),
        SaveBlock::new(2, b"GAMESTATUS", vec![0; 7056]).unwrap(),
        SaveBlock::new(3, b"MINICG", mini).unwrap(),
        SaveBlock::new(4, b"GAMEVAR", vars).unwrap(),
        SaveBlock::new(5, b"EFFECT", vec![0; 16384]).unwrap(),
        SaveBlock::new(6, b"EFFECTFILENAME", vec![0; 4096]).unwrap(),
        SaveBlock::new(7, b"MESSAGE", vec![0; 16384]).unwrap(),
    ];
    (NativeSaveFile { blocks }, profile)
}
#[test]
fn native_save_has_original_sizes_roundtrips_preserves_padding_and_rejects_foreign_envelope() {
    let (mut file, profile) = save_fixture();
    let bytes = file.encode(&profile).unwrap();
    let decoded = NativeSaveFile::parse(&bytes, &profile).unwrap();
    assert_eq!(decoded.encode(&profile).unwrap(), bytes);
    assert_eq!(decoded.variables().unwrap(), vec![0; 1000]);
    let mut vars = vec![0; 1000];
    vars[310] = 42;
    file.replace_variables(&vars).unwrap();
    assert_eq!(file.variables().unwrap()[310], 42);
    assert!(file.blocks[4].payload[4000..].iter().all(|v| *v == 0xa5));
    assert_eq!(file.messages(64).unwrap().len(), 64);
    let mut foreign = profile.clone();
    foreign.header_name = b"foreign".to_vec();
    assert!(NativeSaveFile::parse(&bytes, &foreign).is_err());
    for bad in [0, 31, u32::MAX] {
        let mut corrupt = bytes.clone();
        corrupt[..4].copy_from_slice(&bad.to_le_bytes());
        assert!(NativeSaveFile::parse(&corrupt, &profile).is_err());
    }
    let mut modern = file.clone();
    modern.blocks[7].payload.resize(64 * 1024, 0);
    assert!(modern.encode(&profile).is_err());
}
fn program(text: &str) -> Program {
    let mut words = vec![0u32; 32];
    words[0] = 32;
    words[1] = 0x66660001;
    words[2] = 0x55550001;
    words.extend([6, 0x66660006, 0x22220001, 0, 7, 0]);
    words.extend([3, 0x66660002, 0x55560005]);
    words[4] = 1;
    words[5] = words.len() as u32;
    words.push(words.len() as u32 + 1);
    words[18] = words.len() as u32;
    let mut text = sjis(text);
    text.push(0);
    while text.len() % 4 != 0 {
        text.push(0);
    }
    words.extend(
        text.chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
    );
    Program::parse(
        &words
            .into_iter()
            .flat_map(|w| (!w).to_le_bytes())
            .collect::<Vec<_>>(),
        XorProfile::default(),
    )
    .unwrap()
}
#[test]
#[ignore = "reads caller-supplied licensed runtime font; no font bundled"]
fn shared_text_worker_publication_reveal_click_and_original_pc_are_real() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"schema":2,"name":"NNN text","content_revision":"1","assets":"assets","canvas":[800,600],"clients":[{"name":"native","adapter":"crysirror-script-v1","game":"authored"}]})).unwrap()).unwrap();
    let mut rt = crysirror_runtime::Runtime::load(&path, &dir.path().join("state")).unwrap();
    rt.host
        .text
        .add_font(
            "fixture",
            std::fs::read(std::env::var_os("CRYSIRROR_TEST_FONT").unwrap()).unwrap(),
        )
        .unwrap();
    rt.host.submit_engine(ClientId(1),1,&serde_json::to_vec(&json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,800,600],"paint":{"kind":"solid","color":[0,0,0,255]}}}]})).unwrap()).unwrap();
    let p = program("名前\r\n日本語");
    let mut m = Machine::default();
    let Step::Native(call) = m.poll(&p, 100).unwrap() else {
        panic!()
    };
    let mut bridge = NativeTextBridge::new(profile()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while bridge.api_calls == 0 {
        assert!(!bridge
            .try_commit(&mut m, &call, &mut rt.host, InputFrame::default())
            .unwrap());
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(m.pc(), 32);
    let layer = rt.host.engine.layers.get(ClientId(1), 200).unwrap();
    assert_eq!(layer.paint.glyph_reveal_status().unwrap().visible, 0);
    rt.tick(17, InputFrame::default()).unwrap();
    assert_eq!(
        rt.host
            .engine
            .layers
            .get(ClientId(1), 200)
            .unwrap()
            .paint
            .glyph_reveal_status()
            .unwrap()
            .visible,
        1
    );
    let click = InputFrame {
        buttons_down: buttons::MOUSE_LEFT,
        buttons_pressed: buttons::MOUSE_LEFT,
        ..InputFrame::default()
    };
    assert!(!bridge
        .try_commit(&mut m, &call, &mut rt.host, click)
        .unwrap());
    assert_eq!(m.pc(), 32);
    assert_eq!(bridge.api_calls, 2);
    assert!(
        !bridge
            .try_commit(&mut m, &call, &mut rt.host, click)
            .unwrap(),
        "one click cannot both finish reveal and advance"
    );
    rt.tick(17, InputFrame::default()).unwrap();
    assert!(bridge
        .try_commit(&mut m, &call, &mut rt.host, click)
        .unwrap());
    assert_eq!(m.pc(), call.next);
    assert_eq!(bridge.state.mode, 0x22220001);
    assert!(bridge.state.window_visible);
    bridge.clear(&mut rt.host).unwrap();
    assert_eq!(bridge.api_calls, 3);
    assert!(rt.host.engine.layers.get(ClientId(1), 200).is_none());
    assert!(bridge.state.rows.is_empty());
    assert_eq!(bridge.state.mode, 0x22220001);
    assert!(
        bridge.state.window_visible,
        "native Clear preserves window policy"
    );
}

#[test]
#[ignore = "reads runtime system font; authored images only, no bundled font or owner game data"]
fn translucent_native_window_is_exact_shared_composition_and_retained_text() {
    use crysirror_core::image_processing::{ImageInput, ImageJob, ImageProcessState};
    use crysirror_system_nnn::native_text::MessageWindow;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let file = dir.path().join("project.json");
    std::fs::write(&file,br#"{"schema":2,"name":"window","content_revision":"1","assets":"assets","canvas":[800,600],"clients":[{"name":"native","adapter":"crysirror-script-v1","game":"authored"}]}"#).unwrap();
    let mut rt = crysirror_runtime::Runtime::load(&file, &dir.path().join("state")).unwrap();
    rt.host
        .text
        .add_font(
            "fixture",
            std::fs::read(std::env::var_os("CRYSIRROR_TEST_FONT").unwrap()).unwrap(),
        )
        .unwrap();
    rt.host.submit_engine(ClientId(1),1,&serde_json::to_vec(&json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,800,600],"paint":{"kind":"solid","color":[255,255,255,255]}}}]})).unwrap()).unwrap();
    let finish = |mut job: ImageJob| {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            match job.poll().unwrap() {
                ImageProcessState::Ready(image) => break image,
                ImageProcessState::Failed(error) => panic!("{error}"),
                ImageProcessState::Pending => {
                    assert!(Instant::now() < until);
                    std::thread::yield_now();
                }
            }
        }
    };
    let images = rt.host.image_metadata(ClientId(1)).unwrap();
    let background = finish(
        images
            .begin_process(
                ImageInput::Solid {
                    size: [800, 600],
                    color: [255, 255, 255, 255],
                },
                b"[]".as_slice().into(),
            )
            .unwrap(),
    );
    let bytes: Arc<[u8]> = crysirror_image::encode_png(&crysirror_image::ImageSurface {
        width: 3,
        height: 1,
        rgba: vec![1, 1, 1, 0, 1, 1, 1, 128, 1, 1, 1, 255],
    })
    .unwrap()
    .into();
    let source = finish(
        images
            .begin_process(
                ImageInput::Encoded {
                    name: "window.png".into(),
                    bytes,
                    size: [3, 1],
                },
                b"[]".as_slice().into(),
            )
            .unwrap(),
    );
    let published = rt
        .host
        .publish_image_resource(ClientId(1), 202, source.clone())
        .unwrap();
    let mut profile = profile();
    profile.name_enabled = false;
    profile.window = Some(MessageWindow {
        id: 202,
        asset: published.path.relative().to_owned(),
        rect: [10, 11, 3, 1],
        source_size: [3, 1],
        percent: 50,
        resource: Some(source),
    });
    let mut text = NativeTextBridge::new(profile).unwrap();
    text.set_background(background).unwrap();
    let program = program("日本語");
    let mut machine = Machine::default();
    let Step::Native(call) = machine.poll(&program, 100).unwrap() else {
        panic!()
    };
    let until = Instant::now() + Duration::from_secs(10);
    while rt.host.engine.layers.get(ClientId(1), 200).is_none() {
        assert!(!text
            .try_commit(&mut machine, &call, &mut rt.host, Default::default())
            .unwrap());
        rt.tick(1, Default::default()).unwrap();
        assert!(Instant::now() < until);
        std::thread::yield_now();
    }
    assert_eq!(machine.pc(), call.pc);
    let layer = rt.host.engine.layers.get(ClientId(1), 202).unwrap();
    assert_eq!(layer.rect, [10., 11., 3., 1.]);
    let crysirror_scene::layer::Paint::Image { asset, .. } = &layer.paint else {
        panic!()
    };
    let image = rt.assets.surface(ClientId(1), asset).unwrap();
    let at = (11 * 800 + 10) * 4;
    assert_eq!(
        &image.rgba[at..at + 12],
        [255, 255, 255, 255, 190, 190, 190, 255, 128, 128, 128, 255]
    );
    assert!(
        rt.host
            .engine
            .layers
            .get(ClientId(1), 200)
            .unwrap()
            .paint
            .glyph_reveal_status()
            .is_some(),
        "text remains an independent engine glyph run"
    );
    rt.tick(1000, Default::default()).unwrap();
    assert!(text
        .try_commit(
            &mut machine,
            &call,
            &mut rt.host,
            InputFrame {
                buttons_pressed: buttons::MOUSE_LEFT,
                ..Default::default()
            }
        )
        .unwrap());
    assert_eq!(machine.pc(), call.next);
}

#[test]
#[ignore = "reads selected owner MSI config/module resources on one worker; never executes the module"]
fn owner_message_setup_and_original_character_mapping_are_read_in_place() {
    use crysirror_core::service::AssetReader;
    use crysirror_resource::{installer::InstallerCatalog, read_all, DecodeContext};
    use std::{
        io::{Read, Seek, SeekFrom},
        sync::{Arc, Mutex},
    };
    struct Reader(Mutex<std::fs::File>, u64);
    impl AssetReader for Reader {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(self.1)
        }
        fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
            let mut f = self.0.lock().unwrap();
            f.seek(SeekFrom::Start(offset))
                .and_then(|_| f.read(out))
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))
        }
    }
    let path = std::env::var_os("CRYSIRROR_TEST_MSI").unwrap();
    let pool = crysirror_tasks::TaskPool::new("nnn-owner-services", 1, 1).unwrap();
    let mut task=pool.spawn(move|cancel|->anyhow::Result<()> {
        let f=std::fs::File::open(path)?;let size=f.metadata()?.len();let mut cx=DecodeContext::new(ClientId(1)).with_cancellation(cancel.clone());cx.limits.max_entry_size=1<<30;
        let catalog=InstallerCatalog::open(Arc::new(Reader(Mutex::new(f),size)),&cx)?;
        let read=|name:&str,limit:u64|->anyhow::Result<Option<Vec<u8>>>{let Some(id)=catalog.files().iter().position(|f|f.path.eq_ignore_ascii_case(name)) else{return Ok(None)};Ok(Some(read_all(catalog.read(id,&cx)?.as_ref(),limit)?))};
        let module=read("mugen2/mugen2.exe",64<<20)?.unwrap();let map=crysirror_system_nnn::native_text::mugen2_single_byte_map(&module)?;assert_eq!(map.len(),192);assert_eq!(plain_cells(b"A\\a",Some(&map))?,"Ａ￥ａ");
        let config=if let Some(bytes)=read("mugen2/init2/PrintMessage.fxf",1<<20)? {Names::parse(&bytes,true)?}else{Names::parse(&read("mugen2/init/PrintMessage.xtx",1<<20)?.unwrap(),false)?};
        let names=Names::parse(&read("mugen2/spt/namecol.xtx",1<<20)?.unwrap(),false)?;
        let profile=MessageProfile::from_legacy_setup(&config,MessageBindings{client:ClientId(1),root:1,body_layer:200,name_layer:201,order:30,canvas:[800,600],window:None},"runtime-host-font",2,Some(&names),Some(map),buttons::MOUSE_LEFT)?;
        println!("owner PrintMessage config={} tokens, names={} entries, body={:?} size={} next={:?} name={:?} size={} rows={}/{} rate={} character_map=192 exact reviewed module bytes",config.tokens.len(),profile.name_colors.len(),profile.print_origin,profile.body.size,profile.print_next,profile.name_origin,profile.name.size,profile.window_rows,profile.full_rows,profile.reveal_per_second);
        let game=Names::parse(&read("mugen2/init2/game.fxf",1<<20)?.unwrap(),true)?;
        let prepared=crysirror_system_nnn::native_services_bootstrap::PreparedNativeServices::mugen2(read,"mugen2",&module,&game,None,AudioBindings{music:0,sound_base:10,voice_base:20},false,&cancel)?;
        println!("owner native prefs origin={:?} music={} sound={} voice={} soundvoice={} speed={} windowConfig={} windowTransparency={}",prepared.preferences.origin,prepared.audio.settings.music_volume,prepared.audio.settings.sound_volume,prepared.audio.settings.voice_volume,prepared.audio.settings.sound_voice_volume,prepared.preferences.message_speed,prepared.preferences.window_number,prepared.preferences.window_transparency);
        assert_eq!(prepared.preferences.origin,crysirror_system_nnn::native_services_bootstrap::PreferencesOrigin::OriginalTemplate);assert_eq!(prepared.audio.settings.music_volume,87);assert_eq!(prepared.preferences.message_speed,2);assert!(prepared.music_names.tokens.len()>2);assert!(prepared.sound_names.tokens.len()>2);
        let windows=prepared.windows(0,0)?.unwrap();
        println!("owner service factory preferences={:?} audio={:?} musicPairs={} soundPairs={} windowRect={:?} nativeNameWindow={} selectionTokens={}",prepared.preferences.origin,prepared.audio.settings,prepared.music_names.tokens.len()/2,prepared.sound_names.tokens.len()/2,windows.body.rect,windows.name.is_some(),prepared.selection_setup.tokens.len());
        cancel.check()?;Ok(())
    }).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if let Some(()) = task.try_take().unwrap() {
            break;
        }
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
}

#[test]
#[ignore = "reads caller-supplied licensed runtime font; no font bundled"]
fn shared_selection_widgets_choose_original_case_and_raw_outside_click_cannot_advance() {
    use crysirror_system_nnn::native_ui::{NativeSelectionBridge, SelectionProfile, SelectionRow};
    let mut words = vec![0u32; 32];
    words[0] = 32;
    words[1] = 0x66660001;
    words[2] = 0x55550001;
    words.extend([
        7, 0x66660006, 0x22220006, 2, 0, 1, 99, 4, 0x66660002, 0xa000007b, 18, 4, 0x66660002,
        0x55560008, 1, 3, 0x66660002, 0x55560005, 4, 0x66660002, 0x55560008, 2, 3, 0x66660002,
        0x55560005, 3, 0x66660002, 0xa000007d,
    ]);
    words[6] = 2;
    let table = words.len();
    words[7] = table as u32;
    words.extend([0, 0]);
    words[18] = words.len() as u32;
    for (n, text) in ["左", "右"].iter().enumerate() {
        words[table + n] = words.len() as u32;
        let mut raw = sjis(text);
        raw.push(0);
        while raw.len() % 4 != 0 {
            raw.push(0);
        }
        words.extend(
            raw.chunks_exact(4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
        );
    }
    let p = Program::parse(
        &words
            .into_iter()
            .flat_map(|v| (!v).to_le_bytes())
            .collect::<Vec<_>>(),
        XorProfile::default(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"schema":2,"name":"NNN selection","content_revision":"1","assets":"assets","canvas":[800,600],"clients":[{"name":"native","adapter":"crysirror-script-v1","game":"authored"}]})).unwrap()).unwrap();
    let mut rt = crysirror_runtime::Runtime::load(&path, &dir.path().join("state")).unwrap();
    rt.host
        .text
        .add_font(
            "fixture",
            std::fs::read(std::env::var_os("CRYSIRROR_TEST_FONT").unwrap()).unwrap(),
        )
        .unwrap();
    rt.host.submit_engine(ClientId(1),1,&serde_json::to_vec(&json!({"layers":[{"op":"put","layer":{"id":1,"rect":[0,0,800,600],"paint":{"kind":"solid","color":[0,0,0,255]}}}]})).unwrap()).unwrap();
    let profile = SelectionProfile {
        client: ClientId(1),
        root: 1,
        widget_base: 300,
        layer_base: 400,
        order: 30,
        canvas: [800, 600],
        face: profile().body,
        rows: vec![
            SelectionRow {
                text_origin: [10, 100],
                hit_rect: [10, 100, 200, 40],
            },
            SelectionRow {
                text_origin: [10, 150],
                hit_rect: [10, 150, 200, 40],
            },
        ],
        message_origins: vec![],
        single_byte_map: None,
    };
    let mut bridge = NativeSelectionBridge::new(profile).unwrap();
    let mut m = Machine::default();
    let Step::Native(call) = m.poll(&p, 100).unwrap() else {
        panic!()
    };
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while bridge.api_calls == 0 {
        assert!(!bridge.try_commit(&mut m, &call, &p, &mut rt.host).unwrap());
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    assert_eq!(m.pc(), 32);
    assert!(rt.host.engine.ui.get(1, 300).is_some());
    for input in [
        InputFrame {
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            pointer_x: 700,
            pointer_y: 500,
        },
        InputFrame {
            pointer_x: 700,
            pointer_y: 500,
            ..InputFrame::default()
        },
    ] {
        rt.tick(16, input).unwrap();
        assert!(!bridge.try_commit(&mut m, &call, &p, &mut rt.host).unwrap());
    }
    rt.tick(
        16,
        InputFrame {
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            pointer_x: 20,
            pointer_y: 160,
        },
    )
    .unwrap();
    assert!(!bridge.try_commit(&mut m, &call, &p, &mut rt.host).unwrap());
    rt.tick(
        16,
        InputFrame {
            pointer_x: 20,
            pointer_y: 160,
            ..InputFrame::default()
        },
    )
    .unwrap();
    // An unpolled activation survives until owner completion, after the transient edge clears.
    rt.tick(16, InputFrame::default()).unwrap();
    assert!(!rt.host.engine.ui.interaction(1, 301).unwrap().activated);
    assert!(bridge.try_commit(&mut m, &call, &p, &mut rt.host).unwrap());
    assert_eq!(m.pc(), 54);
    assert!(rt.host.engine.ui.get(1, 301).is_none());
    assert!(rt.host.engine.layers.get(ClientId(1), 400).is_none());
}

#[test]
fn native_preference_record_uses_saved_fields_and_proven_constructor_defaults() {
    use crysirror_system_nnn::native_services_bootstrap::{NativePreferences, PreferencesOrigin};
    let defaults = NativePreferences::original_defaults();
    assert_eq!(defaults.audio.music_volume, 90);
    assert_eq!(defaults.message_speed, 2);
    let mut bytes = vec![0u8; 64 + 2048];
    bytes[..4].copy_from_slice(&64u32.to_le_bytes());
    bytes[32..36].copy_from_slice(&2u32.to_le_bytes());
    bytes[64..68].copy_from_slice(&2048u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&1u32.to_le_bytes());
    for (offset, value) in [
        (176, 20i32),
        (180, 30),
        (184, 40),
        (212, 50),
        (192, 1),
        (196, 0),
        (200, 1),
        (216, 1),
        (136, 3),
        (140, 1),
    ] {
        let at = 64 + offset;
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let p = NativePreferences::parse(&bytes, PreferencesOrigin::SavedSystemFile).unwrap();
    assert_eq!(p.origin, PreferencesOrigin::SavedSystemFile);
    assert_eq!(p.audio.music_volume, 20);
    assert_eq!(p.audio.sound_volume, 40);
    assert_eq!(p.audio.voice_volume, 30);
    assert_eq!(p.audio.sound_voice_volume, 50);
    assert!(!p.audio.voice_enabled);
    assert_eq!(p.message_speed, 3);
    assert_eq!(p.skip_mode, 1);
    let mut corrupt = bytes.clone();
    corrupt[64..68].copy_from_slice(&4096u32.to_le_bytes());
    assert!(NativePreferences::parse(&corrupt, PreferencesOrigin::SavedSystemFile).is_err());
}
