#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Validate synthetic service-contract project data. Python 3.11+, no dependencies.

This checks data, not execution of the Rust engine or either upstream VM.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
from typing import Any

MAX_PROJECT = 4 * 1024 * 1024

class ValidationError(ValueError):
    pass

def fail(message: str) -> None:
    raise ValidationError(message)

def obj(value: Any, fields: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{label}: expected object")
    if set(value) != fields:
        fail(f"{label}: missing={sorted(fields-set(value))}, extra={sorted(set(value)-fields)}")
    return value

def integer(value: Any, low: int, high: int, label: str) -> None:
    if type(value) is not int or not low <= value <= high:
        fail(f"{label}: expected integer in {low}..{high}")

def string(value: Any, label: str, *, empty: bool = False, maximum: int = 256) -> None:
    if not isinstance(value, str) or (not empty and not value) or len(value.encode("utf-8")) > maximum:
        fail(f"{label}: invalid string")

def asset_path(value: Any) -> None:
    string(value, "asset", maximum=4096)
    value = value.replace("\\", "/")
    if value.startswith("/") or ":" in value or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value):
        fail("asset: host path or control character")
    if ".." in value.split("/") or not any(p not in ("", ".") for p in value.split("/")):
        fail("asset: empty path or parent traversal")

def validate(project: Any) -> None:
    root = obj(project, {"schema", "name", "console_targets", "clients", "expected"}, "project")
    integer(root["schema"], 1, 1, "schema")
    string(root["name"], "name")
    targets = root["console_targets"]
    if not isinstance(targets, list) or len(targets) != 2 or sorted(map(str, targets)) != ["psv", "switch"]:
        fail("console_targets: exactly switch and psv are supported in scope")
    clients = root["clients"]
    if not isinstance(clients, list) or not 1 <= len(clients) <= 64:
        fail("clients: expected 1..64 entries")
    names = set()
    for client in clients:
        c = obj(client, {"name", "engine", "game", "layout_profile", "commands"}, "client")
        for key in ("name", "game", "layout_profile"):
            string(c[key], key)
        if c["engine"] not in ("rfvp", "siglus"):
            fail("engine: expected rfvp or siglus")
        if c["name"] in names:
            fail("duplicate client name")
        names.add(c["name"])
        if not isinstance(c["commands"], list) or len(c["commands"]) > 4096:
            fail("commands: expected at most 4096 entries")
        for command in c["commands"]:
            validate_command(command)
    expected = obj(root["expected"], {"visuals", "voices"}, "expected")
    integer(expected["visuals"], 0, 4096, "expected visuals")
    integer(expected["voices"], 0, 128, "expected voices")

def validate_command(command: Any) -> None:
    if not isinstance(command, dict):
        fail("command: expected object")
    op = command.get("op")
    visual = {"op", "id", "x", "y", "width", "height", "z", "rgba"}
    if op == "text.put":
        obj(command, visual | {"text", "font", "size_px"}, op)
        string(command["text"], "text", empty=True, maximum=1024*1024)
        string(command["font"], "font")
        integer(command["size_px"], 1, 65535, "size_px")
    elif op == "sprite.put":
        obj(command, visual | {"asset"}, op)
        asset_path(command["asset"])
    elif op == "visual.remove":
        obj(command, {"op", "id"}, op)
    elif op == "audio.play":
        obj(command, {"op", "channel", "asset", "looping", "gain_milli"}, op)
        asset_path(command["asset"])
        if type(command["looping"]) is not bool:
            fail("looping: expected boolean")
        integer(command["gain_milli"], 0, 1000, "gain_milli")
    elif op == "audio.stop":
        obj(command, {"op", "channel"}, op)
    else:
        fail(f"unsupported operation: {op!r}")
    if "id" in command:
        integer(command["id"], 0, 2**32-1, "id")
    if "channel" in command:
        integer(command["channel"], 0, 2**32-1, "channel")
    if op in ("text.put", "sprite.put"):
        for key in ("x", "y", "z"):
            integer(command[key], -(2**31), 2**31-1, key)
        for key in ("width", "height"):
            integer(command[key], 1, 2**32-1, key)
        rgba = command["rgba"]
        if not isinstance(rgba, list) or len(rgba) != 4:
            fail("rgba: expected four channels")
        for v in rgba:
            integer(v, 0, 255, "rgba")

def read_project(path: Path) -> Any:
    with path.open("rb") as source:
        raw = source.read(MAX_PROJECT + 1)
    if len(raw) > MAX_PROJECT:
        fail("project exceeds 4 MiB")
    return json.loads(raw)

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project", type=Path)
    args = parser.parse_args()
    try:
        validate(read_project(args.project))
    except (OSError, ValueError, UnicodeError) as exc:
        parser.exit(1, f"validation failed: {exc}\n")
    print("Project data valid. This is not a native-VM or Rust execution test.")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
