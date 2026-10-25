// SPDX-License-Identifier: MPL-2.0
//! Self-authored native Seen fixtures. No game, font, engine host or devices.
use super::*;
use bytecode::{Opcode, asm};
use expr::{encode, op};
use memory::IntRef;
use std::{collections::BTreeMap, rc::Rc};
fn command(t: u8, m: u8, o: u16, args: &[Vec<u8>]) -> Vec<u8> {
    asm::command(Opcode::new(t, m, o, 0), args.len() as u16, args)
}
fn make(scenes: Vec<(i32, Vec<u8>)>) -> Machine {
    let packed = archive::build(
        &scenes
            .into_iter()
            .map(|(n, code)| (n, scenario::build(&code, &[1_000_000], &[])))
            .collect::<Vec<_>>(),
    );
    let archive = Rc::new(Archive::from_bytes(packed, "synthetic", Nls::Sjis).unwrap());
    Machine::new(
        archive,
        Rc::new(Gameexe::parse("#SEEN_START=1\n#A[0]=5\n#G[0]=12\n")),
    )
    .unwrap()
}
#[test]
fn pending_shared_operation_cannot_advance_or_execute_twice_before_ack() {
    let mut code = asm::line(7);
    code.extend(command(1, 4, 0, &[encode::quoted(b"native title")]));
    code.extend(encode::binary(
        &encode::int_a(0),
        op::ASSIGN,
        &encode::int(99),
    ));
    let mut vm = make(vec![(1, code)]);
    assert!(matches!(
        vm.run_budget(0).unwrap(),
        RunResult::BudgetExhausted
    ));
    assert_eq!(vm.instructions, 0);
    let RunResult::Event(VmEvent::Command(c)) = vm.run_budget(20).unwrap() else {
        panic!("native syscall")
    };
    assert_eq!(c.op, Opcode::new(1, 4, 0, 0));
    assert_eq!(vm.str_param(&c, 0).unwrap(), "native title");
    let ip = vm.frame().ip;
    let instructions = vm.instructions;
    assert_eq!(vm.read_int(IntRef::named("A", 0).unwrap()).unwrap(), 5);
    for _ in 0..3 {
        assert!(matches!(vm.run_budget(20).unwrap(), RunResult::Event(_)));
        assert_eq!((vm.frame().ip, vm.instructions), (ip, instructions));
    }
    vm.ack(Next::Advance).unwrap();
    assert!(matches!(vm.run_budget(20).unwrap(), RunResult::Halted));
    assert_eq!(vm.read_int(IntRef::named("A", 0).unwrap()).unwrap(), 99);
    assert!(vm.ack(Next::Advance).is_err());
}
#[test]
fn original_farcall_banks_and_return_resume_caller() {
    let mut first = encode::binary(&encode::int_a(0), op::ASSIGN, &encode::int(1));
    first.extend(command(0, 1, 12, &[encode::int(2), encode::int(0)]));
    first.extend(command(1, 4, 0, &[encode::quoted(b"after return")]));
    let mut second = asm::marker(0);
    second.extend(encode::binary(
        &encode::int_a(0),
        op::ASSIGN,
        &encode::int(42),
    ));
    second.extend(command(0, 1, 13, &[]));
    let mut vm = make(vec![(1, first), (2, second)]);
    assert!(matches!(vm.run_budget(100).unwrap(), RunResult::Event(_)));
    assert_eq!(vm.scene_number(), 1);
    assert_eq!(vm.stack.len(), 1);
    assert_eq!(vm.read_int(IntRef::named("A", 0).unwrap()).unwrap(), 42);
}
#[test]
fn original_expression_budget_and_packed_bank_semantics_are_retained() {
    let bit = encode::mem(IntRef::named("Ab", 3).unwrap().to_bytecode(), 3);
    let mut code = encode::binary(&bit, op::ASSIGN, &encode::int(1));
    code.extend(encode::binary(
        &encode::int_a(1),
        op::ASSIGN,
        &encode::int(13),
    ));
    code.extend(command(1, 4, 0, &[]));
    let mut vm = make(vec![(1, code)]);
    assert!(matches!(
        vm.run_budget(1).unwrap(),
        RunResult::BudgetExhausted
    ));
    assert_eq!(vm.instructions, 1);
    assert_eq!(vm.read_int(IntRef::named("A", 0).unwrap()).unwrap(), 13); // initial word 5 OR bit 3
    assert_eq!(vm.read_int(IntRef::named("A", 1).unwrap()).unwrap(), 0);
    assert!(matches!(
        vm.run_budget(1).unwrap(),
        RunResult::BudgetExhausted
    ));
    assert_eq!(vm.read_int(IntRef::named("A", 1).unwrap()).unwrap(), 13);
    assert!(matches!(vm.run_budget(1).unwrap(), RunResult::Event(_)));
    assert!(vm.run_budget(10_000_001).is_err());
}
#[test]
fn text_is_nls_decoded_and_requires_engine_ack() {
    let text = "「日本語」";
    let code = Nls::Sjis.encode(text);
    let mut vm = make(vec![(1, code)]);
    let RunResult::Event(VmEvent::Text(value)) = vm.run_budget(10).unwrap() else {
        panic!("native text")
    };
    assert_eq!(value, text);
    assert_eq!(vm.frame().ip, 0);
    vm.ack(Next::Advance).unwrap();
    assert!(matches!(vm.run_budget(1).unwrap(), RunResult::Halted));
}
#[test]
fn loose_scenes_override_native_archive_and_bad_layouts_fail() {
    let packed = archive::build(&[(
        1,
        scenario::build(&command(1, 4, 0, &[encode::quoted(b"packed")]), &[], &[]),
    )]);
    let loose = BTreeMap::from([(
        1,
        scenario::build(&command(1, 4, 0, &[encode::quoted(b"loose")]), &[], &[]),
    )]);
    let archive = Rc::new(
        Archive::from_bytes(packed.clone(), "x", Nls::Sjis)
            .unwrap()
            .with_loose(loose.clone())
            .unwrap(),
    );
    let mut vm = Machine::new(archive, Rc::new(Gameexe::parse("#SEEN_START=1"))).unwrap();
    let RunResult::Event(VmEvent::Command(c)) = vm.run_budget(10).unwrap() else {
        panic!()
    };
    assert_eq!(vm.str_param(&c, 0).unwrap(), "loose");
    assert!(Archive::from_parts(Vec::new(), loose, "x", Nls::Sjis).is_ok());
    assert!(Archive::from_bytes(vec![0; 12], "x", Nls::Sjis).is_err());
    let mut bad = packed;
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Archive::from_bytes(bad, "x", Nls::Sjis).is_err());
    assert!(compression::decompress(&[], 64 * 1024 * 1024 + 1).is_err());
}
#[test]
fn native_savepoint_snapshots_keep_original_bank_and_stack_semantics() {
    let mut vm = make(vec![(1, command(1, 4, 0, &[]))]);
    let a = IntRef::named("A", 0).unwrap();
    let g = IntRef::named("G", 0).unwrap();
    vm.write_int(a, 25).unwrap();
    vm.write_int(g, 40).unwrap();
    let serial = vm.savepoint_serial;
    vm.mark_savepoint();
    vm.write_int(a, 60).unwrap();
    vm.write_int(g, 70).unwrap();
    assert_eq!(vm.memory.savepoint_int(a), Some(25));
    assert_eq!(vm.read_int(a).unwrap(), 60);
    assert_eq!(vm.read_int(g).unwrap(), 70);
    assert_eq!(vm.savepoint_serial, serial + 1);
    assert_eq!(vm.savepoint_stack.len(), vm.stack.len());
}

