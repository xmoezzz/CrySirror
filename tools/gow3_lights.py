#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""GoW III PS3 WAD light records (kind 0x00010017, version 6) → engine lights.

Own implementation from the observed 156-byte big-endian layout. No game data.

| Offset | Field (observed) |
|---|---|
| 0  | u32 version = 6 |
| 16 | u32 flags; bits 0-1 are the type (0 ambient, 1 point, 2 directional); bits 2-5 are unverified flags |
| 20 | float3 position (level space, same as the rest-space geometry) |
| 36 | float3 unit vector of directional lights (points toward the light) |
| 124| float3 linear color, may exceed 1 |
| 136| float intensity multiplier |
| 140| float radius |

Directional and ambient records carry a radius, so they map to the engine's
range-limited `directional`/`ambient` volume lights. Native falloff curves and
the meaning of the remaining flag bits are not verified. They are reported,
and the conversion uses explicit, documented approximations
(`APPROXIMATIONS`). Unknown versions/types fail or are reported, never guessed.
"""
from __future__ import annotations
import argparse, json, math, struct
from collections import Counter
from pathlib import Path

from gow3_wad import WadError
from ue_lights import euler_xyz, normalize

KIND = 0x00010017
SIZE = 156
TYPES = {0: "ambient", 1: "point", 2: "directional"}
APPROXIMATIONS = {
    "falloff": "exponent 2 over the native radius; the native curve is not decoded",
    "ambient": "engine ambient volumes use irradiance; native color×intensity is scaled by π so diffuse radiance equals albedo×color×intensity",
    "units": "native color×intensity used as engine illuminance without photometric calibration",
    "flags": "bits 2-5 of the type byte are reported, not interpreted (all records converted)",
}


def check(v, msg):
    if not v:
        raise WadError(msg)


def decode(payload: bytes, name: str) -> dict:
    check(len(payload) == SIZE, f"{name}: light record size {len(payload)}")
    u = lambda o: struct.unpack_from(">I", payload, o)[0]
    f3 = lambda o: list(struct.unpack_from(">3f", payload, o))
    f = lambda o: struct.unpack_from(">f", payload, o)[0]
    check(u(0) == 6, f"{name}: unknown light record version {u(0)}")
    flags = u(16)
    kind = TYPES.get(flags & 3)
    check(kind is not None, f"{name}: unknown light type {flags & 3} (flags 0x{flags:08x})")
    rec = {
        "name": name,
        "type": kind,
        "flags": flags,
        "position": f3(20),
        "toward_light": f3(36),
        "color": f3(124),
        "intensity": f(136),
        "radius": f(140),
    }
    values = [*rec["position"], *rec["color"], rec["intensity"], rec["radius"]]
    check(all(math.isfinite(x) for x in values), f"{name}: non-finite light")
    check(rec["radius"] > 0 and all(c >= 0 for c in rec["color"]), f"{name}: invalid radius/color")
    return rec


def to_entity(rec: dict) -> dict | None:
    kind = rec["type"]
    peak = max(rec["color"])
    color = [c / peak for c in rec["color"]] if peak > 0 else [0.0, 0.0, 0.0]
    intensity = peak * rec["intensity"]
    if kind == "ambient":
        intensity *= math.pi
    light = {
        "shape": {"kind": kind},
        "color": [min(c, 1000.0) for c in color],
        "intensity": max(0.0, intensity),
        "range": min(max(rec["radius"], 0.01), 100000.0),
        "falloff": {"kind": "exponent", "exponent": 2.0},
    }
    entity = {"name": f"gow3-light:{rec['name']}", "transform": {"translation": rec["position"]}, "light": light}
    if kind == "directional":
        travel = [-c for c in normalize(rec["toward_light"])]
        up = [0.0, 1.0, 0.0] if abs(travel[1]) < 0.99 else [1.0, 0.0, 0.0]
        entity["transform"]["rotation_degrees"] = euler_xyz(travel, up)
    return entity


def lights(data: bytes, records: list[dict]) -> tuple[list[dict], dict]:
    decoded, errors = [], []
    for r in records:
        if int(r["kind"], 16) != KIND:
            continue
        payload = data[r["offset"] + 64 : r["offset"] + 64 + r["size"]]
        try:
            decoded.append(decode(payload, r["name"]))
        except (WadError, struct.error) as e:
            errors.append({"name": r["name"], "error": str(e)})
    entities = [e for e in map(to_entity, decoded) if e is not None]
    report = {
        "records": len(decoded) + len(errors),
        "converted": len(entities),
        "by_type": dict(Counter(d["type"] for d in decoded)),
        "type_byte_flags": {hex(k): v for k, v in sorted(Counter(d["flags"] & 0x3C for d in decoded).items())},
        "high_flags": {hex(k): v for k, v in sorted(Counter(d["flags"] & ~0xFF for d in decoded).items())},
        "errors": errors,
        "approximations": APPROXIMATIONS,
        "original_lighting": False,
    }
    return entities, report


def apply(project: Path, entities: list[dict]) -> int:
    """Add light entities to a data-driven preview project (replacing earlier ones)."""
    p = json.loads(project.read_text())
    systems = p["clients"][0].setdefault("systems", {})
    ents = [e for e in systems.get("entities", []) if not e["name"].startswith("gow3-light:")]
    next_id = max((e["id"] for e in ents), default=0) + 1
    for i, e in enumerate(entities):
        ents.append({"id": next_id + i, **e})
    systems["entities"] = ents
    project.write_text(json.dumps(p, indent=1))
    return len(entities)


def main() -> None:
    from gow3_scene import resources
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("wad", type=Path)
    ap.add_argument("project", type=Path, help="data-driven preview project.json to update")
    a = ap.parse_args()
    data = a.wad.read_bytes()
    info, _ = resources(data)
    entities, report = lights(data, info["records"])
    apply(a.project, entities)
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
