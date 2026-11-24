// SPDX-License-Identifier: MPL-2.0
use super::*;
use crysirror_core::{
    Result as CoreResult,
    service::{AssetReader, ClientStorage, FileSystem, MemoryAssetReader},
    state::Command,
};
use std::time::{Duration, Instant};

const CLIENT: ClientId = ClientId(17);

#[derive(Default)]
struct Assets(BTreeMap<String, Vec<u8>>);

impl ClientStorage for Assets {
    fn read(&self, path: &VfsPath) -> CoreResult<Vec<u8>> {
        if path.mount() != Mount::Asset {
            return Err(Error::ReadOnly);
        }
        self.0.get(path.relative()).cloned().ok_or(Error::NotFound)
    }
    fn open_read(&self, path: &VfsPath) -> CoreResult<Arc<dyn AssetReader>> {
        Ok(Arc::new(MemoryAssetReader(self.read(path)?.into())))
    }
    fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
        Err(Error::ReadOnly)
    }
    fn remove(&self, _: &VfsPath) -> CoreResult<()> {
        Err(Error::ReadOnly)
    }
    fn list(&self, _: Mount, _: &str) -> CoreResult<Vec<String>> {
        Err(Error::Unsupported("unused scenario test list"))
    }
}

struct Host(Arc<Assets>);
impl FileSystem for Host {
    fn read(&mut self, client: ClientId, path: &VfsPath) -> CoreResult<Vec<u8>> {
        self.storage(client)?.read(path)
    }
    fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
        Err(Error::ReadOnly)
    }
}
impl HostServices for Host {
    fn submit(&mut self, _: ClientId, _: &[Command]) -> CoreResult<()> {
        Err(Error::Unsupported("scenario load should not render"))
    }
    fn now_ms(&self) -> u64 {
        0
    }
    fn storage(&mut self, client: ClientId) -> CoreResult<Arc<dyn ClientStorage>> {
        ensure_client(client)?;
        Ok(self.0.clone())
    }
}

fn ensure_client(client: ClientId) -> CoreResult<()> {
    if client == CLIENT {
        Ok(())
    } else {
        Err(Error::NotFound)
    }
}

fn scenario(abi: u32, records: &[u8], n: u32) -> Vec<u8> {
    let mut bytes = b"\0DLR".to_vec();
    for word in [abi, if abi == 1 { 16 } else { 12 }, n] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(records);
    bytes
}

fn vm(abi: u32) -> Result<Vm> {
    let spec = serde_json::from_value(json!({
        "name":"test", "adapter":"exhibit", "game":"fixture",
        "asset_prefix":"does-not-belong-in-native-T2",
        "config": { "entry":"Start", "abi":abi, "canvas":[1024,576] }
    }))?;
    let mut vm = Factory::scenario(CLIENT, &spec)?;
    vm.registers = super::native_state_tests::fresh(abi)?;
    if abi == 3 {
        vm.seeds = Some(profile::Seeds::unencoded_fixture());
    }
    Ok(vm)
}

