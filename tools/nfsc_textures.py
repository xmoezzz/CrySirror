#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""NFS Carbon (PC) texture packs (TPK) -> DDS, for the engine's own codecs.

CrySirror's own decoder (ADR 0042). The texture::info field order follows
the BSD-3 hyperlinked header (src/hyperlib/assets/textures.hpp); the
serialized record is the 88-byte fixed part followed by a length-prefixed
name padded to 4 bytes, established from the data:

* `0xB3310000` info: `0x33310001` pack header (name, file name),
  `0x33310004` texture records, `0x33310005` per-texture platform records
  (24 bytes, FourCC at +12).
* `0xB3320000` VRAM: `0x33320002` image data; 0x11 fill up to a 128-byte
  file boundary, then each texture at its `image_placement`.

Record fields used: key +12, image placement +20, image size +28,
width/height +40/+42, compression +46 (34 DXT1, 36 DXT3, 38 DXT5,
32 RGBA8), mip count +50, tiling +51, alpha usage +57 (1 punch-through,
2 modulated), blend type +58 (0 opaque copy, 1 blend, 2 additive, ...).
Other compressions are reported and skipped.
"""
from __future__ import annotations
import struct, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import nfsc_bun as bun

FOURCC = {34: b"DXT1", 36: b"DXT3", 38: b"DXT5"}
ALPHA = {0: "none", 1: "punchthru", 2: "modulated"}
# texture::alpha_blend_type; 0 (src_copy) draws opaque even when the alpha
# channel is "modulated" (terrain and cliffs keep a mask there).
BLEND = {0: "src_copy", 1: "blend", 2: "additive", 3: "subtractive", 4: "overbright",
         5: "dest_blend", 6: "dest_additive", 7: "dest_subtractive", 8: "dest_overbright"}


class TextureError(ValueError):
    pass


def _children(data: bytes, off: int, size: int):
    o, end = off + 8, off + 8 + size
    while o + 8 <= end:
        cid, n = struct.unpack_from("<II", data, o)
        yield cid, o, n
        o += 8 + n


def packs(data: bytes):
    """Yields (pack_name, [texture dict]) for every TPK in a buffer; each
    texture carries `image` (all mips) when its format is supported."""
    for _, off, cid, size in bun.chunks(data):
        if cid != 0xB3300000:
            continue
        kids = list(_children(data, off, size))
        info = [k for k in kids if k[0] == 0xB3310000]
        vram = [k for k in kids if k[0] == 0xB3320000]
        if len(info) != 1 or len(vram) != 1:
            raise TextureError(f"TPK at 0x{off:x}: {len(info)} info / {len(vram)} vram blocks")
        ik = list(_children(data, info[0][1], info[0][2]))
        header = [k for k in ik if k[0] == 0x33310001]
        name = data[header[0][1] + 12:header[0][1] + 40].split(b"\0")[0].decode("ascii", "replace") if header else ""
        recs = [k for k in ik if k[0] == 0x33310004]
        vk = [k for k in _children(data, vram[0][1], vram[0][2]) if k[0] == 0x33320002]
        if len(recs) != 1 or len(vk) != 1:
            raise TextureError(f"TPK {name}: missing texture records or image data")
        body = data[recs[0][1] + 8:recs[0][1] + 8 + recs[0][2]]
        vstart = vk[0][1] + 8
        base = (vstart + 127) & ~127
        if data[vstart:base].strip(b"\x11"):
            raise TextureError(f"TPK {name}: unexpected VRAM alignment fill")
        vend = vk[0][1] + 8 + vk[0][2]
        out, o = [], 0
        while o + 89 <= len(body):
            key, = struct.unpack_from("<I", body, o + 12)
            place, = struct.unpack_from("<i", body, o + 20)
            isize, = struct.unpack_from("<i", body, o + 28)
            w, h = struct.unpack_from("<HH", body, o + 40)
            comp, mips, tile = body[o + 46], body[o + 50], body[o + 51]
            alpha, blend = body[o + 57], body[o + 58]
            nlen = body[o + 88]
            tname = body[o + 89:o + 89 + nlen].split(b"\0")[0].decode("ascii", "replace")
            o += 88 + ((1 + nlen + 3) & ~3)
            t = {"key": key, "name": tname, "width": w, "height": h, "compression": comp, "mips": max(1, mips),
                 "tiling": tile, "alpha": ALPHA.get(alpha, f"unknown{alpha}"), "blend": BLEND.get(blend, f"unknown{blend}"),
                 "pack": name}
            if not (0 < w <= 4096 and 0 < h <= 4096):
                raise TextureError(f"TPK {name}: texture {tname} size {w}x{h}")
            if place < 0 or base + place + isize > vend:
                raise TextureError(f"TPK {name}: texture {tname} data outside the pack")
            if comp in FOURCC or comp == 32:
                t["image"] = data[base + place:base + place + isize]
            out.append(t)
        if o != len(body) and body[o:].strip(b"\0"):
            raise TextureError(f"TPK {name}: {len(body) - o} trailing record bytes")
        yield name, out


def _level(w: int, h: int, comp: int) -> int:
    if comp == 32:
        return w * h * 4
    return max(1, (w + 3) // 4) * max(1, (h + 3) // 4) * (8 if comp == 34 else 16)


def dds(t: dict, max_size: int = 1024) -> bytes:
    """A DDS file of one texture, dropping mips above `max_size`."""
    w, h, comp, mips, img = t["width"], t["height"], t["compression"], t["mips"], t["image"]
    expected = sum(_level(max(1, w >> i), max(1, h >> i), comp) for i in range(mips))
    if len(img) < expected:
        # Some packs store only the top level; accept a single complete level.
        if len(img) >= _level(w, h, comp):
            mips = 1
        else:
            raise TextureError(f"{t['name']}: {len(img)} bytes for {w}x{h} {mips} mips")
    while max(w, h) > max_size and mips > 1:
        img = img[_level(w, h, comp):]
        w, h, mips = max(1, w // 2), max(1, h // 2), mips - 1
    img = img[:sum(_level(max(1, w >> i), max(1, h >> i), comp) for i in range(mips))]
    flags = 0x1007 | (0x20000 if mips > 1 else 0) | (0x80000 if comp != 32 else 0x8)
    hdr = struct.pack("<4sIIIIIII", b"DDS ", 124, flags, h, w, _level(w, h, comp) if comp != 32 else w * 4, 0, mips)
    hdr += b"\0" * 44
    if comp == 32:
        # BGRA8 (D3DFMT_A8R8G8B8)
        pf = struct.pack("<II4sIIIII", 32, 0x41, b"\0\0\0\0", 32, 0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000)
    else:
        pf = struct.pack("<II4sIIIII", 32, 0x4, FOURCC[comp], 0, 0, 0, 0, 0)
    caps = struct.pack("<IIIII", 0x1000 | (0x400008 if mips > 1 else 0), 0, 0, 0, 0)
    return hdr + pf + caps + img
