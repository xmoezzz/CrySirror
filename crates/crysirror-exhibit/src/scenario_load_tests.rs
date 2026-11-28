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

struct Host<T = Assets>(Arc<T>);
impl<T: ClientStorage + 'static> FileSystem for Host<T> {
    fn read(&mut self, client: ClientId, path: &VfsPath) -> CoreResult<Vec<u8>> {
        self.storage(client)?.read(path)
    }
    fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
        Err(Error::ReadOnly)
    }
}
impl<T: ClientStorage + 'static> HostServices for Host<T> {
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

#[derive(Default)]
struct DialogueHost {
    time: u64,
    layers: Vec<serde_json::Value>,
    voice_stops: usize,
}

impl FileSystem for DialogueHost {
    fn read(&mut self, _: ClientId, _: &VfsPath) -> CoreResult<Vec<u8>> {
        Err(Error::NotFound)
    }
    fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> CoreResult<()> {
        Err(Error::ReadOnly)
    }
}

impl HostServices for DialogueHost {
    fn submit(&mut self, _: ClientId, _: &[Command]) -> CoreResult<()> {
        Err(Error::Unsupported("dialogue uses retained layer commands"))
    }
    fn now_ms(&self) -> u64 {
        self.time
    }
    fn submit_engine(&mut self, client: ClientId, schema: u32, bytes: &[u8]) -> CoreResult<()> {
        ensure_client(client)?;
        assert_eq!(schema, 1);
        self.layers.push(serde_json::from_slice(bytes).unwrap());
        Ok(())
    }
    fn submit_audio(
        &mut self,
        client: ClientId,
        request: crysirror_core::service::AudioRequest,
    ) -> CoreResult<()> {
        ensure_client(client)?;
        assert!(matches!(request, crysirror_core::service::AudioRequest::Stop {
            channel: voice::CHANNEL, fade_ms: 0.0,
        }));
        self.voice_stops += 1;
        Ok(())
    }
}

#[test]
fn v1_dialogue_keeps_the_line_on_screen_until_a_fresh_advance() -> Result<()> {
    use crysirror_core::service::buttons;

    let mut vm = vm(1)?;
    let mut host = DialogueHost::default();
    let line = printer::Line {
        character: 0,
        ordinal: 1,
        wait: true,
        say: printer::OrdinarySay::decode(b"Name", b"Line")?,
    };
    let commands = json!({"layers":[{"op":"put","layer":{"id":308}}]});
    vm.v1_dialogue = Some(V1Dialogue::Voice(line, commands, [84, 60], None));

    assert!(vm.tick_say(&mut host, InputFrame::default())?.is_some());
    assert_eq!(host.layers[0]["layers"][0]["layer"]["id"], 308);
    assert_eq!(host.layers[0]["layers"][1]["layer"]["id"], printer::MARKER_LAYER);
    assert_eq!(vm.v1_history.back().unwrap().ordinal, 1);

    host.time = 150;
    let held = InputFrame { buttons_down: buttons::ENTER, ..InputFrame::default() };
    assert!(vm.tick_say(&mut host, held)?.is_some());
    assert_ne!(host.layers[1]["layers"][0]["layer"]["uv"], host.layers[0]["layers"][1]["layer"]["uv"]);
    assert_eq!(host.voice_stops, 0);

    let fresh = InputFrame { buttons_pressed: buttons::ENTER, ..held };
    assert!(vm.tick_say(&mut host, fresh)?.is_none());
    assert_eq!(host.layers[2], json!({"layers":[{"op":"remove","id":printer::MARKER_LAYER}]}));
    assert_eq!(host.voice_stops, 1);
    assert_eq!(vm.v1_history.len(), 1);
    Ok(())
}

#[test]
fn v1_prologue_fade_closes_the_active_printer_at_the_black_cover() -> Result<()> {
    let mut vm = vm(1)?;
    let mut host = DialogueHost::default();
    vm.message_areas.define(message_area::Area::parse(
        b"6,2,-1,16,,,,0,256,800,420,32,32,728,404,0,0,0,0,JP,34304,400,30,75,128,0,16777215,0,JP,34561,700,20,18,128,0,16777215,0",
    )?);
    vm.bind_v1_message_frame(6, &mut host)?;
    vm.message_areas.open(6, &mut vm.message_backdrop, &mut host, CLIENT, &vm.registers, false)?;
    let record: Vec<_> = [0x0005_004e_u32, 800, 0x801, 400, 0, 83]
        .into_iter().flat_map(u32::to_le_bytes).collect();
    vm.script = Some(Script::new(scenario(1, &record, 1), 1)?);

    vm.step(&mut host, InputFrame::default(), 1)?;
    assert!(vm.message_backdrop.opened);
    host.time = 799;
    vm.step(&mut host, InputFrame::default(), 1)?;
    assert!(vm.message_backdrop.opened);
    assert_eq!(host.layers.last().unwrap()["layers"][0]["opacity"], 240);

    host.time = 800;
    vm.step(&mut host, InputFrame::default(), 1)?;
    assert!(!vm.message_backdrop.opened);
    assert!(vm.scene_fade.is_none());
    assert_eq!(vm.draw_scene[0], i32::MIN);
    assert!(host.layers.iter().any(|batch| {
        batch["layers"][0] == json!({"op":"set_visible","id":307,"visible":false})
    }));
    let layers = &host.layers.last().unwrap()["layers"];
    assert_eq!(layers[0]["layer"]["paint"]["color"], json!([0, 0, 0, 255]));
    assert_eq!(layers[1], json!({"op":"remove","id":339}));
    Ok(())
}

