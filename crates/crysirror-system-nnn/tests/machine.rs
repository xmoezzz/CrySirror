use crysirror_system_nnn::{
    machine::{Machine, Payload, Step},
    Program, XorProfile,
};
fn program(first: [u32; 5]) -> Program {
    program_name(first, b"native-name")
}
fn program_name(first: [u32; 5], name: &[u8]) -> Program {
    let size = (41 + (name.len() + 1).div_ceil(4)).max(44);
    let mut words = vec![0u32; size];
    words[0] = 32;
    words[1] = 0x66660001;
    words[2] = 0x55550001;
    words[6] = 1;
    words[7] = 40;
    words[18] = size as u32;
    words[32..37].copy_from_slice(&first);
    words[37] = 3;
    words[38] = 0x66660002;
    words[39] = 0x555600ff;
    words[40] = 41;
    let mut source = name.to_vec();
    source.push(0);
    for (i, b) in source.chunks(4).enumerate() {
        let mut value = [0; 4];
        value[..b.len()].copy_from_slice(b);
        words[41 + i] = u32::from_le_bytes(value);
    }
    let bytes = words
        .iter()
        .flat_map(|w| (!w).to_le_bytes())
        .collect::<Vec<_>>();
    Program::parse(&bytes, XorProfile::default()).unwrap()
}
#[test]
fn original_callback_remains_pending_until_exact_receipt_then_unknown_stays_at_pc() {
    let p = program([5, 0x66660007, 0x33330051, 1, 0]);
    let mut m = Machine::default();
    assert_eq!(m.poll(&p, 1).unwrap(), Step::Budget);
    assert_eq!(m.pc(), 32);
    let Step::Native(call) = m.poll(&p, 20).unwrap() else {
        panic!("native event")
    };
    assert_eq!(call.payload, Payload::StoryName(b"native-name".to_vec()));
    assert_eq!(call.pc, 32);
    assert_eq!(call.next, 37);
    assert_eq!(m.poll(&p, 20).unwrap(), Step::Native(call.clone()));
    let mut wrong = call.clone();
    wrong.serial += 1;
    assert!(m.acknowledge(&wrong).is_err());
    assert_eq!(m.pc(), 32);
    let mut metadata = crysirror_system_nnn::metadata::Metadata::default();
    assert!(metadata.commit(&mut m, &wrong).is_err());
    assert!(metadata.story_name.is_empty());
    metadata.commit(&mut m, &call).unwrap();
    assert_eq!(metadata.story_name, b"native-name");
    assert_eq!(m.pc(), 37);
    assert_eq!(m.executed(), 2);
    assert!(m.acknowledge(&call).is_err());
    assert!(m
        .poll(&p, 20)
        .unwrap_err()
        .to_string()
        .contains("unsupported"));
    assert_eq!(m.pc(), 37);
    assert_eq!(m.executed(), 2);
}
#[test]
fn invalid_jump_return_and_budget_have_no_partial_instruction_commit() {
    let p = program([5, 0x66660002, 0x55560020, u32::MAX, 0]);
    let mut m = Machine::default();
    assert!(m.poll(&p, 0).is_err());
    assert_eq!(m.pc(), 0);
    assert_eq!(m.executed(), 0);
    m.poll(&p, 1).unwrap();
    assert!(m.poll(&p, 1).is_err());
    assert_eq!(m.pc(), 32);
    assert_eq!(m.executed(), 1);
    let p = program([5, 0x66660002, 0x55560007, 0, 0]);
    let mut m = Machine::default();
    m.poll(&p, 1).unwrap();
    assert!(m.poll(&p, 1).is_err());
    assert_eq!(m.pc(), 32);
}

#[test]
fn native_metadata_truncates_bytes_without_normalizing_original_encoding() {
    let mut name = vec![b'a'; 31];
    name.extend([0x82, 0xa0].repeat(40));
    let p = program_name([5, 0x66660007, 0x33330051, 1, 0], &name);
    let mut m = Machine::default();
    let mut metadata = crysirror_system_nnn::metadata::Metadata::default();
    assert!(metadata.advance(&mut m, &p, 100).is_err());
    assert_eq!(metadata.story_name, &name[..62]);
    assert_eq!(metadata.story_name.last(), Some(&0x82));
    assert_eq!(m.pc(), 37);
}
