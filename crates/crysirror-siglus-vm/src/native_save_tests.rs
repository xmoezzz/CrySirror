// SPDX-License-Identifier: MPL-2.0
use crate::{
    adapter::{Config, SiglusVm},
    fixture::{SceneBuilder, pack_scene},
    runtime::{
        constants::{cd, fm},
        forms::codes,
    },
};
use crysirror_core::{
    Error,
    path::VfsPath,
    service::{FileSystem, HostServices, InputFrame},
    state::ClientId,
    vm::{VmAdapter, VmYield},
};
struct Host {
    scene: Vec<u8>,
    clock: u64,
    engine: crysirror_scene::EngineSystems,
}
impl FileSystem for Host {
    fn read(&mut self, _: ClientId, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        Ok(self.scene.clone())
    }
    fn write_atomic(&mut self, _: ClientId, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
        Err(Error::ReadOnly)
    }
}
impl HostServices for Host {
    fn now_ms(&self) -> u64 {
        self.clock
    }
    fn submit(
        &mut self,
        _: ClientId,
        _: &[crysirror_core::state::Command],
    ) -> crysirror_core::Result<()> {
        Ok(())
    }
    fn submit_engine(&mut self, c: ClientId, s: u32, b: &[u8]) -> crysirror_core::Result<()> {
        assert_eq!(s, 1);
        let data = serde_json::from_slice(b).unwrap();
        self.engine
            .submit(c, &data)
            .map_err(|e| Error::Io(e.to_string()))
    }
}
#[test]
fn original_savepoint_exports_native_codec_and_import_resumes_with_loaded_flag() {
    let mut b = SceneBuilder::default();
    b.command(
        &[codes::ELM_GLOBAL_FRONT, codes::ELM_STAGE_CREATE_OBJECT],
        0,
        &[1],
        fm::VOID,
    );
    b.command(
        &[
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
            codes::ELM_OBJECT_CREATE_RECT,
        ],
        2,
        &[0, 0, 10, 10, 0, 255, 0, 255, 1, 28, 20],
        fm::VOID,
    );
    b.command(&[codes::ELM_GLOBAL_SAVEPOINT], 0, &[], fm::INT);
    b.code.push(cd::GOTO_TRUE);
    b.i32(0);
    b.command(&[codes::ELM_GLOBAL_TIMEWAIT], 0, &[100], fm::VOID);
    b.code.push(cd::EOF);
    b.labels.push(b.code.len() as i32);
    b.command(
        &[
            codes::ELM_GLOBAL_FRONT,
            codes::ELM_STAGE_OBJECT,
            codes::ELM_ARRAY,
            0,
            codes::ELM_OBJECT_X,
        ],
        1,
        &[240],
        fm::VOID,
    );
    b.code.push(cd::EOF);
    let mut host = Host {
        scene: pack_scene(&b.finish()),
        clock: 0,
        engine: Default::default(),
    };
    let config: Config =
        serde_json::from_value(serde_json::json!({"scene":"Scene.pck","string_codec":"plain"}))
            .unwrap();
    let mut vm = SiglusVm::new(ClientId(1), config).unwrap();
    assert_eq!(
        vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
        VmYield::WaitUntil(100)
    );
    let native = vm.export_original_save().unwrap();
    crate::original_save::OriginalSaveHeader::from_bytes(&native).unwrap();
    let before = vm.snapshot().unwrap();
    assert!(vm.import_original_save(b"bad original file").is_err());
    assert!(
        before == vm.snapshot().unwrap(),
        "failed native import must be transactional"
    );
    vm.import_original_save(&native).unwrap();
    assert_eq!(
        vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
        VmYield::Halted
    );
    assert_eq!(host.engine.frame([320, 180]).unwrap()[0].quad[0][0], 240.);
}

#[test]
fn native_timewait_key_requires_decide_release_and_does_not_reuse_edge() {
    let mut b = SceneBuilder::default();
    for _ in 0..2 {
        b.command(&[codes::ELM_GLOBAL_TIMEWAIT_KEY], 0, &[100], fm::INT);
    }
    b.code.push(cd::EOF);
    let mut host = Host {
        scene: pack_scene(&b.finish()),
        clock: 0,
        engine: Default::default(),
    };
    let config: Config =
        serde_json::from_value(serde_json::json!({"scene":"Scene.pck","string_codec":"plain"}))
            .unwrap();
    let mut vm = SiglusVm::new(ClientId(1), config).unwrap();
    assert_eq!(
        vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
        VmYield::WaitUntil(100)
    );
    host.clock = 10;
    assert_eq!(
        vm.step(
            &mut host,
            InputFrame {
                buttons_down: 1,
                buttons_pressed: 1,
                ..Default::default()
            },
            1024
        )
        .unwrap(),
        VmYield::WaitUntil(100)
    );
    host.clock = 20;
    assert_eq!(
        vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
        VmYield::WaitUntil(120)
    );
    assert_eq!(vm.diagnostics()["int_stack"], serde_json::json!([1]));
    host.clock = 30;
    assert_eq!(
        vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
        VmYield::WaitUntil(120)
    );
    host.clock = 120;
    assert_eq!(
        vm.step(&mut host, InputFrame::default(), 1024).unwrap(),
        VmYield::Halted
    );
    assert_eq!(vm.diagnostics()["int_stack"], serde_json::json!([1, 0]));
}
