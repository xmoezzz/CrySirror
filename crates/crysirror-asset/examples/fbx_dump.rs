// SPDX-License-Identifier: MPL-2.0
//! Print an FBX file's records (debug aid):
//! `cargo run -p crysirror-asset --example fbx_dump -- file.fbx [depth] [mat]`.
use crysirror_asset::interchange::fbx::{self, Node, Prop};
fn show(n: &Node, depth: usize, max: usize) {
    if depth > max {
        return;
    }
    let props: Vec<String> = n
        .props
        .iter()
        .take(6)
        .map(|p| match p {
            Prop::Str(s) => format!("{:?}", s.replace("\u{0}\u{1}", "::")),
            Prop::F64s(v) => format!("d[{}]", v.len()),
            Prop::F32s(v) => format!("f[{}]", v.len()),
            Prop::I32s(v) => format!("i[{}]", v.len()),
            Prop::I64s(v) => format!("l[{}]", v.len()),
            Prop::Raw(v) => format!("raw[{}]", v.len()),
            p => format!("{p:?}"),
        })
        .collect();
    println!("{}{} {}", "  ".repeat(depth), n.name, props.join(" "));
    for c in &n.children {
        show(c, depth + 1, max);
    }
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&a[1]).unwrap();
    let (v, top) = fbx::parse(&bytes).unwrap();
    println!("version {v}");
    let depth = a.get(2).and_then(|d| d.parse().ok()).unwrap_or(3);
    if a.get(3).map(String::as_str) == Some("mat") {
        for n in &top {
            for o in &n.children {
                for c in &o.children {
                    if let Some(Prop::F64s(v)) = c.props.first() {
                        if v.len() == 16 {
                            println!(
                                "{} {:?} {}: {:?}",
                                o.name,
                                o.props.get(1),
                                c.name,
                                v.iter()
                                    .map(|x| (x * 1000.).round() / 1000.)
                                    .collect::<Vec<_>>()
                            );
                        }
                    }
                }
            }
        }
        return;
    }
    for n in &top {
        if n.name != "Definitions" && n.name != "FBXHeaderExtension" {
            show(n, 0, depth);
        }
    }
}
