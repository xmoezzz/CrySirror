# SPDX-License-Identifier: MPL-2.0
"""Device-free authoring checks for crysirror-vfx's bounded schema (ADR 0064).

This mirrors schema.rs and SystemSpec.compile, including its explicit GPU
capability boundary. It neither simulates particles nor accepts executable code.
Runtime world reservations and GPU/mesh-resource availability are checked by the
engine separately. Values are not rewritten; f32 arithmetic checks bindings as
the Rust compiler does.
"""
from __future__ import annotations

import math
import struct
import unicodedata
from desktop_project import obj, require, integer, logical_path


def _float(value, label="VFX number"):
    require(type(value) in (int, float), label)
    try:
        value = struct.unpack("<f", struct.pack("<f", value))[0]
    except (OverflowError, struct.error):
        require(False, label)
    require(math.isfinite(value), label)
    return value


def _number(value, low=-1_000_000, high=1_000_000, label="VFX number"):
    value = _float(value, label)
    require(low <= value <= high, label)
    return value


def _vector(value, count, low=-1_000_000, high=1_000_000):
    require(isinstance(value, list) and len(value) == count, "VFX vector length")
    return [_number(v, low, high) for v in value]


def _nonzero(value, label):
    require(sum(v*v for v in value) > 1e-12, label)
    return value


def _name(value):
    require(isinstance(value, str) and bool(value)
            and not any(unicodedata.category(c) in ("Cc", "Cs") for c in value), "VFX name")
    require(len(value.encode("utf-8")) <= 128, "VFX name length")
    return value


def _flags(value, names):
    for name in names:
        if name in value:
            require(type(value[name]) is bool, f"VFX boolean: {name}")


def _list(value, maximum, label):
    require(isinstance(value, list) and len(value) <= maximum, label)
    return value


def asset_path(path):
    require(isinstance(path, str) and not any(unicodedata.category(c) == "Cs" for c in path),
            "VFX asset path must be Unicode text")
    require(logical_path(path) == path and len(path.encode("utf-8")) <= 1024,
            "VFX asset path must be canonical and client relative")
    return path


def _hdr(value):
    result = _vector(value, 4)
    require(all(v >= 0 for v in result[:3]) and 0 <= result[3] <= 1, "VFX HDR color/opacity")
    return result


def parameter(value):
    obj(value, {"kind", "value"})
    kind, payload = value["kind"], value["value"]
    require(kind in ("float", "vector", "color", "bool"), "VFX parameter kind")
    if kind == "float":
        _number(payload)
    elif kind == "vector":
        _vector(payload, 3)
    elif kind == "color":
        _hdr(payload)
    else:
        require(type(payload) is bool, "VFX bool parameter")


def _binding(parameters, name, kind):
    _name(name)
    require(name in parameters and parameters[name]["kind"] == kind,
            f"VFX {kind} parameter missing or wrong type: {name}")
    return parameters[name]["value"]


def scalar(value, parameters):
    if isinstance(value, dict):
        obj(value, {"parameter"}, {"scale", "offset"})
        raw = _number(_binding(parameters, value["parameter"], "float"))
        scale = _number(value.get("scale", 1))
        offset = _number(value.get("offset", 0))
        return _number(_float(raw*scale)+offset, label="VFX resolved scalar")
    return _number(value)


def vector(value, parameters):
    if isinstance(value, dict):
        obj(value, {"parameter"}, {"scale"})
        raw = _vector(_binding(parameters, value["parameter"], "vector"), 3)
        scale = _number(value.get("scale", 1))
        return [_number(v*scale, label="VFX resolved vector") for v in raw]
    return _vector(value, 3)


