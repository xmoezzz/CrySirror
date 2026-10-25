#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""GoW III native environment cubemaps (material sampler slot 22) → equirect
Radiance `.hdr` sources for engine reflection probes (ADR 0025).

Own implementation; no game data in the repository. Observed layout of the
OLY100 slot-22 textures: native DDS wrapper (0x10007) whose DDS header has
caps2 = 0xFE00 (cubemap, all six faces), DXT5, all mips inline. The payload
size equals six face-major mip chains, as in a standard DDS cube.

Face order and orientation are not assumed silently. `seam_check` measures
the colour difference across all 12 cube edges under the D3D/DDS face
convention and compares it with permuted face orders. The conversion refuses
(fails visibly) unless the convention fits clearly better. A global mirror of
the whole cube cannot be detected by seams; it is reported as unverified.
Colours are treated as sRGB-encoded LDR. Alpha statistics are reported and
not interpreted (an RGBM-style scale is possible but unverified).
"""
from __future__ import annotations
import argparse, itertools, json, math, struct, subprocess
from collections import Counter
from pathlib import Path

import numpy as np

from gow3_scene import Reader, WadError, check, resources

SLOT = 22
CUBE_CAPS2 = 0xFE00


def split_faces(payload: bytes) -> tuple[list[bytes], dict]:
    """Six standalone 2D DDS files (all mips) from a native cube texture."""
    r = Reader(payload)
    check(r.u32(0) == 0x10007 and r.raw(76, 4) == b'DDS ', 'native DDS wrapper')
    header = bytearray(r.raw(76, 128))
    get = lambda at: struct.unpack_from('<I', header, at)[0]
    width, height, mips, four = get(16), get(12), max(1, get(28)), bytes(header[84:88])
    check(get(112) == CUBE_CAPS2, f'not a full cubemap (caps2 0x{get(112):x})')
    check(four in (b'DXT1', b'DXT5') and width == height and 0 < width <= 2048 and mips <= 14, 'unsupported cube format')
    check(r.u32(64) in (0xFFFFFFFF, mips), 'cube mips must be inline')
    block = 8 if four == b'DXT1' else 16
    chain = sum(max(1, (max(1, width >> i) + 3) // 4) ** 2 * block for i in range(mips))
    pixels = payload[204:]
    check(len(pixels) == 6 * chain, f'cube data length {len(pixels)} != 6 x {chain}')
    struct.pack_into('<I', header, 8, 0x1007 | (0x20000 if mips > 1 else 0) | 0x80000)
    struct.pack_into('<I', header, 20, max(1, (width + 3) // 4) ** 2 * block)
    struct.pack_into('<I', header, 112, 0)  # plain 2D faces
    struct.pack_into('<I', header, 108, get(108) & ~0x8)  # drop DDSCAPS_COMPLEX for faces
    return [bytes(header) + pixels[i * chain:(i + 1) * chain] for i in range(6)], {
        'size': width, 'mips': mips, 'format': four.decode(), 'face_bytes': chain}


# D3D/DDS cube convention: face order +X -X +Y -Y +Z -Z; sc, tc in [-1, 1].
def face_dir(face: int, sc, tc):
    one = np.ones_like(sc)
    return [
        (one, -tc, -sc), (-one, -tc, sc), (sc, one, tc),
        (sc, -one, -tc), (sc, -tc, one), (-sc, -tc, -one),
    ][face]


def face_coords(face: int, d):
    x, y, z = d
    return [
        (-z / abs(x), -y / abs(x)), (z / abs(x), -y / abs(x)), (x / abs(y), z / abs(y)),
        (x / abs(y), -z / abs(y)), (x / abs(z), -y / abs(z)), (-x / abs(z), -y / abs(z)),
    ][face]


def sample(face_img: np.ndarray, sc, tc):
    n = face_img.shape[0]
    px = np.clip(((sc + 1) * 0.5 * n).astype(int), 0, n - 1)
    py = np.clip(((tc + 1) * 0.5 * n).astype(int), 0, n - 1)
    return face_img[py, px]


def major_face(d):
    x, y, z = d
    ax, ay, az = abs(x), abs(y), abs(z)
    if ax >= ay and ax >= az:
        return 0 if x > 0 else 1
    if ay >= az:
        return 2 if y > 0 else 3
    return 4 if z > 0 else 5


def seam_error(faces: list[np.ndarray], order) -> float:
    """Mean |ΔRGB| between both faces sharing each cube edge (half-texel inset)."""
    n = faces[0].shape[0]
    t = np.linspace(-1 + 1 / n, 1 - 1 / n, n)
    edge = 1 - 1 / n
    errs = []
    for f in range(6):
        for sc, tc in [(np.full(n, edge), t), (np.full(n, -edge), t), (t, np.full(n, edge)), (t, np.full(n, -edge))]:
            d = face_dir(f, sc, tc)
            for k in range(n):
                dk = tuple(float(c[k]) for c in d)
                # Push across the edge to find the neighbouring face.
                ax = max(range(3), key=lambda i: abs(dk[i]) if [0, 0, 1, 1, 2, 2][f] != i else -1)
                pushed = list(dk)
                pushed[ax] *= 1.0 + 4.0 / n
                g = major_face(pushed)
                if g == f:
                    continue
                gs, gt = face_coords(g, pushed)
                a = sample(faces[order[f]], np.array([sc[k]]), np.array([tc[k]]))[0]
                b = sample(faces[order[g]], np.array([gs]), np.array([gt]))[0]
                errs.append(np.abs(a[:3].astype(float) - b[:3].astype(float)).mean())
    return float(np.mean(errs))


def srgb_to_linear(c):
    c = c / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def equirect(faces: list[np.ndarray], width: int = 512) -> np.ndarray:
    h = width // 2
    u = (np.arange(width) + 0.5) / width
    v = (np.arange(h) + 0.5) / h
    uu, vv = np.meshgrid(u, v)
    # Same direction convention as the engine prefilter (`dir_from_uv`).
    phi = (uu - 0.5) * 2 * math.pi
    theta = vv * math.pi
    d = np.stack([np.sin(theta) * np.cos(phi), np.cos(theta), np.sin(theta) * np.sin(phi)])
    out = np.zeros((h, width, 3))
    a = np.abs(d)
    face = np.where((a[0] >= a[1]) & (a[0] >= a[2]), np.where(d[0] > 0, 0, 1),
                    np.where(a[1] >= a[2], np.where(d[1] > 0, 2, 3), np.where(d[2] > 0, 4, 5)))
    for f in range(6):
        m = face == f
        sc, tc = face_coords(f, (d[0][m], d[1][m], d[2][m]))
        out[m] = srgb_to_linear(sample(faces[f], sc, tc)[:, :3].astype(float))
    return out


def write_hdr(path: Path, rgb: np.ndarray) -> None:
    """Flat (non-RLE) Radiance RGBE."""
    h, w, _ = rgb.shape
    m = rgb.max(axis=2)
    e = np.where(m > 1e-32, np.floor(np.log2(np.maximum(m, 1e-32))) + 1, 0)
    scale = np.where(m > 1e-32, 256.0 / np.exp2(e), 0)
    rgbe = np.zeros((h, w, 4), dtype=np.uint8)
    rgbe[..., :3] = np.clip(rgb * scale[..., None], 0, 255).astype(np.uint8)
    rgbe[..., 3] = np.where(m > 1e-32, e + 128, 0).astype(np.uint8)
    path.write_bytes(b'#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n' + f'-Y {h} +X {w}\n'.encode() + rgbe.tobytes())


def slot_textures(data: bytes, info: dict) -> Counter:
    uses = Counter()
    for r in info['records']:
        if r['kind'] != '0x00010056':
            continue
        rd = Reader(data[r['offset'] + 64:r['offset'] + 64 + r['size']])
        for i in range(rd.raw(104, 1)[0]):
            base = 208 + i * 104
            if rd.raw(base, 1)[0] == SLOT:
                uses[rd.raw(base + 48, 56).split(b'\0')[0].decode('ascii')] += 1
    return uses


def convert(wad: Path, out: Path, codec: Path, width: int = 512) -> dict:
    from PIL import Image
    data = wad.read_bytes()
    info, _ = resources(data)
    tex = {r['name']: r for r in info['records'] if r['kind'] == '0x0001000d'}
    work = out.parent / 'cubemap-source'
    work.mkdir(parents=True, exist_ok=True)
    report = {'slot': SLOT, 'cubemaps': [], 'colour': 'sRGB LDR → linear', 'global_mirror': 'unverified'}
    for name, uses in slot_textures(data, info).most_common():
        r = tex[name]
        files, meta = split_faces(data[r['offset'] + 64:r['offset'] + 64 + r['size']])
        stem = name.removesuffix('.dds')
        jobs = []
        for i, f in enumerate(files):
            (work / f'{stem}-{i}.dds').write_bytes(f)
            jobs.append({'file': f'{stem}-{i}.dds', 'output': f'{stem}-{i}.png', 'max_size': meta['size']})
        (work / f'{stem}-jobs.json').write_text(json.dumps({'schema': 1, 'textures': jobs}))
        subprocess.run([str(codec), str(work / f'{stem}-jobs.json'), str(work / f'{stem}-decoded.json')], check=True)
        faces = [np.asarray(Image.open(work / j['output']).convert('RGBA')) for j in jobs]
        conv = seam_error(faces, list(range(6)))
        perms = [seam_error(faces, list(p)) for p in itertools.islice(itertools.permutations(range(6)), 1, 720, 37)]
        alpha = np.concatenate([f[..., 3].ravel() for f in faces])
        entry = {'texture': name, 'materials': uses, **meta, 'seam_error_d3d': round(conv, 3),
                 'seam_error_permuted_min': round(min(perms), 3), 'seam_error_permuted_mean': round(float(np.mean(perms)), 3),
                 'alpha_min': int(alpha.min()), 'alpha_max': int(alpha.max()), 'alpha_mean': round(float(alpha.mean()), 2)}
        if conv * 1.5 > min(perms):
            entry['error'] = 'D3D face convention not confirmed by seams; not converted'
        else:
            hdr = out / f'{stem}.hdr'
            write_hdr(hdr, equirect(faces, width))
            entry['equirect'] = hdr.name
        report['cubemaps'].append(entry)
    return report


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('wad', type=Path)
    ap.add_argument('output', type=Path, help='directory for the .hdr probe sources (outside the repository)')
    ap.add_argument('--codec', type=Path, required=True)
    ap.add_argument('--width', type=int, default=512)
    a = ap.parse_args()
    out = a.output.resolve()
    check(not out.is_relative_to(Path(__file__).resolve().parents[1]), 'game assets must remain external')
    print(json.dumps(convert(a.wad, out, a.codec.resolve(), a.width), indent=1))


if __name__ == '__main__':
    main()
