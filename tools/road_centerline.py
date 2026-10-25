#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Centreline of a road surface in a glTF scene (Y-up), for AI drivers.

The road triangles (nodes whose name starts with --prefix) are rasterized
top-down into a grid that keeps each cell's height. A distance transform
gives every road cell its distance to the road edge. The route from the
start cell to the geodesically farthest road cell then minimizes
1 + k / (1 + edge distance), which keeps it centred. Neighbouring cells
must be within `max_step` in height, so stacked roads (bridges,
switchbacks above each other) stay separate where their heights differ;
the first height written to a cell wins. Output: points every --spacing
metres.
"""
from __future__ import annotations
import argparse, heapq, json, math, struct
from collections import deque
from pathlib import Path
import numpy as np


def road_triangles(gltf: Path, prefix: str) -> np.ndarray:
    g = json.loads(gltf.read_text())
    blob = (gltf.parent / g["buffers"][0]["uri"]).read_bytes()

    def acc(i, comps, kind):
        a = g["accessors"][i]
        v = g["bufferViews"][a["bufferView"]]
        n = a["count"] * comps
        return np.frombuffer(blob, dtype=kind, count=n, offset=v.get("byteOffset", 0) + a.get("byteOffset", 0)).reshape(-1, comps)

    tris = []
    for node in g["nodes"]:
        if "mesh" not in node or not node.get("name", "").startswith(prefix):
            continue
        for p in g["meshes"][node["mesh"]]["primitives"]:
            pos = acc(p["attributes"]["POSITION"], 3, np.float32)
            idx = acc(p["indices"], 1, np.uint32).ravel()
            tris.append(pos[idx].reshape(-1, 3, 3))
    if not tris:
        raise SystemExit(f"no nodes start with {prefix!r}")
    return np.concatenate(tris)


def rasterize(tris: np.ndarray, cell: float):
    lo = tris[:, :, [0, 2]].reshape(-1, 2).min(0) - cell
    hi = tris[:, :, [0, 2]].reshape(-1, 2).max(0) + cell
    w, h = (np.ceil((hi - lo) / cell).astype(int) + 1)
    height = np.full((h, w), np.nan, dtype=np.float32)
    for t in tris:
        xz = (t[:, [0, 2]] - lo) / cell
        x0, z0 = np.floor(xz.min(0)).astype(int)
        x1, z1 = np.ceil(xz.max(0)).astype(int)
        (ax, az), (bx, bz), (cx, cz) = xz
        den = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz)
        if abs(den) < 1e-9:
            continue
        gx, gz = np.meshgrid(np.arange(x0, x1 + 1) + 0.5, np.arange(z0, z1 + 1) + 0.5)
        l1 = ((bz - cz) * (gx - cx) + (cx - bx) * (gz - cz)) / den
        l2 = ((cz - az) * (gx - cx) + (ax - cx) * (gz - cz)) / den
        l3 = 1 - l1 - l2
        inside = (l1 >= -0.01) & (l2 >= -0.01) & (l3 >= -0.01)
        y = l1 * t[0, 1] + l2 * t[1, 1] + l3 * t[2, 1]
        zi, xi = np.nonzero(inside)
        for z, x in zip(zi + z0, xi + x0):
            if 0 <= z < h and 0 <= x < w and np.isnan(height[z, x]):
                height[z, x] = y[z - z0, x - x0]
    return height, lo


NEIGHBOURS = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)]


def centreline(height, lo, cell, start_xyz, max_step=1.5, k=6.0, spacing=2.0):
    h, w = height.shape
    road = ~np.isnan(height)
    # Edge distance: BFS from non-road cells into the road.
    dist = np.where(road, np.inf, 0.0)
    q = deque(zip(*np.nonzero(~road)))
    while q:
        z, x = q.popleft()
        for dz, dx in NEIGHBOURS[:4]:
            nz, nx = z + dz, x + dx
            if 0 <= nz < h and 0 <= nx < w and dist[nz, nx] > dist[z, x] + 1:
                dist[nz, nx] = dist[z, x] + 1
                q.append((nz, nx))

    def ok(a, b):
        return road[b] and abs(height[a] - height[b]) <= max_step

    sx, sz = ((np.array([start_xyz[0], start_xyz[2]]) - lo) / cell).astype(int)
    cells = np.argwhere(road)
    d2 = (cells[:, 0] - sz) ** 2 + (cells[:, 1] - sx) ** 2 + ((height[road] - start_xyz[1]) / cell) ** 2
    start = tuple(cells[int(np.argmin(d2))])
    # Farthest reachable road cell (geodesic) is the other end.
    seen = {start: 0}
    q = deque([start])
    while q:
        c = q.popleft()
        for dz, dx in NEIGHBOURS:
            n = (c[0] + dz, c[1] + dx)
            if 0 <= n[0] < h and 0 <= n[1] < w and n not in seen and ok(c, n):
                seen[n] = seen[c] + 1
                q.append(n)
    goal = max(seen, key=seen.get)
    # Dijkstra with a centring cost.
    best = {start: 0.0}
    prev = {}
    heap = [(0.0, start)]
    while heap:
        cost, c = heapq.heappop(heap)
        if c == goal:
            break
        if cost > best[c]:
            continue
        for dz, dx in NEIGHBOURS:
            n = (c[0] + dz, c[1] + dx)
            if n not in seen or not ok(c, n):
                continue
            step = math.hypot(dz, dx) * (1 + k / (1 + dist[n]))
            if cost + step < best.get(n, math.inf):
                best[n] = cost + step
                prev[n] = c
                heapq.heappush(heap, (cost + step, n))
    path = [goal]
    while path[-1] != start:
        path.append(prev[path[-1]])
    path.reverse()
    pts = [[lo[0] + (x + 0.5) * cell, float(height[z, x]), lo[1] + (z + 0.5) * cell] for z, x in path]
    # Smooth, then resample every `spacing` metres.
    arr = np.array(pts)
    for _ in range(3):
        arr[1:-1] = (arr[:-2] + 2 * arr[1:-1] + arr[2:]) / 4
    out, acc = [arr[0].tolist()], 0.0
    for a, b in zip(arr[:-1], arr[1:]):
        acc += float(np.linalg.norm(b - a))
        if acc >= spacing:
            out.append(b.tolist())
            acc = 0.0
    edge = [float(dist[c]) * cell for c in path]
    return out, {"cells": int(road.sum()), "reachable": len(seen), "length_m": round(sum(
        math.dist(out[i], out[i + 1]) for i in range(len(out) - 1)), 1), "min_edge_m": min(edge), "mean_edge_m": round(sum(edge) / len(edge), 2)}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("gltf", type=Path)
    p.add_argument("--prefix", required=True)
    p.add_argument("--start", type=float, nargs=3, required=True, help="x y z near one end")
    p.add_argument("--cell", type=float, default=1.0)
    p.add_argument("--spacing", type=float, default=2.0)
    p.add_argument("--out", type=Path, required=True)
    a = p.parse_args(argv)
    height, lo = rasterize(road_triangles(a.gltf, a.prefix), a.cell)
    pts, report = centreline(height, lo, a.cell, a.start, spacing=a.spacing)
    a.out.write_text(json.dumps([[round(v, 2) for v in q] for q in pts]))
    print(json.dumps(report))


if __name__ == "__main__":
    main()
