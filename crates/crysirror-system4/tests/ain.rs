use crysirror_system4::ain::{decode, Program};
use std::io::Write;
fn word(out: &mut Vec<u8>, n: u32) {
    out.extend(n.to_le_bytes());
}
fn minimal() -> Vec<u8> {
    let mut b = b"VERS".to_vec();
    word(&mut b, 5);
    b.extend(b"CODE");
    word(&mut b, 2);
    b.extend([0x2f, 0]);
    b.extend(b"FUNC");
    word(&mut b, 1);
    word(&mut b, 0);
    b.extend(b"main\0");
    for n in [0, 0, u32::MAX, 0, 0, 0] {
        word(&mut b, n);
    }
    b.extend(b"MAIN");
    word(&mut b, 0);
    b
}
#[test]
fn program_preserves_native_strings_addresses_and_envelope_profiles() {
    let plain = minimal();
    let p = Program::parse(&plain).unwrap();
    assert_eq!(p.version, 5);
    assert_eq!(p.functions[0].name, b"main");
    assert_eq!(p.code, [0x2f, 0]);
    assert_eq!(p.validate_code_layout().unwrap(), 1);
    assert_eq!(crysirror_system4::opcodes::layout(0x62), Some(("EOF", 1)));
    assert_eq!(crysirror_system4::opcodes::layout(0x59), Some(("MSG", 1)));
    assert_eq!(p.functions[0].return_structure, -1);
    let mut encrypted = plain.clone();
    crysirror_resource::legacy_stream::mt19937_1998_xor(&mut encrypted, 0x5d3e3);
    assert_eq!(decode(&encrypted).unwrap(), plain);
    assert_eq!(Program::parse(&encrypted).unwrap().functions[0].address, 0);
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&plain).unwrap();
    let body = z.finish().unwrap();
    let mut ai2 = b"AI2\0\0\0\0\0".to_vec();
    word(&mut ai2, plain.len() as u32);
    word(&mut ai2, body.len() as u32);
    ai2.extend(body);
    assert_eq!(Program::parse(&ai2).unwrap().main, 0);
    ai2.push(0);
    assert!(Program::parse(&ai2).is_err());
}
#[test]
fn truncated_duplicate_unknown_and_excessive_sections_fail() {
    let b = minimal();
    for end in 0..b.len() {
        assert!(Program::parse(&b[..end]).is_err(), "prefix {end}");
    }
    let mut duplicate = b.clone();
    duplicate.extend(b"MAIN\0\0\0\0");
    assert!(Program::parse(&duplicate)
        .unwrap_err()
        .to_string()
        .contains("duplicate"));
    let mut unknown = b.clone();
    unknown.extend(b"NOPE");
    assert!(Program::parse(&unknown)
        .unwrap_err()
        .to_string()
        .contains("unsupported AIN section"));
    let mut huge = b"VERS\x05\0\0\0FUNC".to_vec();
    word(&mut huge, u32::MAX);
    assert!(Program::parse(&huge).is_err());
}
#[test]
#[ignore = "reads owner-installed AIN at runtime; no game files committed"]
fn owner_rance7_program() {
    let bytes = std::fs::read(std::env::var("CRYSIRROR_TEST_AIN").unwrap()).unwrap();
    let p = Program::parse(&bytes).unwrap();
    println!("version={} code={} functions={} globals={} structures={} strings={} messages={} libraries={} main={} main_name={}",p.version,p.code.len(),p.functions.len(),p.globals.len(),p.structures.len(),p.strings.len(),p.messages.len(),p.libraries.len(),p.main,String::from_utf8_lossy(&p.functions[p.main as usize].name));
    let count = p.validate_code_layout().unwrap();
    println!(
        "instructions={count} main_pc={:#x}",
        p.functions[p.main as usize].address
    );
    let mut pc = p.functions[p.main as usize].address;
    for _ in 0..12 {
        let i = p.instruction(pc).unwrap();
        println!("{:#x} {} {:?}", i.pc, i.name, i.arguments);
        pc = i.next;
    }
    assert_eq!(p.version, 5);
    assert!(!p.messages.is_empty());
}