def curve(value, arity=1):
    obj(value, {"keys"}, {"interpolation"})
    require(value.get("interpolation", "linear") in ("constant", "linear", "smooth"), "VFX curve interpolation")
    keys = _list(value["keys"], 32, "VFX curve key count")
    require(len(keys) >= 2, "VFX curve needs 2..32 keys")
    times = []
    for key in keys:
        obj(key, {"time", "value"})
        times.append(_number(key["time"], 0, 1))
        if arity == 1:
            _number(key["value"])
        elif arity == 2:
            _vector(key["value"], 2, 0)
        else:
            require(arity == 4, "VFX curve arity")
            _hdr(key["value"])
    require(times[0] == 0 and times[-1] == 1
            and all(a < b for a, b in zip(times, times[1:])), "VFX curve times must increase from 0 to 1")


def shape(value):
    require(isinstance(value, dict), "VFX spawn shape")
    kind = value.get("kind")
    fields = {"point": set(), "sphere": {"radius", "surface"}, "box": {"half_extent"},
              "cone": {"radius", "height"}, "line": {"end"}}
    require(isinstance(kind, str) and kind in fields, "VFX unknown spawn shape")
    obj(value, {"kind"} | fields[kind])
    if kind == "sphere":
        _number(value["radius"], 0, 10000)
        _flags(value, ("surface",))
    elif kind == "box":
        _vector(value["half_extent"], 3, 0)
    elif kind == "cone":
        _number(value["radius"], 0)
        _number(value["height"], 0)
    elif kind == "line":
        _vector(value["end"], 3)


def render(value):
    require(isinstance(value, dict), "VFX renderer")
    kind = value.get("kind")
    if kind == "sprite":
        obj(value, {"kind"}, {"texture", "columns", "rows", "frames", "fps", "soft_distance", "blend", "facing"})
        columns, rows, frames = (value.get(k, 1) for k in ("columns", "rows", "frames"))
        require(integer(columns, 1, 256) and integer(rows, 1, 256)
                and integer(frames, 1, columns*rows), "VFX flipbook grid/frame limits")
        require(value.get("texture") is not None or (columns == rows == frames == 1), "VFX flipbook requires texture")
        _number(value.get("fps", 0), 0, 4096)
        _number(value.get("soft_distance", 0), 0, 10000)
        facing = value.get("facing", {"kind": "camera"})
        require(isinstance(facing, dict), "VFX facing")
        mode = facing.get("kind")
        require(mode in ("camera", "velocity", "axis"), "VFX unknown facing")
        obj(facing, {"kind"} | ({"axis"} if mode == "axis" else set()))
        if mode == "axis":
            _nonzero(_vector(facing["axis"], 3), "VFX zero facing axis")
    elif kind == "mesh":
        obj(value, {"kind"}, {"mesh", "texture", "blend"})
        mesh = value.get("mesh", {"kind": "cube"})
        require(isinstance(mesh, dict) and mesh.get("kind") in ("cube", "quad", "asset"), "VFX unknown mesh shape")
        obj(mesh, {"kind"} | ({"path"} if mesh["kind"] == "asset" else set()))
        if mesh["kind"] == "asset":
            asset_path(mesh["path"])
    elif kind == "ribbon":
        obj(value, {"kind", "width"}, {"texture", "uv_scale", "max_segments", "blend"})
        _number(value["width"], _float(.0001), 10000)
        _number(value.get("uv_scale", 1), _float(.0001), 10000)
        require(integer(value.get("max_segments", 2048), 1, 65536), "VFX ribbon segment count")
    else:
        require(False, "VFX unknown renderer")
    require(value.get("blend", "additive") in ("alpha", "additive", "premultiplied", "multiply"), "VFX blend")
    if value.get("texture") is not None:
        asset_path(value["texture"])


_MODULE_FIELDS = {
    "acceleration": {"value"}, "drag": {"coefficient"}, "curl_noise": {"strength", "frequency"},
    "attractor": {"point", "strength", "radius"}, "orbit": {"axis", "speed"},
    "rotate": {"radians_per_second"}, "color_over_life": {"curve"}, "size_over_life": {"curve"},
    "collision_plane": {"normal", "offset", "restitution", "friction", "kill"},
    "collision_sphere": {"center", "radius", "restitution", "kill"},
}
_FORCES = {"acceleration", "drag", "curl_noise", "attractor", "orbit"}


