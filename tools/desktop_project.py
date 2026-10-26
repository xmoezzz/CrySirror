#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Strict schema-v2 validation and asset inventory; standard library only."""
from __future__ import annotations
import argparse
import json
import math
from pathlib import Path
import re
from typing import Any

MAX_BYTES = 4 * 1024 * 1024
class ProjectError(ValueError):
    pass

def require(condition: bool, message: str) -> None:
    if not condition:
        raise ProjectError(message)

def obj(value: Any, required: set[str], optional: set[str] = frozenset()) -> dict:
    require(isinstance(value, dict), "expected an object")
    require(required <= value.keys(), f"missing fields: {sorted(required - value.keys())}")
    require(not (value.keys() - required - optional), f"unknown fields: {sorted(value.keys() - required - optional)}")
    return value

def integer(n: Any, minimum: int, maximum: int) -> bool:
    return type(n) is int and minimum <= n <= maximum

def finite(n: Any) -> bool:
    return type(n) in (int, float) and math.isfinite(n) and abs(n) <= 1_000_000

def identity(value: Any) -> bool:
    return isinstance(value, str) and re.fullmatch(r"[A-Za-z0-9_-]{1,128}", value) is not None

def logical_path(value: Any) -> str:
    require(isinstance(value, str) and 0 < len(value.encode()) <= 4096, "invalid path length")
    require(not value.startswith("/") and not any(c in value for c in ("\\", ":"))
            and not any(ord(c) < 32 or 127 <= ord(c) < 160 for c in value), "non-canonical path")
    parts = [part for part in value.split("/") if part not in ("", ".")]
    require(bool(parts) and ".." not in parts, "path escapes root")
    return "/".join(parts)

def vector(value: Any, count: int, check=finite) -> bool:
    return isinstance(value, list) and len(value) == count and all(check(x) for x in value)

def color(value: Any) -> bool:
    return vector(value, 4, lambda n: integer(n, 0, 255))

def action(a: Any) -> None:
    require(isinstance(a, dict) and isinstance(a.get("op"), str), "missing action op")
    op = a["op"]
    if op in ("sprite", "text"):
        base = {"op", "id", "rect", "z"}
        if op == "sprite":
            obj(a, base | {"asset"}, {"tint"})
            logical_path(a["asset"])
            require(color(a.get("tint", [255] * 4)), "sprite tint")
        else:
            obj(a, base | {"text", "font", "size"}, {"color", "layout"})
            require(isinstance(a["text"], str) and len(a["text"].encode()) <= 1024 * 1024, "text length")
            require(isinstance(a["font"], str) and bool(a["font"]), "font alias")
            require(integer(a["size"], 1, 512), "font size")
            require(isinstance(a.get("layout", "simple-horizontal-v1"), str) and bool(a.get("layout", "simple-horizontal-v1")), "text layout")
            require(color(a.get("color", [255] * 4)), "text color")
        require(integer(a["id"], 0, 2**32 - 1), "visual id")
        require(integer(a["z"], -(2**31), 2**31 - 1), "z index")
        require(vector(a["rect"], 4, lambda n: integer(n, -(2**31), 2**31 - 1)), "rectangle")
        x, y, w, h = a["rect"]
        require(abs(x) <= 1_000_000 and abs(y) <= 1_000_000 and 0 < w <= 16384 and 0 < h <= 16384, "rectangle bounds")
    elif op in ("remove", "stop"):
        key = "id" if op == "remove" else "channel"
        obj(a, {"op", key})
        require(integer(a[key], 0, 2**32 - 1), "handle")
    elif op == "play":
        obj(a, {"op", "channel", "asset"}, {"looping", "gain_milli"})
        logical_path(a["asset"])
        require(integer(a["channel"], 0, 2**32 - 1), "audio channel")
        require(type(a.get("looping", False)) is bool and integer(a.get("gain_milli", 1000), 0, 1000), "audio settings")
    elif op == "wait":
        obj(a, {"op", "ms"})
        require(integer(a["ms"], 0, 86_400_000), "wait duration")
    elif op in ("wait_input", "halt"):
        obj(a, {"op"})
    else:
        raise ProjectError(f"unsupported action: {op}")

