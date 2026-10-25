#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Package validated data + an already-built host. Never claims to cross-compile."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import zipfile
from desktop_project import load, asset_inventory, require, logical_path

MAX_TOTAL = 512 * 1024 * 1024
ROOT = Path(__file__).resolve().parents[1]

def executable_kind(path: Path) -> str:
    with path.open("rb") as stream:
        magic = stream.read(4)
    if magic.startswith(b"MZ"):
        return "windows"
    if magic == b"\x7fELF":
        return "linux"
    if magic in (b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca", b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca"):
        return "macos"
    raise ValueError("host is not a recognized ELF/PE/Mach-O executable")

def package(project: Path, output: Path, *, executable: Path | None = None,
            target: str | None = None, data_only: bool = False) -> dict:
    p = load(project)
    require(all(c["adapter"] == "crysirror-script-v1" for c in p["clients"]), "native packaging requires complete resource inventory; declared aliases are not an exhaustive bytecode asset scan")
    inventory = asset_inventory(project, p)
    require((executable is not None) != data_only, "choose an executable or explicit --data-only")
    files: dict[str, tuple[bytes, int]] = {}
    files["licenses/CrySirror-MPL-2.0.txt"] = ((ROOT / "LICENSE").read_bytes(), 0o644)
    files["licenses/CrySirror-NOTICE.md"] = ((ROOT / "NOTICE.md").read_bytes(), 0o644)
    files["licenses/RELEASE-CHECKLIST.txt"] = (
        b"This is an unverified development package, not a signed release.\n"
        b"Audit licenses/notices of resolved third-party dependencies and game assets.\n"
        b"Check dynamic library requirements on each supported target.\n"
        b"Executable magic is a format check, not an execution test.\n"
        b"Ensure included engine sources actually correspond to the chosen executable.\n", 0o644)
    files["game/project.json"] = (json.dumps(p, ensure_ascii=False, indent=2).encode()+b"\n", 0o644)
    total = 0
    for name, source in inventory.items():
        total += source.stat().st_size
        require(total <= MAX_TOTAL, "package assets exceed 512 MiB")
        files[f"game/{logical_path(p['assets'])}/{name}"] = (source.read_bytes(), 0o644)
    if executable is not None:
        kind = executable_kind(executable)
        require(target is None or kind == target, "executable format does not match target")
        target = kind
        require(executable.stat().st_size <= 512*1024*1024, "host executable exceeds 512 MiB")
        # Include our current engine sources, never third-party/SDK/font binaries.
        source_paths = [ROOT / "Cargo.toml", ROOT / "rust-toolchain.toml", ROOT / "LICENSE"]
        source_paths += sorted(p for p in (ROOT / "crates").rglob("*")
                               if p.is_file() and (p.suffix in (".rs", ".toml", ".wgsl") or p.name in ("UPSTREAM.md", "upstream-files.json")))
        if (ROOT / "Cargo.lock").is_file():
            source_paths.append(ROOT / "Cargo.lock")
        for source in source_paths:
            files[f"source/crysirror/{source.relative_to(ROOT).as_posix()}"] = (source.read_bytes(), 0o644)
        exe_name = "crysirror.exe" if kind == "windows" else "crysirror"
        files[f"bin/{exe_name}"] = (executable.read_bytes(), 0o755)
        if kind == "windows":
            launcher = '@echo off\r\n"%~dp0bin\\crysirror.exe" run "%~dp0game\\project.json" %*\r\n'
            files["launch.cmd"] = (launcher.encode(), 0o644)
        else:
            launcher = '#!/bin/sh\nset -eu\nROOT="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"\nexec "$ROOT/bin/crysirror" run "$ROOT/game/project.json" "$@"\n'
            files["launch.sh"] = (launcher.encode(), 0o755)
    else:
        target = "data-only"
    manifest = {"schema": 1, "name": p["name"], "runtime_included": executable is not None,
                "target": target, "native_vm_compatibility_tested": False,
                "runtime_execution_verified": False, "third_party_notices_audited": False,
                "source_matches_binary_verified": False,
                "files": {name: {"sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}
                          for name, (data, _) in sorted(files.items())}}
    files["package-manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True).encode()+b"\n", 0o644)
    output.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix=".crysirror-package-", suffix=".zip", dir=output.parent)
    os.close(fd)
    try:
        with zipfile.ZipFile(temp_name, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            for name, (data, mode) in sorted(files.items()):
                info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = (0o100000 | mode) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                z.writestr(info, data)
        os.replace(temp_name, output)
    finally:
        if os.path.exists(temp_name):
            os.unlink(temp_name)
    return manifest

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--executable", type=Path)
    parser.add_argument("--target", choices=("windows", "linux", "macos"))
    parser.add_argument("--data-only", action="store_true")
    args = parser.parse_args()
    try:
        result = package(args.project, args.output, executable=args.executable, target=args.target, data_only=args.data_only)
        print(json.dumps({"output": str(args.output), "runtime_included": result["runtime_included"], "files": len(result["files"])}))
    except (ValueError, OSError, TypeError) as exc:
        parser.exit(1, f"CrySirror: {exc}\n")
if __name__ == "__main__":
    main()