def module(value, parameters):
    require(isinstance(value, dict), "VFX module")
    op = value.get("op")
    require(isinstance(op, str) and op in _MODULE_FIELDS, "VFX unknown module")
    obj(value, {"op"} | _MODULE_FIELDS[op])
    for key in ("value", "point", "axis", "normal", "center"):
        if key in value:
            resolved = vector(value[key], parameters)
            if key in ("axis", "normal"):
                _nonzero(resolved, "VFX zero module axis/normal")
    for key in ("coefficient", "strength", "frequency", "radius", "speed", "radians_per_second", "offset"):
        if key in value:
            resolved = scalar(value[key], parameters)
            if key in ("coefficient", "frequency"):
                require(resolved >= 0, "VFX negative drag/frequency")
            if key == "radius":
                require(resolved > 0, "VFX module radius")
    if op in ("color_over_life", "size_over_life"):
        curve(value["curve"], 4 if op == "color_over_life" else 2)
    for key in ("restitution", "friction"):
        if key in value:
            _number(value[key], 0, 1)
    _flags(value, ("kill",))


def sequence_rng(value):
    obj(value, {"algorithm", "state", "multiplier", "increment", "output_shift", "output_mask"})
    require(value["algorithm"] == "lcg32", "VFX sequence algorithm")
    for key in ("state", "multiplier", "increment"):
        require(integer(value[key], 0, 2**32-1), "VFX sequence word")
    require(integer(value["output_shift"], 0, 31) and integer(value["output_mask"], 1, 2**32-1), "VFX sequence output domain")


def screen_target(value):
    obj(value, {"canvas", "layer", "anchor"}, {"parent", "order"})
    require(isinstance(value["canvas"], list) and len(value["canvas"]) == 2
            and all(integer(n, 1, 16384) for n in value["canvas"]), "VFX screen canvas")
    require(integer(value["layer"], 1, 2**32-1), "VFX screen layer")
    if value.get("parent") is not None:
        require(integer(value["parent"], 0, 2**32-1) and value["parent"] != value["layer"], "VFX screen parent")
    order = value.get("order", [0, 0])
    require(isinstance(order, list) and len(order) == 2
            and all(integer(n, -(2**31), 2**31-1) for n in order), "VFX screen order")
    _vector(value["anchor"], 2, 0, 1)