def siglus_config(config: Any) -> dict:
    config = obj(config, {"scene"}, {"format", "string_codec", "scene_name", "gameexe", "resources", "message_window"})
    def path(value):
        require(logical_path(value) == value, "non-canonical native config path")
    path(config["scene"])
    require(config.get("format", "pck") in ("pck", "scn"), "unsupported native format")
    require(config.get("string_codec", "xor") in ("plain", "xor"), "unsupported native string codec")
    name = config.get("scene_name")
    require(name is None or isinstance(name, str) and 0 < len(name.encode()) <= 256 and not any(ord(c) < 32 or 127 <= ord(c) < 160 for c in name), "native scene name")
    if config.get("gameexe") is not None:
        path(config["gameexe"])
    resources = config.get("resources", {})
    require(isinstance(resources, dict) and len(resources) <= 4096, "native resources")
    for alias, source in resources.items():
        path(alias)
        path(source)
    if config.get("message_window") is not None:
        require(config.get("gameexe") is None, "native Gameexe MWND conversion is not implemented")
        window = obj(config["message_window"], {"font", "rect", "name_rect", "size", "name_size", "spacing", "color", "name_color"})
        require(isinstance(window["font"], str) and 0 < len(window["font"].encode()) <= 256
                and not any(ord(c) < 32 or 127 <= ord(c) < 160 for c in window["font"]), "native message font alias")
        for key in ("rect", "name_rect"):
            rect = window[key]
            require(vector(rect, 4, lambda n: integer(n, -32767, 32767))
                    and rect[2] > 0 and rect[3] > 0, "native message rectangle")
        require(integer(window["size"], 1, 512) and integer(window["name_size"], 1, 512), "native message size")
        require(vector(window["spacing"], 2, lambda n: integer(n, -511, 4096))
                and window["size"] + window["spacing"][1] > 0, "native message spacing")
        require(color(window["color"]) and color(window["name_color"]), "native message colors")
    return config


def reallive_config(config: Any) -> dict:
    """Native RealLive bytecode adapter; no hosted-engine configuration."""
    config = obj(config, set(), {"root", "font_alias", "encoding", "start_scene"})
    root = config.get("root", "")
    require(isinstance(root, str) and (not root or logical_path(root) == root), "canonical RealLive root required")
    require(isinstance(config.get("font_alias", "default"), str) and bool(config.get("font_alias", "default")), "font alias required")
    encoding = config.get("encoding")
    if encoding is not None:
        require(isinstance(encoding, str), "RealLive encoding")
        key = "".join(c.lower() for c in encoding if c.isascii() and c.isalnum())
        require(key in {"sjis", "shiftjis", "cp932", "ms932", "ja", "gbk", "gb2312", "gb18030", "cp936", "chs", "zhcn", "big5", "cp950", "cht", "zhtw", "utf8", "utf", "unicode", "western", "cp1252", "latin1", "en", "korean", "cp949", "euckr", "ko"}, "unknown RealLive encoding")
    require(config.get("start_scene") is None or integer(config["start_scene"], -(2**31), 2**31-1), "RealLive start scene")
    return config


