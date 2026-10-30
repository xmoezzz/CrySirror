use crysirror_catsystem2::kcs::{Event, Machine, Program};
use std::io::Write;
fn operand(code: &mut Vec<u8>, flags: u8, value: u32) {
    code.push(flags);
    code.extend(value.to_le_bytes());
}
fn binary(code: &mut Vec<u8>, op: u8, a: (u8, u32), b: (u8, u32)) {
    code.push(op);
    operand(code, a.0, a.1);
    operand(code, b.0, b.1);
}
fn program(code: Vec<u8>) -> Program {
    Program {
        code,
        globals_bytes: 64,
        stack_bytes: 1024,
        strings: Vec::new(),
    }
}
fn file(code: &[u8]) -> Vec<u8> {
    let mut h = vec![0u32; 13];
    h[0] = u32::from_le_bytes(*b"KCS\0");
    h[1] = 2;
    h[3] = 52;
    h[4] = code.len() as u32;
    h[5] = 52 + h[4];
    h[8] = 64;
    h[9] = 1024;
    h[10] = h[5];
    h[12] = code.len() as u32;
    let mut out = h.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(code).unwrap();
    out.extend(z.finish().unwrap());
    out
}
#[test]
fn native_call_is_pending_until_owner_acknowledges() {
    let mut code = Vec::new();
    code.push(0x25);
    operand(&mut code, 3, 123);
    binary(&mut code, 0x2b, (3, 17), (3, 4));
    code.push(0x3b);
    let p = Program::parse(&file(&code)).unwrap();
    let mut m = Machine::new(&p, 4096).unwrap();
    let Event::Native(call) = m.step(&p, 20).unwrap() else {
        panic!()
    };
    assert_eq!(call.id, 17);
    assert_eq!(call.arguments, [123]);
    assert_eq!(call.pc, 6);
    assert_eq!(m.pc, 6);
    assert_eq!(m.step(&p, 20).unwrap(), Event::Native(call));
    m.acknowledge(Some(456)).unwrap();
    assert_eq!(m.read_word(64).unwrap(), 456);
    assert_eq!(m.step(&p, 20).unwrap(), Event::Halted);
}
#[test]
fn native_call_frame_and_return_preserve_original_stack_offsets() {
    let mut code = Vec::new();
    binary(&mut code, 0x2a, (3, 12), (3, 0));
    code.push(0x3b);
    code.push(0x31);
    operand(&mut code, 3, 8);
    code.push(0x25);
    operand(&mut code, 3, 42);
    binary(&mut code, 0x2c, (3, 4), (3, 0));
    let p = program(code);
    let mut m = Machine::new(&p, 64).unwrap();
    assert_eq!(m.step(&p, 20).unwrap(), Event::Halted);
    assert_eq!(m.read_word(64).unwrap(), 42);
    assert_eq!(m.stack, 68);
    assert_eq!(m.frame_base, 64);
    assert_eq!(m.frame_end, 64);
}
#[test]
fn signed_operands_and_byte_stores_use_original_width() {
    let mut code = Vec::new();
    binary(&mut code, 0x15, (1, 0), (3, 0x1234));
    code.push(0x3c);
    binary(&mut code, 0x10, (0x21, 0xffff_ffff), (3, 1));
    code.push(0x3b);
    let p = program(code);
    let mut m = Machine::new(&p, 64).unwrap();
    assert_eq!(m.step(&p, 20).unwrap(), Event::Halted);
    assert_eq!(m.memory[0], 0x34);
    assert_eq!(m.read_word(64).unwrap(), 1);
}
#[test]
fn rejected_instruction_restores_pc_stack_and_written_bytes() {
    let p = program(vec![0x2a, 3, 255, 255, 255, 255, 3, 0, 0, 0, 0]);
    let mut m = Machine::new(&p, 64).unwrap();
    let before = m.memory.clone();
    assert!(m.step(&p, 1).is_err());
    assert_eq!(m.pc, 0);
    assert_eq!(m.stack, 64);
    assert_eq!(m.memory, before);
    let p = program(vec![0xff]);
    assert!(m.step(&p, 1).unwrap_err().to_string().contains("0xff"));
    assert_eq!(m.pc, 0);
}
#[test]
fn malformed_containers_are_rejected_before_allocation() {
    let good = file(&[0x3b]);
    assert!(Program::parse(&good).is_ok());
    for n in [0, 1, 4, 20, 51, good.len() - 1] {
        assert!(Program::parse(&good[..n]).is_err());
    }
    let mut bad = good.clone();
    bad[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Program::parse(&bad).is_err());
    let mut trailing = good;
    trailing.push(0);
    assert!(Program::parse(&trailing).is_err());
}

#[test]
fn manually_constructed_program_and_aliased_strings_stay_bounded() {
    let p = Program {
        code: vec![0x3b],
        globals_bytes: u32::MAX,
        stack_bytes: 64,
        strings: Vec::new(),
    };
    assert!(Machine::new(&p, 64).is_err());
    let mut table = Vec::new();
    for _ in 0..3 {
        table.extend(24u32.to_le_bytes());
        table.extend(0u32.to_le_bytes());
    }
    table.extend(vec![b'X'; 6 * 1024 * 1024]);
    table.push(0);
    let mut body = vec![0x3b];
    body.extend(&table);
    let h = [
        u32::from_le_bytes(*b"KCS\0"),
        2,
        0,
        52,
        1,
        53,
        3,
        table.len() as u32,
        7 * 1024 * 1024,
        64,
        52 + body.len() as u32,
        0,
        body.len() as u32,
    ];
    let mut bytes = h.into_iter().flat_map(u32::to_le_bytes).collect::<Vec<_>>();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&body).unwrap();
    bytes.extend(z.finish().unwrap());
    assert!(Program::parse(&bytes)
        .unwrap_err()
        .to_string()
        .contains("cumulative"));
}

#[test]
fn native_division_and_modulo_zero_are_defined_zero_results() {
    let mut code = Vec::new();
    binary(&mut code, 4, (3, 42), (3, 0));
    binary(&mut code, 5, (3, 42), (3, 0));
    code.push(0x3b);
    let p = program(code);
    let mut m = Machine::new(&p, 64).unwrap();
    assert_eq!(m.step(&p, 10).unwrap(), Event::Halted);
    assert_eq!(m.read_word(64).unwrap(), 0);
    assert_eq!(m.read_word(68).unwrap(), 0);
}
