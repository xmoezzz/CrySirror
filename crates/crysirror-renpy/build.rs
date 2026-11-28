// SPDX-License-Identifier: MPL-2.0
//! Embeds the Python shells under `py/` with their module names.
use std::{env, fs, path::Path};

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String, bool)>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(root, &p, out);
        } else if p.extension().is_some_and(|e| e == "py") {
            let rel = p.strip_prefix(root).unwrap().with_extension("");
            let mut parts: Vec<String> = rel
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect();
            let package = parts.last().is_some_and(|l| l == "__init__");
            if package {
                parts.pop();
            }
            out.push((
                parts.join("."),
                p.canonicalize().unwrap().to_string_lossy().into_owned(),
                package,
            ));
        }
    }
}

fn emit(code: &mut String, static_name: &str, files: &[(String, String, bool)]) {
    code.push_str(&format!(
        "pub static {static_name}: &[(&str, &str, bool)] = &[\n"
    ));
    for (name, path, package) in files {
        code.push_str(&format!(
            "    ({name:?}, include_str!({path:?}), {package}),\n"
        ));
    }
    code.push_str("];\n");
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("py");
    println!("cargo:rerun-if-changed=py");
    let mut files = Vec::new();
    walk(&root, &root.join("renpy"), &mut files);
    // Shells of the native runtime (`rt`): module names relative to py/rt.
    let mut rt_files = Vec::new();
    if root.join("rt").is_dir() {
        walk(&root.join("rt"), &root.join("rt"), &mut rt_files);
    }
    let mut code = String::new();
    emit(&mut code, "SOURCES", &files);
    emit(&mut code, "RT_SOURCES", &rt_files);
    fs::write(
        Path::new(&env::var("OUT_DIR").unwrap()).join("py_sources.rs"),
        code,
    )
    .unwrap();
}
