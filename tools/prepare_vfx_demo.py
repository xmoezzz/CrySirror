#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Generate our procedural VFX sheet; copy optional, already acquired demo assets.

No game files, fonts, UE source or network access. All generated/copied files
stay under the gitignored aaa-vfx/assets directory.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct
import zlib


def sheet() -> bytes:
    """Eight independent, deterministic RGBA fire/smoke frames (4 by 2)."""
    cell, columns, rows = 64, 4, 2
    width, height = cell * columns, cell * rows
    pixels = bytearray()
    for y in range(height):
        pixels.append(0)  # PNG row filter: None
        for x in range(width):
            frame = (y // cell) * columns + x // cell
            u, v = ((x % cell + 0.5) / cell * 2 - 1), ((y % cell + 0.5) / cell * 2 - 1)
            swirl = 0.06 * math.sin(v * 9 + frame * 0.8)
            radius = math.hypot((u + swirl) * (1.0 + frame * 0.035), v)
            alpha = max(0, 1 - radius) ** 1.8
            glow = max(0, 1 - radius * 1.6)
            pixels.extend((255, round(110 + 120 * glow), round(35 + 150 * glow), round(255 * alpha)))

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(pixels, 9)) + chunk(b"IEND", b"")


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--helmet-assets", type=Path, default=root / "examples/aaa-helmet/assets")
    args = parser.parse_args()
    out = root / "examples/aaa-vfx/assets"
    out.mkdir(parents=True, exist_ok=True)
    (out / "flame-sheet.png").write_bytes(sheet())
    entries = [{"file": "flame-sheet.png", "license": "MPL-2.0", "attribution": "CrySirror procedural texture (prepare_vfx_demo.py)"}]
    upstream = json.loads((args.helmet_assets / "THIRD_PARTY.json").read_text())
    for name in ("DamagedHelmet.glb", "royal_esplanade_2k.hdr"):
        source = args.helmet_assets / name
        attribution = next(item for item in upstream if item["file"] == name)
        digest = hashlib.sha256(source.read_bytes()).hexdigest()
        if digest != attribution["sha256"]:
            raise ValueError(f"source manifest hash mismatch: {source}")
        shutil.copyfile(source, out / name)
        entries.append(attribution)
    for entry in entries:
        data = (out / entry["file"]).read_bytes()
        entry.update(sha256=hashlib.sha256(data).hexdigest(), bytes=len(data))
    (out / "THIRD_PARTY.json").write_text(json.dumps(entries, indent=2) + "\n")
    print(json.dumps({"directory": str(out), "assets": entries}, indent=2))


if __name__ == "__main__":
    main()
