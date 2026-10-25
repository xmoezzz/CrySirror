#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Fetch pinned upstream worktrees and audit Cargo dependency boundaries.

Never builds or executes fetched code, never modifies the original repositories,
never replaces an existing checkout, and never claims that fetching wires a VM.
"""
from __future__ import annotations
import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
URLS = {
    "rfvp": "https://github.com/xmoezzz/rfvp.git",
    "siglus_rs": "https://github.com/xmoezzz/siglus_rs.git",
}
HOST_DEPS = {"wgpu", "winit", "kira", "cpal", "rodio", "egui", "egui-wgpu", "softbuffer"}

class AuditError(ValueError):
    pass

def run(args: list[str], *, cwd: Path | None = None) -> str:
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True, timeout=180, check=False)
    if result.returncode:
        raise AuditError(f"command failed ({result.returncode}): {args[0]} {args[1]}\n{result.stderr.strip()}")
    return result.stdout.strip()

def validate_config(config: dict[str, Any]) -> None:
    if type(config.get("schema")) is not int or config["schema"] != 1 or config.get("console_targets") != ["switch", "psv"]:
        raise AuditError("invalid schema or console scope")
    sources = config.get("sources")
    if not isinstance(sources, dict) or set(sources) != set(URLS):
        raise AuditError("expected exactly rfvp and siglus_rs sources")
    for name, source in sources.items():
        if source.get("url") != URLS[name]:
            raise AuditError(f"unexpected URL for {name}")
        ref = source.get("ref", "")
        if not isinstance(ref, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]{0,199}", ref):
            raise AuditError("invalid upstream ref")
        if ".." in ref or ref.endswith("/"):
            raise AuditError("invalid upstream ref")
        if not isinstance(source.get("focus_manifests"), list) or not source["focus_manifests"]:
            raise AuditError("missing focus manifests")
        for seam in source["focus_manifests"] + source.get("candidate_seams", []):
            p = Path(seam)
            if p.is_absolute() or ".." in p.parts:
                raise AuditError("invalid candidate seam")

def fetch(config: dict[str, Any], source_root: Path | None = None) -> dict[str, Any]:
    validate_config(config)
    target_root = source_root if source_root is not None else ROOT / "upstream"
    if target_root.is_symlink():
        raise AuditError("upstream directory must not be a symlink")
    target_root.mkdir(parents=True, exist_ok=True)
    lock_path = ROOT / "upstream-lock.json"
    old = json.loads(lock_path.read_text()) if lock_path.exists() else {"sources": {}}
    result: dict[str, Any] = {"schema": 1, "fetched_at": dt.datetime.now(dt.timezone.utc).isoformat(), "sources": dict(old.get("sources", {}))}
    for name, source in config["sources"].items():
        target = target_root / name
        previous = old.get("sources", {}).get(name)
        if target.exists() or target.is_symlink():
            if target.is_symlink() or not previous:
                raise AuditError(f"refusing to replace untracked checkout: {target}")
            sha = run(["git", "rev-parse", "HEAD"], cwd=target)
            origin = run(["git", "remote", "get-url", "origin"], cwd=target)
            if sha != previous.get("commit") or origin != source["url"] or run(["git", "status", "--porcelain"], cwd=target):
                raise AuditError(f"checkout differs from lock or is dirty: {target}")
        else:
            ref = previous["commit"] if previous else source["ref"]
            staging = Path(tempfile.mkdtemp(prefix=f".{name}-", dir=target_root))
            try:
                run(["git", "init", "-q", str(staging)])
                run(["git", "remote", "add", "origin", source["url"]], cwd=staging)
                run(["git", "-c", "protocol.file.allow=never", "fetch", "--depth=1", "--no-tags", "origin", ref], cwd=staging)
                run(["git", "-c", "core.hooksPath=/dev/null", "checkout", "--detach", "FETCH_HEAD"], cwd=staging)
                sha = run(["git", "rev-parse", "HEAD"], cwd=staging)
                if not re.fullmatch(r"[a-f0-9]{40,64}", sha):
                    raise AuditError("unexpected commit ID")
                staging.rename(target)
            finally:
                if staging.exists():
                    shutil.rmtree(staging)
        result["sources"][name] = {"url": source["url"], "commit": sha, "requested_ref": source["ref"]}
        # Persist after each successful source, so a later network failure can
        # resume without treating an already-fetched checkout as untracked.
        temporary = lock_path.with_suffix(".json.tmp")
        temporary.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        os.replace(temporary, lock_path)
    return result

def read_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as stream:
        return tomllib.load(stream)

def inside(path: Path, root: Path) -> bool:
    try:
        path.resolve().relative_to(root.resolve())
        return True
    except ValueError:
        return False

def dependency_tables(manifest: dict[str, Any]):
    for section in ("dependencies", "build-dependencies", "dev-dependencies"):
        yield section, manifest.get(section, {})
    for condition, table in manifest.get("target", {}).items():
        for section in ("dependencies", "build-dependencies", "dev-dependencies"):
            yield f"target.{condition}.{section}", table.get(section, {})

def audit_repository(repo: Path, config: dict[str, Any]) -> dict[str, Any]:
    if not repo.is_dir():
        raise AuditError(f"source checkout missing: {repo}; fetch it first")
    workspace = read_toml(repo / "Cargo.toml").get("workspace", {})
    inherited = workspace.get("dependencies", {})
    candidates = sorted((repo / "crates").glob("*/Cargo.toml"))
    packages: dict[str, Any] = {}
    by_path = {}
    for manifest_path in candidates:
        if not inside(manifest_path, repo):
            raise AuditError(f"manifest escapes checkout: {manifest_path}")
        manifest = read_toml(manifest_path)
        package = manifest.get("package", {})
        name = package.get("name")
        if not isinstance(name, str):
            continue
        if name in packages:
            raise AuditError(f"duplicate package name: {name}")
        by_path[manifest_path.parent.resolve()] = name
        packages[name] = {
            "manifest": str(manifest_path.relative_to(repo)), "raw": manifest,
            "directory": manifest_path.parent, "local_dependencies": [], "host_dependencies": [],
            "unresolved_paths": [],
        }
    for name, info in packages.items():
        for section, table in dependency_tables(info["raw"]):
            for alias, spec in table.items():
                base = info["directory"]
                if isinstance(spec, dict) and spec.get("workspace"):
                    if alias not in inherited:
                        raise AuditError(f"missing workspace dependency {alias}")
                    local_options = {k: v for k, v in spec.items() if k != "workspace"}
                    inherited_spec = inherited[alias]
                    inherited_spec = inherited_spec if isinstance(inherited_spec, dict) else {"version": inherited_spec}
                    spec = dict(inherited_spec, **local_options)
                    base = repo
                spec = spec if isinstance(spec, dict) else {"version": spec}
                real_name = spec.get("package", alias)
                if real_name in HOST_DEPS:
                    info["host_dependencies"].append({"package": real_name, "section": section, "optional": bool(spec.get("optional", False))})
                if "path" in spec:
                    dep_path = (base / spec["path"]).resolve()
                    if not inside(dep_path, repo):
                        raise AuditError(f"dependency escapes checkout: {name} -> {alias}")
                    local = by_path.get(dep_path)
                    if local:
                        info["local_dependencies"].append(local)
                    else:
                        info["unresolved_paths"].append(str(dep_path.relative_to(repo.resolve())))
    focus = []
    for relative in config["focus_manifests"]:
        path = repo / relative
        if not inside(path, repo):
            raise AuditError("focus manifest escapes checkout")
        package = by_path.get(path.parent.resolve())
        if path.name != "Cargo.toml" or package is None:
            raise AuditError(f"focus manifest absent; inspect upstream changes: {relative}")
        focus.append(package)
    selected: set[str] = set()
    queue = list(focus)
    while queue:
        name = queue.pop()
        if name in selected:
            continue
        selected.add(name)
        queue.extend(packages[name]["local_dependencies"])
    report = {}
    for name in sorted(selected):
        info = packages[name]
        report[name] = {key: value for key, value in info.items() if key not in ("raw", "directory")}
    return {
        "scope": "static manifest audit; all optional/target/dev path edges are included conservatively",
        "native_vm_connected": False,
        "packages": report,
        "candidate_seams": {seam: (repo / seam).exists() for seam in config.get("candidate_seams", [])},
        "unselected_package_count": len(packages) - len(selected),
    }

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("validate-config", "fetch", "audit"))
    parser.add_argument("--source-root", type=Path, default=ROOT / "upstream")
    args = parser.parse_args()
    try:
        config = json.loads((ROOT / "upstreams.json").read_text(encoding="utf-8"))
        validate_config(config)
        if args.action == "validate-config":
            output = {"valid": True, "console_targets": config["console_targets"]}
        elif args.action == "fetch":
            output = fetch(config, args.source_root)
        else:
            output = {name: audit_repository(args.source_root / name, source) for name, source in config["sources"].items()}
        print(json.dumps(output, indent=2))
    except (AuditError, OSError, ValueError, subprocess.TimeoutExpired) as exc:
        parser.exit(1, f"upstream operation failed: {exc}\n")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
