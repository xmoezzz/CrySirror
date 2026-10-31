use crysirror_majiro::{
    machine::{Event, Machine, Value},
    program::{name_hash, xor_bytecode, Program},
};
use std::sync::Arc;
fn short(b: &mut Vec<u8>, n: u16) {
    b.extend(n.to_le_bytes());
}
fn word(b: &mut Vec<u8>, n: u32) {
    b.extend(n.to_le_bytes());
}
fn module(code: &[u8], encrypted: bool) -> Vec<u8> {
    let mut b = if encrypted {
        b"MajiroObjX1.000\0".to_vec()
    } else {
        b"MajiroObjV1.000\0".to_vec()
    };
    for n in [0, 0, 1, 0x12345678, 0, code.len() as u32] {
        word(&mut b, n);
    }
    let mut code = code.to_vec();
    if encrypted {
        xor_bytecode(&mut code);
    }
    b.extend(code);
    b
}
#[test]
fn original_pending_operation_is_unchanged_until_valid_host_receipt() {
    let mut code = vec![];
    short(&mut code, 0x800);
    word(&mut code, 123);
    short(&mut code, 0x834);
    word(&mut code, 0x11223344);
    short(&mut code, 1);
    short(&mut code, 0x82b);
    let p = Program::parse(&module(&code, true)).unwrap();
    let mut m = Machine::new(Arc::new(p));
    m.step().unwrap();
    let event = m.step().unwrap().unwrap();
    assert_eq!(
        event,
        Event::Native {
            pc: 6,
            hash: 0x11223344,
            arguments: vec![Value::Integer(123)],
            returns: true
        }
    );
    assert_eq!(m.pc, 6);
    assert_eq!(m.stack, vec![Value::Integer(123)]);
    assert_eq!(m.step().unwrap(), Some(event.clone()));
    assert!(m.acknowledge(vec![]).is_err());
    assert_eq!(m.pc, 6);
    assert_eq!(m.pending(), Some(&event));
    m.acknowledge(vec![Value::Integer(42)]).unwrap();
    assert_eq!(m.pc, 14);
    assert_eq!(m.stack, vec![Value::Integer(42)]);
    m.step().unwrap();
    assert_eq!(m.step().unwrap(), Some(Event::Halted));
}
#[test]
fn boundary_validation_and_original_hashes_are_not_per_game_keys() {
    assert_eq!(name_hash(b"$abs@MAJIRO_INTER"), 0x959b0a16);
    let mut code = vec![];
    short(&mut code, 0x800);
    word(&mut code, 1);
    short(&mut code, 0x82b);
    let plain = module(&code, false);
    let x = module(&code, true);
    assert_eq!(
        Program::parse(&plain).unwrap().instructions,
        Program::parse(&x).unwrap().instructions
    );
    for n in 0..plain.len() {
        assert!(Program::parse(&plain[..n]).is_err());
    }
    let mut branch = vec![];
    short(&mut branch, 0x82c);
    word(&mut branch, (-5i32) as u32);
    assert!(Program::parse(&module(&branch, false)).is_err());
    let mut bad_string = vec![];
    short(&mut bad_string, 0x801);
    short(&mut bad_string, 0);
    assert!(Program::parse(&module(&bad_string, false)).is_err());
}
#[test]
fn unsupported_and_arithmetic_faults_do_not_consume_operands() {
    let mut code = vec![];
    short(&mut code, 0x800);
    word(&mut code, 5);
    short(&mut code, 0x800);
    word(&mut code, 0);
    short(&mut code, 0x108);
    short(&mut code, 0x82b);
    let mut m = Machine::new(Arc::new(Program::parse(&module(&code, false)).unwrap()));
    m.step().unwrap();
    m.step().unwrap();
    let before = m.stack.clone();
    assert!(m.step().is_err());
    assert_eq!(m.pc, 12);
    assert_eq!(m.stack, before);
}
#[test]
fn native_arguments_follow_original_top_first_parameter_abi() {
    let mut code = vec![];
    for n in [600, 800] {
        short(&mut code, 0x800);
        word(&mut code, n);
    }
    short(&mut code, 0x835);
    word(&mut code, 0x83d81f59);
    short(&mut code, 2);
    short(&mut code, 0x82b);
    let mut m = Machine::new(Arc::new(Program::parse(&module(&code, false)).unwrap()));
    m.step().unwrap();
    m.step().unwrap();
    assert_eq!(
        m.step().unwrap(),
        Some(Event::Native {
            pc: 12,
            hash: 0x83d81f59,
            arguments: vec![Value::Integer(800), Value::Integer(600)],
            returns: false
        })
    );
}
#[test]
fn original_env_tokens_preserve_paths_urls_and_inline_comments() {
    let e=crysirror_majiro::environment::parse(b"// header\r\nCLIENT_WIDTH 800 // width\nCLIENT_HEIGHT 600\nURL http://example.test/\nTITLEBAR A|B\n[EOF]\nCLIENT_WIDTH 1").unwrap();
    assert_eq!(e["CLIENT_WIDTH"], "800");
    assert_eq!(e["URL"], "http://example.test/");
    assert_eq!(e["TITLEBAR"], "A B");
    assert!(crysirror_majiro::environment::parse(b"CLIENT_WIDTH 800\nCLIENT_WIDTH 1").is_err());
}
