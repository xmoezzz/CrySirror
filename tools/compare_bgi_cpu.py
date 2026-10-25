#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Run generated-pixel/glyph differential tests against the unchanged BGI CPU kernels.

The complete local upstream may have unrelated changes; the referenced core,
snapshot and bitmap files must match its verified HEAD. No games or devices.
Font bytes are read only from the explicitly supplied licensed font path.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PIN = "4c41dc3a566338d2ea8400635e23b062613e3782"
FILES = ["crates/ethornell-core/src/lib.rs", "crates/ethornell-app/src/snapshot.rs", "crates/ethornell-app/src/bitmap_blend.rs"]


def between(text, start, end):
    return text[text.index(start):text.index(end, text.index(start))]


def source_snapshot(source):
    source = source.expanduser().resolve()
    head = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if head != PIN:
        raise ValueError("BGI source HEAD differs from audited pin")
    changed = subprocess.check_output(["git", "-C", str(source), "diff", "--name-only", "HEAD", "--", *FILES], text=True).strip()
    if changed:
        raise ValueError(f"tracked BGI reference source changed:\n{changed}")
    contents, hashes = {}, {}
    for name in FILES:
        path = (source / name).resolve()
        if not path.is_relative_to(source):
            raise ValueError(f"reference source escapes checkout: {name}")
        data = path.read_bytes()
        expected = subprocess.check_output(["git", "-C", str(source), "show", f"HEAD:{name}"])
        if hashlib.sha256(data).digest() != hashlib.sha256(expected).digest():
            raise ValueError(f"BGI reference bytes differ from HEAD: {name}")
        contents[name] = data.decode("utf-8")
        hashes[name] = hashlib.sha256(data).hexdigest()
    return source, contents, {"source_head": head, "reference_sha256": hashes}


def output_paths(source, work, target):
    work = (work.expanduser().resolve() / "bgi-cpu-differential").resolve()
    target = target.expanduser().resolve() if target else work / "target"
    generated = [work / "src" / name for name in ["main.rs", "original_core.rs", "original_bitmap.rs", "original_snapshot.rs"]]
    generated += [work / "src", work / "Cargo.toml", work / "Cargo.lock"]
    for output in [work, target, *generated]:
        resolved = output.resolve()
        if resolved.is_relative_to(ROOT) or resolved.is_relative_to(source):
            raise ValueError("differential outputs must stay outside both source trees")
        if output in generated and not resolved.is_relative_to(work):
            raise ValueError("generated output escapes the external work directory")
    manifest = work / "Cargo.toml"
    if manifest.exists():
        content = manifest.read_text()
        package = re.search(r"(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)", content)
        if not package or not re.search(r'^\s*name\s*=\s*"crysirror-bgi-cpu-differential"\s*$', package[1], re.MULTILINE):
            raise ValueError(f"refusing to overwrite unrelated Cargo project: {manifest}")
    return work, target


def run(source, work, target, font, face):
    source, contents, metadata = source_snapshot(source)
    work, target = output_paths(source, work, target)
    font = font.expanduser().resolve()
    if not font.is_file() or not 0 <= face <= 0xFFFFFFFF:
        raise ValueError("an explicit readable font and valid face index are required")
    font_bytes = font.read_bytes()
    metadata["font"] = {"path": str(font), "face_index": face, "bytes": len(font_bytes),
                        "sha256": hashlib.sha256(font_bytes).hexdigest()}
    print(json.dumps(metadata, sort_keys=True), flush=True)
    (work / "src").mkdir(parents=True, exist_ok=True)
    core = contents[FILES[0]]
    core = '#[derive(Debug,Clone,Copy)]\n' + between(core, "pub enum NativeBlendPath", "#[cfg(test)]\nmod native_blend_tests")
    (work / "src/original_core.rs").write_text(core)
    bitmap = contents[FILES[2]].replace("use ethornell_image::DecodedImage;", "use crate::DecodedImage;")
    (work / "src/original_bitmap.rs").write_text(bitmap)
    snapshot = contents[FILES[1]]
    reference = '''use crate::DecodedImage;
use crate::original_core::composite_native_rgba;
use ab_glyph::{Font,FontArc,Glyph};
fn record_raster_error(message:&str){panic!("{message}")}
fn valid_raster(image:&DecodedImage)->bool{image.width>0&&image.height>0&&image.rgba.len()==image.width as usize*image.height as usize*4}
'''
    reference += between(snapshot, "fn valid_text_position(", "const SNAPSHOT_WIDTH")
    reference += between(snapshot, "fn point_in_clip(", "pub(crate) fn composite_runtime_item(")
    reference += between(snapshot, "fn composite_affine_mode5(", "fn composite_nearest(")
    reference += between(snapshot, "fn composite_nearest_with_format(", "#[cfg(test)]\nmod font_session_tests")
    reference += between(snapshot, "fn draw_glyph(", "fn runtime_text_style_at(")
    reference += '''
pub fn image(dst:&mut DecodedImage,src:&DecodedImage,d:[f32;4],s:[f32;4],opacity:f32,mode:i32,clip:Option<crate::graph::RuntimeClipRect>,ignore:bool,quad:Option<[[f32;2];4]>,linear:bool){
if let Some(q)=quad{composite_affine_mode5(dst,src,q,s[0],s[1],s[2],s[3],opacity,mode,clip,linear,ignore);}else{composite_nearest_with_format(dst,src,d[0],d[1],d[2],d[3],s[0],s[1],s[2],s[3],opacity,mode,clip,ignore);}}
pub fn glyph(dst:&mut DecodedImage,font:&FontArc,glyph:Glyph,color:[f32;4],bold:bool,italic:bool,clip:Option<crate::graph::RuntimeClipRect>){draw_glyph(dst,font,glyph,color,0,0,bold,italic,clip);}
'''
    (work / "src/original_snapshot.rs").write_text(reference)
    (work / "src/main.rs").write_text((ROOT / "tools/bgi_cpu_reference.rs").read_text())
    manifest = '# Generated by CrySirror tools/compare_bgi_cpu.py\n[package]\nname="crysirror-bgi-cpu-differential"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\nab_glyph="=0.2.32"\n'
    for crate in ["crysirror-image", "crysirror-text"]:
        manifest += f'{crate}={{path={json.dumps(str(ROOT / "crates" / crate))}}}\n'
    (work / "Cargo.toml").write_text(manifest)
    env = os.environ.copy()
    env.update(AVG32_NO_AUDIO="1", REALLIVE_NO_AUDIO="1")
    subprocess.run(["cargo", "run", "--quiet", "--manifest-path", str(work / "Cargo.toml"), "--target-dir", str(target), "--", str(font.expanduser().resolve()), str(face)], env=env, check=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--font", type=Path, required=True)
    parser.add_argument("--font-face-index", type=int, default=0)
    args = parser.parse_args()
    try:
        run(args.source_root, args.work_dir, args.target_dir, args.font, args.font_face_index)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"BGI CPU comparison failed: {error}\n")