fn load(vm: &mut Vm, host: &mut Host) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        vm.step(host, InputFrame::default(), 1)?;
        if vm.script.is_some() {
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "scenario worker did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn t2_follows_only_accepted_midori_loads_across_script_transfer() -> Result<()> {
    for abi in [3, 1] {
        let fixtures = Assets(BTreeMap::from([
            (
                "rld/Start.rld".into(),
                scenario(abi, b"\x11\0\0\x12MiXeD\0*\0", 1),
            ),
            (
                "rld/MiXeD.rld".into(),
                scenario(abi, b"\x11\0\0\x12Broken\0*\0", 1),
            ),
            (
                "rld/Broken.rld".into(),
                scenario(1 + (abi == 1) as u32 * 2, b"", 0),
            ),
        ]));
        let mut host = Host(Arc::new(fixtures));
        let mut vm = vm(abi)?;
        assert_eq!(vm.registers.text(2)?, b"");
        load(&mut vm, &mut host)?;
        let before: &[u8] = if abi == 3 { br"rld\Start.rld" } else { b"" };
        assert_eq!(vm.registers.text(2)?, before);
        if abi == 3 {
            // V3's authored opcode is gated on complete native capture.
            // Exercise its loader request boundary without admitting 0x11.
            vm.request_transferred_script("MiXeD".into());
        } else {
            vm.step(&mut host, InputFrame::default(), 1)?;
        }
        assert_eq!(vm.name, "MiXeD");
        assert!(vm.script.is_none());
        assert_eq!(vm.registers.text(2)?, before);
        load(&mut vm, &mut host)?;
        let after: &[u8] = if abi == 3 { br"rld\MiXeD.rld" } else { b"" };
        assert_eq!(vm.registers.text(2)?, after);
        if abi == 3 {
            // V3's authored opcode is gated on complete native capture.
            // Exercise its loader request boundary without admitting 0x11.
            vm.request_transferred_script("Broken".into());
        } else {
            vm.step(&mut host, InputFrame::default(), 1)?;
        }
        assert_eq!(vm.name, "Broken");
        let err = load(&mut vm, &mut host).expect_err("wrong RLD version is rejected");
        assert!(err.to_string().contains("unsupported ExHibit RLD version"));
        assert_eq!(vm.registers.text(2)?, after);
        assert!(vm.script.is_none());
        assert_eq!(vm.registers.text(201)?, b"");
    }
    Ok(())
}

#[test]
fn midori_debug_clear_updates_all_pairs_and_guards_visible_overlay() -> Result<()> {
    let mut records =
        b"\x47\0\0\x21536870912,-1,0,0,0,0,-1,8,9,10,-11,12,13,14,15,16,17,18,19,20,21,22\0"
            .to_vec();
    records.extend(b"\x47\0\0\x2165536,-1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0\0");
    let fixtures = Assets(BTreeMap::from([(
        "rld/Start.rld".into(),
        scenario(3, &records, 2),
    )]));
    let mut h = Host(Arc::new(fixtures));
    let mut vm = vm(3)?;
    load(&mut vm, &mut h)?;
    assert!(vm.midori_debug.is_none());
    vm.step(&mut h, InputFrame::default(), 1)?;
    let mode = vm.midori_debug.clone().unwrap();
    assert_eq!(mode.mode, 0);
    assert_eq!(
        mode.pairs,
        [
            [-1, 8],
            [9, 10],
            [-11, 12],
            [13, 14],
            [15, 16],
            [17, 18],
            [19, 20],
            [21, 22]
        ]
    );
    let pc = vm.script.as_ref().unwrap().pc;
    assert!(vm.step(&mut h, InputFrame::default(), 1).is_err());
    assert_eq!(vm.script.as_ref().unwrap().pc, pc);
    assert_eq!(vm.midori_debug, Some(mode));
    Ok(())
}

#[test]
fn midori_control_definitions_commit_only_the_addressed_parts_and_sound_lanes() -> Result<()> {
    let mut records = b"\xbf\0\0\x01338,-1,0,4,0,0,true,299.-1.7\0".to_vec();
    records.extend(b"\x0b\0\0\x0157,-1,4,5,736,374,res\\g\\sy\\01\\01_922.gyu\0");
    records.extend(b"\xbf\0\0\x01338,-1,0,5,0,0,false,18.19\0");
    records.extend(b"\xbf\0\0\x01338,-1,0,6,0,0,true,-1\0");
    records.extend(b"\x0b\0\0\x0157,-1,4,1,-2,24,res\\g\\sy\\01\\01_922.gyu\0");
    records.extend(b"\x0b\0\0\x0157,-1,4,5,1,2,res\\g\\sy\\01\\01_922.gyu\0");
    records.extend(b"\xbf\0\0\x01338,-1,0,99,0,0,true,12.13\0");
    let fixtures = Assets(BTreeMap::from([(
        "rld/Start.rld".into(),
        scenario(3, &records, 7),
    )]));
    let mut h = Host(Arc::new(fixtures));
    let mut vm = vm(3)?;
    load(&mut vm, &mut h)?;
    assert!(vm.midori_control_defs.sound_slots().is_empty());
    for _ in 0..6 {
        vm.step(&mut h, InputFrame::default(), 1)?;
    }
    let sounds = BTreeMap::from([(4, 299), (5, -1), (6, -1)]);
    assert_eq!(vm.midori_control_defs.sound_slots(), &sounds);
    assert_eq!(
        vm.midori_control_defs.part_status()?,
        [
            (1, "res/g/sy/01/01_922.gyu".into(), [-2, 24]),
            (5, "res/g/sy/01/01_922.gyu".into(), [1, 2]),
        ]
    );
    let pc = vm.script.as_ref().unwrap().pc;
    assert!(vm.step(&mut h, InputFrame::default(), 1).is_err());
    assert_eq!(vm.script.as_ref().unwrap().pc, pc);
    assert_eq!(vm.midori_control_defs.sound_slots(), &sounds);
    assert!(midori_control_parts::SoundUpdate::parse(b"338,-1,1,0,0,0,true,299/18").is_err());
    Ok(())
}

