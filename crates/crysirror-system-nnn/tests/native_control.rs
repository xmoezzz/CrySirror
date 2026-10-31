use crysirror_system_nnn::{
    expression::{VariableProfile, Variables},
    machine::{Machine, Payload, Step},
    native_control::ScriptSession,
    native_resources::ResourceNames,
    Program, XorProfile,
};
use std::sync::Arc;
fn p(records: &[u32], strings: &[&[u8]]) -> Arc<Program> {
    let table = 32 + records.len();
    let mut words = vec![0u32; table + strings.len()];
    words[0] = 32;
    words[1] = 0x66660001;
    words[2] = 0x55550001;
    words[6] = strings.len() as u32;
    words[7] = table as u32;
    words[18] = words.len() as u32;
    words[32..table].copy_from_slice(records);
    for (id, string) in strings.iter().enumerate() {
        words[table + id] = words.len() as u32;
        let mut bytes = string.to_vec();
        bytes.push(0);
        for chunk in bytes.chunks(4) {
            let mut v = [0u8; 4];
            v[..chunk.len()].copy_from_slice(chunk);
            words.push(u32::from_le_bytes(v));
        }
    }
    Arc::new(
        Program::parse(
            &words
                .iter()
                .flat_map(|v| (!v).to_le_bytes())
                .collect::<Vec<_>>(),
            XorProfile::default(),
        )
        .unwrap(),
    )
}
fn native(s: &mut ScriptSession) -> crysirror_system_nnn::machine::NativeCall {
    let Step::Native(v) = s.poll(100).unwrap() else {
        panic!("native call expected")
    };
    v
}
#[test]
fn script_push_jump_next_share_native_bank_and_keep_loader_failure_atomic() {
    let root = p(
        &[5, 0x66660002, 0x5556000a, 0, 7, 3, 0x66660002, 0x55560005],
        &[b"first"],
    );
    let first = p(
        &[
            8, 0x66660005, 0, 0xa0000000, 3, 0x80000136, 0x90000009, 0xa000003d, 4, 0x66660002,
            0x55560015, 0,
        ],
        &[b"second"],
    );
    let second = p(&[3, 0x66660002, 0x55560006], &[]);
    let mut s = ScriptSession::new(
        root,
        Machine::with_variables(Variables::fresh(VariableProfile::Legacy1000)),
    );
    let call = native(&mut s);
    let pc = s.machine().pc();
    assert!(s
        .dispatch_control(&call, |_| anyhow::bail!("missing original resource"))
        .is_err());
    assert_eq!(s.machine().pc(), pc);
    assert_eq!(s.depth(), 0);
    s.dispatch_control(&call, |name| {
        assert_eq!(name, b"first");
        Ok(first.clone())
    })
    .unwrap();
    assert_eq!(s.depth(), 1);
    let call = native(&mut s);
    assert_eq!(s.machine().variables().unwrap().read_stored(310), 9);
    s.dispatch_control(&call, |name| {
        assert_eq!(name, b"second");
        Ok(second.clone())
    })
    .unwrap();
    assert_eq!(s.depth(), 1);
    let call = native(&mut s);
    s.dispatch_control(&call, |_| unreachable!()).unwrap();
    assert_eq!(s.depth(), 0);
    assert_eq!(s.machine().variables().unwrap().read_stored(310), 9);
    assert_eq!(s.poll(100).unwrap(), Step::Halted);
    s.restart_controller_after_end().unwrap();
    assert_eq!(s.machine().pc(), 0);
}
#[test]
fn user_expression_candidate_is_committed_only_after_matching_receipt() {
    let program = p(
        &[
            10, 0x66660003, 0, 1, 0xa0000000, 3, 0x80000064, 0x90000009, 0xa000003d, 17,
        ],
        &[],
    );
    let mut m = Machine::with_variables(Variables::fresh(VariableProfile::Legacy1000));
    let Step::Native(call) = m.poll(&program, 100).unwrap() else {
        panic!()
    };
    assert_eq!(
        call.payload,
        Payload::UserCall {
            command: true,
            parameters: vec![9],
            resume_id: Some(17)
        }
    );
    assert_eq!(m.variables().unwrap().read_stored(100), 0);
    let mut wrong = call.clone();
    wrong.serial += 1;
    assert!(m.acknowledge(&wrong).is_err());
    assert_eq!(m.variables().unwrap().read_stored(100), 0);
    m.acknowledge(&call).unwrap();
    assert_eq!(m.variables().unwrap().read_stored(100), 9);
}
#[test]
fn resource_affecting_variable_write_yields_owner_receipt_before_bank_commit() {
    let program = p(
        &[
            8, 0x66660005, 0, 0xa0000000, 3, 0x80000064, 0x90000001, 0xa000003d,
        ],
        &[],
    );
    let mut vars = Variables::fresh(VariableProfile::Legacy1000);
    vars.mark_write_effect(100, b"layerOffVar2".to_vec());
    let mut machine = Machine::with_variables(vars);
    let Step::Native(call) = machine.poll(&program, 100).unwrap() else {
        panic!()
    };
    assert_eq!(call.pc, 32);
    assert_eq!(call.next, 40);
    let Payload::VariableWrites { writes } = &call.payload else {
        panic!()
    };
    assert_eq!(
        writes,
        &[crysirror_system_nnn::expression::NativeVariableWrite {
            variable: 100,
            layer: 2,
            value: 1
        }]
    );
    assert_eq!(machine.variables().unwrap().read_stored(100), 0);
    let candidate = machine.native_variables_candidate(&call).unwrap();
    assert_eq!(candidate.read_stored(100), 1);
    assert_eq!(
        machine.poll(&program, 1).unwrap(),
        Step::Native(call.clone())
    );
    machine.acknowledge(&call).unwrap();
    assert_eq!(machine.variables().unwrap().read_stored(100), 1);
    assert!(machine.variables().unwrap().native_writes().is_empty());
}
#[test]
fn native_selection_receipt_enters_the_original_case_and_footer_is_preserved() {
    let program = p(
        &[
            7, 0x66660006, 0x22220006, 2, 0, 1, 99, 4, 0x66660002, 0xa000007b, 18, 4, 0x66660002,
            0x55560008, 1, 3, 0x66660002, 0x55560005, 4, 0x66660002, 0x55560008, 2, 3, 0x66660002,
            0x55560005, 3, 0x66660002, 0xa000007d,
        ],
        &[b"first", b"second"],
    );
    let mut machine = Machine::default();
    let Step::Native(call) = machine.poll(&program, 100).unwrap() else {
        panic!()
    };
    let Payload::Selection {
        choices,
        messages,
        string_ids,
        strings,
        resume_id,
        ..
    } = &call.payload
    else {
        panic!()
    };
    assert_eq!((*choices, *messages), (2, 0));
    assert_eq!(*resume_id, 99);
    assert_eq!(string_ids, &[0, 1]);
    assert_eq!(strings, &[b"first".to_vec(), b"second".to_vec()]);
    machine.acknowledge_case(&call, &program, 2).unwrap();
    assert_eq!(machine.pc(), 54);
    assert_eq!(machine.poll(&program, 1).unwrap(), Step::Halted);
}
#[test]
fn original_resource_names_are_scoped_and_strict_shift_jis() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<crysirror_system_nnn::native_bootstrap::NativeConfiguration>();
    let names = ResourceNames::new("mugen2").unwrap();
    assert_eq!(
        names.script(b"preprologue").unwrap(),
        "mugen2/spt/preprologue.spt"
    );
    assert_eq!(
        names.encoded_path(b"spt\\sys.spt").unwrap(),
        "mugen2/spt/sys.spt"
    );
    assert!(names.script(b"../escape").is_err());
    assert!(names.path("../escape").is_err());
    assert!(names.encoded_path(&[0x81]).is_err());
    assert!(ResourceNames::new("/absolute").is_err());
    let picture = names.script_image(b"BG_001", 3).unwrap();
    assert_eq!(picture.loose, "mugen2/dwq/BG/BG_001.dwq");
    assert_eq!(
        picture
            .packed
            .iter()
            .map(|p| p.archive.as_str())
            .collect::<Vec<_>>(),
        [
            "mugen2/dwq/bg2.gpk",
            "mugen2/dwq/bg1.gpk",
            "mugen2/dwq/bg0.gpk"
        ]
    );
    assert_eq!(picture.packed[0].member, "bg_001.dwq");
    let system = names.picture(b"sys\\sc_01", 1).unwrap();
    assert_eq!(system.packed[0].archive, "mugen2/dwq/sc0.gpk");
    let packed = names.packed_voice(b"aa000001").unwrap();
    assert_eq!(packed.archive, "mugen2/cdvaw/aa.vpk");
    assert_eq!(packed.member, "aa000001.vaw");
    assert_eq!(
        names.system_wave(Some(b"sys"), b"select").unwrap(),
        "mugen2/vaw/sys/select.vaw"
    );
    assert_eq!(
        names.music_lookup(b"m01").unwrap().control,
        "mugen2/wgq/m01.mml"
    );
}
#[test]
fn native_calculated_bindings_preserve_bank_aliases_and_consume_shared_rng_state() {
    use crysirror_system_nnn::{
        config::{configured_variables, Names},
        expression::evaluate,
    };
    let names = Names::parse(
        b"nullpo,date,rnd,time,week,holiday,,,,,year,,,,,,,,,,,,,,,,,,,,,mode",
        false,
    )
    .unwrap();
    let setup = Names::parse(b"", false).unwrap();
    let mut vars = configured_variables(VariableProfile::Legacy1000, &names, &setup).unwrap();
    assert_eq!(
        vars.read_calculated(names.index(b"date").unwrap()).unwrap(),
        101
    );
    assert_eq!(
        vars.read_calculated(names.index(b"year").unwrap()).unwrap(),
        2000
    );
    let mode = names.index(b"mode").unwrap();
    assert!(vars.read_calculated(mode).is_err());
    assert!(vars.bind_calculated_value(b"mode", 7).unwrap());
    assert_eq!(vars.read_calculated(mode).unwrap(), 7);
    assert!(vars.bind_calculated_value(b"rnd", 0).is_err());
    let program = p(&[], &[]);
    let rnd = 0x80000000 | u32::from(names.index(b"rnd").unwrap());
    assert!(evaluate(&[rnd], &program, &mut vars).is_err());
    let seed = (0..100000)
        .find(|seed| {
            let mut state = crysirror_image::temporal_pattern::PatternRng::msvc(*seed);
            state.next_draw().unwrap() >= 30000 && state.next_draw().unwrap() < 30000
        })
        .unwrap();
    let state = crysirror_image::temporal_pattern::PatternRng::msvc(seed);
    let mut expected = state.clone();
    expected.next_draw().unwrap();
    let result = expected.next_draw().unwrap() % 10000;
    vars.replace_random_state(state).unwrap();
    assert_eq!(
        evaluate(&[rnd], &program, &mut vars).unwrap().0,
        result as i32
    );
    assert_eq!(vars.random_state().unwrap(), expected);
    let snapshot = vars.random_state().unwrap();
    let mut candidate = vars.clone();
    evaluate(&[rnd], &program, &mut candidate).unwrap();
    assert_eq!(vars.random_state().unwrap(), snapshot);
}