def validate(p: Any) -> dict:
    obj(p, {"schema", "name", "content_revision", "canvas", "assets", "clients"}, {"background", "fonts", "scene3d"})
    require(type(p["schema"]) is int and p["schema"] == 2, "expected schema 2")
    for key in ("name", "content_revision"):
        require(isinstance(p[key], str) and 0 < len(p[key].encode()) <= 256 and not any(ord(c) < 32 or 127 <= ord(c) < 160 for c in p[key]), f"invalid {key}")
    require(vector(p["canvas"], 2, lambda n: integer(n, 1, 8192)), "canvas")
    require(color(p.get("background", [16, 20, 30, 255])), "background")
    logical_path(p["assets"])
    clients = p["clients"]
    require(isinstance(clients, list) and 1 <= len(clients) <= 64, "client count")
    names: set[str] = set()
    for c in clients:
        obj(c, {"name", "adapter", "game"}, {"asset_prefix", "config", "program", "systems"})
        require(identity(c["name"]) and c["name"] not in names and identity(c["adapter"]) and identity(c["game"]), "client identity")
        names.add(c["name"])
        prefix = c.get("asset_prefix", "")
        require(isinstance(prefix, str), "asset prefix must be a string")
        if prefix:
            logical_path(prefix)
        program = c.get("program", [])
        require(isinstance(program, list) and len(program) <= 100_000, "program length")
        if c["adapter"] == "siglus":
            require(not program, "Siglus uses original bytecode, not data VM actions")
            siglus_config(c.get("config"))
        if c["adapter"] == "reallive":
            require(not program, "RealLive uses original bytecode, not data VM actions")
            reallive_config(c.get("config"))
        require(c["adapter"] not in ("bgi", "rfvp"), "hosted engine adapter removed; CrySirror API bridge required")
        for a in program:
            action(a)
        if c.get("systems") is not None:
            import engine_schema
            engine_schema.validate(c["systems"])
    for section in ("entities", "layers", "widgets"):
        require(sum(len((c.get("systems") or {}).get(section,[])) for c in clients)<=4096, f"global {section} limit")
    fonts = p.get("fonts", {})
    require(isinstance(fonts, dict), "fonts must be an object")
    for alias, path in fonts.items():
        require(identity(alias), "font alias")
        logical_path(path)
    scene = p.get("scene3d")
    if scene is not None:
        obj(scene, {"camera", "meshes"})
        camera = obj(scene["camera"], {"eye", "target", "up", "fov_y_degrees", "near", "far"})
        for k in ("eye", "target", "up"):
            require(vector(camera[k], 3), "camera vector")
        require(finite(camera["fov_y_degrees"]) and 1 <= camera["fov_y_degrees"] < 179, "camera FOV")
        require(finite(camera["near"]) and finite(camera["far"]) and 0 < camera["near"] < camera["far"], "clip planes")
        d = [a-b for a, b in zip(camera["target"], camera["eye"])]
        u = camera["up"]
        cross = [d[1]*u[2]-d[2]*u[1], d[2]*u[0]-d[0]*u[2], d[0]*u[1]-d[1]*u[0]]
        require(sum(x*x for x in cross) > 1e-12, "degenerate camera")
        meshes = scene["meshes"]
        require(isinstance(meshes, list) and len(meshes) <= 4096, "mesh count")
        mesh_names: set[str] = set()
        total = 0
        for m in meshes:
            obj(m, {"name", "geometry"}, {"translation", "rotation_degrees", "scale", "color"})
            require(isinstance(m["name"], str) and bool(m["name"]) and m["name"] not in mesh_names, "mesh name")
            mesh_names.add(m["name"])
            for k in ("translation", "rotation_degrees", "scale"):
                require(vector(m.get(k, [1, 1, 1] if k == "scale" else [0, 0, 0]), 3), "mesh transform")
            require(all(abs(s) > 1e-6 for s in m.get("scale", [1, 1, 1])), "singular mesh scale")
            require(color(m.get("color", [255]*4)) and m.get("color", [255]*4)[3] == 255, "3D requires opaque color")
            g = m["geometry"]
            require(isinstance(g, dict), "geometry")
            if g.get("kind") == "cube":
                obj(g, {"kind"})
                total += 36
            else:
                obj(g, {"kind", "vertices", "indices"})
                require(g["kind"] == "triangles" and isinstance(g["vertices"], list) and bool(g["vertices"]), "triangle geometry")
                require(all(vector(v, 3) for v in g["vertices"]), "vertex")
                ix = g["indices"]
                require(isinstance(ix, list) and bool(ix) and len(ix)%3 == 0 and all(integer(i, 0, len(g["vertices"])-1) for i in ix), "triangle indices")
                total += len(ix)
            require(total <= 1_000_000, "expanded vertex count")
    return p