def falling_atlas(value):
    obj(value, {"canvas", "tick_hz", "x_bands", "fall_range", "fall_divisor", "sway_range",
                "phase_modulus", "phase_step", "wind_step", "rows", "row_frames", "cell", "stride",
                "texture_extent", "scale", "alpha_base", "alpha_step", "alpha_count", "alpha_offset",
                "atlas_interval_ms", "atlas_clock_bits"})
    for key in ("canvas", "cell", "stride", "texture_extent"):
        require(isinstance(value[key], list) and len(value[key]) == 2
                and all(integer(n, 1, 16384) for n in value[key]), "VFX falling pixel extent")
    require(integer(value["tick_hz"], 1, 240), "VFX falling fixed clock")
    bands = _list(value["x_bands"], 32, "VFX falling x bands")
    require(bool(bands) and all(integer(n, -16, 16) for n in bands), "VFX falling x bands")
    for key in ("fall_range", "sway_range"):
        pair = value[key]
        require(isinstance(pair, list) and len(pair) == 2 and all(integer(n, 0, 65535) for n in pair)
                and pair[0] <= pair[1], "VFX falling random interval")
    divisor = _float(value["fall_divisor"])
    require(value["fall_range"][0] > 0 and divisor > 0
            and _float(value["fall_range"][1] / divisor) <= value["canvas"][1] * 2,
            "VFX falling speed")
    require(integer(value["phase_modulus"], 1, 65536), "VFX falling phase modulus")
    _number(value["phase_step"], -10, 10)
    _number(value["wind_step"], -16384, 16384)
    rows = _list(value["rows"], 256, "VFX falling rows")
    frames = _list(value["row_frames"], 256, "VFX falling row frame counts")
    require(bool(rows) and bool(frames) and all(integer(n, 1, 256) for n in frames), "VFX falling atlas rows")
    for row in rows:
        require(integer(row, 0, len(frames)-1), "VFX falling row index")
        require((frames[row]-1)*value["stride"][0]+value["cell"][0] <= value["texture_extent"][0]
                and row*value["stride"][1]+value["cell"][1] <= value["texture_extent"][1], "VFX falling backing extent")
    _number(value["scale"], _float(.0001), 16)
    require(integer(value["alpha_count"], 1, 256), "VFX falling alpha choices")
    base, step, offset = (_float(value[k]) for k in ("alpha_base", "alpha_step", "alpha_offset"))
    for choice in (0, value["alpha_count"]-1):
        require(0 <= _float(_float(base + _float(step*choice)) + offset) <= 1, "VFX falling alpha range")
    require(integer(value["atlas_interval_ms"], 1, 600000) and value["atlas_clock_bits"] in (32, 64)
            and type(value["atlas_clock_bits"]) is int, "VFX falling atlas clock")


def emitter(value, parameters):
    obj(value, set(), {"id", "enabled", "enabled_parameter", "target", "space", "capacity", "seed", "overflow", "spawn", "initialize", "update", "render", "events", "screen", "falling_atlas"})
    _name(value.get("id", "emitter"))
    _flags(value, ("enabled",))
    require(value.get("target", "cpu") in ("cpu", "gpu") and value.get("space", "world") in ("world", "local"), "VFX target/space")
    require(value.get("overflow", "drop_new") in ("drop_new", "kill_oldest"), "VFX overflow policy")
    require(integer(value.get("capacity", 1024), 1, 200000) and integer(value.get("seed", 1), 0, 2**32-1), "VFX capacity/seed")
    if value.get("enabled_parameter") is not None:
        _binding(parameters, value["enabled_parameter"], "bool")
    spawn = value.get("spawn", {})
    obj(spawn, set(), {"rate", "bursts", "duration", "looping", "burst_looping", "auto_start"})
    require(scalar(spawn.get("rate", 20), parameters) >= 0, "VFX spawn rate")
    duration = _number(spawn.get("duration", 5), _float(.0001), 86400)
    _flags(spawn, ("looping", "burst_looping", "auto_start"))
    for burst in _list(spawn.get("bursts", []), 256, "VFX burst count"):
        obj(burst, {"time", "count"})
        require(0 <= _number(burst["time"]) < duration and integer(burst["count"], 0, 200000), "VFX burst bounds")
    init = value.get("initialize", {})
    obj(init, set(), {"shape", "lifetime", "speed", "direction", "spread_degrees", "color", "size", "rotation", "ribbon_id", "color_parameter", "size_parameter"})
    shape(init.get("shape", {"kind": "point"}))
    life = _vector(init.get("lifetime", [1, 2]), 2)
    speed = _vector(init.get("speed", [1, 2]), 2)
    rotation = _vector(init.get("rotation", [0, 0]), 2)
    require(0 < life[0] <= life[1] <= 3600 and 0 <= speed[0] <= speed[1]
            and rotation[0] <= rotation[1], "VFX initialization ranges")
    _nonzero(vector(init.get("direction", [0, 1, 0]), parameters), "VFX zero direction")
    _number(init.get("spread_degrees", 20), 0, 180)
    _hdr(init.get("color", [1]*4))
    _vector(init.get("size", [.1, .1]), 2, 0)
    require(integer(init.get("ribbon_id", 0), 0, 2**32-1), "VFX ribbon id")
    if init.get("color_parameter") is not None:
        _binding(parameters, init["color_parameter"], "color")
    if init.get("size_parameter") is not None:
        size = _binding(parameters, init["size_parameter"], "vector")
        require(all(_float(v) >= 0 for v in size[:2]), "VFX negative parameter size")
    collided = False
    for instruction in _list(value.get("update", []), 32, "VFX module count"):
        module(instruction, parameters)
        op = instruction["op"]
        require(not collided or op not in _FORCES, "VFX forces must precede collisions")
        collided |= op in ("collision_plane", "collision_sphere")
    events = _list(value.get("events", []), 16, "VFX event handler count")
    for event in events:
        obj(event, {"event", "emitter", "count"}, {"inherit_velocity", "probability"})
        require(event["event"] in ("birth", "death", "collision"), "VFX event kind")
        _name(event["emitter"])
        require(integer(event["count"], 0, 4096), "VFX event spawn count")
        _number(event.get("inherit_velocity", 0), 0, 16)
        _number(event.get("probability", 1), 0, 1)
    draw = value.get("render", {"kind": "sprite"})
    render(draw)
    if value.get("screen") is not None:
        screen_target(value["screen"])
        require(value.get("target", "cpu") == "cpu" and value.get("capacity", 1024) <= 4096,
                "VFX screen profile requires bounded CPU particles")
        require(draw["kind"] == "sprite" and draw.get("texture") is not None
                and draw.get("facing", {"kind": "camera"}) == {"kind": "camera"}
                and draw.get("blend", "alpha") in ("alpha", "additive", "multiply"), "VFX screen render profile")
    if value.get("falling_atlas") is not None:
        falling_atlas(value["falling_atlas"])
        require(value.get("target", "cpu") == "cpu" and value.get("screen") is not None
                and value.get("space", "world") == "local" and not value.get("update", []) and not events,
                "VFX falling node profile")
    if value.get("target", "cpu") == "gpu":
        require(value.get("overflow", "drop_new") == "drop_new", "GPU VFX kill_oldest unsupported")
        require(not events, "GPU VFX event sub-emitters unsupported")
        require(draw["kind"] != "ribbon", "GPU VFX ribbons unsupported")


