//! Diagnostic: decompile local TJS2 bytecode files (`DECOMPILE=a,b`), writing
//! `<file>.decompiled.tjs` beside each. Inputs are explicit local paths only.
#[test]
#[ignore = "diagnostic tool; requires DECOMPILE paths"]
fn decompile_files() {
    for path in std::env::var("DECOMPILE").unwrap().split(',') {
        let bytes = std::fs::read(path).unwrap();
        let file = krkr_tjs2::bytecode::BytecodeFile::parse(&bytes).unwrap();
        let out = krkr_tjs2::decompile::decompile(&file, &Default::default()).unwrap();
        let text: String = out.sources.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n");
        std::fs::write(format!("{path}.decompiled.tjs"), text).unwrap();
        println!("DECOMPILED {path}");
    }
}

/// Diagnostic: disassemble local TJS2 bytecode files (`DISASM=a,b`), writing
/// `<file>.disasm.txt` beside each.
#[test]
#[ignore = "diagnostic tool; requires DISASM paths"]
fn disassemble_files() {
    for path in std::env::var("DISASM").unwrap().split(',') {
        let bytes = std::fs::read(path).unwrap();
        let file = krkr_tjs2::bytecode::BytecodeFile::parse(&bytes).unwrap();
        let text = file.disassemble(&krkr_tjs2::bytecode::DisasmOptions::full()).unwrap();
        std::fs::write(format!("{path}.disasm.txt"), text).unwrap();
        println!("DISASSEMBLED {path}");
    }
}