#[test]
fn midori_control_bundle_and_shortcuts_preserve_op_specific_owners() -> Result<()> {
    let mut records = b"\x0b\0\0\x0157,-1,4,5,-10,374,res\\g\\sy\\01\\01_922.gyu\0".to_vec();
    records.extend(b"\x2c\0\0\x013,res\\g\\sy\\01\\01_920.gyu,736,374,res\\g\\sy\\01\\01_921.gyu,736,375,-1,0,0,-1,0,0,-1,0,0,-1,0,0,-1,0,0,-1,0,0\0");
    records.extend(b"\x6d\0\0\x1111,106,1,0,0,0,0\0");
    records.extend(b"\x6d\0\0\x1113,135,129,0,0,0,0\0");
    records.extend(b"\x6d\0\0\x211,104,1,0,1,0,0,57; 14;*\0");
    records.extend(b"\x6d\0\0\x11112,111,3,-1,1,1,0\0");
    records.extend(b"\x6d\0\0\x1117,106,129,0,0,0,0\0");
    let fixtures = Assets(BTreeMap::from([(
        "rld/Start.rld".into(),
        scenario(3, &records, 7),
    )]));
    let mut h = Host(Arc::new(fixtures));
    let mut vm = vm(3)?;
    load(&mut vm, &mut h)?;
    for _ in 0..5 {
        vm.step(&mut h, InputFrame::default(), 1)?;
    }
    assert_eq!(
        vm.midori_control_defs.part_status()?,
        [
            (0, "res/g/sy/01/01_920.gyu".into(), [736, 374]),
            (1, "res/g/sy/01/01_921.gyu".into(), [736, 375]),
            (5, "res/g/sy/01/01_922.gyu".into(), [-10, 374]),
        ],
    );
    let shortcuts = serde_json::to_value(&vm.midori_shortcuts)?;
    assert_eq!(vm.midori_shortcuts.ordinary_count(), 1);
    assert_eq!(shortcuts["legacy_key_slots"], json!([-1, -1, -1, -1]));
    assert_eq!(shortcuts["right"]["key"], 2);
    assert_eq!(shortcuts["right"]["allowed"], 1);
    assert_eq!(shortcuts["right"]["action"], 135);
    assert_eq!(shortcuts["keys"]["112"]["parameter"], "57, 14,*");
    vm.step(&mut h, InputFrame::default(), 1)?;
    let before = serde_json::to_value(&vm.midori_shortcuts)?;
    assert!(before["keys"]["112"]["parameter"].is_null());
    assert_eq!(before["keys"]["112"]["args"], json!([-1, 1, 1, 0]));
    let pc = vm.script.as_ref().unwrap().pc;
    assert!(vm.step(&mut h, InputFrame::default(), 1).is_err());
    assert_eq!(vm.script.as_ref().unwrap().pc, pc);
    assert_eq!(serde_json::to_value(&vm.midori_shortcuts)?, before);
    Ok(())
}
