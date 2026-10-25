#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Compile the pinned ORIGINAL Siglus G00 decoder and compare the local port.

Only synthetic fixtures are used. No game/font files or upstream mutations.
This is a resource harness, NOT a build/test of the original SceneVm or host.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True, help="Complete siglus_rs checkout")
    args = parser.parse_args()
    source = args.source_root.resolve(strict=True)
    imported = json.loads((ROOT/"crates/crysirror-siglus/upstream-files.json").read_text())
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip()
    if revision != imported["commit"]:
        parser.error("checkout differs from the imported commit")
    for relative, expected in imported["reviewed_complete_resource_files"].items():
        if hashlib.sha256((source/relative).read_bytes()).hexdigest() != expected:
            parser.error(f"upstream source differs from recorded hash: {relative}")
    # Read-only inclusion of the actual upstream files, not re-created behavior.
    modules = {"native":"crates/siglus_scene_vm/src/assets/g00.rs",
               "g00":"crates/siglus_assets/src/g00.rs", "lzss":"crates/siglus_assets/src/lzss.rs"}
    prefix = "extern crate self as siglus_assets;\n"
    prefix += "pub mod assets { pub use crysirror_siglus::RgbaImage; }\n"
    for name, relative in modules.items():
        prefix += f"#[path={json.dumps(str(source/relative))}] pub mod {name};\n"
    fixtures = (ROOT/"crates/crysirror-siglus/tests/resources.rs").read_text().split("#[test]", 1)[0]
    program = prefix + fixtures + r'''
fn main() {
    let mut type0=vec![0,3,0,1,0];type0.extend(0u32.to_le_bytes());type0.extend(12u32.to_le_bytes());type0.extend([1,10,20,30,0x11,0]);
    let mut type1=vec![1,2,0,1,0];
    type1.extend(compressed(&[2,0,10,20,30,40,50,60,70,80,1,0]));
    let mut results=Vec::new();
    for (name,file) in [("type0-overlap",type0),("type1-palette-alpha",type1),("type2-reused-missing-crop",type2())] {
        let original=native::decode_g00(&file).unwrap();
        let port=decode_g00(&file).unwrap();
        assert_eq!(original.kind as u8, port.kind as u8);
        assert_eq!((original.width,original.height),(port.width,port.height));
        assert_eq!(original.original_sizes,port.original_sizes);
        assert_eq!(original.frames.len(),port.frames.len());
        for (a,b) in original.frames.iter().zip(&port.frames) {
            assert_eq!((a.width,a.height,a.center_x,a.center_y),(b.width,b.height,b.center_x,b.center_y));
            assert_eq!(a.rgba,b.rgba);
        }
        results.push(serde_json::json!({"fixture":name,"frames":port.frames.len(),"pixels_and_metadata_match":true}));
    }
    println!("{}",serde_json::json!({"original_decoder_executed":true,"vm_executed":false,"fixtures":results}));
}
'''
    with tempfile.TemporaryDirectory(prefix="crysirror-siglus-reference-") as directory:
        directory = Path(directory)
        (directory/"src").mkdir()
        manifest = f'''[package]
name="siglus-resource-comparison"
version="0.1.0"
edition="2024"
[workspace]
[dependencies]
anyhow="1"
image={{version="0.24.9",default-features=false,features=["png","jpeg","bmp"]}}
serde_json="1"
crysirror-image={{path={json.dumps(str(ROOT/"crates/crysirror-image"))}}}
crysirror-siglus={{path={json.dumps(str(ROOT/"crates/crysirror-siglus"))}}}
'''
        (directory/"Cargo.toml").write_text(manifest)
        (directory/"src/main.rs").write_text(program)
        # Cache compiled dependencies without including any reference source in
        # distributable crate paths. No shell interpolation or fetched execution.
        command = ["cargo", "run", "--quiet", "--offline", "--manifest-path", str(directory/"Cargo.toml"),
                   "--target-dir", str(ROOT/"target/siglus-reference")]
        result = subprocess.run(command, text=True, capture_output=True, check=False)
        if result.returncode:
            parser.exit(result.returncode, result.stderr)
        report = json.loads(result.stdout)
        report["commit"] = revision
        print(json.dumps(report, indent=2))

if __name__ == "__main__":
    main()
