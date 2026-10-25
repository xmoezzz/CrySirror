#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Generate self-authored native image resources for the renderer; no game data."""
import argparse
import json
from pathlib import Path
import struct
import zlib


def generate(root: Path):
    assets = root / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    width = height = 64
    colors = [(240, 40, 60, 255), (30, 210, 90, 255), (40, 90, 230, 255), (245, 210, 40, 255)]
    rgba = bytes(c for y in range(height) for x in range(width) for c in colors[x // 32 + 2 * (y // 32)])
    bgra = bytes(c for i in range(0, len(rgba), 4) for c in (rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]))
    literal = b"".join(bytes([(1 << len(part)) - 1]) + part for part in (bgra[i:i + 8] for i in range(0, len(bgra), 8)))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(b"".join(b"\0" + rgba[y * width * 4:(y + 1) * width * 4] for y in range(height)))) + chunk(b"IEND", b"")
    files = {
        "pattern.teyl": b"TEYL" + struct.pack("<II", width, height) + bgra,
        "pattern.pgx": b"PGX\0" + bytes(4) + struct.pack("<IIHHI", width, height, 1, 0, len(literal)) + literal,
        "pattern.hzc": b"hzc1" + struct.pack("<II", len(bgra), 32) + b"NVSG" + struct.pack("<HHHH", 0, 1, width, height) + bytes(8) + struct.pack("<I", 1) + bytes(8) + zlib.compress(bgra),
        "pattern.png": png,
    }
    table, data = bytearray(), bytearray()
    for name, content in files.items():
        table.extend(name.encode() + b"\0" + struct.pack(">II", len(data), len(content)))
        data.extend(content)
    (assets / "images.sar").write_bytes(struct.pack(">HI", len(files), 6 + len(table)) + table + data)
    layers = [{"id": i + 1, "rect": [x, y, 128, 128], "paint": {"kind": "image", "asset": "pack/" + name}}
              for i, (x, y, name) in enumerate([(0, 0, "pattern.teyl"), (128, 0, "pattern.pgx"), (0, 128, "pattern.hzc"), (128, 128, "pattern.png")])]
    project = {"schema": 2, "name": "Native image archive GPU smoke", "content_revision": "self-authored-1", "assets": "assets", "canvas": [256, 256],
               "clients": [{"name": "game", "adapter": "crysirror-script-v1", "game": "native-images-smoke", "systems": {"layers": layers}}]}
    (root / "project.json").write_text(json.dumps(project, indent=2) + "\n")
    (root / "mount.ts").write_text('function onStart():void { resources.mount("images.sar", "pack", {format:"nscripter/sar"}); }\n')
    return colors


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="destination for these generated test assets")
    args = parser.parse_args()
    print(json.dumps({"expected_rgba": generate(args.output), "root": str(args.output.resolve())}))