def validate(value):
    obj(value, set(), {"revision", "emitters", "parameters", "fixed_step_hz", "warmup_seconds", "max_steps_per_advance", "enabled", "paused", "time_scale", "clock", "sequence_rng", "sequence_exchange"})
    clock = value.get("clock", {"source": "engine"})
    require(isinstance(clock, dict), "VFX clock")
    if clock.get("source") == "engine":
        obj(clock, {"source"})
    else:
        obj(clock, {"source", "tick", "wall_time_ms"})
        require(clock["source"] == "external" and all(integer(clock[k], 0, 2**64-1) for k in ("tick", "wall_time_ms")), "VFX external clock")
    if value.get("sequence_rng") is not None:
        sequence_rng(value["sequence_rng"])
        require(clock["source"] == "external", "VFX sequence needs an external clock")
    if value.get("sequence_exchange") is not None:
        exchange = value["sequence_exchange"]
        obj(exchange, {"epoch", "base_tick", "state"})
        require(value.get("sequence_rng") is not None and integer(exchange["epoch"], 1, 2**64-1)
                and integer(exchange["base_tick"], 0, 2**64-1), "VFX sequence exchange")
        sequence_rng(exchange["state"])
    require(integer(value.get("revision", 0), 0, 2**64-1), "VFX revision")
    require(integer(value.get("fixed_step_hz", 60), 1, 240)
            and integer(value.get("max_steps_per_advance", 64), 1, 4096), "VFX fixed clock/step limit")
    _number(value.get("warmup_seconds", 0), 0, 10)
    _number(value.get("time_scale", 1), 0, 16)
    _flags(value, ("enabled", "paused"))
    parameters = value.get("parameters", {})
    require(isinstance(parameters, dict) and len(parameters) <= 128, "VFX parameter count")
    for name, payload in parameters.items():
        _name(name)
        parameter(payload)
    emitters = _list(value.get("emitters", [{}]), 64, "VFX emitter count")
    require(bool(emitters), "VFX system needs emitters")
    names = {}
    screens = set()
    for item in emitters:
        emitter(item, parameters)
        name = item.get("id", "emitter")
        require(name not in names, "VFX duplicate emitter id")
        names[name] = item
        if value.get("sequence_rng") is not None:
            require(item.get("target", "cpu") == "cpu", "VFX sequence requires CPU emitters")
        if item.get("screen") is not None:
            layer = item["screen"]["layer"]
            require(layer not in screens, "duplicate VFX screen target")
            screens.add(layer)
        if item.get("falling_atlas") is not None:
            require(clock["source"] == "external" and value.get("sequence_rng") is not None
                    and value.get("warmup_seconds", 0) == 0 and value.get("time_scale", 1) == 1
                    and item["falling_atlas"]["tick_hz"] == value.get("fixed_step_hz", 60), "VFX falling clock profile")
    for item in emitters:
        for event in item.get("events", []):
            target = event["emitter"]
            require(target in names, "VFX event target absent")
            require(names[target].get("falling_atlas") is None, "VFX falling child event target unsupported")
            require(item.get("target", "cpu") == names[target].get("target", "cpu") == "cpu",
                    "VFX child events require CPU source and target")
    return value


