use ethornell_script::{BpInstruction, BpOpcode};
use ethornell_vm::{NativeCallFrame, SysApi, Value, Vm, VmResult};
use std::path::PathBuf;

struct OptInApi;
impl SysApi for OptInApi {
    fn allow_direct_filesystem_fallback(&self) -> bool {
        true
    }
    fn call_sys(&mut self, _: &mut NativeCallFrame) -> VmResult<Value> {
        Ok(Value::None)
    }
}
impl ethornell_vm::GraphApi for OptInApi {
    fn call_graph(&mut self, _: &mut NativeCallFrame) -> VmResult<Value> {
        Ok(Value::None)
    }
}
impl ethornell_vm::SoundApi for OptInApi {
    fn call_sound(&mut self, _: &mut NativeCallFrame) -> VmResult<Value> {
        Ok(Value::None)
    }
}
fn instruction(group: u8, id: u8) -> BpInstruction {
    BpInstruction {
        offset: 0,
        opcode: BpOpcode::Known {
            code: group,
            name: if group == 0x81 { "sys2" } else { "sys1" },
        },
        operands: Vec::new().into(),
        known_call: None,
        raw: vec![group, id].into(),
        warning: None,
    }
}
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn embedded_profile_blocks_every_system81_ambient_file_fallback_even_if_api_opts_in() {
    assert!(cfg!(feature = "cpu-embedded"));
    assert!(!ethornell_vm::host_filesystem_fallback_allowed(&OptInApi));
    let path = std::env::temp_dir().join(format!(
        "crysirror-bgi-native-boundary-{}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    let fixture = Fixture(path);
    let sentinel = fixture.0.join("self-authored-outside-capability.txt");
    std::fs::write(&sentinel, b"self-authored denied native resource").unwrap();
    let file = || Value::Str(sentinel.to_string_lossy().into_owned());
    let ptr = || Value::Ptr(0x1000);
    let cases = vec![
        (0x28, vec![ptr(), file(), Value::Int(0)], 3),
        (
            0x2c,
            vec![Value::Ptr(0), Value::Ptr(0), Value::Ptr(0), file()],
            0,
        ),
        (
            0x2d,
            vec![file(), Value::Ptr(0), Value::Ptr(0), Value::Ptr(0)],
            0,
        ),
        (
            0x2f,
            vec![Value::Str(fixture.0.to_string_lossy().into_owned())],
            0,
        ),
        (
            0x32,
            vec![Value::Int(256), ptr(), file(), Value::Ptr(0x2000)],
            1,
        ),
        (0x35, vec![Value::Ptr(0), file()], 0),
        (0x37, vec![ptr(), file()], 0),
        (0x3c, vec![file()], 0),
        (0x3e, vec![ptr(), file()], 0),
    ];
    for (id, args, status) in cases {
        let mut vm = Vm::new();
        vm.stack = args;
        vm.dispatch(&instruction(0x81, id), &mut OptInApi).unwrap();
        assert_eq!(vm.stack.last(), Some(&Value::Int(status)), "Sys81:{id:02x}");
        assert!(
            vm.memory[0x1000..0x1010].iter().all(|byte| *byte == 0),
            "ambient bytes reached guest memory at Sys81:{id:02x}"
        );
    }
    assert_eq!(
        std::fs::read_dir(&fixture.0).unwrap().count(),
        1,
        "native writable probe touched ambient directory"
    );
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"self-authored denied native resource"
    );
}

#[test]
fn physical_memory_comes_from_actual_kernel_metadata() {
    let (total, available) = ethornell_vm::embedding_metadata::physical_memory_bytes().unwrap();
    assert!(total > 0);
    assert!(available <= total);
    let mut api = OptInApi;
    assert_eq!(api.host_physical_memory_bytes().0, total);
    eprintln!("actual kernel memory: total={total} available={available}");
}

#[test]
fn native_calendar_uses_real_local_wall_time_not_a_zero_record() {
    let mut vm = Vm::new();
    vm.stack.push(Value::Ptr(0x1000));
    vm.dispatch(&instruction(0x80, 0x0c), &mut OptInApi)
        .unwrap();
    let field = |index: usize| {
        u16::from_le_bytes(
            vm.memory[0x1000 + index * 2..0x1002 + index * 2]
                .try_into()
                .unwrap(),
        )
    };
    assert!(field(0) >= 2020);
    assert!((1..=12).contains(&field(1)));
    assert!((1..=31).contains(&field(3)));
    assert!(field(4) < 24 && field(5) < 60 && field(6) < 61);
}
