#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
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
    args = parser.parse_args()
    subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-v"], cwd=ROOT, check=True)
    if not shutil.which("cargo"):
        parser.exit(2, "Rust checks NOT RUN: cargo is not installed. Python checks are not a substitute.\n")
    extra = ["--no-default-features"] if args.no_audio else []
    steps = [
        ["cargo", "generate-lockfile"],
        ["cargo", "fmt", "--all"],
        ["cargo", "check", "--locked", "--workspace", "--all-targets", *extra],
        ["cargo", "test", "--locked", "--workspace", "--all-targets", *extra],
    ]
    if args.gpu:
        steps.append(["cargo", "test", "--locked", "-p", "crysirror-wgpu", "--test", "gpu_smoke", "--", "--ignored", "--nocapture"])
    for command in steps:
        subprocess.run(command, cwd=ROOT, check=True)
if __name__ == "__main__":
    main()
