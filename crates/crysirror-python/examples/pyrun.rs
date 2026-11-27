// SPDX-License-Identifier: MPL-2.0
//! Runs a Python file: `cargo run --example pyrun -- file.py [py2]`.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&args[1]).expect("read");
    let d = if args.get(2).map(|s| s == "py2").unwrap_or(false) { crysirror_python::Dialect::Py2 } else { crysirror_python::Dialect::Py3 };
    let mut vm = crysirror_python::Vm::with_host(Box::new(Out));
    crysirror_python::modules::sys::set_dialect(&mut vm, d);
    if let Err(e) = crysirror_python::run_source(&mut vm, &src, d) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
struct Out;
impl crysirror_python::Host for Out {
    fn write_stdout(&mut self, s: &str) {
        print!("{s}");
    }
    fn as_any(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
