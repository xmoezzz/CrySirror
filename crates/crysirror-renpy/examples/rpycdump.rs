// SPDX-License-Identifier: MPL-2.0
//! Loads .rpyc files and reports node statistics / conversion errors.
use crysirror_renpy::{ast::Arena, rpyc};
fn main() {
    let mut vm = crysirror_python::Vm::new();
    let mut shells = rpyc::Shells::new(&mut vm).unwrap();
    let reg = crysirror_renpy::sl2::Registry::default();
    let mut total = 0;
    let mut stats = std::collections::BTreeMap::<String, usize>::new();
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).unwrap();
        match rpyc::unpickle_script(&mut vm, &mut shells, &bytes) {
            Ok((_data, stmts)) => {
                let mut arena = Arena::default();
                let mut c = rpyc::Converter::new(&mut vm, &mut arena, &reg);
                c.convert_block(&stmts);
                for e in &c.errors {
                    println!("{path}: {e}");
                }
                if std::env::var("DUMP_PY").is_ok() {
                    for n in &arena.nodes {
                        use crysirror_renpy::ast::NodeKind::*;
                        match &n.kind {
                            Python { code, .. } | EarlyPython { code, .. } => println!(
                                "#PY {}:{}\n{}",
                                n.filename, n.linenumber, code.source.text
                            ),
                            Define { code, name, .. } | Default { code, name, .. } => {
                                println!("#DEF {name} = {}", code.source.text)
                            }
                            Image {
                                code: Some(code),
                                name,
                                ..
                            } => println!("#IMG {} = {}", name.join(" "), code.source.text),
                            UserStatement { line, .. } => println!("#US {line}"),
                            _ => {}
                        }
                    }
                }
                if std::env::var("DUMP_ALL").is_ok() {
                    for n in &arena.nodes {
                        println!("{}:{} {:?}", n.filename, n.linenumber, n.kind);
                    }
                }
                total += arena.nodes.len();
                for n in &arena.nodes {
                    let k = format!("{:?}", n.kind);
                    let k = k
                        .split(|c: char| !c.is_alphanumeric())
                        .next()
                        .unwrap()
                        .to_string();
                    *stats.entry(k).or_default() += 1;
                }
            }
            Err(e) => println!("{path}: LOAD ERROR {e}"),
        }
    }
    println!("{total} nodes: {stats:?}");
}
