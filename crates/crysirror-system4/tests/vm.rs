use crysirror_system4::{
    ain::{Function, InitialValue, Program, Structure, Variable},
    vm::{Machine, Step},
};
use std::sync::Arc;
fn v(kind: i32, structure: i32) -> Variable {
    Variable {
        name: vec![],
        kind,
        structure,
        rank: 0,
        group: None,
    }
}
fn f(name: &str, address: u32, args: u32, vars: Vec<Variable>) -> Function {
    Function {
        name: name.as_bytes().to_vec(),
        address,
        is_label: false,
        return_kind: 10,
        return_structure: -1,
        arguments: args,
        variables: vars,
        crc: 0,
    }
}
fn op(code: &mut Vec<u8>, opcode: u16, args: &[i32]) -> u32 {
    let pc = code.len() as u32;
    code.extend(opcode.to_le_bytes());
    for a in args {
        code.extend(a.to_le_bytes());
    }
    pc
}
fn run(m: &mut Machine) -> anyhow::Result<Step> {
    for _ in 0..10000 {
        let s = m.step()?;
        if s != Step::Advanced {
            return Ok(s);
        }
    }
    anyhow::bail!("authored VM failed to converge")
}
#[test]
fn native_call_moves_arguments_and_return_values() {
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    op(&mut p.code, 0, &[8]);
    op(&mut p.code, 0x30, &[1]);
    op(&mut p.code, 0x2f, &[]);
    let callee = p.code.len() as u32;
    op(&mut p.code, 5, &[]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 2, &[]);
    op(&mut p.code, 0, &[3]);
    op(&mut p.code, 9, &[]);
    op(&mut p.code, 0x2f, &[]);
    p.functions = vec![
        f("main", 0, 0, vec![]),
        f("add", callee, 1, vec![v(10, -1)]),
    ];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    assert_eq!(run(&mut m).unwrap(), Step::Halted);
    assert_eq!(m.stack, [11]);
    assert_eq!(m.frames.len(), 0);
    assert_eq!(m.heap.live_slots(), 1);
}
#[test]
fn globals_allocated_before_initializers_and_constructors() {
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    // constructor sees the later global's initial value, writes member0.
    op(&mut p.code, 0x5b, &[]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0x5d, &[1]);
    op(&mut p.code, 0x19, &[]);
    op(&mut p.code, 1, &[]);
    op(&mut p.code, 0x2f, &[]);
    let main = p.code.len() as u32;
    op(&mut p.code, 0x5d, &[0]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 2, &[]);
    op(&mut p.code, 0x2f, &[]);
    p.functions = vec![
        f("unused", 0, 0, vec![]),
        f("ctor", 0, 0, vec![]),
        f("main", main, 0, vec![]),
    ];
    p.main = 2;
    p.globals = vec![v(13, 0), v(10, -1)];
    p.initial_values = vec![InitialValue::Integer {
        global: 1,
        kind: 10,
        value: 73,
    }];
    p.structures = vec![Structure {
        name: vec![],
        constructor: 1,
        destructor: -1,
        members: vec![v(10, -1)],
    }];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    assert_eq!(run(&mut m).unwrap(), Step::Halted);
    assert_eq!(m.stack, [73]);
    assert!(m.entered_main);
}
#[test]
fn external_fence_and_capture_preserve_exact_pc_stack_identity() {
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    op(&mut p.code, 0, &[123]);
    let pc = op(&mut p.code, 0x63, &[22]);
    p.functions = vec![f("main", 0, 0, vec![])];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    let s = run(&mut m).unwrap();
    assert!(matches!(s, Step::External(_)));
    assert_eq!(m.pc, pc);
    assert_eq!(m.stack, [123]);
    let mut restored = m.clone();
    for _ in 0..3 {
        assert_eq!(restored.step().unwrap(), s);
        assert_eq!(restored.pc, pc);
        assert_eq!(restored.stack, [123]);
    }
    assert_eq!(
        restored
            .heap
            .slot(restored.frames[0].local_page)
            .unwrap()
            .sequence,
        m.heap.slot(m.frames[0].local_page).unwrap().sequence
    );
}
#[test]
fn unknown_instruction_fails_without_advance_or_ack() {
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    op(&mut p.code, 0x4a, &[]);
    p.functions = vec![f("main", 0, 0, vec![])];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    let e = run(&mut m).unwrap_err();
    assert!(format!("{e:#}").contains("SF_CREATE"));
    assert_eq!(m.pc, 0);
    assert!(m.stack.is_empty());
    assert_eq!(m.instructions, 0);
}
#[test]
#[ignore = "owner-installed original AIN; does not execute original PE or HLL library"]
fn owner_rance7_bootstrap() {
    let p = Program::parse(&std::fs::read(std::env::var("CRYSIRROR_TEST_AIN").unwrap()).unwrap())
        .unwrap();
    let mut m = Machine::new(Arc::new(p)).unwrap();
    let dll = std::fs::read(std::env::var("CRYSIRROR_TEST_OUTPUTLOG").unwrap()).unwrap();
    let mut bridge = crysirror_system4::adapter::Bridge::default();
    bridge.debug_log =
        Some(crysirror_system4::adapter::AbsentDebugLog::verify_native_library(&dll).unwrap());
    let mut host = ClockHost(0);
    for _ in 0..2_000_000 {
        match m.step() {
            Ok(Step::Advanced) => {}
            Ok(s) => {
                if let Step::External(ref c) = s {
                    match bridge.dispatch(&mut m, c, &mut host) {
                        Ok(_) => {
                            if let crysirror_system4::vm::ExternalTarget::Hll {
                                library,
                                function,
                            } = c.target
                            {
                                let l = &m.program.libraries[library];
                                println!(
                                    "completed reviewed absent-interface branch {}.{} pc={:#x}",
                                    String::from_utf8_lossy(&l.name),
                                    String::from_utf8_lossy(&l.functions[function].name),
                                    c.pc
                                );
                            }
                            continue;
                        }
                        Err(e) => println!("bridge wall: {e:#}"),
                    }
                }
                println!("{} {s:?}", m.location());
                assert_eq!(m.pc, 0xa4a);
                assert_eq!(m.instructions, 1540);
                assert!(!m.entered_main);
                assert_eq!(bridge.absent_debug_calls, 2);
                assert!(matches!(
                    s,
                    Step::External(crysirror_system4::vm::ExternalCall {
                        target: crysirror_system4::vm::ExternalTarget::System(6),
                        ..
                    })
                ));
                let saved_stack = m.stack.clone();
                assert_eq!(m.step().unwrap(), s);
                assert_eq!(m.stack, saved_stack);
                if let Step::External(c) = s {
                    if let crysirror_system4::vm::ExternalTarget::Hll { library, function } =
                        c.target
                    {
                        let l = &m.program.libraries[library];
                        println!(
                            "HLL {}.{} declaration={:?} args_tail={:?}",
                            String::from_utf8_lossy(&l.name),
                            String::from_utf8_lossy(&l.functions[function].name),
                            l.functions[function],
                            m.stack.iter().rev().take(12).collect::<Vec<_>>()
                        );
                    }
                }
                return;
            }
            Err(e) => panic!("{} unexpected VM error={e:#}", m.location()),
        }
    }
    panic!("owner bootstrap instruction budget");
}

