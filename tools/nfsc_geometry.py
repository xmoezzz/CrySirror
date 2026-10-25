#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Decode NFS Carbon (PC) solids from BUN stream sections and export glTF.

CrySirror's own decoder (ADR 0042). Struct layouts follow the BSD-3
hyperlinked runtime structs (geometry::solid, platform_info, mesh_entry at
MaxHwoy/hyperlinked@f1f3fbfd src/hyperlib/assets/geometry.hpp), which match
the serialized chunks once each chunk's data is aligned to 16 bytes in the
file (the gap is 0x11 fill). Everything else was established from the data
and is checked while decoding:

* `0x80134010` solid: `0x00134011` header (name at +160, bbox at +32/+48,
  pivot matrix at +64), `0x00134012` texture keys, `0x80134100` mesh.
* `0x00134900` platform info: submesh count (+16), vertex buffer count (+24).
* `0x00134B02` mesh entries, 144 bytes each: texture slots (+24..+29),
  shader type (+48), flags (+56), vertex count (+64), triangle count (+96),
  index start (+100), index count (+124).
* `0x00134B01` vertex buffers in entry order; an entry consumes
  vertex count x stride bytes and buffers end exactly on entry boundaries.
  Stride by shader type: 0 -> 36 (pos, normal, BGRA8 colour, uv),
  3 -> 52 (the same plus a tangent with sign). Unknown types fail.
* `0x00134B03` u16 indices relative to the entry's vertex buffer.
* `0x00134C02` one material name per entry.