#[test]
fn original_gosub_pointer_and_ret_preserve_native_continuation() {
    let mut code = command(0, 1, 5, &[]);
    let patch = code.len();
    code.extend(0u32.to_le_bytes());
    code.extend(command(1, 4, 0, &[]));
    let target = code.len() as u32;
    code.extend(encode::binary(
        &encode::int_a(2),
        op::ASSIGN,
        &encode::int(81),
    ));
    code.extend(command(0, 1, 10, &[]));
    code[patch..patch + 4].copy_from_slice(&target.to_le_bytes());
    let mut vm = make(vec![(1, code)]);
    assert!(matches!(vm.run_budget(100).unwrap(), RunResult::Event(_)));
    assert_eq!(vm.stack.len(), 1);
    assert_eq!(vm.read_int(IntRef::named("A", 2).unwrap()).unwrap(), 81);
}
#[test]
fn unsupported_vm_builtins_fail_at_the_original_pc() {
    let mut vm = make(vec![(1, command(1, 10, 999, &[]))]);
    let ip = vm.frame().ip;
    assert!(vm.run_budget(10).is_err());
    assert!(vm.halted);
    assert_eq!(vm.frame().ip, ip);
    assert_eq!(vm.diagnostics.errors.len(), 1);
    assert_eq!(vm.diagnostics.unimplemented.len(), 1);
}

#[test]
fn a_native_refresh_yield_survives_event_round_trips_until_next_host_frame() {
    let mut code = command(1, 31, 0, &[]);
    code.extend(command(1, 4, 0, &[]));
    let mut vm = make(vec![(1, code)]);
    assert!(matches!(vm.run_budget(10).unwrap(), RunResult::Event(_)));
    vm.yield_frame = true;
    vm.ack(Next::Advance).unwrap();
    let count = vm.instructions;
    assert!(matches!(
        vm.run_budget(10).unwrap(),
        RunResult::BudgetExhausted
    ));
    assert_eq!(vm.instructions, count);
    vm.begin_frame();
    assert!(matches!(vm.run_budget(10).unwrap(), RunResult::Event(_)));
    assert_eq!(vm.instructions, count + 1);
}

#[test]
fn malformed_native_table_lengths_and_signed_offsets_are_errors() {
    let bytes = scenario::build(&command(1, 4, 0, &[]), &[], &[]);
    for at in [0x08usize, 0x0c, 0x14, 0x18, 0x1c, 0x20, 0x24, 0x28] {
        let mut bad = bytes.clone();
        bad[at..at + 4].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(
            scenario::Scenario::parse(1, &bad, None, Nls::Sjis).is_err(),
            "offset {at:#x}"
        );
    }
}
