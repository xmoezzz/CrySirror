// SPDX-License-Identifier: MPL-2.0
//! Synthetic RPC2 container with Ren'Py 7-style pickled nodes.

use crysirror_python::{Value, Vm};
use crysirror_renpy::ast::{Arena, NodeKind};
use crysirror_renpy::rpyc::{Converter, Shells, unpickle_script};
use std::io::Write;

const BUILD: &str = r#"
Say = _make('renpy.ast', 'Say')
Label = _make('renpy.ast', 'Label')
Ret = _make('renpy.ast', 'Return')
Menu = _make('renpy.ast', 'Menu')
def node(cls, line, **kw):
    n = cls()
    n.__dict__.update(filename='game/script.rpy', linenumber=line, name=('game/script.rpy', 1500000000, line), next=None)
    n.__dict__.update(kw)
    return n
say = node(Say, 3, who=PyExpr('e', 'game/script.rpy', 3), who_fast=True, what='Hello, world.', with_=None, interact=True, attributes=('happy',))
ret = node(Ret, 4, expression=None)
menu = node(Menu, 5, items=[('Yes', PyExpr('True', 'game/script.rpy', 5), [ret]), ('Caption', PyExpr('True', 'game/script.rpy', 6), None)], set=None, with_=None)
lab = node(Label, 2, block=[say, menu], parameters=None, hide=False)
lab.name = 'start'
import pickle
blob = pickle.dumps(({'version': 5003000, 'key': 'unlocked'}, [lab]), 2)
"#;

fn container(payload: &[u8]) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(payload).unwrap();
    let body = z.finish().unwrap();
    let mut out = b"RENPY RPC2".to_vec();
    let start = (10 + 36) as u32;
    out.extend(1u32.to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend((body.len() as u32).to_le_bytes());
    out.extend([0u8; 24]);
    out.extend(body);
    out.extend([0u8; 16]);
    out
}

#[test]
fn rpc2_roundtrip() {
    let mut vm = Vm::new();
    let mut shells = Shells::new(&mut vm).unwrap();
    // Build the pickle with the same shell classes the loader uses.
    let ns = shells.namespace();
    let code = vm
        .compile_exec(BUILD, "<build>", crysirror_python::Dialect::Py3)
        .unwrap();
    vm.run_code(code, ns.clone(), None)
        .unwrap_or_else(|e| panic!("{}", crysirror_renpy::rpyc::py_err(&mut Vm::new(), e)));
    let blob = ns.dict_get_str_raw("blob").unwrap();
    let Some(crysirror_python::Payload::Bytes(b)) = blob.payload() else {
        panic!("no bytes")
    };
    let file = container(b);
    let (data, stmts) = unpickle_script(&mut vm, &mut shells, &file).unwrap();
    assert!(!matches!(data, Value::None));
    let mut arena = Arena::default();
    let reg = crysirror_renpy::sl2::Registry::default();
    let mut c = Converter::new(&mut vm, &mut arena, &reg);
    let top = c.convert_block(&stmts);
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    let NodeKind::Label { name, block, .. } = &arena.get(top[0]).kind else {
        panic!()
    };
    assert_eq!(&**name, "start");
    let NodeKind::Say {
        who,
        what,
        attributes,
        ..
    } = &arena.get(block[0]).kind
    else {
        panic!()
    };
    assert_eq!(who.as_ref().unwrap().text.as_ref(), "e");
    assert_eq!(what, "Hello, world.");
    assert_eq!(attributes.as_deref(), Some(&["happy".to_string()][..]));
    let NodeKind::Menu { items, .. } = &arena.get(block[1]).kind else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert!(items[1].block.is_none());
}
