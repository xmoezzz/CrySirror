use crysirror_system_nnn::{Program, XorProfile};
fn fixture() -> Vec<u8> {
    let mut words = vec![0u32; 37];
    words[0] = 32;
    words[1] = 0x66660001;
    words[2] = 0x55550001;
    words[4] = 1;
    words[5] = 35;
    words[7] = 36;
    words[10] = 1;
    words[18] = 36;
    words[32] = 3;
    words[33] = 0x66660002;
    words[34] = 0x55560001;
    words[35] = 36;
    words[36] = 0x00434241;
    words.iter().flat_map(|v| (!v).to_le_bytes()).collect()
}
#[test]
fn native_tables_text_and_word_pc_preserved() {
    let p = Program::parse(&fixture(), XorProfile::default()).unwrap();
    assert_eq!(p.word_count(), 37);
    assert_eq!(p.text(0, 0).unwrap(), b"ABC");
    let i = p.instruction(32).unwrap();
    assert_eq!(i.next, 35);
    assert_eq!(i.identify, 0x66660002);
    assert_eq!(i.code, 0x55560001);
    assert!(p.text(0, 1).is_err());
    assert!(p.text(2, 0).is_err());
    assert!(p.instruction(36).is_err());
}
#[test]
fn truncated_malformed_and_wrong_profile_fail() {
    let encoded = fixture();
    for size in 0..encoded.len() {
        assert!(Program::parse(&encoded[..size], XorProfile::default()).is_err());
    }
    assert!(Program::parse(
        &encoded,
        XorProfile {
            xor: 0,
            xor_increment: 0,
            add_increment: 0
        }
    )
    .is_err());
    let mut bad = encoded;
    bad[35 * 4..36 * 4].copy_from_slice(&0u32.to_le_bytes());
    assert!(Program::parse(&bad, XorProfile::default()).is_err());
    let mut unaligned = vec![1, 2, 3];
    assert!(
        crysirror_resource::legacy_stream::rolling_word_xor_add(&mut unaligned, 1, 2, 3).is_err()
    );
    assert_eq!(unaligned, [1, 2, 3]);
    let mut words = [u32::MAX, 0, 1, 2]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect::<Vec<_>>();
    crysirror_resource::legacy_stream::rolling_word_xor_add(
        &mut words, 0xfffffff0, 0x11, 0xffffffff,
    )
    .unwrap();
    let actual = words
        .chunks_exact(4)
        .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(actual, [15, 0, 0x11, 0x1e]);
}
#[test]
#[ignore = "reads an owner MSI and original SPT on workers; no game data committed"]
fn owner_mugen2_original_script_through_shared_installer() {
    use crysirror_core::{service::AssetReader, state::ClientId};
    use crysirror_resource::{installer::InstallerCatalog, read_all, DecodeContext};
    use std::sync::{Arc, Mutex};
    struct File(Mutex<std::fs::File>, u64);
    impl AssetReader for File {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(self.1)
        }
        fn read_at(&self, at: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = self.0.lock().unwrap();
            f.seek(SeekFrom::Start(at))
                .and_then(|_| f.read(out))
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))
        }
    }
    let f = std::fs::File::open(std::env::var("CRYSIRROR_TEST_MSI").unwrap()).unwrap();
    let len = f.metadata().unwrap().len();
    let mut cx = DecodeContext::new(ClientId(1));
    cx.limits.max_entry_size = 1024 * 1024 * 1024;
    let catalog = InstallerCatalog::open(Arc::new(File(Mutex::new(f), len)), &cx).unwrap();
    if let Some(output)=std::env::var_os("CRYSIRROR_TEST_NNN_EXE_OUT") {
        let (id,_)=catalog.files().iter().enumerate().find(|(_,f)|f.path=="mugen2/mugen2.exe").unwrap();
        let resource=catalog.read(id,&cx).unwrap();
        let bytes=read_all(resource.as_ref(),32*1024*1024).unwrap();
        std::fs::write(output,bytes).unwrap();
    }
    let (id, file) = catalog
        .files()
        .iter()
        .enumerate()
        .find(|(_, f)| f.path.ends_with("/preprologue.spt"))
        .unwrap();
    let reader = catalog.read(id, &cx).unwrap();
    let bytes = read_all(reader.as_ref(), 16 * 1024 * 1024).unwrap();
    let p = Program::parse(&bytes, XorProfile::default()).unwrap();
    if std::env::var_os("CRYSIRROR_TEST_SPT_TRACE").is_some() {
        for f in catalog.files().iter() {
            println!("installed {}", f.path);
        }
        for name in ["spt/var.fxf","spt/varinitdata.xtx","spt/project.xtx","init2/game.fxf"] {
            let (id,_)=catalog.files().iter().enumerate().find(|(_,f)|f.path.ends_with(name)).unwrap();
            let input=catalog.read(id,&cx).unwrap();let data=read_all(input.as_ref(),1024*1024).unwrap();
            let names=crysirror_system_nnn::config::Names::parse(&data,name.ends_with(".fxf")).unwrap();
            println!("config {name} count={} first={:?} v310={:?}",names.tokens.len(),&names.tokens[..names.tokens.len().min(20)],names.tokens.get(310));
            if name=="init2/game.fxf"{println!("native varInitFlag={:?}",names.integer(b"varInitFlag").unwrap());}
        }
        let mut at=p.header_words;
        for _ in 0..120 {
            let i=p.instruction(at).unwrap();
            println!("pc={:#x} next={:#x} id={:#x} code={:#x} operands={:x?}", i.pc,i.next,i.identify,i.code,i.operands);
            at=i.next;
        }
    }
    let first = p.instruction(p.header_words).unwrap();
    println!(
        "source={} words={} messages={} strings={} first_pc={:#x} identify={:#x} code={:#x}",
        file.path,
        p.word_count(),
        p.message_count(0).unwrap(),
        p.message_count(1).unwrap(),
        first.pc,
        first.identify,
        first.code
    );
    assert_eq!(p.header_words, 32);
    assert!(p.message_count(0).unwrap() > 0);
    let load_names=|path:&str,ciphered:bool| {
        let (id,_)=catalog.files().iter().enumerate().find(|(_,f)|f.path.ends_with(path)).unwrap();
        let input=catalog.read(id,&cx).unwrap();let data=read_all(input.as_ref(),1024*1024).unwrap();
        crysirror_system_nnn::config::Names::parse(&data,ciphered).unwrap()
    };
    let names=load_names("spt/var.fxf",true);
    assert_eq!(names.tokens.len(),1000,"explicit installed legacy variable ABI");
    let setup=load_names("init2/game.fxf",true);
    let initial=load_names("spt/varinitdata.xtx",false);
    let profile=crysirror_system_nnn::expression::VariableProfile::Legacy1000;
    let mut variables=crysirror_system_nnn::config::configured_variables(profile,&names,&setup).unwrap();
    let use_initial=setup.integer(b"varInitFlag").unwrap().unwrap_or(0)!=0;
    crysirror_system_nnn::config::initialize_new_game(&mut variables,profile,use_initial.then_some(&initial)).unwrap();
    println!("original_new_game var_init={use_initial} var310={}",variables.read_stored(310));
    let mut machine = crysirror_system_nnn::machine::Machine::with_variables(variables);
    let mut metadata = crysirror_system_nnn::metadata::Metadata::default();
    // This is a partial native VM boundary test, not a runnable-game test.
    let frontier=metadata.advance(&mut machine, &p, 1000).unwrap();
    let crysirror_system_nnn::machine::Step::Native(call)=frontier else {panic!("expected retained original native frontier: {frontier:?}")};
    assert_ne!(machine.pc(),0x41,"original IF must actually execute");
    assert!(!metadata.story_name.is_empty());
    let executed = machine.executed();
    assert_eq!(machine.poll(&p,1).unwrap(),crysirror_system_nnn::machine::Step::Native(call.clone()));
    assert_eq!(machine.pc(),call.pc);
    assert_eq!(machine.executed(), executed);
    println!(
        "partial_vm_instructions={} pc={:#x} story_bytes={} film_bytes={} boundary={call:?}",
        executed,
        machine.pc(),
        metadata.story_name.len(),
        metadata.film_name.len()
    );
}
