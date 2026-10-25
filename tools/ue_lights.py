#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Convert lights from `ue-import level` output into CrySirror light entities.

Unreal space is centimeters, left-handed, Z up; CrySirror is meters,
right-handed, Y up. Positions map as (x, z, y) * 0.01, matching
`crysirror_ue_import::level::to_crysirror` and the scene preview glTF.
UE lights emit along their local +X; CrySirror spot/rect lights emit along
local -Z, so each light gets a rotation whose -Z is the converted +X axis.

Units follow UE's delta serialization: a missing IntensityUnits means the
class default (Unitless), converted to candela by 1/625 (UE5's default 8 cd
corresponds to the legacy 5000 unitless). Lumens divide by 4π for point and
spot lights (UE keeps spot brightness independent of cone) and by π for rect
lights. Missing values take UE class defaults listed in DEFAULTS.
"""
from __future__ import annotations
import argparse, json, math
from pathlib import Path

DEFAULTS = {
    # Unitless intensity 5000 is UE's local light default.
    "intensity": 5000.0,
    "radius_cm": 1000.0,
    "inner_cone": 0.0,
    "outer_cone": 44.0,
    "rect_size_cm": 64.0,
    "directional_lux": 10.0,
}
KINDS = {"PointLightComponent": "point", "SpotLightComponent": "spot", "RectLightComponent": "rect"}


def srgb_to_linear(c: float) -> float:
    c = c / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def color(v) -> list[float]:
    if not isinstance(v, list) or len(v) < 3:
        return [1.0, 1.0, 1.0]
    # Serialized FColor is BGRA in memory but the importer emits it as decoded R,G,B,A.
    return [round(srgb_to_linear(float(x)), 6) for x in v[:3]]


def axis(m, col: int) -> list[float]:
    """Column `col` of a UE world matrix, converted to CrySirror axes."""
    x, y, z = m[0][col], m[1][col], m[2][col]
    return [x, z, y]


def normalize(v):
    n = math.sqrt(sum(c * c for c in v))
    if n < 1e-9:
        raise ValueError("degenerate light axis")
    return [c / n for c in v]


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def euler_xyz(forward, up_hint):
    """XYZ Euler degrees (glam EulerRot::XYZ) of a rotation mapping -Z to forward."""
    z = [-c for c in normalize(forward)]
    x = cross(up_hint, z)
    if math.sqrt(sum(c * c for c in x)) < 1e-6:
        x = cross([1.0, 0.0, 0.0] if abs(z[0]) < 0.9 else [0.0, 0.0, 1.0], z)
    x = normalize(x)
    y = cross(z, x)
    # R = [x y z] columns; R = Rx(a) Ry(b) Rz(c).
    r02, r12, r22, r01, r00 = z[0], z[1], z[2], y[0], x[0]
    b = math.asin(max(-1.0, min(1.0, r02)))
    a = math.atan2(-r12, r22)
    c = math.atan2(-r01, r00)
    return [math.degrees(a), math.degrees(b), math.degrees(c)]


def candela(kind: str, intensity: float, units) -> float:
    if units in (None, "Unitless"):
        return intensity / 625.0
    if units == "Candelas":
        return intensity
    if units == "Lumens":
        return intensity / (math.pi if kind == "rect" else 4.0 * math.pi)
    if units == "EV":
        # EV100 → candela: 2^EV * 100/625 (UE's Exposure-Value light units).
        return 2.0 ** intensity / 6.25
    raise ValueError(f"unknown UE light units {units!r}")


def num(v, default: float) -> float:
    return float(v) if isinstance(v, (int, float)) else default


def convert(level: dict) -> dict:
    lights, skipped = [], []
    sun = None
    for i, l in enumerate(level.get("lights", [])):
        cls = l["class"]
        m = l["matrix"]
        if cls == "DirectionalLightComponent":
            sun = {
                "direction": normalize(axis(m, 0)),
                "color": color(l.get("color")),
                "intensity": num(l.get("intensity"), DEFAULTS["directional_lux"]),
                "shadows": l.get("cast_shadows", 1) != 0,
            }
            continue
        kind = KINDS.get(cls)
        if kind is None:
            skipped.append({"index": i, "class": cls, "reason": "no CrySirror equivalent yet"})
            continue
        if l.get("visible") is False:
            skipped.append({"index": i, "class": cls, "reason": "hidden in game"})
            continue
        cd = candela(kind, num(l.get("intensity"), DEFAULTS["intensity"]), l.get("units"))
        shape = {"kind": kind}
        if kind == "spot":
            outer = min(max(num(l.get("outer_cone"), DEFAULTS["outer_cone"]), 0.1), 89.0)
            shape |= {"inner_degrees": min(num(l.get("inner_cone"), DEFAULTS["inner_cone"]), outer), "outer_degrees": outer}
        elif kind == "rect":
            # Rect lights face +X; width spans local Y and height local Z.
            shape["size"] = [
                max(num(l.get("source_width"), DEFAULTS["rect_size_cm"]) * 0.01, 0.001),
                max(num(l.get("source_height"), DEFAULTS["rect_size_cm"]) * 0.01, 0.001),
            ]
        position = [m[0][3] * 0.01, m[2][3] * 0.01, m[1][3] * 0.01]
        lights.append({
            "name": f"ue-light-{i}:{l.get('component', '')}",
            "position": position,
            "rotation": euler_xyz(axis(m, 0), normalize(axis(m, 2))),
            "light": {
                "shape": shape,
                "color": color(l.get("color")),
                "intensity": round(cd, 6),
                "units": "candela",
                "range": max(num(l.get("radius"), DEFAULTS["radius_cm"]) * 0.01, 0.01),
                # Stored for the renderer's shadow budget; see STATUS for support.
                "shadows": l.get("cast_shadows", 1) != 0,
            },
        })
    return {"source": level.get("package"), "lights": lights, "sun": sun, "skipped": skipped}


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("level", type=Path)
    ap.add_argument("output", type=Path)
    a = ap.parse_args()
    out = convert(json.loads(a.level.read_text()))
    a.output.write_text(json.dumps(out, indent=1))
    print(json.dumps({"lights": len(out["lights"]), "sun": out["sun"] is not None, "skipped": len(out["skipped"])}))


if __name__ == "__main__":
    main()
