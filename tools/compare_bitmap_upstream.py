#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Compare shared CPU bitmaps against the complete pinned siglus_rs source.

Build output stays in --work-dir / --target-dir outside both source trees.
Only generated pixels are used; no game data, fonts or devices are required.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = "crysirror-bitmap-differential"


def git(source_root: Path, *args: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(source_root), *args], text=True
    ).strip()


def validate_source(source_root: Path, pin: str) -> None:
    if git(source_root, "rev-parse", "HEAD") != pin:
        raise ValueError("siglus_rs HEAD differs from upstream-lock.json pin")
    # Check index and working-tree edits against HEAD, including dependency crates
    # and build scripts. Untracked game files are neither read nor distributed.
    changed = git(source_root, "diff", "--name-only", "HEAD", "--",
                  "Cargo.toml", "Cargo.lock", "rust-toolchain", "rust-toolchain.toml", "crates")
    if changed:
        raise ValueError(f"tracked upstream source changed:\n{changed}")
    for relative in ["Cargo.toml", "Cargo.lock", "crates/reallive/Cargo.toml",
                     "crates/reallive/src/surface.rs", "crates/avg32/Cargo.toml",
                     "crates/avg32/src/buffer.rs", "crates/avg32/src/pdtmgr.rs"]:
        if not (source_root / relative).is_file():
            raise ValueError(f"complete siglus_rs checkout required: missing {relative}")


def external(path: Path, source_root: Path) -> Path:
    path = path.expanduser().resolve()
    if path.is_relative_to(ROOT) or path.is_relative_to(source_root.resolve()):
        raise ValueError("work and target directories must be outside both source trees")
    return path


def run(source_root: Path, work_dir: Path, target_dir: Path | None = None) -> None:
    source_root = source_root.expanduser().resolve()
    pin = json.loads((ROOT / "upstream-lock.json").read_text())["sources"]["siglus_rs"]["commit"]
    validate_source(source_root, pin)
    work_dir = external(external(work_dir, source_root) / "bitmap-differential", source_root)
    target_dir = external(target_dir if target_dir else work_dir / "target", source_root)
    manifest = external(work_dir / "Cargo.toml", source_root)
    if manifest.exists() and f'name = "{PACKAGE}"' not in manifest.read_text():
        raise ValueError(f"refusing to overwrite unrelated Cargo project: {manifest}")
    source_dir = external(work_dir / "src", source_root)
    driver_source = external(source_dir / "main.rs", source_root)
    source_dir.mkdir(parents=True, exist_ok=True)
    dependencies = {
        "crysirror-image": ROOT / "crates/crysirror-image",
        "reallive": source_root / "crates/reallive",
        "avg32": source_root / "crates/avg32",
    }
    lines = ["[package]", f'name = "{PACKAGE}"', 'version = "0.0.0"',
             'edition = "2024"', "[workspace]", "[dependencies]"]
    lines.extend(f"{name} = {{ path = {json.dumps(str(path))} }}" for name, path in dependencies.items())
    manifest.write_text("\n".join(lines) + "\n")
    driver_source.write_text((ROOT / "tools/bitmap_reference.rs").read_text())
    env = os.environ.copy()
    env.update(AVG32_NO_AUDIO="1", REALLIVE_NO_AUDIO="1")
    subprocess.run(["cargo", "run", "--quiet", "--manifest-path", str(manifest),
                    "--target-dir", str(target_dir)], env=env, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-root", type=Path, required=True,
                        help="complete external siglus_rs Git checkout at the locked pin")
    parser.add_argument("--work-dir", type=Path, required=True,
                        help="external cache directory for the generated Cargo driver")
    parser.add_argument("--target-dir", type=Path,
                        help="external Cargo target directory (default: driver-local target)")
    args = parser.parse_args()
    try:
        run(args.source_root, args.work_dir, args.target_dir)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"bitmap comparison failed: {error}\n")


if __name__ == "__main__":
    main()
