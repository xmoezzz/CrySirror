#!/usr/bin/env python3
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
"""Run actual checks; absence of Rust is a failure, never a fake pass."""
import argparse
from pathlib import Path
import shutil
import subprocess
import sys
ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gpu", action="store_true")
    parser.add_argument("--no-audio", action="store_true")
    parser.add_argument("--fmt", action="store_true", help="Also check formatting without rewriting pinned sources")
    args = parser.parse_args()
    subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-v"], cwd=ROOT, check=True)
    if not shutil.which("cargo"):
        parser.exit(2, "Rust checks NOT RUN: cargo is not installed. Python checks are not a substitute.\n")
    extra = ["--no-default-features"] if args.no_audio else []
    steps = [
        ["cargo", "check", "--release", "--locked", "--workspace", "--all-targets", *extra],
        ["cargo", "test", "--release", "--locked", "--workspace", "--all-targets", *extra],
    ]
    if args.fmt:
        steps.insert(0,["cargo", "fmt", "--all", "--", "--check"])
    if args.gpu:
        steps.append(["cargo", "test", "--release", "--locked", "-p", "crysirror-wgpu", "--test", "gpu_smoke", "--", "--ignored", "--nocapture"])
    for command in steps:
        subprocess.run(command, cwd=ROOT, check=True)
if __name__ == "__main__":
    main()
