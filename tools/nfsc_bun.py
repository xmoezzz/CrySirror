#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Inspect Need for Speed: Carbon (PC) chunked BUN/BIN files (ADR 0042).

Reads the owner's installer archives (`0compressed.zip`/`1compressed.zip` on
the mounted disc) directly; nothing is extracted to disk. This is CrySirror's
own implementation. Chunk ids and the 32-bit `streamer::section` layout
follow the BSD-3 hyperlinked headers (MaxHwoy/hyperlinked @ f1f3fbfd,
src/hyperlib/chunk.hpp, src/hyperlib/streamer/streamer.hpp); no code is
copied from them or from GPL tools.

Chunk framing: little-endian (id: u32, size: u32) followed by `size` bytes;
ids with bit 31 set are containers of further chunks. Unknown ids are
reported, never interpreted.
"""
from __future__ import annotations
import argparse, collections, json, struct, sys, zipfile, zlib
from pathlib import Path

NAMES = {
    0x00034110: "track_streaming_sections", 0x00034111: "track_streaming_infos",
    0x00034112: "track_streaming_barriers", 0x00034113: "track_streaming_discs",
    0x00034201: "track_infos", 0x00034202: "sun_infos",
    0x80034100: "scenery_section", 0x00034101: "scenery_header", 0x00034102: "scenery_infos",
    0x00034103: "scenery_instances", 0x00034105: "scenery_tree_nodes",
    0x8003410B: "model_hierarchy_tree", 0x80034150: "visible_section_manager",
    0x00034152: "visible_section_boundaries", 0x00034153: "visible_section_drivables",
    0x80036000: "event_trigger_pack", 0x8003B600: "parameter_maps",
    0x8003B810: "event_systems", 0x8003B900: "collision_volumes",
    0x0003B800: "world_road_network", 0x0003B801: "world_collision_assets",
    0x00037220: "world_anim_header", 0x00037240: "world_anim_frames",
    0xB3300000: "texture_pack", 0xB3310000: "texture_pack_info", 0xB3320000: "texture_pack_vram",
    0x33310001: "texture_pack_info_header", 0x33310004: "texture_pack_info_textures",
    0x33320002: "texture_pack_vram_data", 0x00000000: "padding",
}
SECTION_SIZE = 92  # 32-bit streamer::section


class BunError(ValueError):
    pass


def chunks(data: bytes, start: int = 0, end: int | None = None, depth: int = 0):
    """Yields (depth, offset, id, size) for every chunk, depth first."""
    end = len(data) if end is None else end
    off = start
    while off + 8 <= end:
        cid, size = struct.unpack_from("<II", data, off)
        body = off + 8
        if body + size > end:
            raise BunError(f"chunk 0x{cid:08x} at 0x{off:x} overruns its parent (size {size})")
        yield depth, off, cid, size
        if cid & 0x80000000 and depth < 32:
            yield from chunks(data, body, body + size, depth + 1)
        off = body + size
    if off != end and end - off >= 8:
        raise BunError(f"trailing bytes at 0x{off:x}")


def stats(data: bytes) -> dict:
    count = collections.Counter()
    size = collections.Counter()
    for _, _, cid, n in chunks(data):
        count[cid] += 1
        size[cid] += n
    return {
        f"0x{cid:08x}": {"name": NAMES.get(cid, "unknown"), "count": count[cid], "bytes": size[cid]}
        for cid in sorted(count)
    }


def sections(data: bytes) -> list[dict]:
    """Track streaming sections of a track header BUN (e.g. TRACKS/L5RA.BUN)."""
    out = []
    for _, off, cid, n in chunks(data):
        if cid != 0x00034110:
            continue
        if n % SECTION_SIZE:
            raise BunError(f"streaming section block size {n} is not a multiple of {SECTION_SIZE}")
        for i in range(n // SECTION_SIZE):
            b = off + 8 + i * SECTION_SIZE
            name = data[b:b + 8].split(b"\0")[0].decode("ascii", "replace")
            number, = struct.unpack_from("<H", data, b + 8)
            ftype, fofs, fsize, csize, perm, prio = struct.unpack_from("<IIIIIi", data, b + 16)
            cx, cy, radius, checksum = struct.unpack_from("<fffI", data, b + 40)
            out.append({"name": name, "number": number, "type": ftype, "offset": fofs, "size": fsize,
                        "compressed_size": csize, "perm_size": perm, "priority": prio,
                        "center": [round(cx, 2), round(cy, 2)], "radius": round(radius, 2), "checksum": checksum})
    return out


def open_member(archives: list[Path], member: str) -> bytes:
    want = member.replace("/", "\\").upper()
    for a in archives:
        with zipfile.ZipFile(a) as z:
            for info in z.infolist():
                if info.filename.upper() == want:
                    return z.read(info)
    raise BunError(f"{member} not found in {', '.join(map(str, archives))}")


def read_range(archives: list[Path], member: str, offset: int, size: int) -> bytes:
    """A byte range of a (possibly huge) member without holding all of it."""
    want = member.replace("/", "\\").upper()
    for a in archives:
        with zipfile.ZipFile(a) as z:
            for info in z.infolist():
                if info.filename.upper() == want:
                    with z.open(info) as f:
                        f.seek(offset)
                        data = f.read(size)
                        if len(data) != size:
                            raise BunError("short read")
                        return data
    raise BunError(f"{member} not found")


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--disc", type=Path, required=True, help="mounted disc root containing *compressed.zip")
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("stats", help="chunk id histogram of a member")
    s.add_argument("member")
    s = sub.add_parser("sections", help="streaming sections of a track header BUN")
    s.add_argument("member", nargs="?", default="TRACKS/L5RA.BUN")
    s = sub.add_parser("section-stats", help="chunk histogram of one streamed section")
    s.add_argument("number", type=int)
    s.add_argument("--header", default="TRACKS/L5RA.BUN")
    s.add_argument("--stream", default="TRACKS/STREAML5RA.BUN")
    a = p.parse_args(argv)
    archives = sorted(a.disc.glob("*compressed.zip"))
    if not archives:
        raise SystemExit("no *compressed.zip under --disc")
    if a.cmd == "stats":
        print(json.dumps(stats(open_member(archives, a.member)), indent=1))
    elif a.cmd == "sections":
        print(json.dumps(sections(open_member(archives, a.member)), indent=1))
    else:
        sec = [s for s in sections(open_member(archives, a.header)) if s["number"] == a.number]
        if not sec:
            raise SystemExit(f"section {a.number} not in {a.header}")
        s = sec[0]
        data = read_range(archives, a.stream, s["offset"], s["size"])
        print(json.dumps({"section": s, "chunks": stats(data)}, indent=1))


if __name__ == "__main__":
    main()
