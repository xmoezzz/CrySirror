# SPDX-License-Identifier: MPL-2.0
"""MCP/packaging validates the same authored effects as the Rust engine."""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import desktop_project as desktop
import engine_schema as engine
import vfx_schema as vfx


def effect(**overrides):
    value = {
        "parameters": {"wind": {"kind": "vector", "value": [0, 1, 0]},
                       "rate": {"kind": "float", "value": 20},
                       "tint": {"kind": "color", "value": [4, 2, 1, .5]},
                       "on": {"kind": "bool", "value": True}},
        "emitters": [{"id": "sparks", "capacity": 128, "enabled_parameter": "on",
                      "spawn": {"rate": {"parameter": "rate", "scale": 2, "offset": 1}},
                      "initialize": {"direction": {"parameter": "wind"}, "color_parameter": "tint"},
                      "update": [{"op": "acceleration", "value": {"parameter": "wind", "scale": 2}}]}],
    }
    value.update(overrides)
    return value


def project(system=None):
    return {"schema": 2, "name": "vfx", "content_revision": "1", "canvas": [64, 64], "assets": "assets",
            "clients": [{"name": "a", "adapter": "crysirror-script-v1", "game": "vfx", "asset_prefix": "a",
                         "systems": {"entities": [{"id": 1, "name": "effect", "vfx": effect() if system is None else system}]}}]}


