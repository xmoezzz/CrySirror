#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Create xmoezzz/CrySirror and push this repository. Requires authenticated gh.

Defaults to PRIVATE. Refuses a different account, an existing origin, a dirty
worktree, or an uninitialized checkout. Does not request or print credentials.
"""
from __future__ import annotations
import argparse
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
REPO = "xmoezzz/CrySirror"

def checked(args: list[str]) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True, stderr=subprocess.STDOUT).strip()

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    visibility = parser.add_mutually_exclusive_group()
    visibility.add_argument("--private", action="store_true", help="default")
    visibility.add_argument("--public", action="store_true", help="explicitly publish source publicly")
    args = parser.parse_args()
    if not shutil.which("gh") or not shutil.which("git"):
        parser.exit(1, "Install GitHub CLI and Git, then run gh auth login locally.\n")
    try:
        if checked(["git", "rev-parse", "--show-toplevel"]) != str(ROOT):
            parser.exit(1, "Run this from the CrySirror repository cloned from its Git bundle.\n")
        if checked(["git", "status", "--porcelain"]):
            parser.exit(1, "Commit or discard local changes before publication.\n")
        if "origin" in checked(["git", "remote"]).splitlines():
            parser.exit(1, "An origin remote already exists; inspect it instead of creating another repository.\n")
        login = checked(["gh", "api", "user", "--jq", ".login"])
        if login.casefold() != "xmoezzz":
            parser.exit(1, f"Authenticated account is {login!r}, not xmoezzz. No repository was created.\n")
        flag = "--public" if args.public else "--private"
        # gh refuses to create an already-existing repository. No force-push.
        subprocess.run(["gh", "repo", "create", REPO, flag, "--source=.", "--remote=origin",
                        "--description", "Data-driven runtime with RFVP and Siglus compatibility boundaries"],
                       cwd=ROOT, check=True)
        print(f"Created {REPO}. Pushing main...")
        subprocess.run(["git", "push", "-u", "origin", "main"], cwd=ROOT, check=True)
        print(f"Pushed {REPO} successfully.")
    except subprocess.CalledProcessError as exc:
        # Creation and push are separate operations. A push failure may leave the
        # repository created; never incorrectly report that nothing changed.
        parser.exit(1, f"Publication stopped (exit {exc.returncode}). Inspect gh/git state before retrying.\n")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