def asset_paths(value):
    """Return only validated explicit resource paths, relative to this client."""
    validate(value)
    result = set()
    for item in value.get("emitters", [{}]):
        draw = item.get("render", {"kind": "sprite"})
        if draw.get("texture") is not None:
            result.add(draw["texture"])
        mesh = draw.get("mesh", {})
        if draw["kind"] == "mesh" and mesh.get("kind") == "asset":
            result.add(mesh["path"])
    return result


def legacy(value):
    """Validate old scene particles, then the engine's real modular lowering."""
    obj(value, set(), {"rate", "burst", "lifetime", "speed", "direction", "spread_degrees", "gravity", "drag", "size", "color_start", "color_end", "max", "additive", "enabled", "seed"})
    rate = _number(value.get("rate", 20), 0, 10000)
    life = _vector(value.get("lifetime", [1, 1.5]), 2)
    size = _vector(value.get("size", [.1, .02]), 2)
    require(integer(value.get("max", 512), 1, 65536) and integer(value.get("burst", 0), 0, 65536), "legacy particle capacity/burst")
    _flags(value, ("additive", "enabled"))
    item = {"id": "legacy", "capacity": value.get("max", 512), "seed": value.get("seed", 0),
            "enabled": value.get("enabled", True),
            "spawn": {"rate": rate, "duration": max(life[1], 1), "bursts": [{"time": 0, "count": value.get("burst", 0)}]},
            "initialize": {"lifetime": life, "speed": value.get("speed", [1, 2]), "direction": value.get("direction", [0, 1, 0]),
                           "spread_degrees": value.get("spread_degrees", 0), "color": value.get("color_start", [4, 2, .6, 1]), "size": [size[0]*2]*2},
            "update": [{"op": "acceleration", "value": value.get("gravity", [0, 0, 0])},
                       {"op": "drag", "coefficient": value.get("drag", 0)},
                       {"op": "color_over_life", "curve": {"keys": [{"time": 0, "value": value.get("color_start", [4, 2, .6, 1])}, {"time": 1, "value": value.get("color_end", [1, .2, .05, 0])}]}},
                       {"op": "size_over_life", "curve": {"keys": [{"time": 0, "value": [size[0]*2]*2}, {"time": 1, "value": [size[1]*2]*2}]}}]}
    validate({"emitters": [item], "fixed_step_hz": 30, "warmup_seconds": life[1] if rate > 0 else 0})