World coordinates are Z-up metres; glTF output is Y-up: (x, y, z) -> (x, z, -y).
"""
from __future__ import annotations
import argparse, base64, json, struct, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import nfsc_bun as bun

# Vertex stride per shader type, measured from buffers that hold one entry.
STRIDE = {0: 36, 1: 52, 3: 52, 4: 48, 5: 48, 15: 44, 25: 36, 26: 36}
# Byte offsets of (normal, BGRA colour, uv). Layout A: pos, normal, colour,
# uv. Layout B (water, decal-like types): pos, colour, uv, normal. Chosen
# because it makes 100% of the sampled normals unit length (all 962 sections).
LAYOUT_A = (12, 24, 28)
LAYOUT_B = (24, 12, 16)
LAYOUT = {0: LAYOUT_A, 1: LAYOUT_A, 3: LAYOUT_A, 4: LAYOUT_A, 15: LAYOUT_A, 5: LAYOUT_B, 25: LAYOUT_B, 26: LAYOUT_B}
ENTRY = 144


class GeometryError(ValueError):
    pass


# Chunks whose data starts on a 16-byte file boundary (0x11 fill before it).
ALIGNED = {0x00134011, 0x00134900, 0x00134B01, 0x00134B02, 0x00134B03}


def _body(data: bytes, off: int, size: int) -> bytes:
    cid, = struct.unpack_from("<I", data, off)
    start = off + 8
    pad = (-start) % 16 if cid in ALIGNED else 0
    if pad > size:
        raise GeometryError(f"chunk 0x{cid:08x} at 0x{off:x} smaller than its alignment")
    if data[start:start + pad].strip(b"\x11"):
        raise GeometryError(f"unexpected alignment fill at 0x{start:x}: {data[start:start + pad].hex()}")
    return data[start + pad:start + size]


def _children(data: bytes, off: int, size: int):
    o, end = off + 8, off + 8 + size
    while o + 8 <= end:
        cid, n = struct.unpack_from("<II", data, o)
        yield cid, o, n
        o += 8 + n


def solids(data: bytes, stats: dict | None = None):
    """Yields decoded solids of one section."""
    stats = stats if stats is not None else {}
    for _, off, cid, size in bun.chunks(data):
        if cid != 0x80134010:
            continue
        kids = list(_children(data, off, size))
        heads = [k for k in kids if k[0] == 0x00134011]
        meshes = [k for k in kids if k[0] == 0x80134100]
        if len(heads) != 1:
            raise GeometryError(f"solid at 0x{off:x}: {len(heads)} headers")
        h = _body(data, heads[0][1], heads[0][2])
        name = h[160:].split(b"\0")[0].decode("ascii", "replace")
        key, = struct.unpack_from("<I", h, 16)
        bmin = struct.unpack_from("<3f", h, 32)
        bmax = struct.unpack_from("<3f", h, 48)
        pivot = struct.unpack_from("<16f", h, 64)
        tex = [k for k in kids if k[0] == 0x00134012]
        texkeys = []
        if tex:
            t = _body(data, tex[0][1], tex[0][2])
            texkeys = [struct.unpack_from("<I", t, i)[0] for i in range(0, len(t) - len(t) % 8, 8)]
        out = {"name": name, "key": key, "bbox": [bmin, bmax], "pivot": pivot, "textures": texkeys, "meshes": []}
        if not meshes:
            stats["solids_without_mesh"] = stats.get("solids_without_mesh", 0) + 1
            yield out
            continue
        mk = list(_children(data, meshes[0][1], meshes[0][2]))
        entries = [k for k in mk if k[0] == 0x00134B02]
        if len(entries) != 1:
            raise GeometryError(f"{name}: {len(entries)} entry tables")
        eb = _body(data, entries[0][1], entries[0][2])
        if len(eb) % ENTRY:
            raise GeometryError(f"{name}: entry table of {len(eb)} bytes is not a multiple of {ENTRY}")
        count = len(eb) // ENTRY
        vbs = [_body(data, k[1], k[2]) for k in mk if k[0] == 0x00134B01]
        ibk = [k for k in mk if k[0] == 0x00134B03]
        ib = _body(data, ibk[0][1], ibk[0][2]) if ibk else b""
        indices = struct.unpack(f"<{len(ib) // 2}H", ib[: len(ib) // 2 * 2])
        mats = [_body(data, k[1], k[2]).split(b"\0")[0].decode("ascii", "replace") for k in mk if k[0] == 0x00134C02]
        buf, cursor = 0, 0
        for i in range(count):
            e = eb[i * ENTRY:(i + 1) * ENTRY]
            stype, = struct.unpack_from("<I", e, 48)
            vc, = struct.unpack_from("<I", e, 64)
            tri, istart = struct.unpack_from("<2I", e, 96)
            icount, = struct.unpack_from("<I", e, 124)
            if stype not in STRIDE:
                stats.setdefault("unknown_shader_types", {})
                stats["unknown_shader_types"][stype] = stats["unknown_shader_types"].get(stype, 0) + 1
                raise GeometryError(f"{name}: shader type {stype} has no known vertex stride")
            stride = STRIDE[stype]
            no, co, uo = LAYOUT[stype]
            while buf < len(vbs) and cursor >= len(vbs[buf]) - (len(vbs[buf]) % stride):
                buf, cursor = buf + 1, 0
            if buf >= len(vbs):
                raise GeometryError(f"{name}: entry {i} runs past the vertex buffers")
            base = cursor // stride
            raw = vbs[buf][cursor:cursor + vc * stride]
            if len(raw) != vc * stride:
                raise GeometryError(f"{name}: entry {i} vertex data short")
            cursor += vc * stride
            idx = indices[istart:istart + icount]
            if len(idx) != icount or icount != tri * 3 and icount != tri + 2:
                raise GeometryError(f"{name}: entry {i} index range {istart}+{icount} vs {tri} triangles")
            if icount != tri * 3:
                raise GeometryError(f"{name}: entry {i} looks like a strip ({icount} indices, {tri} triangles)")
            local = [j - base for j in idx]
            if local and (min(local) < 0 or max(local) >= vc):
                raise GeometryError(f"{name}: entry {i} indices outside its {vc} vertices")
            pos, nrm, uv, col = [], [], [], []
            for v in range(vc):
                o = v * stride
                x, y, z = struct.unpack_from("<3f", raw, o)
                nx, ny, nz = struct.unpack_from("<3f", raw, o + no)
                bb, gg, rr, aa = raw[o + co:o + co + 4]
                u, w = struct.unpack_from("<2f", raw, o + uo)
                if abs(nx * nx + ny * ny + nz * nz - 1) < 0.04:
                    stats["unit_normals"] = stats.get("unit_normals", 0) + 1
                stats["vertices"] = stats.get("vertices", 0) + 1
                pos.append((x, z, -y))
                nrm.append((nx, nz, -ny))
                uv.append((u, w))
                col.append((rr / 255, gg / 255, bb / 255, aa / 255))
            out["meshes"].append({
                "material": mats[i] if i < len(mats) else f"entry{i}",
                "texture_slots": list(e[24:30]),
                "shader_type": stype,
                "positions": pos, "normals": nrm, "uvs": uv, "colors": col,
                # (x, y, z) -> (x, z, -y) is a rotation, so the winding is kept;
                # 79% of triangles then agree with their vertex normals in the
                # canyon export (the reversed order agreed for only 21%).
                "indices": list(local),
            })
        stats["solids"] = stats.get("solids", 0) + 1
        stats["triangles"] = stats.get("triangles", 0) + sum(len(m["indices"]) // 3 for m in out["meshes"])
        yield out


def diffuse_key(solid: dict, mesh: dict):
    slot = mesh["texture_slots"][0]
    return solid["textures"][slot] if slot < len(solid["textures"]) else None


def to_gltf(solid_list, path: Path, material_colors: dict | None = None, textures: dict | None = None) -> dict:
    """Writes a glTF (+ .bin) with one node per solid. Materials are keyed by
    diffuse texture key when `textures` maps keys to {uri, alpha}; otherwise
    by material name with a neutral colour."""
    blob = bytearray()
    views, accessors, meshes, nodes, materials, mat_index = [], [], [], [], [], {}
    images, gltf_textures = [], []

    def add(fmt: str, rows, comps: int, kind: str, target: int | None, minmax=False):
        start = len(blob)
        for r in rows:
            blob.extend(struct.pack(f"<{comps}{fmt}", *r) if comps > 1 else struct.pack(f"<{fmt}", r))
        while len(blob) % 4:
            blob.append(0)
        view = {"buffer": 0, "byteOffset": start, "byteLength": len(blob) - start}
        if target:
            view["target"] = target
        views.append(view)
        acc = {"bufferView": len(views) - 1, "componentType": 5126 if fmt == "f" else 5125,
               "count": len(rows), "type": kind}
        if minmax:
            acc["min"] = [min(r[i] for r in rows) for i in range(comps)]
            acc["max"] = [max(r[i] for r in rows) for i in range(comps)]
        accessors.append(acc)
        return len(accessors) - 1

    for s in solid_list:
        prims = []
        for m in s["meshes"]:
            if not m["indices"]:
                continue
            key = diffuse_key(s, m)
            tex = (textures or {}).get(key)
            mkey = ("tex", key) if tex else ("name", m["material"])
            if mkey not in mat_index:
                c = (material_colors or {}).get(m["material"], [0.7, 0.7, 0.7, 1.0])
                mat = {"name": m["material"], "pbrMetallicRoughness": {"baseColorFactor": c if not tex else [1, 1, 1, 1],
                                                                       "metallicFactor": 0.0, "roughnessFactor": 0.85}}
                if tex:
                    images.append({"uri": tex["uri"]})
                    gltf_textures.append({"source": len(images) - 1, "sampler": 0})
                    mat["pbrMetallicRoughness"]["baseColorTexture"] = {"index": len(gltf_textures) - 1}
                    if tex["blend"] != "src_copy":
                        mat.update({"alphaMode": "BLEND", "doubleSided": True})
                    elif tex["alpha"] == "punchthru":
                        mat.update({"alphaMode": "MASK", "alphaCutoff": 0.5, "doubleSided": True})
                mat_index[mkey] = len(materials)
                materials.append(mat)
            prims.append({
                "attributes": {
                    "POSITION": add("f", m["positions"], 3, "VEC3", 34962, True),
                    "NORMAL": add("f", m["normals"], 3, "VEC3", 34962),
                    "TEXCOORD_0": add("f", m["uvs"], 2, "VEC2", 34962),
                    "COLOR_0": add("f", m["colors"], 4, "VEC4", 34962),
                },
                "indices": add("I", m["indices"], 1, "SCALAR", 34963),
                "material": mat_index[mkey],
            })
        if prims:
            meshes.append({"name": s["name"], "primitives": prims})
            nodes.append({"name": s["name"], "mesh": len(meshes) - 1})
    path = Path(path)
    bin_path = path.with_suffix(".bin")
    bin_path.write_bytes(bytes(blob))
    doc = {
        "asset": {"version": "2.0", "generator": "CrySirror tools/nfsc_geometry.py"},
        "scene": 0, "scenes": [{"nodes": list(range(len(nodes)))}],
        "nodes": nodes, "meshes": meshes, "materials": materials,
        "accessors": accessors, "bufferViews": views,
        "buffers": [{"uri": bin_path.name, "byteLength": len(blob)}],
    }
    if images:
        doc.update({"images": images, "textures": gltf_textures,
                    "samplers": [{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}]})
    path.write_text(json.dumps(doc))
    return {"nodes": len(nodes), "materials": len(materials), "bytes": len(blob)}


def export_textures(archives, a, all_solids, report) -> dict:
    """Diffuse textures of the exported meshes -> DDS -> PNG via the engine
    codec. Returns key -> {uri, alpha}. Missing keys are reported."""
    import nfsc_textures as tx, subprocess
    index = json.loads(a.texture_index.read_text())
    keys = {diffuse_key(s, m) for s in all_solids for m in s["meshes"]} - {None}
    missing = sorted(k for k in keys if str(k) not in index)
    by_where = {}
    for k in keys - set(missing):
        by_where.setdefault(index[str(k)]["where"], set()).add(k)
    tdir = a.out.parent / "textures"
    tdir.mkdir(exist_ok=True)
    table = {s["name"]: s for s in bun.sections(bun.open_member(archives, a.header))}
    jobs, out, unsupported = [], {}, []
    for where, want in sorted(by_where.items()):
        kind, name = where.split(":", 1)
        if kind == "section":
            s = table[name]
            data = bun.read_range(archives, a.stream, s["offset"], s["size"])
        else:
            data = bun.open_member(archives, name.replace("\\", "/"))
        for _, texs in tx.packs(data):
            for t in texs:
                if t["key"] not in want or t["key"] in out:
                    continue
                if "image" not in t:
                    unsupported.append(t["name"])
                    continue
                stem = f"{t['key']:08x}"
                (tdir / f"{stem}.dds").write_bytes(tx.dds(t, a.max_texture))
                jobs.append({"file": f"textures/{stem}.dds", "output": f"textures/{stem}.png", "max_size": a.max_texture})
                out[t["key"]] = {"uri": f"textures/{stem}.png", "alpha": t["alpha"], "blend": t["blend"], "name": t["name"]}
    for j in jobs:
        png = a.out.parent / j["output"]
        if png.exists():
            png.unlink()
    manifest = a.out.parent / "texture-jobs.json"
    manifest.write_text(json.dumps({"schema": 1, "textures": jobs}))
    subprocess.run([str(a.transcode.resolve()), str(manifest), str(a.out.parent / "texture-results.json")], check=True)
    for j in jobs:
        (a.out.parent / j["file"]).unlink()
    report["textures"] = {"exported": len(out), "missing_keys": len(missing), "unsupported": unsupported}
    return out


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--disc", type=Path, required=True)
    p.add_argument("--sections", required=True, help="comma-separated section names, e.g. P41,P42")
    p.add_argument("--out", type=Path, required=True, help="output .gltf path")
    p.add_argument("--header", default="TRACKS/L5RA.BUN")
    p.add_argument("--stream", default="TRACKS/STREAML5RA.BUN")
    p.add_argument("--texture-index", type=Path, help="texture key index (see export_textures)")
    p.add_argument("--transcode", type=Path, help="engine codec: target/release/examples/transcode")
    p.add_argument("--max-texture", type=int, default=512)
    a = p.parse_args(argv)
    archives = sorted(a.disc.glob("*compressed.zip"))
    table = {s["name"]: s for s in bun.sections(bun.open_member(archives, a.header))}
    stats, all_solids, report = {}, [], {"sections": {}}
    for name in a.sections.split(","):
        if name not in table:
            raise SystemExit(f"unknown section {name}")
        s = table[name]
        data = bun.read_range(archives, a.stream, s["offset"], s["size"])
        sec_stats = {}
        got = list(solids(data, sec_stats))
        all_solids.extend(got)
        report["sections"][name] = sec_stats
    a.out.parent.mkdir(parents=True, exist_ok=True)
    textures = None
    if a.texture_index:
        textures = export_textures(archives, a, all_solids, report)
    report["gltf"] = to_gltf(all_solids, a.out, textures=textures)
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