class VfxSchemaTests(unittest.TestCase):
    def assert_bad(self, system):
        with self.assertRaises(desktop.ProjectError):
            vfx.validate(system)

    def test_minimal_and_real_serde_defaults_are_accepted_without_rewriting(self):
        vfx.validate({})
        docs = json.loads((ROOT / "docs/api/api.json").read_text())
        fields = next(d for d in docs["data"] if d["name"] == "VfxSystemSpec")["fields"]
        defaults = {f["name"]: f["default"] for f in fields}
        original = copy.deepcopy(defaults)
        self.assertIs(vfx.validate(defaults), defaults)
        self.assertEqual(defaults, original)
        desktop.validate(project())
        engine.command({"op": "set_vfx", "id": 1, "vfx": effect()})
        engine.command({"op": "set_vfx", "id": 1, "vfx": None})

    def test_every_shape_and_real_render_kind_and_all_cpu_modules(self):
        shapes = [{"kind": "point"}, {"kind": "sphere", "radius": 2, "surface": True},
                  {"kind": "box", "half_extent": [1, 2, 3]}, {"kind": "cone", "radius": 2, "height": 3},
                  {"kind": "line", "end": [1, 2, 3]}]
        modules = [{"op": "acceleration", "value": [0, -1, 0]}, {"op": "drag", "coefficient": .2},
                   {"op": "curl_noise", "strength": 2, "frequency": 3},
                   {"op": "attractor", "point": [1, 2, 3], "strength": -2, "radius": 3},
                   {"op": "orbit", "axis": [0, 1, 0], "speed": 2},
                   {"op": "rotate", "radians_per_second": 2},
                   {"op": "color_over_life", "curve": {"keys": [{"time": 0, "value": [5, 2, 1, 1]}, {"time": 1, "value": [1, 1, 1, 0]}]}},
                   {"op": "size_over_life", "curve": {"interpolation": "smooth", "keys": [{"time": 0, "value": [.1, .1]}, {"time": 1, "value": [1, 1]}]}},
                   {"op": "collision_plane", "normal": [0, 1, 0], "offset": 0, "restitution": .5, "friction": .1, "kill": False},
                   {"op": "collision_sphere", "center": [0, 0, 0], "radius": 2, "restitution": .2, "kill": True}]
        for shape in shapes:
            system = effect()
            system["emitters"][0]["initialize"]["shape"] = shape
            system["emitters"][0]["update"] = modules
            vfx.validate(system)
        renders = [{"kind": "sprite", "texture": "fx/sparks.png", "columns": 4, "rows": 2, "frames": 7, "fps": 24,
                    "soft_distance": .5, "facing": {"kind": "axis", "axis": [0, 1, 0]}, "blend": "premultiplied"},
                   {"kind": "sprite", "facing": {"kind": "velocity"}, "blend": "multiply"},
                   {"kind": "mesh", "mesh": {"kind": "cube"}}, {"kind": "mesh", "mesh": {"kind": "quad"}},
                   {"kind": "mesh", "mesh": {"kind": "asset", "path": "fx/petal.glb"}, "texture": "fx/petal.png"},
                   {"kind": "ribbon", "width": .1, "uv_scale": 2, "max_segments": 20, "blend": "alpha"}]
        for renderer in renders:
            system = effect()
            system["emitters"][0]["render"] = renderer
            vfx.validate(system)
        system["emitters"][0]["render"] = {"kind": "mesh"}
        system["emitters"][0]["target"] = "gpu"
        vfx.validate(system)

    def test_parameter_bindings_are_typed_and_resolved_with_f32_bounds(self):
        value = effect()
        self.assertEqual(vfx.scalar({"parameter": "rate", "scale": 2, "offset": 1}, value["parameters"]), 41)
        cases = [({"kind": "float", "value": True}), ({"kind": "bool", "value": 1}),
                 ({"kind": "vector", "value": [1, 2]}), ({"kind": "color", "value": [1, 1, 1, 2]}),
                 ({"kind": "color", "value": [-1, 1, 1, 1]}), ({"kind": "float", "value": 1000001}),
                 ({"kind": "shader", "value": "code"}), ({"kind": "float", "value": 2, "source": "code"})]
        for parameter in cases:
            with self.subTest(parameter=parameter):
                value = effect(); value["parameters"]["rate"] = parameter
                self.assert_bad(value)
        for binding in [{"parameter": "missing"}, {"parameter": "wind"}, {"parameter": "rate", "scale": True},
                        {"parameter": "rate", "offset": float("nan")}, {"parameter": "rate", "scale": 1000000},
                        {"parameter": "rate", "client": "other"}, {"parameter": "rate", "offset": 1e300}]:
            with self.subTest(binding=binding):
                value = effect(); value["emitters"][0]["spawn"]["rate"] = binding
                self.assert_bad(value)
        value = effect(); value["emitters"][0]["initialize"]["direction"] = {"parameter": "rate"}
        self.assert_bad(value)
        value = effect(); value["emitters"][0]["initialize"]["size_parameter"] = "wind"
        vfx.validate(value)
        value["parameters"]["wind"]["value"][0] = -1
        self.assert_bad(value)

    def test_numbers_booleans_lists_names_and_unknown_fields_fail_visibly(self):
        cases = [{"revision": True}, {"revision": 2**64}, {"fixed_step_hz": 0}, {"fixed_step_hz": 1.5},
                 {"max_steps_per_advance": 4097}, {"time_scale": 17}, {"warmup_seconds": 11},
                 {"paused": 1}, {"enabled": None}, {"emitters": []}, {"emitters": [{}]*65},
                 {"parameters": []}, {"parameters": {"x\0": {"kind": "float", "value": 1}}},
                 {"parameters": {"x"*129: {"kind": "float", "value": 1}}}, {"source": "shader code"}]
        for fields in cases:
            with self.subTest(fields=fields): self.assert_bad(effect(**fields))
        for key, bad in [("id", "a\n"), ("capacity", True), ("capacity", 200001), ("seed", -1),
                         ("target", "cpu_fallback"), ("space", "screen"), ("overflow", "overwrite"),
                         ("initialize", {"lifetime": [0, 1]}), ("initialize", {"direction": [0, 0, 0]}),
                         ("initialize", {"ribbon_id": 2**32}), ("initialize", {"size": [-1, 1]}),
                         ("initialize", {"rotation": [2, 1]}), ("initialize", {"speed": [1, -1]}),
                         ("initialize", {"spread_degrees": 181}), ("initialize", {"shape": {"kind": "sphere", "radius": 1}}),
                         ("spawn", {"duration": 0}), ("spawn", {"bursts": [{"time": 5, "count": 1}]}),
                         ("spawn", {"auto_start": 1}), ("spawn", {"rate": -1}), ("hidden_vm", {})]:
            with self.subTest(key=key, bad=bad):
                value = effect(); value["emitters"][0][key] = bad; self.assert_bad(value)
        for bad in [float("nan"), float("inf"), -float("inf"), 10**1000, True, None, "1"]:
            value = effect(); value["emitters"][0]["initialize"]["lifetime"] = [1, bad]
            self.assert_bad(value)
        for path, fields in [("spawn", {"typo": 1}), ("initialize", {"shader": "code"}),
                             ("render", {"kind": "sprite", "typo": True})]:
            value = effect(); value["emitters"][0][path] = fields; self.assert_bad(value)

    def test_lifetime_curves_and_module_order_are_checked(self):
        for times in [[0, 0], [0, .5], [.1, 1], [0, .8, .7, 1], [0, float("nan"), 1], [0, 1]*17]:
            value = effect(); value["emitters"][0]["update"] = [{"op": "size_over_life", "curve": {"keys": [{"time": t, "value": [1, 1]} for t in times]}}]
            self.assert_bad(value)
        for instruction in [{"op": "rotate", "radians_per_second": 1, "typo": True},
                            {"op": "drag", "coefficient": -1}, {"op": "curl_noise", "strength": 1, "frequency": -1},
                            {"op": "attractor", "point": [0, 0, 0], "strength": 1, "radius": 0},
                            {"op": "orbit", "axis": [0, 0, 0], "speed": 2}, {"op": "shader", "source": "code"},
                            {"op": "collision_plane", "normal": [0, 1, 0], "offset": 0, "restitution": 2, "friction": 0, "kill": False}]:
            value = effect(); value["emitters"][0]["update"] = [instruction]; self.assert_bad(value)
        value = effect(); value["emitters"][0]["update"] = [{"op": "collision_sphere", "center": [0, 0, 0], "radius": 1, "restitution": 1, "kill": False}, {"op": "drag", "coefficient": 1}]
        self.assert_bad(value)
        value["emitters"][0]["update"].reverse(); vfx.validate(value)
        for arity, values in [(1, [0, 1]), (2, [[0, 0], [1, 1]]), (4, [[1, 1, 1, 1], [1, 1, 1, 0]])]:
            vfx.curve({"keys": [{"time": 0, "value": values[0]}, {"time": 1, "value": values[1]}]}, arity)

    def test_sub_emitters_are_system_local_and_backend_limits_are_explicit(self):
        value = effect(); parent = value["emitters"][0]
        parent["events"] = [{"event": "collision", "emitter": "child", "count": 4, "inherit_velocity": .5}]
        value["emitters"].append({"id": "child", "spawn": {"auto_start": False}, "render": {"kind": "ribbon", "width": .1}})
        vfx.validate(value)
        for name, changes in [("missing", {}), ("child", {"target": "gpu"})]:
            bad = copy.deepcopy(value); bad["emitters"][0]["events"][0]["emitter"] = name; bad["emitters"][1].update(changes)
            self.assert_bad(bad)
        bad = copy.deepcopy(value); bad["emitters"][1]["id"] = "sparks"; self.assert_bad(bad)
        for fields in [{"overflow": "kill_oldest"}, {"render": {"kind": "ribbon", "width": .1}},
                       {"events": [{"event": "birth", "emitter": "sparks", "count": 1}]}]:
            bad = effect(); bad["emitters"][0].update(target="gpu", **fields); self.assert_bad(bad)
        for fields in [{"count": 4097}, {"probability": -1}, {"inherit_velocity": 17}, {"event": "script"}, {"client": "b"}]:
            bad = copy.deepcopy(value); bad["emitters"][0]["events"][0].update(fields); self.assert_bad(bad)

    def test_render_validation_rejects_unknowns_and_capability_escape_paths(self):
        cases = [{"kind": "unknown"}, {"kind": "mesh", "mesh": {"kind": "ribbon"}},
                 {"kind": "sprite", "columns": 2}, {"kind": "sprite", "frames": 2, "texture": "a.png"},
                 {"kind": "sprite", "fps": -1}, {"kind": "sprite", "soft_distance": float("inf")},
                 {"kind": "sprite", "blend": "oit"}, {"kind": "sprite", "facing": {"kind": "axis", "axis": [0, 0, 0]}},
                 {"kind": "ribbon", "width": 0}, {"kind": "ribbon", "width": .1, "max_segments": True},
                 {"kind": "mesh", "mesh": {"kind": "asset", "path": "mesh.glb", "client": "b"}}]
        for renderer in cases:
            value = effect(); value["emitters"][0]["render"] = renderer; self.assert_bad(value)
        for path in ["../other/a.png", "a//b.png", "./a.png", "/a.png", "C:/a.png", "a\\b.png", "a\0b", "x"*1025, "\ud800"]:
            for renderer in [{"kind": "sprite", "texture": path}, {"kind": "mesh", "mesh": {"kind": "asset", "path": path}}]:
                value = effect(); value["emitters"][0]["render"] = renderer; self.assert_bad(value)

    def test_entity_and_widget_mutual_exclusion_and_local_target_identity(self):
        p = project(); entity = p["clients"][0]["systems"]["entities"][0]
        entity["particles"] = {}
        with self.assertRaises(desktop.ProjectError): desktop.validate(p)
        entity["vfx"] = None; desktop.validate(p)
        systems = p["clients"][0]["systems"]
        systems["widgets"] = [{"id": 1, "rect": [0, 0, 10, 10], "on_activate": [{"op": "set_vfx", "id": 1, "vfx": effect()}]}]
        with self.assertRaises(desktop.ProjectError): desktop.validate(p)
        systems["widgets"][0]["on_activate"].insert(0, {"op": "set_particles", "id": 1, "particles": None})
        desktop.validate(p)
        systems["widgets"][0]["on_activate"][-1]["id"] = 999
        with self.assertRaisesRegex(desktop.ProjectError, "target"): desktop.validate(p)
        p["clients"].append({"name": "b", "game": "b", "adapter": "crysirror-script-v1", "systems": {"entities": [{"id": 999, "name": "foreign"}]}})
        with self.assertRaisesRegex(desktop.ProjectError, "target"): desktop.validate(p)