def load(path: Path) -> dict:
    with path.open("rb") as stream:
        data = stream.read(MAX_BYTES+1)
    require(len(data) <= MAX_BYTES, "project exceeds 4 MiB")
    def constant(value: str):
        raise ProjectError(f"non-finite JSON value: {value}")
    return validate(json.loads(data, parse_constant=constant))

def asset_inventory(path: Path, p: dict, *, require_fonts: bool = True) -> dict[str, Path]:
    """Enumerate declared assets only; native bytecode can request additional files."""
    root = path.resolve().parent
    assets = (root / logical_path(p["assets"])).resolve(strict=True)
    require(assets.is_relative_to(root) and assets.is_dir(), "asset directory escapes project")
    wanted = set(p.get("fonts", {}).values())
    for c in p["clients"]:
        require(c["adapter"] in ("crysirror-script-v1", "siglus", "reallive"), "native asset enumeration is not implemented")
        if c["adapter"] == "crysirror-script-v1":
            require(c.get("config") is None, "built-in data script does not accept native config")
        prefix = c.get("asset_prefix", "")
        base = assets
        if prefix:
            base = (assets/logical_path(prefix)).resolve(strict=True)
            require(base.is_relative_to(assets) and base.is_dir(), "client asset prefix escapes root")
        if c["adapter"] == "siglus":
            config = siglus_config(c.get("config"))
            native_files = [config["scene"], *config.get("resources", {}).values()]
            if config.get("gameexe") is not None:
                native_files.append(config["gameexe"])
            for file in native_files:
                source = (base / file).resolve(strict=True)
                require(source.is_relative_to(base), "native asset escapes client root")
                wanted.add("/".join(x for x in (prefix, file) if x))
        if c["adapter"] == "reallive":
            config = reallive_config(c.get("config"))
            folder = (base / config.get("root", "")).resolve(strict=True)
            require(folder.is_relative_to(base) and folder.is_dir(), "RealLive root escapes client root")
            # Bootstrap declarations only: bytecode can request further archives.
            names = {}
            for file in folder.iterdir():
                key = file.name.casefold()
                require(key not in names, "ambiguous RealLive case-insensitive filename")
                names[key] = file
            for name in ("gameexe.ini", "seen.txt"):
                require(name in names, f"RealLive bootstrap missing: {name}")
                source = names[name].resolve(strict=True)
                require(source.is_relative_to(base) and source.is_file(), "RealLive bootstrap escapes client root")
                wanted.add(source.relative_to(assets).as_posix())
            if require_fonts:
                require(config.get("font_alias", "default") in p.get("fonts", {}), "RealLive requires a licensed project font alias")
        systems = c.get("systems") or {}
        entities=list(systems.get("entities",[]))
        materials=[entity.get("material") for entity in entities]
        vfx_systems=[entity.get("vfx") for entity in entities]
        for widget in systems.get("widgets",[]):
            for action in widget.get("on_activate",[]):
                if action.get("op")=="set_material":materials.append(action.get("material"))
                elif action.get("op")=="set_vfx":vfx_systems.append(action.get("vfx"))
                elif action.get("op")=="spawn":
                    spawned=action["entity"]
                    entities.append(spawned)
                    materials.append(spawned.get("material"))
                    vfx_systems.append(spawned.get("vfx"))
        declared=[]
        hdr=(systems.get("environment")or{}).get("hdr")
        if hdr is not None:declared.append(hdr)
        for entity in entities:
            g=entity.get("geometry") or {}
            if g.get("kind")in("model","scene"):declared.append(g["asset"])
        for system in vfx_systems:
            if system is not None:
                import vfx_schema
                declared.extend(vfx_schema.asset_paths(system))
        for material in materials:
            effect=(material or {}).get("effect") or {}
            if effect.get("kind")=="custom":declared.append(effect["asset"])
            for key,value in (material or {}).items():
                if key.endswith("_texture") and value is not None:
                    source=(base/logical_path(value)).resolve(strict=True)
                    require(source.is_relative_to(base),"material asset escapes client root")
                    wanted.add("/".join(x for x in(prefix,value)if x))
        for file in declared:
            source=(base/logical_path(file)).resolve(strict=True)
            require(source.is_relative_to(base),"shader/model asset escapes client root")
            wanted.add("/".join(x for x in(prefix,file)if x))
        for layer in systems.get("layers", []):
            paint = layer["paint"]
            if paint["kind"] == "image":
                source = (base / logical_path(paint["asset"])).resolve(strict=True)
                require(source.is_relative_to(base), "layer asset escapes client root")
                wanted.add("/".join(x for x in (prefix,paint["asset"]) if x))
            if paint["kind"] == "puppet":
                # The rig and the textures it names (ADR 0029). Vendor
                # containers never enter a package.
                rig_path = (base / logical_path(paint["asset"])).resolve(strict=True)
                require(rig_path.is_relative_to(base), "puppet rig escapes client root")
                rig = json.loads(rig_path.read_text(encoding="utf-8"))
                require(rig.get("format") == "crysirror-puppet" and rig.get("version") == 1, "not a crysirror-puppet v1 rig")
                folder = paint["asset"].rsplit("/", 1)[0] if "/" in paint["asset"] else ""
                for name in [paint["asset"]] + ["/".join(x for x in (folder, t) if x) for t in rig.get("textures", [])]:
                    require(not name.lower().endswith(".psb"), "packages never contain PSB")
                    source = (base / logical_path(name)).resolve(strict=True)
                    require(source.is_relative_to(base), "puppet asset escapes client root")
                    wanted.add("/".join(x for x in (prefix, name) if x))
            if require_fonts and paint["kind"] in ("text", "glyph_run"):
                require(paint["font"] in p.get("fonts",{}), "packaging layers requires a licensed project font")
        for widget in systems.get("widgets", []):
            if require_fonts and widget.get("text"):
                require(widget.get("font") in p.get("fonts",{}), "packaging widgets requires a licensed project font")
        for a in c.get("program", []):
            if a["op"] in ("sprite", "play"):
                source = (base / logical_path(a["asset"])).resolve(strict=True)
                require(source.is_relative_to(base), "asset escapes client root")
                wanted.add("/".join(x for x in (prefix, a["asset"]) if x))
            if a["op"] == "text":
                require(a.get("layout", "simple-horizontal-v1") == "simple-horizontal-v1", "unsupported native text layout")
            if require_fonts and a["op"] == "text":
                require(a["font"] in p.get("fonts", {}), "packaging text requires an explicitly licensed project font")
    result = {}
    for name in sorted(wanted):
        logical = logical_path(name)
        source = (assets/logical).resolve(strict=True)
        require(source.is_relative_to(assets) and source.is_file(), f"asset escapes root: {name}")
        require(source.stat().st_size <= 64*1024*1024, "asset exceeds 64 MiB")
        result[logical] = source
    return result

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("project", type=Path)
    parser.add_argument("--assets", action="store_true")
    args = parser.parse_args()
    try:
        p = load(args.project)
        result = {"schema_valid": True, "name": p["name"], "native_vms_verified": False}
        if args.assets:
            result["asset_files"] = len(asset_inventory(args.project, p))
        print(json.dumps(result, ensure_ascii=False))
    except (ValueError, OSError, TypeError) as exc:
        parser.exit(1, f"CrySirror: {exc}\n")
if __name__ == "__main__":
    main()
