#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Create an original synthetic G00 resource demo in a selected output folder.

This uses the built-in data VM to exercise Siglus resources, not Siglus bytecode.
No game/font data is read. Usage: python3 create_resource_fixture.py /tmp/siglus-demo
"""
import json
from pathlib import Path
import struct
import sys

def main():
    root = Path(sys.argv[1])
    root.mkdir(parents=True, exist_ok=False)
    (root/"assets").mkdir()
    bgr = bytes([30, 210, 40, 20, 120, 230, 200, 80, 30, 60, 190, 220])
    # Type 0: 2x2 pixels, four literal BGR values, opaque implicit alpha.
    payload = bytes([15]) + bgr
    g00 = bytes([0]) + struct.pack("<HHII", 2, 2, 8 + len(payload), 16) + payload
    (root/"assets/panel.g00").write_bytes(g00)
    arena = Path(__file__).resolve().parents[1]/"desktop/arena.json"
    project = json.loads(arena.read_text())
    project["name"] = "Siglus G00 resource slice (data VM)"
    project["canvas"] = [640, 360]
    project["clients"][0]["program"] = [
        {"op":"sprite", "id":0, "asset":"panel.g00", "rect":[16,16,96,96], "z":1}, {"op":"halt"}
    ]
    (root/"project.json").write_text(json.dumps(project, indent=2)+"\n")
    print(root/"project.json")

if __name__ == "__main__":
    main()
