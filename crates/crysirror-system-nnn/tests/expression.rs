use crysirror_system_nnn::{Program,XorProfile,expression::{evaluate,Variables,VariableProfile},machine::{Machine,Step}};
fn program(mut tail:Vec<u32>)->Program {
    let mut words=vec![0;32];words[0]=32;words[1]=0x66660001;words[2]=0x55550001;
    words[18]=32+tail.len() as u32;words.append(&mut tail);words.push(0x7fffffff);
    let encoded:Vec<_>=words.iter().flat_map(|w|(!w).to_le_bytes()).collect();
    Program::parse(&encoded,XorProfile::default()).unwrap()
}
fn expression(tokens:&[u32],p:&Program,vars:&mut Variables)->i32 {
    let mut words=vec![0xa0000000,tokens.len() as u32];words.extend(tokens);
    evaluate(&words,p,vars).unwrap().0
}
#[test]
fn original_references_survive_assignments_compound_and_unary_operations() {
    let p=program(vec![]);let mut v=Variables::fresh(VariableProfile::Legacy1000);
    // Both '=' results retain the same variable reference. The final '+'
    // observes the second write twice, rather than freezing values 2 and 3.
    assert_eq!(expression(&[0x80000064,0x90000002,0xa000003d,0x80000064,0x90000003,0xa000003d,0xa000002b],&p,&mut v),6);
    assert_eq!(v.read_stored(100),3);
    assert_eq!(expression(&[0x80000064,0x90000002,0xa200002b],&p,&mut v),5);
    assert_eq!(expression(&[0x80000064,0xa0000021],&p,&mut v),0);
    assert_eq!(v.read_stored(100),0);
    assert_eq!(expression(&[0x80000064,0xa0000021],&p,&mut v),1);
    assert_eq!(expression(&[0x80008007,0x90000063,0xa000003d],&p,&mut v),7);
    assert_eq!(v.read_stored(100),1);
}
#[test]
fn native_full_width_pool_and_direct_nibble_profiles_are_distinct() {
    let p=program(vec![]);let mut v=Variables::fresh(VariableProfile::Legacy1000);
    assert_eq!(evaluate(&[0x91000000],&p,&mut v).unwrap(),(0,1));
    assert_eq!(expression(&[0x91000000],&p,&mut v),i32::MAX);
    assert_eq!(expression(&[0x91000000,0x90000001,0xa000002b],&p,&mut v),i32::MIN);
    assert_eq!(evaluate(&[0x90008000],&p,&mut v).unwrap().0,-32768);
    assert_eq!(expression(&[0x90000007,0x90000000,0xa000002f],&p,&mut v),7);
    assert_eq!(expression(&[0x90000007,0x90000000,0xa0000025],&p,&mut v),0);
    assert_eq!(expression(&[0x9000ffff,0x90000000,0xa500003c],&p,&mut v),1);
    v.mark_calculated(0,b"mode".to_vec());
    assert!(evaluate(&[0x80000000],&p,&mut v).unwrap_err().to_string().contains("binding"));
    assert!(evaluate(&[0xa0000000,1,0x9100ffff],&p,&mut v).is_err());
    assert!(evaluate(&[0xa0000000,2,0x90000001,0xa000013d],&p,&mut v).is_err());
}
#[test]
fn failed_expression_keeps_machine_pc_executed_count_and_variable_bank() {
    let p=program(vec![10,0x66660005,0,0xa0000000,5,0x80000064,0x90000009,0xa000003d,0x90000001,0xa000007e]);
    let mut m=Machine::with_variables(Variables::fresh(VariableProfile::Legacy1000));
    assert_eq!(m.poll(&p,1).unwrap(),Step::Budget);
    assert!(m.poll(&p,1).is_err());assert_eq!(m.pc(),32);assert_eq!(m.executed(),1);
    assert_eq!(m.variables().unwrap().read_stored(100),0);
}
#[test]
fn original_positive_condition_block_offsets_and_else_skipping() {
    for (condition,expected) in [(0x9000ffff,2),(0x90000000,2),(0x90000001,1)] {
        let p=program(vec![
            4,0x66660002,0x55560002,condition,
            4,0x66660002,0xa000007b,15,
            8,0x66660005,0,0xa0000000,3,0x80000064,0x90000001,0xa000003d,
            3,0x66660002,0xa000007d,
            3,0x66660002,0x55560004,
            4,0x66660002,0xa000007b,15,
            8,0x66660005,0,0xa0000000,3,0x80000064,0x90000002,0xa000003d,
            3,0x66660002,0xa000007d,
            3,0x66660002,0x55560005,
        ]);
        let mut m=Machine::with_variables(Variables::fresh(VariableProfile::Legacy1000));
        assert_eq!(m.poll(&p,100).unwrap(),Step::Halted);
        assert_eq!(m.variables().unwrap().read_stored(100),expected);
    }
}
#[test]
fn encoded_name_tables_preserve_empty_slots_and_calculated_bindings() {
    use crysirror_system_nnn::config::{Names,fresh_variables,integer};
    let source=b"rnd,,mode, plain,\r\nother";let cipher:Vec<_>=source.iter().map(|b|b^255).collect();
    let names=Names::parse(&cipher,true).unwrap();
    assert_eq!(names.tokens,[b"rnd".to_vec(),vec![],b"mode".to_vec(),b"plain".to_vec(),b"other".to_vec()]);
    assert_eq!(names.index(b"mode"),Some(2));
    let vars=fresh_variables(VariableProfile::Legacy1000,&names).unwrap();
    assert!(vars.read_calculated(0).is_err());assert_eq!(vars.read_calculated(3).unwrap(),0);
    assert_eq!(integer(b"-42tail").unwrap(),-42);assert_eq!(integer(b"none").unwrap(),0);
    assert!(Names::parse(b"nul\0",false).is_err());
}
