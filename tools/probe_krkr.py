#!/usr/bin/env python3
# Copyright (c) 2026 CrySirror contributors.
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
"""CPU-only local KiriKiri (HXV4 or CXDEC) probe through public Resources and native APIs.

Never launches the game EXE/DLL. The temporary project sits beside the game
folder so the normal asset-root containment check remains enabled. Logs/state
stay in the explicit output directory; original game files are read only.
No window, GPU, renderer or PNG capture is requested. Completed VM ticks do not
establish whole-game compatibility.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def _trace_report(text):
    """Find a complete runtime report among logs, including pretty JSON."""
    decoder = json.JSONDecoder()
    found = None
    offset = 0
    for line in text.splitlines(keepends=True):
        stripped = line.lstrip()
        if stripped.startswith("{"):
            start = offset + len(line) - len(stripped)
            try:
                value, size = decoder.raw_decode(text[start:])
            except json.JSONDecodeError:
                pass
            else:
                if (isinstance(value, dict) and isinstance(value.get("runtime"), dict)
                        and isinstance(value["runtime"].get("sessions"), list)):
                    found = value, start, start + size
        offset += len(line)
    return found


def _companion(game, name):
    """A capability-relative companion inside the game directory, never an ambient path."""
    parts = name.split("/")
    if not name or "\\" in name or ":" in name or any(p in ("", ".", "..") for p in parts):
        raise ValueError(f"companion must be a relative path inside the game directory: {name!r}")
    if not (game / name).is_file():
        raise ValueError(f"companion absent: {name}")
    return name


def probe(game, executable, binary, output, *, font=None, frames=300, canvas=(1280, 720),
          protection="hxv4", modules=()):
    game, binary, output = Path(game).resolve(), Path(binary).resolve(), Path(output).resolve()
    if not game.is_dir() or not (game / "data.xp3").is_file():
        raise ValueError("game must contain data.xp3")
    # A companion name, never an ambient path. Runtime applies the same check.
    if not executable or any(x in executable for x in ("/", "\\", ":")) or executable in (".", ".."):
        raise ValueError("executable must be a basename inside the game directory")
    if not (game / executable).is_file() or not binary.is_file():
        raise ValueError("explicit companion executable or CrySirror binary absent")
    if not 1 <= frames <= 100000 or len(canvas) != 2 or any(not 1 <= n <= 8192 for n in canvas):
        raise ValueError("invalid frame/canvas bounds")
    if protection not in ("hxv4", "cxdec"):
        raise ValueError("protection must be hxv4 or cxdec")
    output.mkdir(parents=True, exist_ok=True)
    if protection == "hxv4":
        startup = "__bootstrap/startup.tjs"
        bootstrap_profile = "__bootstrap/protection.json"
        config = dict(canvas=list(canvas), startup=startup, bootstrap_profile=bootstrap_profile)
        options = dict(hxv4_executable=executable, hxv4_startup_target=startup,
                       hxv4_bootstrap_profile=bootstrap_profile, hxv4_hashed_root="__hxv4")
        mounts = [("data.xp3", "")]
    else:
        # Parameters are recovered by the engine from these user-owned modules
        # at mount time; nothing title-specific is passed or stored here.
        companions = [_companion(game, m) for m in (modules or [executable])]
        # The startup archive publishes the validated setupArchiveData profile.
        bootstrap_profile = "__cxdec/protection.json"
        config = dict(canvas=list(canvas), startup="startup.tjs", executable=executable,
                      bootstrap_profile=bootstrap_profile)
        options = dict(cxdec_modules=companions, cxdec_bootstrap_profile=bootstrap_profile)
        # data.xp3 is the root; every other archive is a shared container that
        # native `name.xp3>` storages resolve through its mount point.
        mounts = [("data.xp3", "")] + [(p.name, f"__xp3/{p.stem.lower()}")
                                       for p in sorted(game.glob("*.xp3")) if p.name != "data.xp3"]
    project = dict(schema=2, name="Local KiriKiri resource probe", content_revision="1",
                   assets=game.name, canvas=list(canvas), clients=[dict(
                       name="game", game="local-krkr-probe", adapter="krkr", config=config)])
    profile = dict(limits=dict(max_entry_size=256 * 1024 * 1024, max_index_size=64 * 1024 * 1024,
                               max_entries=1_000_000), parameters={"kirikiri/xp3": options})
    script = output / "mount.ts"
    lines = ["function onStart():void {", f" resources.defineProfile('{protection}',{json.dumps(profile)});"]
    lines += [f" resources.mount({json.dumps(src)},{json.dumps(point)},{{format:'kirikiri/xp3',profile:'{protection}'}});"
              for src, point in mounts]
    lines += [f" engine.log('shared {protection.upper()} resources mounted');", "}", ""]
    script.write_text("\n".join(lines))
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", prefix=".crysirror-krkr-",
                                     suffix=".json", dir=game.parent, delete=False) as f:
        json.dump(project, f)
        project_path = Path(f.name)
    log_path = output / "trace.log"
    command = [str(binary), "trace", str(project_path), "--script", str(script),
               "--script-client", "game", "--frames", str(frames), "--data-dir", str(output / "state"),
               ]
    if font:
        command += ["--font", str(Path(font).resolve())]
    try:
        with log_path.open("w") as log:
            try:
                result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=300, check=False)
                code = result.returncode
            except subprocess.TimeoutExpired:
                log.write("\nCPU probe timed out after 60 seconds\n")
                code = 124
    finally:
        project_path.unlink(missing_ok=True)
    text = log_path.read_text(errors="replace")
    parsed = _trace_report(text)
    report = parsed[0] if parsed else None
    diagnostics = (text[:parsed[1]] + text[parsed[2]:]) if parsed else text
    summary = dict(exit_code=code, log=str(log_path), execution="cpu-trace",
                   bundled_plugins=sorted(p.name for p in (game / "plugin").glob("*.dll")),
                   native_sessions=report["runtime"]["sessions"] if report else [],
                   trace_report_available=report is not None,
                   diagnostic=[line for line in diagnostics.splitlines() if line.strip()][-12:],
                   rendered_frame=None,
                   whole_game_compatibility="not established")
    (output / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("game", type=Path)
    parser.add_argument("--exe", required=True, help="explicit companion executable basename")
    parser.add_argument("--protection", choices=("hxv4", "cxdec"), default="hxv4")
    parser.add_argument("--module", action="append", default=[],
                        help="CXDEC companion module (EXE/DLL/TPM) relative to the game; repeatable, default --exe")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/crysirror")
    parser.add_argument("--output", type=Path, default=ROOT / "target/verification/krkr-probe")
    parser.add_argument("--font", type=Path)
    parser.add_argument("--frames", type=int, default=300)
    parser.add_argument("--canvas", nargs=2, type=int, default=(1280, 720))
    args = parser.parse_args()
    summary = probe(args.game, args.exe, args.binary, args.output,
                    font=args.font, frames=args.frames, canvas=args.canvas,
                    protection=args.protection, modules=args.module)
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return summary["exit_code"]


if __name__ == "__main__":
    raise SystemExit(main())