class VfxInventoryTests(unittest.TestCase):
    def make_project(self, root):
        p = project(); p["clients"].append(copy.deepcopy(p["clients"][0]))
        p["clients"][1].update(name="b", asset_prefix="b")
        for client in p["clients"]:
            folder = root / "assets" / client["name"]; folder.mkdir(parents=True)
            for name in ["shared.png", "particle.glb", "spawn.png", "action.png", "trail.png"]:
                (folder / name).write_bytes(f"authored {client['name']} {name}".encode())
            system = client["systems"]["entities"][0]["vfx"]
            system["emitters"][0]["render"] = {"kind": "sprite", "texture": "shared.png"}
            system["emitters"].append({"id": "mesh", "render": {"kind": "mesh", "mesh": {"kind": "asset", "path": "particle.glb"}}})
            spawned = {"id": 2, "name": "spawned", "vfx": {"emitters": [{"render": {"kind": "sprite", "texture": "spawn.png"}}]}}
            command_system = {"emitters": [{"render": {"kind": "mesh", "texture": "action.png", "mesh": {"kind": "asset", "path": "particle.glb"}}},
                                           {"id": "trail", "render": {"kind": "ribbon", "width": .1, "texture": "trail.png"}}]}
            client["systems"]["widgets"] = [{"id": 1, "rect": [0, 0, 10, 10], "on_activate": [
                {"op": "spawn", "entity": spawned}, {"op": "set_vfx", "id": 2, "vfx": command_system}, {"op": "set_vfx", "id": 1, "vfx": None}]}]
        path = root / "project.json"; path.write_text(json.dumps(p)); desktop.validate(p)
        return path, p

    def test_assets_include_initial_spawn_and_set_vfx_actions_with_scoped_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            path, p = self.make_project(Path(tmp))
            inventory = desktop.asset_inventory(path, p)
            self.assertEqual(set(inventory), {f"{c}/{n}" for c in ("a", "b") for n in ("shared.png", "particle.glb", "spawn.png", "action.png", "trail.png")})
            self.assertNotEqual(inventory["a/shared.png"].read_bytes(), inventory["b/shared.png"].read_bytes())
            from package_game import package
            manifest = package(path, Path(tmp)/"vfx.zip", data_only=True)
            self.assertIn("game/assets/a/spawn.png", manifest["files"])
            self.assertIn("game/assets/b/trail.png", manifest["files"])

    def test_initial_and_action_assets_cannot_symlink_into_other_clients_or_outside(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); path, p = self.make_project(root)
            for name in ("shared.png", "particle.glb", "spawn.png", "action.png", "trail.png"):
                source = root / "assets/a" / name; data = source.read_bytes(); source.unlink()
                source.symlink_to(root / "assets/b" / name)
                with self.subTest(name=name), self.assertRaisesRegex(desktop.ProjectError, "escapes client root"):
                    desktop.asset_inventory(path, p)
                source.unlink(); source.write_bytes(data)
            outside = root / "outside.png"; outside.write_bytes(b"outside capability")
            source = root / "assets/a/shared.png"; source.unlink(); source.symlink_to(outside)
            with self.assertRaisesRegex(desktop.ProjectError, "escapes client root"): desktop.asset_inventory(path, p)

    def test_mcp_replace_validates_vfx_before_any_write(self):
        from mcp_server import Server
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); path, p = self.make_project(root)
            server = Server(path, allow_write=True)
            original = path.read_bytes()
            bad = copy.deepcopy(p); bad["clients"][0]["systems"]["entities"][0]["vfx"]["emitters"][0]["render"] = {"kind": "unknown"}
            with patch("mcp_server.subprocess.run") as run, self.assertRaises(desktop.ProjectError):
                server.call("project_replace", {"expected_sha256": hashlib.sha256(original).hexdigest(), "project": bad})
            run.assert_not_called()
            self.assertEqual(path.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