#[test]
fn v1_signed_background_paints_and_waits_without_recording_cg_experience() -> Result<()> {
    let mut vm = vm(1)?;
    let mut host = DialogueHost::default();
    vm.canvas = [800, 600];
    let color = 0x8012_3456_u32;
    let records: Vec<u8> = [(color, 9, 800), (0x80ff_ffff, 255, 0), (5, 255, 0)]
        .into_iter()
        .flat_map(|(id, mode, ms)| {
            [0x0010_004a, id, mode, 0, ms, 0, 0, 0, 0, 0, 0, 0, u32::MAX, u32::MAX, 0, 0, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
        })
        .collect();
    vm.script = Some(Script::new(scenario(1, &records, 3), 1)?);
    let seen = vm.registers.capture_experience()?.nonzero_words();
    assert_eq!(vm.step(&mut host, InputFrame::default(), 1)?, VmYield::WaitUntil(800));
    let paint = &host.layers[0]["layers"][1]["layer"];
    assert_eq!(paint["id"], 1);
    assert_eq!(paint["paint"], json!({"kind":"solid","color":[18,52,86,255]}));
    assert_eq!(vm.draw_scene[0], color as i32);
    assert_eq!(vm.draw_scene[8..12], [-1, -1, 0, 800 | (600 << 16)]);
    assert_eq!(vm.registers.capture_experience()?.nonzero_words(), seen);
    assert_eq!(host.layers[0]["layers"][0]["transition"]["duration_ms"], 800);
    assert_eq!(host.layers[0]["layers"][0]["transition"]["easing"]["progress"]["shape"]["segments"], 16);

    host.time = 799;
    assert_eq!(vm.step(&mut host, InputFrame::default(), 1)?, VmYield::WaitUntil(800));
    assert_eq!(host.layers.len(), 1);
    host.time = 800;
    vm.step(&mut host, InputFrame::default(), 1)?;
    assert_eq!(host.layers[1]["layers"][0]["layer"]["paint"]["color"], json!([255,255,255,255]));
    assert_eq!(vm.registers.capture_experience()?.nonzero_words(), seen);
    vm.step(&mut host, InputFrame::default(), 1)?;
    assert_eq!(host.layers[2]["layers"][0]["layer"]["paint"]["asset"], "res/g/ev/00/0005.gyu");
    assert_eq!(vm.registers.capture_experience()?.nonzero_words(), seen + 1);
    Ok(())
}

#[test]
fn v1_magnification_can_freeze_or_wait_without_changing_the_destination() -> Result<()> {
    let canvas = [800, 600];
    let target = [200, 0, 800, 600];
    for mode in [0, 1] {
        let mut vm = vm(1)?;
        let mut host = DialogueHost::default();
        vm.canvas = canvas;
        let mut motion = clip::Clip::new(
            "fixture.gyu".into(), [1000, 600], [0, 0, 800, 600], target, 1000, 0,
        )?;
        assert!(!motion.present(&mut host, CLIENT, canvas)?);
        vm.clip = Some(motion);
        vm.magnify_target = Some(target);
        // With settle clear, the supplied rectangle and replacement
        // duration are ignored by leaveClip.
        let record: Vec<_> = [0x000a_0052_u32, mode, 0, 50, 30, 700, 500, 400, 0, 0, 0]
            .into_iter().flat_map(u32::to_le_bytes).collect();
        vm.script = Some(Script::new(scenario(1, &record, 1), 1)?);
        host.time = 500;
        assert_eq!(vm.step(&mut host, InputFrame::default(), 1)?, VmYield::WaitUntil(501));
        let layer = &host.layers.last().unwrap()["layers"][0]["layer"];
        assert_eq!(layer["uv"], json!([0.1_f32, 0.0, 0.9_f32, 1.0]));
        if mode == 0 {
            assert_eq!(vm.script.as_ref().unwrap().pc, 16);
            assert!(vm.clip.is_some());
            host.time = 1000;
            assert_eq!(vm.step(&mut host, InputFrame::default(), 1)?, VmYield::WaitUntil(1001));
            assert_eq!(vm.draw_scene[2..6], target);
        } else {
            assert_eq!(vm.draw_scene[2..6], [100, 0, 800, 600]);
        }
        assert_eq!(vm.script.as_ref().unwrap().remaining, 0);
        assert!(vm.clip.is_none());
        assert!(vm.magnify_target.is_none());
    }
    Ok(())
}

pub(super) fn scenario(abi: u32, records: &[u8], n: u32) -> Vec<u8> {
    let mut bytes = b"\0DLR".to_vec();
    for word in [abi, if abi == 1 { 16 } else { 12 }, n] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(records);
    bytes
}

pub(super) fn vm(abi: u32) -> Result<Vm> {
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

#[derive(Default)]
struct TransferFiles {
    assets: Assets,
    written: std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
}
impl ClientStorage for TransferFiles {
    fn read(&self, path: &VfsPath) -> CoreResult<Vec<u8>> {
        match path.mount() {
            Mount::Asset => self.assets.read(path),
            Mount::Save => self.written.lock().unwrap().get(path.relative()).cloned().ok_or(Error::NotFound),
            _ => Err(Error::NotFound),
        }
    }
    fn open_read(&self, path: &VfsPath) -> CoreResult<Arc<dyn AssetReader>> {
        Ok(Arc::new(MemoryAssetReader(self.read(path)?.into())))
    }
    fn write_atomic(&self, path: &VfsPath, bytes: &[u8]) -> CoreResult<()> {
        if path.mount() != Mount::Save { return Err(Error::ReadOnly); }
        self.written.lock().unwrap().insert(path.relative().into(), bytes.to_vec());
        Ok(())
    }
    fn remove(&self, _: &VfsPath) -> CoreResult<()> { Err(Error::ReadOnly) }
    fn list(&self, _: Mount, _: &str) -> CoreResult<Vec<String>> { Err(Error::Unsupported("unused transfer directory listing")) }
}

#[test]
fn midori_os_query_uses_original_value_table() -> Result<()> {
    let mut vm = vm(3)?;
    let predicate = Condition::for_abi(b"Q3\x81\x83104", 3)?;
    for (version, code, branch) in [
        (None, -1, true),
        (Some([1, 4, 10]), 2, true),
        (Some([2, 5, 2]), 103, true),
        (Some([2, 6, 0]), 104, false),
        (Some([2, 6, 1]), 105, false),
        (Some([2, 10, 0]), -1, true),
    ] {
        vm.registers.set_windows_version(version);
        assert_eq!(registers::Expression::parse(b"q3")?.eval(&vm.registers)?, code);
        assert_eq!(predicate.matches(&vm.registers)?, branch);
    }
    Ok(())
}

#[test]
fn midori_transfer_flushes_a_native_system_bank_and_installs_only_a_valid_successor() -> Result<()> {
    let path = VfsPath::new(Mount::Save, "fixture.system.dat")?;
    for (mode, successor) in [(0, true), (1, true), (2, true), (3, false), (0, false)] {
        let mut vm = vm(3)?;
        vm.limits = Some(native_state_tests::LIMITS);
        vm.seeds.as_mut().unwrap().checkpoint_layout = true;
        vm.midori_positions = Some(midori_positions::Work::begin_script());
        vm.midori_system_path = Some(path.clone());
        vm.registers.set_normal(3, 17)?;
        vm.registers.set_normal(6, 1)?;
        vm.registers.set_system(0, 88)?;
        vm.registers.set_system(7, 5)?;
        vm.registers.set_local(29, 123)?;
        vm.registers.change(Change::TextAssign {
            dest: 20, mode: 0, source: registers::TextValue::Literal(b"next".to_vec()),
            replacement: Vec::new(), when: Condition::text(b"*")?,
        })?;
        vm.registers.change(Change::MidoriStringDb(registers::MidoriStringDb::create(
            1, b"3\ttrue,unreachable\n1\tfalse,unreachable\n2\tR29\x81\x81123,next",
        )?))?;
        let mut records = b"\x11\0\0\x21no_such_file,false,0\0\x11\0\0\x21".to_vec();
        records.extend(match mode {
            0 => &b"next,*,0\0"[..],
            1 => b"20,*,1\0",
            2 => b"1,*,2\0",
            _ => b"ignored,*,3\0",
        });
        vm.script = Some(Script::new(scenario(3, &records, 2), 3)?);
        let mut files = TransferFiles::default();
        if successor {
            files.assets.0.insert("rld/next.rld".into(), scenario(3, b"\x18\0\0\0", 1));
        }
        let files = Arc::new(files);
        let mut host = Host(files.clone());
        vm.step(&mut host, InputFrame::default(), 1)?;
        let pc = vm.script.as_ref().unwrap().pc;
        assert!(vm.midori_transfer.is_none());
        assert!(files.written.lock().unwrap().is_empty());
        vm.step(&mut host, InputFrame::default(), 1)?;
        assert_eq!(vm.script.as_ref().unwrap().pc, pc);
        assert_eq!(vm.registers.read_normal(3)?, 17);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut result = Ok(VmYield::BudgetExhausted);
        while vm.midori_transfer.is_some() {
            result = vm.tick(&mut host, InputFrame::default(), 1);
            ensure!(Instant::now() < deadline, "ExHibit transfer worker timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        if successor {
            result?;
            assert_eq!(vm.name, "next");
            assert_eq!(vm.registers.text(2)?, br"rld\next.rld");
            assert_eq!(vm.registers.read_normal(2)?, 0);
            assert_eq!(vm.registers.read_normal(3)?, -1);
            assert_eq!(vm.registers.read_normal(6)?, 0);
            assert_eq!(vm.registers.read_normal(29)?, 123);
            let value = v1_save::read_system_path(files.as_ref(), &path, vm.limits.unwrap())?.unwrap();
            assert_eq!([value[0], value[7]], [0, 5]);
        } else if mode == 3 {
            assert_eq!(result?, VmYield::Halted);
            assert!(vm.halted);
            assert_eq!(vm.script.as_ref().unwrap().remaining, 0);
            let value = v1_save::read_system_path(files.as_ref(), &path, vm.limits.unwrap())?.unwrap();
            assert_eq!([value[0], value[7]], [0, 5]);
        } else {
            assert!(result.is_err());
            assert_eq!(vm.script.as_ref().unwrap().pc, pc);
            assert_eq!(vm.registers.read_normal(3)?, 17);
            assert_eq!(vm.registers.read(Value::System(0))?, 88);
            assert!(files.written.lock().unwrap().is_empty());
        }
    }
    Ok(())
}

#[test]
fn v3_scene_checkpoints_commit_the_native_register_packets() -> Result<()> {
    use profile::Resident;

    for resident in [Resident::Midori2011, Resident::LateVol1, Resident::LateVol2] {
        let mut vm = vm(3)?;
        let mut host = Host(Arc::new(Assets::default()));
        let mut seeds = profile::Seeds::unencoded_fixture();
        seeds.resident = resident;
        seeds.checkpoint_layout = true;
        vm.seeds = Some(seeds);
        vm.midori_positions = Some(midori_positions::Work::begin_script());
        let bytes = b"\x05\0\0\x11-1,66,-1,*,$s block $b\0\x05\0\0\x11-1,0,-1,*,\0";
        let mut script = Script::new(scenario(3, bytes, 2), 3)?;
        script.resident = Some(resident);
        vm.script = Some(script);
        vm.registers.scenario_loaded_native(br"rld\fixture.rld".to_vec())?;
        vm.registers.set_normal(4, 0x400)?;
        vm.registers.set_local(10, 444)?;
        vm.registers.set_local(29, 73)?;
        vm.registers.mark_message(1)?;
        let history = vm.registers.capture_experience()?;

        let first = vm.script.as_ref().unwrap().pc;
        let flags = vm.registers.read(Value::System(23))?;
        for (dest, text) in [(208, &b"previous layer packet"[..]), (209, &b"layer memo"[..])] {
            vm.registers.change(Change::TextAssign {
                dest,
                mode: 0,
                source: registers::TextValue::Literal(text.to_vec()),
                replacement: Vec::new(),
                when: Condition::text(b"*")?,
            })?;
        }
        vm.registers.set_system(23, flags | 32)?;
        if resident.late() {
            let error = vm.step(&mut host, InputFrame::default(), 1).unwrap_err();
            assert!(error.to_string().contains("serialized live layers"));
            assert_eq!(vm.script.as_ref().unwrap().pc, first);
            assert_eq!(vm.script.as_ref().unwrap().block, 0);
            assert_eq!(vm.registers.read(Value::Local(10))?, 444);
            assert!(vm.registers.text(205)?.is_empty());
            vm.registers.set_system(23, flags)?;
        }

        vm.step(&mut host, InputFrame::default(), 1)?;
        assert_eq!(vm.registers.text(209)?, b"layer memo");
        assert_eq!(
            vm.registers.text(208)?,
            if resident.late() { &b"previous layer packet"[..] } else { &b""[..] }
        );
        assert_eq!(vm.registers.read_normal(2)?, 1);
        assert_eq!(vm.registers.read_normal(4)?, 0x482);
        assert_eq!(vm.registers.read_normal(6)?, 1);
        assert_eq!(vm.registers.read_normal(7)?, 0);
        assert_eq!(vm.registers.read_normal(8)?, 1024 << 16 | 576);
        assert_eq!(vm.registers.read_normal(9)?, 256);
        assert_eq!(vm.registers.read_normal(10)?, -1);
        assert_eq!(vm.registers.read_normal(29)?, 73);
        assert_eq!(vm.registers.read_normal(78)?, -1);
        assert_eq!(vm.registers.read_normal(147)?, -1);
        assert_eq!(vm.registers.text(1)?, br"rld\fixture.rld block 1");
        assert_eq!(vm.registers.text(5)?, b"-1;8;0,0,0,0;-1;;;;;;;;");
        assert_eq!(vm.registers.text(202)?, midori_checkpoint::DEFAULT_MARKER);
        let rows: Vec<_> = vm.registers.text(205)?.split(|&b| b == b';').collect();
        assert_eq!(rows.len(), 9);
        assert_eq!(rows[0], b"8");
        assert!(rows[1..].iter().all(|row| *row == b"0,-2,-2,-1,-1,-1,-1,16842752,0,0,0,0"));
        assert_eq!(vm.registers.read(Value::System(history.base + 3))?, history.counts[3] as i32);
        if resident.late() {
            assert_eq!(vm.registers.text(8)?, b"3;0;0");
            assert_eq!(vm.registers.text(210)?, b"0,,,");
            assert_eq!(vm.registers.read_normal(61)?, 0x83_0000);
            assert_eq!(vm.registers.text(204)?.split(|&b| b == b';').count(), 10);
        } else {
            assert_eq!(vm.registers.text(8)?, b"1;0");
            assert_eq!(vm.registers.text(210)?, b"");
            assert_eq!(vm.registers.read_normal(61)?, 0x80_0000);
            assert_eq!(vm.registers.text(204)?.split(|&b| b == b';').count(), 6);
        }
        vm.step(&mut host, InputFrame::default(), 1)?;
        assert_eq!(vm.registers.read_normal(2)?, 2);
        assert_eq!(vm.registers.read_normal(4)?, 0x402);
        assert_eq!(vm.registers.read_normal(6)?, 0);
    }
    Ok(())
}

#[test]
fn v3_dialog_definition_routes_resources_and_defaults_by_profile() -> Result<()> {
    use profile::Resident;

    let records = b"\x42\0\0\x011,131074,-1,4,870,10,20,bg.gyu,55,30,40,7,8,*,0,1,2,a.gyu,1,3,4,b.gyu\0\x42\0\0\x012,65537,0,0,50,0,0,bg.gyu,55,0,0,9,10,select.gyu,1,0,0,a.gyu\0";
    for (resident, background, selection) in [
        (Resident::Midori2011, 185, 136),
        (Resident::LateVol1, 210, 143),
        (Resident::LateVol2, 210, 143),
    ] {
        let mut vm = vm(3)?;
        let mut script = Script::new(scenario(3, records, 2), 3)?;
        script.resident = Some(resident);
        vm.script = Some(script);
        let mut host = Host(Arc::new(Assets::default()));
        vm.step(&mut host, InputFrame::default(), 2)?;
        assert_eq!(vm.v3_dialogs.len(), 2);
        let first = serde_json::to_value(&vm.v3_dialogs[0])?;
        let second = serde_json::to_value(&vm.v3_dialogs[1])?;
        assert_eq!(first["dialog"]["id"], 1);
        assert_eq!(first["default_item"], 1);
        assert_eq!(first["selection_parameters"], json!([7, 8]));
        assert_eq!(first["dialog"]["background"]["resource"], background);
        assert_eq!(first["dialog"]["selection"]["resource"], selection);
        assert_eq!(first["dialog"]["selection"]["name"], json!([]));
        assert_eq!(first["dialog"]["items"][1]["position"], json!([3, 4]));
        assert_eq!(second["default_item"], 0);
        assert_eq!(second["selection_parameters"], json!([0, 0]));
        assert_eq!(second["dialog"]["selection"]["name"], json!(b"select.gyu"));
    }
    Ok(())
}

#[test]
fn later_notification_definition_replaces_owned_slots_and_ignores_invalid_indices() -> Result<()> {
    let mut records = Vec::new();
    for args in [
        &b"31,20,-4,first.gyu"[..],
        b"-1,9,8,invalid.gyu",
        b"32,9,8,invalid.gyu",
        b"0,5,6,other.gyu",
        b"31,-10,15,new,copy.gyu",
        b"0,-1,-2,",
    ] {
        records.extend(0x0100_0100u32.to_le_bytes());
        records.extend(args);
        records.push(0);
    }
    for resident in [profile::Resident::LateVol1, profile::Resident::LateVol2] {
        let mut vm = vm(3)?;
        let mut script = Script::new(scenario(3, &records, 6), 3)?;
        script.resident = Some(resident);
        vm.script = Some(script);
        let mut host = Host(Arc::new(Assets::default()));
        vm.step(&mut host, InputFrame::default(), 1)?;
        let slots = vm.midori_control_defs.notification_slots();
        assert_eq!(serde_json::to_value(&slots[&31])?["resource"], json!(b"first.gyu"));
        vm.step(&mut host, InputFrame::default(), 5)?;
        let slots = vm.midori_control_defs.notification_slots();
        assert_eq!(slots.len(), 2);
        let last = serde_json::to_value(&slots[&31])?;
        assert_eq!(last["position"], json!([-10, 15]));
        assert_eq!(last["resource"], json!(b"new,copy.gyu"));
        let empty = serde_json::to_value(&slots[&0])?;
        assert_eq!(empty["position"], json!([-1, -2]));
        assert_eq!(empty["resource"], json!([]));
    }
    Ok(())
}

#[test]
fn v3_controls_and_portrait_pairs_update_the_native_owners() -> Result<()> {
    use profile::Resident;
    for resident in [Resident::Midori2011, Resident::LateVol1, Resident::LateVol2] {
        let late = resident.late();
        let mut fields = vec!["-1"; if late { 105 } else { 91 }];
        for (i, value) in [
            (0, "5"), (3, "control.gyu"), (4, "16"), (5, "-8"),
            (9, "1"), (12, "254"), (13, "1"), (17, "5;6"), (19, "12"),
            (26, "870"), (27, "80"), (28, "20"), (29, "tip.gyu"),
            (32, "menu.gyu"), (33, "30"), (34, "-40"), (39, "14"),
            (40, "70"), (41, "4"), (42, "10"), (43, "item.gyu"),
            (44, "12"), (45, "25"), (46, "4"), (47, "8"),
            (48, "1;2@name;body"),
        ] {
            fields[i] = value;
        }
        let mut records = Vec::new();
        let mut push = |opcode: u16, flags: u8, raw: &[u8]| {
            records.extend(opcode.to_le_bytes());
            records.extend([0, flags]);
            records.extend(raw);
            records.push(0);
        };
        let flags = if late { 0x61 } else { 0x51 };
        push(0x40, flags, fields.join(",").as_bytes());
        push(0xfb, 1, b"7,800");
        push(0xfb, 1, b"8,802");
        fields[4] = "-30";
        fields[42] = "-1";
        push(0x40, flags, fields.join(",").as_bytes());
        fields[41] = "8";
        push(0x40, flags, fields.join(",").as_bytes());
        let mut vm = vm(3)?;
        let mut script = Script::new(scenario(3, &records, 5), 3)?;
        script.resident = Some(resident);
        vm.script = Some(script);
        let mut host = Host(Arc::new(Assets::default()));
        vm.step(&mut host, InputFrame::default(), 3)?;
        let definition = serde_json::to_value(&vm.midori_control_defs.control_slots()[&5])?;
        assert_eq!(definition["action_id"], 215);
        assert_eq!(definition["layer"], if late { 165 } else { 158 });
        assert_eq!(definition["background"]["origin"], json!([16, -8]));
        assert_eq!(definition["menu_backgrounds"][0]["origin"], json!([30, -40]));
        assert_eq!(definition["menu_keys"], json!([121, 70]));
        assert_eq!(definition["items"][0]["action"], json!(b"1,2@name;body"));
        assert_eq!(definition["items"].as_array().unwrap().len(), if late { 9 } else { 7 });
        assert_eq!(definition["button"]["words"][1], if late { 5 } else { -1 });
        let row = vm.midori_actor_slots.checkpoint_rows().unwrap()[7];
        assert_eq!(row[1], Some(if late { 144 } else { 137 }));
        assert_eq!(row[2], Some(if late { 145 } else { 138 }));
        vm.step(&mut host, InputFrame::default(), 1)?;
        let replaced = serde_json::to_value(&vm.midori_control_defs.control_slots()[&5])?;
        assert_eq!(replaced["background"]["origin"], json!([-30, -8]));
        assert!(replaced["items"][0].is_null());
        let pc = vm.script.as_ref().unwrap().pc;
        assert!(vm.step(&mut host, InputFrame::default(), 1).is_err());
        assert_eq!(vm.script.as_ref().unwrap().pc, pc);
        assert_eq!(serde_json::to_value(&vm.midori_control_defs.control_slots()[&5])?, replaced);
    }
    Ok(())
}

#[test]
fn v1_autosave_reentry_consumes_global_block_guard_without_writing() -> Result<()> {
    let mut vm = vm(1)?;
    let mut host = Host(Arc::new(Assets::default()));
    vm.script = Some(Script::new(scenario(1, b"\x0c\0\0\x01\0", 1), 1)?);
    vm.script.as_mut().unwrap().block = 7;
    vm.registers.set_normal(3, 7)?;
    vm.registers.set_local(3, 11)?;
    assert_eq!(
        vm.step(&mut host, InputFrame::default(), 1)?,
        VmYield::BudgetExhausted
    );
    assert_eq!(vm.script.as_ref().unwrap().remaining, 0);
    assert_eq!(vm.registers.read_normal(3)?, -1);
    assert_eq!(vm.registers.read(Value::Local(3))?, 11);

    vm.script.as_mut().unwrap().seek_record(0)?;
    vm.registers.set_local(3, 7)?;
    let error = vm.step(&mut host, InputFrame::default(), 1).unwrap_err();
    assert!(error.to_string().contains("missing ExHibit save class"));
    let script = vm.script.as_ref().unwrap();
    assert_eq!(script.pc, 16);
    assert_eq!(script.remaining, 1);
    assert_eq!(vm.registers.read_normal(3)?, -1);
    assert_eq!(vm.registers.read(Value::Local(3))?, 7);
    Ok(())
}

#[test]
fn t2_follows_successful_loads_across_script_transfer_for_both_residents() -> Result<()> {
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
        let before = br"rld\Start.rld";
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
        let after = br"rld\MiXeD.rld";
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
    assert!(vm.debug_definition.is_none());
    vm.step(&mut h, InputFrame::default(), 1)?;
    let mode = vm.debug_definition.clone().unwrap();
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
    assert_eq!(vm.debug_definition, Some(mode));
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

#[test]
fn retail_view_definitions_route_to_independent_owners_and_override_only_the_load_page()
-> Result<()> {
    let mut records = Vec::new();
    let mut row = |op: u8, flags: u8, data: &[u8]| {
        records.extend([op, 0, 0, flags]);
        records.extend(data);
        records.push(0);
    };
    let mut fields = vec!["0"; 48];
    fields[0] = "3";
    fields[1] = "10";
    fields[7] = "frame";
    fields[19] = "20";
    fields[20] = "8";
    fields[21] = "-3";
    fields[22] = "-2";
    fields[35] = "page";
    fields[36] = "20";
    fields[37] = "30";
    fields[38] = "-2";
    fields[39] = "8";
    let slot = fields.join(",");
    row(0x29, 1, slot.as_bytes());
    row(
        0x2a,
        0x11,
        format!("{slot},font,1,2,3,4,5,6,7,8").as_bytes(),
    );
    row(0x0b, 1, b"36,12,0,1,70,80");
    row(0x0b, 1, b"36,-1,3,3,999,999");
    row(0x2b,0x11,b"bg,1,2,80,81,4,5,-2,7,item,9,10,-1,4,-2,17,font1,1,2,3,4,5,6,7,8,font2,11,12,13,14,15,16,17,18");
    row(
        0x2e,
        1,
        b"frame,9,8,item,3,4,20,30,4,5,face,1,2,3,4,5,6,7,8",
    );
    row(0x0b, 1, b"30,-1,2,3,overridden");
    let mut vm = vm(1)?;
    let mut host = Host(Arc::new(Assets(BTreeMap::from([(
        "rld/Start.rld".into(),
        scenario(1, &records, 7),
    )]))));
    load(&mut vm, &mut host)?;
    vm.step(&mut host, InputFrame::default(), 7)?;
    assert_eq!(
        vm.view_definitions.save.as_ref().unwrap().page_position(1),
        Some([18, 38])
    );
    assert_eq!(
        vm.view_definitions.load.as_ref().unwrap().page_position(1),
        Some([70, 80])
    );
    assert_eq!(
        vm.view_definitions.load.as_ref().unwrap().page_position(3),
        None
    );
    assert_eq!(vm.v1_entries.names()[3], b"overridden");
    let s = serde_json::to_value(&vm.view_definitions)?;
    assert_eq!(s["load"]["item_bounds"], json!([17, 6, 20, 8]));
    assert_eq!(s["load"]["font"]["words"], json!([1, 2, 3, 4, 5, 6, 7, 8]));
    assert_eq!(
        s["save"]["font"]["words"],
        json!([256, 400, 20, 16, 128, 1, 16777215, 0])
    );
    assert_eq!(s["log"]["colors"], json!([0xa0ffa0, 4, 0xe0e0e0, 17]));
    assert_eq!(
        s["log"]["fonts"][1]["words"],
        json!([11, 12, 13, 14, 15, 16, 17, 18])
    );
    assert_eq!(s["selection"]["bounds"], json!([3, 4, 23, 34]));
    Ok(())
}

#[test]
fn selection_2011_keeps_extra_image_ids_and_replaces_geometry_and_mode_properties() -> Result<()> {
    let mut records = Vec::new();
    let rows: [(u8,&[u8]);3] = [
        (0x21, b"frame,0b10,3,base,red,-1,-1,-1,-1,-1,-1,blue,9,6,-2,8,3,7,0xf0000000,-2,11,face,1,2,0b11,4,5,6,7,8"),
        (0x11, b"next,4,5,base2,-1,green,-1,-1,-1,-1,-1,-1,4,5,6,7,8,9,other"),
        (0x01, b"last,6,7,base3,1,2,3,4,5,6,lastface"),
    ];
    for (flags, text) in rows {
        records.extend([0x2e, 0, 0, flags]);
        records.extend(text);
        records.push(0);
    }
    let mut vm = vm(3)?;
    let mut host = Host(Arc::new(Assets(BTreeMap::from([(
        "rld/Start.rld".into(),
        scenario(3, &records, 3),
    )]))));
    load(&mut vm, &mut host)?;
    vm.step(&mut host, InputFrame::default(), 1)?;
    let s = serde_json::to_value(&vm.select_view_2011)?;
    assert_eq!(s["selected"]["bounds"], json!([7, 6, 9, 14]));
    assert_eq!(s["selected"]["background"]["placement"], json!([2, 3]));
    assert_eq!(s["properties"], json!([-268435456, -2, 11]));
    assert_eq!(s["selected"]["font"]["words"][2], 3);
    vm.step(&mut host, InputFrame::default(), 1)?;
    vm.step(&mut host, InputFrame::default(), 1)?;
    let s = serde_json::to_value(&vm.select_view_2011)?;
    assert_eq!(s["selected"]["bounds"], json!([1, 2, 4, 6]));
    assert_eq!(s["selected"]["item_image"], json!(b"base3"));
    assert_eq!(s["extra_images"][0], json!(b"red"));
    assert_eq!(s["extra_images"][1], json!(b"green"));
    assert_eq!(s["extra_images"][7], json!(b"blue"));
    assert!(s["extra_images"][2].is_null());
    assert_eq!(s["uses_extra_images"], true);
    assert_eq!(s["properties"], json!([0, 0, 0]));
    Ok(())
}

#[test]
fn slot_and_log_2011_resolve_layer_aliases_and_keep_ordinal_page_overrides() -> Result<()> {
    let mut records = Vec::new();
    let mut row = |op: u8, flag: u8, raw: &[u8]| {
        records.extend([op, 0, 0, flag]);
        records.extend(raw);
        records.push(0);
    };
    let mut fields = vec!["0"; 48];
    fields[..7].copy_from_slice(&["2", "7", "12", "40", "291", "800", "925"]);
    fields[36] = "5";
    fields[37] = "9";
    fields[38] = "2";
    fields[39] = "7";
    let slot = fields.join(",");
    row(0x29, 1, slot.as_bytes());
    row(
        0x2a,
        0x81,
        format!("{slot},family,0b11,2,3,4,5,6,7,8").as_bytes(),
    );
    row(0x0b, 1, b"36,-1,0,1,0b110,19");
    row(0x2b, 1, b"log,8,9,40,299,0,1,11,-2,marker,7,43");
    let mut vm = vm(3)?;
    let mut host = Host(Arc::new(Assets(BTreeMap::from([(
        "rld/Start.rld".into(),
        scenario(3, &records, 4),
    )]))));
    load(&mut vm, &mut host)?;
    vm.step(&mut host, InputFrame::default(), 4)?;
    let s = serde_json::to_value(&vm.view_definitions)?;
    assert_eq!(s["save"]["layers"], json!([24, 92, 123, 137, 213]));
    assert_eq!(s["log"]["layers"], json!([92, 103]));
    assert_eq!(s["log"]["creation_parameter"], 43);
    assert_eq!(s["load"]["font"]["words"][0], 3);
    assert_eq!(
        vm.view_definitions.load.as_ref().unwrap().page_position(1),
        Some([6, 19])
    );
    assert_eq!(
        vm.view_definitions.save.as_ref().unwrap().page_position(1),
        Some([7, 16])
    );
    let old = serde_json::to_value(&vm.view_definitions)?;
    assert!(
        vm.view_definitions
            .install(view_definitions::Definition::Load, false, b"1,1,1000", 3)
            .is_err()
    );
    assert_eq!(serde_json::to_value(&vm.view_definitions)?, old);
    Ok(())
}

#[test]
fn action_flag_families_resolve_live_registers_and_leave_unrelated_bits_untouched() -> Result<()> {
    let operations: &[(u8, &[u8])] = &[
        (0xbf, b"186,12,0,0,0,0,*"),
        (0xbf, b"187,-1,0,0,0,0,*"),
        (0xbf, b"188,-1,0,0,0,0,*"),
        (0xbf, b"189,-1,0,0,0,0,*"),
        (0xbf, b"190,-1,0,0,0,0,*"),
        (0xbf, b"191,-1,0,0,0,0,*"),
        (0xbf, b"340,-1,R8,C1,0,0,*"),
        (0xbf, b"341,-1,R8,-1,0,0,*"),
        (0xbf, b"342,-1,0xab,0,0,0,*"),
        (0x0b, b"23,12,0,0b11"),
        (0x0b, b"44,12,0,0"),
        (0xbf, b"348,-1,F9,0,0,0,*"),
        (0xbf, b"340,-1,32,1,0,0,*"),
    ];
    let mut records = Vec::new();
    for &(op, raw) in operations {
        records.extend([op, 0, 0, 1]);
        records.extend(raw);
        records.push(0);
    }
    let mut vm = vm(3)?;
    let mut host = Host(Arc::new(Assets(BTreeMap::from([(
        "rld/Start.rld".into(),
        scenario(3, &records, operations.len() as u32),
    )]))));
    load(&mut vm, &mut host)?;
    vm.registers.set_normal(8, 9)?;
    vm.registers.set_normal(12, 123)?;
    vm.registers.set_system(15, 16)?;
    vm.registers.set_system(20, 0x40)?;
    vm.registers.set_system(23, 0x82)?;
    vm.registers.set_system(42, 0xa0)?;
    let mut step = |vm: &mut Vm| vm.step(&mut host, InputFrame::default(), 1);
    step(&mut vm)?;
    assert_eq!(vm.registers.read(Value::System(23))?, 0x80);
    assert_eq!(vm.registers.read(Value::Local(12))?, 0);
    step(&mut vm)?;
    assert_eq!(vm.registers.read(Value::System(23))?, 0x82);
    for expected in [0x48, 0x40, 0xc0, 0x40, 0x240] {
        step(&mut vm)?;
        assert_eq!(vm.registers.read(Value::System(20))?, expected);
    }
    step(&mut vm)?;
    assert_eq!(vm.registers.read(Value::System(23))?, 0x282);
    step(&mut vm)?;
    assert_eq!(vm.registers.read(Value::System(42))?, 0xbb);
    step(&mut vm)?;
    assert_eq!(vm.registers.read(Value::System(23))?, 0x283);
    step(&mut vm)?;
    assert_eq!(vm.registers.read(Value::System(23))?, 0x281);
    step(&mut vm)?;
    assert_eq!(vm.registers.read(Value::System(23))?, 0x181);
    step(&mut vm)?;
    assert_eq!(vm.registers.read(Value::System(20))?, 0x240);
    Ok(())
}

#[test]
fn v1_dialog_definition_keeps_image_data_item_order_and_repeated_ids() -> Result<()> {
    let record = b"\x42\0\0\x01-5,2,6,7,45,-22,11,bg,19,10,20,88,99,select,4,5,6,item1,9,8,7,item2\0";
    let mut records = record.to_vec();
    records.extend(b"\x42\0\0\x01-5,0\0");
    let mut host = Host(Arc::new(Assets(BTreeMap::from([("rld/Start.rld".into(), scenario(1, &records, 2))]))));
    let mut vm = vm(1)?;
    load(&mut vm, &mut host)?;
    vm.step(&mut host, InputFrame::default(), 2)?;
    let definitions = serde_json::to_value(&vm.v1_dialogs)?;
    assert_eq!(definitions[0]["id"], -5);
    assert_eq!(definitions[0]["background"]["position"], json!([-22,11]));
    assert_eq!(definitions[0]["selection"]["name"], json!(b"select"));
    assert_eq!(definitions[0]["items"][1], json!({"resource":9,"position":[8,7],"name":b"item2"}));
    assert_eq!(definitions[1]["id"], -5);
    assert_eq!(definitions[1]["items"], json!([]));
    assert!(v1_dialog::Dialog::parse(b"1,4").is_err());
    Ok(())
}

#[test]
fn v1_precache_yields_until_read_or_miss_then_executes_next_record() -> Result<()> {
    for path in ["res\\part.dat", "res\\absent.dat"] {
        let mut records = vec![0x2d, 0, 1, 2, 1, 0, 0, 0];
        records.extend(path.as_bytes());
        records.extend(b"\0ignored second FCString\0");
        records.extend(b"\x06\0\0\x110,9,0,23,*,\0");
        let mut host = Host(Arc::new(Assets(BTreeMap::from([
            ("rld/Start.rld".into(), scenario(1, &records, 2)),
            ("res/part.dat".into(), b"cache me".to_vec()),
        ]))));
        let mut vm = vm(1)?;
        load(&mut vm, &mut host)?;
        vm.step(&mut host, InputFrame::default(), 2)?;
        assert!(vm.preparation_pending());
        assert_eq!(vm.registers.read(Value::Local(9))?, 0);
        let deadline = Instant::now() + Duration::from_secs(2);
        while vm.preparation_pending() {
            vm.step(&mut host, InputFrame::default(), 1)?;
            ensure!(Instant::now() < deadline, "precache worker did not finish");
            std::thread::yield_now();
        }
        assert_eq!(vm.registers.read(Value::Local(9))?, 23);
    }
    Ok(())
}

#[test]
fn v1_entry_load_keeps_record_index_local_values_and_caller_across_reload() -> Result<()> {
    for (primary, slot, default) in [(0, 1, "title"), (1, 0, "exit")] {
        // 表 has a CP932 trail byte equal to the backslash separator.
        let mut start = format!("\x0b\0\0\x0130,-1,2,{slot},").into_bytes();
        start.extend(b"sub\\\x95\x5c\0");
        start.extend([0x16, 0, 16, 5]);
        for n in [19_i32].into_iter().chain(std::iter::repeat_n(-1_i32, 15)) {
            start.extend(n.to_le_bytes());
        }
        start.extend([0; 5]);
        start.extend([0x18, 0, 0, 0, 0x2f, 0, 1, 0, 19, 0, 0, 0]);
        start.extend(format!("\x0b\0\0\x01{primary},99,0,\0").as_bytes());
        let mut next = Vec::new();
        for _ in 0..5 { next.extend([0x18, 0, 0, 0]); }
        next.extend([0x17, 0, 16, 5]);
        for n in [0_i32].into_iter().chain(std::iter::repeat_n(-1_i32, 15)) {
            next.extend(n.to_le_bytes());
        }
        next.extend([0; 5]);
        let fixtures = Assets(BTreeMap::from([
            ("rld/Start.rld".into(), scenario(1, &start, 5)),
            ("rld/sub/表.rld".into(), scenario(1, &next, 6)),
        ]));
        let mut host = Host(Arc::new(fixtures));
        let mut vm = vm(1)?;
        assert_eq!(vm.v1_entries.names()[slot], default.as_bytes());
        load(&mut vm, &mut host)?;
        vm.registers.set_local(16, 71)?;
        vm.step(&mut host, InputFrame::default(), 4)?;
        assert_eq!(vm.name, "sub\\表");
        load(&mut vm, &mut host)?;
        assert_eq!(vm.registers.text(2)?, b"rld\\sub\\\x95\x5c.rld");
        assert_eq!(vm.registers.read(Value::Local(16))?, 71);
        let script = vm.script.as_ref().unwrap();
        assert_eq!((script.remaining, script.returns.len()), (1, 1));
        vm.step(&mut host, InputFrame::default(), 1)?;
        assert_eq!(vm.script.as_ref().unwrap().remaining, 4);
    }
    Ok(())
}

#[test]
fn midori_system_sound_commands_share_control_slots_and_retain_filenames_on_reset() -> Result<()> {
    let mut records = b"\xbe\0\0\x01-1,..0x31.0b101..-1.\0".to_vec();
    // The resident reads the first string and ignores numeric and later strings.
    records.extend(b"\xea\0\x01\x02\x12\0\0\0\x30,11,0x1,\x89\xb9.wav,custom;12,13,\x95\x57;11,again;;13,duplicate;14,last,junk\0unused\0");
    records.extend(b"\xbe\0\0\x01-2,50\0");
    records.extend(b"\xea\0\0\x010002,99,0,another,ignored,7,discarded\0");
    records.extend(b"\xbf\0\0\x01338,-1,0,2,0,0,true,7\0");
    records.extend(b"\xea\0\0\x01100,45,2,no.wav,new\0");
    let fixtures = Assets(BTreeMap::from([(
        "rld/Start.rld".into(),
        scenario(3, &records, 6),
    )]));
    let mut h = Host(Arc::new(fixtures));
    let mut vm = vm(3)?;
    load(&mut vm, &mut h)?;
    vm.step(&mut h, InputFrame::default(), 1)?;
    assert_eq!(
        vm.midori_control_defs.sound_slots(),
        &BTreeMap::from([(0, 49), (1, 5), (2, -1)])
    );
    vm.step(&mut h, InputFrame::default(), 1)?;
    vm.step(&mut h, InputFrame::default(), 1)?;
    assert_eq!(
        vm.midori_control_defs.sound_slots(),
        &BTreeMap::from([(0, 11), (1, 5), (2, -1)])
    );
    let bindings = serde_json::to_value(&vm.midori_control_defs.sounds.details)?;
    assert_eq!(
        bindings["0"]["filename"],
        serde_json::json!([137, 185, 46, 119, 97, 118])
    );
    assert_eq!(
        bindings["0"]["choices"],
        serde_json::json!([
            {"id":11,"label":[149,87,143,128,137,185]},
            {"id":12,"label":[]}, {"id":13,"label":[149,87]}, {"id":14,"label":[108,97,115,116]}
        ])
    );
    vm.step(&mut h, InputFrame::default(), 1)?;
    let bindings = serde_json::to_value(&vm.midori_control_defs.sounds.details)?;
    assert_eq!(bindings["2"]["choices"].as_array().unwrap().len(), 1);
    vm.step(&mut h, InputFrame::default(), 1)?;
    let bindings = serde_json::to_value(&vm.midori_control_defs.sounds.details)?;
    assert_eq!(bindings["2"]["filename"], serde_json::json!([]));
    assert_eq!(bindings["2"]["choices"][0]["id"], 7);
    vm.step(&mut h, InputFrame::default(), 1)?;
    assert_eq!(vm.midori_control_defs.sound_slots().len(), 100);
    assert!(
        vm.midori_control_defs
            .sound_slots()
            .values()
            .all(|id| *id == -1)
    );
    let bindings = serde_json::to_value(&vm.midori_control_defs.sounds.details)?;
    assert_eq!(
        bindings["0"]["filename"],
        serde_json::json!([137, 185, 46, 119, 97, 118])
    );
    assert_eq!(bindings["0"]["choices"], serde_json::json!([]));
    assert_eq!(bindings["99"]["choices"], serde_json::json!([]));
    Ok(())
}