#[derive(Default)]
struct ClockHost(u64);
impl crysirror_core::service::FileSystem for ClockHost {
    fn read(
        &mut self,
        _: crysirror_core::state::ClientId,
        _: &crysirror_core::path::VfsPath,
    ) -> crysirror_core::Result<Vec<u8>> {
        Err(crysirror_core::Error::NotFound)
    }
    fn write_atomic(
        &mut self,
        _: crysirror_core::state::ClientId,
        _: &crysirror_core::path::VfsPath,
        _: &[u8],
    ) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
}
impl crysirror_core::service::HostServices for ClockHost {
    fn submit(
        &mut self,
        _: crysirror_core::state::ClientId,
        _: &[crysirror_core::state::Command],
    ) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::Unsupported("test scene"))
    }
    fn now_ms(&self) -> u64 {
        self.0
    }
}
#[test]
fn shared_clock_sleep_retains_operand_until_actual_deadline() {
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    op(&mut p.code, 0, &[25]);
    op(&mut p.code, 0x63, &[21]);
    op(&mut p.code, 0x63, &[13]);
    op(&mut p.code, 0x2f, &[]);
    p.functions = vec![f("main", 0, 0, vec![])];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    let Step::External(c) = run(&mut m).unwrap() else {
        panic!()
    };
    let mut bridge = crysirror_system4::adapter::Bridge::default();
    let mut host = ClockHost(100);
    assert_eq!(
        bridge.dispatch(&mut m, &c, &mut host).unwrap(),
        crysirror_system4::adapter::Dispatch::WaitUntil(125)
    );
    assert_eq!(m.stack, [25]);
    host.0 = 124;
    assert_eq!(
        bridge.dispatch(&mut m, &c, &mut host).unwrap(),
        crysirror_system4::adapter::Dispatch::WaitUntil(125)
    );
    host.0 = 125;
    bridge.dispatch(&mut m, &c, &mut host).unwrap();
    let Step::External(c) = run(&mut m).unwrap() else {
        panic!()
    };
    bridge.dispatch(&mut m, &c, &mut host).unwrap();
    assert_eq!(run(&mut m).unwrap(), Step::Halted);
    assert_eq!(m.stack, [125]);
}
#[test]
fn native_reference_arguments_alias_and_keep_global_page_alive() {
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    op(&mut p.code, 4, &[]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0x30, &[1]);
    op(&mut p.code, 0x5d, &[0]);
    op(&mut p.code, 0x2f, &[]);
    let at = p.code.len() as u32;
    op(&mut p.code, 5, &[]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 3, &[]);
    op(&mut p.code, 0, &[7]);
    op(&mut p.code, 0x19, &[]);
    op(&mut p.code, 1, &[]);
    op(&mut p.code, 0x2f, &[]);
    p.functions = vec![
        f("main", 0, 0, vec![]),
        f("write-ref", at, 2, vec![v(18, -1), v(0, -1)]),
    ];
    p.globals = vec![v(10, -1)];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    assert_eq!(run(&mut m).unwrap(), Step::Halted);
    assert_eq!(m.stack, [7]);
    assert_eq!(m.heap.slot(0).unwrap().references, 1);
    assert_eq!(m.heap.live_slots(), 1);
}
#[test]
fn new_and_delete_execute_real_constructor_destructor_continuations() {
    let mut p = Program {
        version: 5,
        main: 1,
        ..Default::default()
    };
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0x76, &[]);
    op(&mut p.code, 0x77, &[]);
    op(&mut p.code, 0x5d, &[0]);
    op(&mut p.code, 0x2f, &[]);
    let ctor = p.code.len() as u32;
    op(&mut p.code, 4, &[]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0, &[41]);
    op(&mut p.code, 0x19, &[]);
    op(&mut p.code, 1, &[]);
    op(&mut p.code, 0x2f, &[]);
    let dtor = p.code.len() as u32;
    op(&mut p.code, 4, &[]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0x31, &[]);
    op(&mut p.code, 0x2f, &[]);
    p.functions = vec![
        f("unused", 0, 0, vec![]),
        f("main", 0, 0, vec![]),
        f("ctor", ctor, 0, vec![]),
        f("dtor", dtor, 0, vec![]),
    ];
    p.globals = vec![v(10, -1)];
    p.structures = vec![Structure {
        name: vec![],
        constructor: 2,
        destructor: 3,
        members: vec![v(10, -1)],
    }];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    assert_eq!(run(&mut m).unwrap(), Step::Halted);
    assert_eq!(m.stack, [42]);
    assert_eq!(m.heap.live_slots(), 1);
}
#[test]
fn original_zero_function_allocates_multidimensional_page_tree_before_main() {
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    op(&mut p.code, 4, &[]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0, &[2]);
    op(&mut p.code, 0, &[3]);
    op(&mut p.code, 0, &[2]);
    op(&mut p.code, 0x51, &[]);
    op(&mut p.code, 0x2f, &[]);
    let main = p.code.len() as u32;
    op(&mut p.code, 0x5d, &[0]);
    op(&mut p.code, 0, &[1]);
    op(&mut p.code, 2, &[]);
    op(&mut p.code, 0, &[2]);
    op(&mut p.code, 2, &[]);
    op(&mut p.code, 0x2f, &[]);
    p.main = 1;
    p.functions = vec![f("0", 0, 0, vec![]), f("main", main, 0, vec![])];
    p.globals = vec![v(14, -1)];
    p.globals[0].rank = 2;
    let mut m = Machine::new(Arc::new(p)).unwrap();
    assert_eq!(run(&mut m).unwrap(), Step::Halted);
    assert_eq!(m.stack, [0]);
    let parent = m.heap.get(0, 0).unwrap();
    let page = m.heap.page(parent).unwrap();
    assert_eq!(page.values.len(), 2);
    assert_ne!(page.values[0], page.values[1]);
    for child in &page.values {
        assert_eq!(m.heap.page(*child).unwrap().values, [0, 0, 0]);
    }
}
#[test]
fn invalid_debug_dll_is_not_authorized_as_a_stub() {
    assert!(
        crysirror_system4::adapter::AbsentDebugLog::verify_native_library(&[0; 33280]).is_err()
    );
}
#[test]
fn adapter_boot_and_native_execution_use_shared_worker_before_owner_completion() {
    use crysirror_core::{
        Error,
        path::{Mount, VfsPath},
        service::{ClientStorage, FileSystem, HostServices},
        state::{ClientId, Command},
        vm::{VmAdapter, VmYield},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Storage {
        bytes: Vec<u8>,
        owner: std::thread::ThreadId,
        reads: Arc<AtomicUsize>,
    }
    impl ClientStorage for Storage {
        fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            assert_ne!(std::thread::current().id(), self.owner);
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(self.bytes.clone())
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
            Err(Error::Unsupported("test listing"))
        }
    }
    struct Host {
        storage: Arc<dyn ClientStorage>,
        owner: std::thread::ThreadId,
    }
    impl FileSystem for Host {
        fn read(&mut self, _: ClientId, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            Err(Error::Unsupported("owner I/O"))
        }
        fn write_atomic(
            &mut self,
            _: ClientId,
            _: &VfsPath,
            _: &[u8],
        ) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
    }
    impl HostServices for Host {
        fn submit(&mut self, _: ClientId, _: &[Command]) -> crysirror_core::Result<()> {
            Err(Error::Unsupported("test scene"))
        }
        fn now_ms(&self) -> u64 {
            assert_eq!(std::thread::current().id(), self.owner);
            321
        }
        fn storage(&mut self, client: ClientId) -> crysirror_core::Result<Arc<dyn ClientStorage>> {
            assert_eq!(client, ClientId(7));
            Ok(self.storage.clone())
        }
    }
    let mut code = Vec::new();
    op(&mut code, 0x63, &[13]);
    op(&mut code, 0x2f, &[]);
    let mut bytes = b"VERS".to_vec();
    bytes.extend(5u32.to_le_bytes());
    bytes.extend(b"CODE");
    bytes.extend((code.len() as u32).to_le_bytes());
    bytes.extend(code);
    bytes.extend(b"FUNC");
    bytes.extend(1u32.to_le_bytes());
    bytes.extend(0u32.to_le_bytes());
    bytes.extend(b"main\0");
    for n in [0, 10, u32::MAX, 0, 0, 0] {
        bytes.extend(n.to_le_bytes());
    }
    bytes.extend(b"MAIN");
    bytes.extend(0u32.to_le_bytes());
    let owner = std::thread::current().id();
    let reads = Arc::new(AtomicUsize::new(0));
    let storage = Arc::new(Storage {
        bytes,
        owner,
        reads: reads.clone(),
    });
    let mut host = Host { storage, owner };
    let mut adapter = crysirror_system4::worker_adapter::NativeAdapter::new(
        ClientId(7),
        crysirror_system4::worker_adapter::Config {
            canvas: None,
            font_aliases: ["default".into(),"default".into()],
            program: "test.ain".into(),
            reviewed_output_log_dll: None,
        },
    )
    .unwrap();
    let mut halted = false;
    for _ in 0..1000 {
        match adapter.step(&mut host, Default::default(), 32).unwrap() {
            VmYield::Halted => {
                halted = true;
                break;
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
    assert!(halted);
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    let diagnostic = String::from_utf8(adapter.diagnostics()).unwrap();
    assert!(diagnostic.contains("\"clock_calls\":1"));
    assert!(diagnostic.contains("\"error\":null"));
    assert!(adapter.snapshot().is_err());
}
#[test]
fn local_and_copy_heap_identity_is_reserved_before_owned_children() {
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    op(&mut p.code, 0x63, &[6]);
    p.functions = vec![f("main", 0, 0, vec![v(12, -1)])];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    assert!(matches!(run(&mut m).unwrap(), Step::External(_)));
    assert_eq!(m.frames[0].local_page, 1);
    assert_eq!(m.heap.get(1, 0).unwrap(), 2);
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    op(&mut p.code, 0x5d, &[0]);
    op(&mut p.code, 0x98, &[0]);
    op(&mut p.code, 0x63, &[6]);
    p.functions = vec![f("main", 0, 0, vec![])];
    p.globals = vec![v(13, 0)];
    p.structures = vec![
        Structure {
            name: vec![],
            constructor: -1,
            destructor: -1,
            members: vec![v(13, 1)],
        },
        Structure {
            name: vec![],
            constructor: -1,
            destructor: -1,
            members: vec![],
        },
    ];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    assert!(matches!(run(&mut m).unwrap(), Step::External(_)));
    assert_eq!(m.stack, [4]);
    assert_eq!(m.heap.get(4, 0).unwrap(), 5);
}
#[test]
fn struct_array_allocates_and_constructs_each_element_before_next_identity() {
    let mut p = Program {
        version: 5,
        main: 1,
        ..Default::default()
    };
    op(&mut p.code, 4, &[]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0, &[2]);
    op(&mut p.code, 0, &[1]);
    op(&mut p.code, 0x51, &[]);
    op(&mut p.code, 0x2f, &[]);
    let main = p.code.len() as u32;
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0x2f, &[]);
    let ctor = p.code.len() as u32;
    op(&mut p.code, 0x2f, &[]);
    p.functions = vec![
        f("0", 0, 0, vec![]),
        f("main", main, 0, vec![]),
        f("ctor", ctor, 0, vec![v(12, -1)]),
    ];
    p.globals = vec![v(17, 0)];
    p.structures = vec![Structure {
        name: vec![],
        constructor: 2,
        destructor: -1,
        members: vec![],
    }];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    assert_eq!(run(&mut m).unwrap(), Step::Halted);
    let array = m.heap.page(m.heap.get(0, 0).unwrap()).unwrap();
    assert_eq!(array.values, [3, 4]);
    assert_eq!(m.heap.slot(3).unwrap().sequence, 4);
    assert_eq!(m.heap.slot(4).unwrap().sequence, 7);
}
#[test]
fn hll_value_argument_cleanup_releases_without_native_destructors() {
    use crysirror_system4::{
        ain::{HllFunction, Library},
        vm::external::HllReturn,
    };
    let mut p = Program {
        version: 5,
        ..Default::default()
    };
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0x76, &[]);
    op(&mut p.code, 0x5a, &[0, 0]);
    op(&mut p.code, 0x5d, &[0]);
    op(&mut p.code, 0x2f, &[]);
    let dtor = p.code.len() as u32;
    op(&mut p.code, 4, &[]);
    op(&mut p.code, 0, &[0]);
    op(&mut p.code, 0x31, &[]);
    op(&mut p.code, 0x2f, &[]);
    p.functions = vec![f("main", 0, 0, vec![]), f("dtor", dtor, 0, vec![])];
    p.globals = vec![v(10, -1)];
    p.structures = vec![Structure {
        name: vec![],
        constructor: -1,
        destructor: 1,
        members: vec![],
    }];
    p.libraries = vec![Library {
        name: b"Authored".to_vec(),
        functions: vec![HllFunction {
            name: b"Consume".to_vec(),
            return_kind: 0,
            arguments: vec![(b"object".to_vec(), 13)],
        }],
    }];
    let mut m = Machine::new(Arc::new(p)).unwrap();
    let Step::External(call) = run(&mut m).unwrap() else {
        panic!()
    };
    assert!(m.complete_hll(&call, HllReturn::Scalar(1)).is_err());
    assert_eq!(m.step().unwrap(), Step::External(call.clone()));
    m.complete_hll(&call, HllReturn::Void).unwrap();
    assert_eq!(run(&mut m).unwrap(), Step::Halted);
    assert_eq!(m.stack, [0]);
    assert_eq!(m.heap.live_slots(), 1);
}

#[test]
fn native_s_mod_uses_shared_byte_format_and_exact_string_argument_release() {
    for (template,arg_kind,arg_string,expected,live_slots) in [
        (b"mixer%03d".as_slice(),2,None,b"mixer007".as_slice(),2),
        (b"[%4s]".as_slice(),4,Some(b"ab".as_slice()),b"[  ab]".as_slice(),2),
        (b"unchanged%d".as_slice(),4,Some(b"unused".as_slice()),b"unchanged%d".as_slice(),3),
    ] {
        let mut p=Program{version:5,..Default::default()};
        p.strings.push(template.to_vec());op(&mut p.code,0x41,&[0]);
        if let Some(value)=arg_string {p.strings.push(value.to_vec());op(&mut p.code,0x41,&[1]);}
        else {op(&mut p.code,0,&[7]);}
        op(&mut p.code,0,&[arg_kind]);op(&mut p.code,0x94,&[]);op(&mut p.code,0x2f,&[]);
        p.functions=vec![f("main",0,0,vec![])];
        let mut m=Machine::new(Arc::new(p)).unwrap();assert_eq!(run(&mut m).unwrap(),Step::Halted);
        assert_eq!(m.heap.string(m.stack[0]).unwrap().as_slice(),expected);
        assert_eq!(m.heap.live_slots(),live_slots,"matched %s releases; unmatched string argument remains as in original native source");
    }
}
#[test]
fn unsupported_native_s_mod_does_not_consume_stack_heap_or_pc() {
    let mut p=Program{version:5,..Default::default()};p.strings.push(b"%D".to_vec());
    op(&mut p.code,0x41,&[0]);op(&mut p.code,0,&[7]);op(&mut p.code,0,&[2]);
    let pc=op(&mut p.code,0x94,&[]);op(&mut p.code,0x2f,&[]);
    p.functions=vec![f("main",0,0,vec![])];let mut m=Machine::new(Arc::new(p)).unwrap();
    for _ in 0..100 {if m.pc==pc {break;}m.step().unwrap();}
    assert_eq!(m.pc,pc);let stack=m.stack.clone();let live=m.heap.live_slots();let count=m.instructions;
    assert!(format!("{:#}",m.step().unwrap_err()).contains("unsupported byte format conversion"));
    assert_eq!(m.pc,pc);assert_eq!(m.stack,stack);assert_eq!(m.heap.live_slots(),live);assert_eq!(m.instructions,count);
}
