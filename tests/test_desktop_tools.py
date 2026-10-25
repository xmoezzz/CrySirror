# SPDX-License-Identifier: MPL-2.0
from __future__ import annotations
import copy
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import desktop_project as d
import package_game as pack
from mcp_server import Server

DEMO = ROOT / "examples/desktop/project.json"
class ProjectTests(unittest.TestCase):
    def setUp(self):
        self.project = d.load(DEMO)
    def test_demo_is_valid(self):
        d.validate(self.project)
    def test_asset_namespace_separates_duplicate_names(self):
        files = d.asset_inventory(DEMO, self.project)
        self.assertIn("a/panel.png", files)
        self.assertIn("b/panel.png", files)
        self.assertNotEqual(files["a/panel.png"].read_bytes(), files["b/panel.png"].read_bytes())
    def test_audio_fixture(self):
        p = DEMO.with_name("audio.json")
        self.assertIn("tone.wav", d.asset_inventory(p, d.load(p)))
    def test_puppet_layers_package_rig_and_textures_but_never_psb(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "assets/chara").mkdir(parents=True)
            rig = {"format": "crysirror-puppet", "version": 1, "size": [10, 10], "textures": ["t.png"], "parts": []}
            (root / "assets/chara/rig.json").write_text(json.dumps(rig))
            (root / "assets/chara/t.png").write_bytes(b"png")
            p = {"schema": 2, "name": "p", "content_revision": "1", "canvas": [64, 64], "assets": "assets",
                 "clients": [{"name": "g", "adapter": "crysirror-script-v1", "game": "g",
                              "systems": {"layers": [{"id": 1, "rect": [0, 0, 64, 64],
                                                      "paint": {"kind": "puppet", "asset": "chara/rig.json"}}]}}]}
            (root / "project.json").write_text(json.dumps(p))
            files = d.asset_inventory(root / "project.json", d.load(root / "project.json"))
            self.assertIn("chara/rig.json", files)
            self.assertIn("chara/t.png", files)
            rig["textures"] = ["model.psb"]
            (root / "assets/chara/model.psb").write_bytes(b"PSB")
            (root / "assets/chara/rig.json").write_text(json.dumps(rig))
            with self.assertRaises(Exception):
                d.asset_inventory(root / "project.json", d.load(root / "project.json"))
            p["clients"][0]["systems"]["layers"][0]["paint"]["asset"] = "chara/model.psb"
            (root / "project.json").write_text(json.dumps(p))
            with self.assertRaises(Exception):
                d.load(root / "project.json")
    def test_text_fixture_requires_user_font_to_package(self):
        p = DEMO.with_name("text.json")
        d.load(p)
        with self.assertRaisesRegex(d.ProjectError, "licensed"):
            d.asset_inventory(p, d.load(p))
    def test_package_rejects_unimplemented_text_layout(self):
        p = d.load(DEMO.with_name("text.json"))
        for c in p["clients"]:
            for a in c.get("program", []):
                if a["op"] == "text":
                    a["layout"] = "siglus-ruby"
        with self.assertRaisesRegex(d.ProjectError, "layout"):
            d.asset_inventory(DEMO, p, require_fonts=False)
    def test_config_is_not_silently_ignored_by_packager(self):
        self.project["clients"][0]["config"] = {"native_entry": "scene"}
        with self.assertRaisesRegex(d.ProjectError, "config"):
            d.asset_inventory(DEMO, self.project)
    def test_symlink_cannot_cross_between_client_roots(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            shutil.copytree(DEMO.parent, root / "demo")
            alias = root / "demo/assets/a/link.png"
            try:
                alias.symlink_to("../b/panel.png")
            except OSError:
                self.skipTest("host does not permit creating test symlinks")
            p = d.load(root / "demo/project.json")
            p["clients"][0]["program"][0]["asset"] = "link.png"
            with self.assertRaisesRegex(d.ProjectError, "client root"):
                d.asset_inventory(root / "demo/project.json", p)
    def test_unknown_fields(self):
        self.project["future_typo"] = True
        with self.assertRaises(d.ProjectError):
            d.validate(self.project)
    def test_unknown_action(self):
        self.project["clients"][0]["program"].append({"op": "silently_ignore"})
        with self.assertRaises(d.ProjectError):
            d.validate(self.project)
    def test_native_adapter_is_not_packaged_as_demo(self):
        for adapter in ("rfvp", "bgi"):
            self.project["clients"][0]["adapter"] = adapter
            with self.subTest(adapter=adapter), self.assertRaisesRegex(d.ProjectError, "hosted engine adapter removed"):
                d.validate(self.project)
    def test_duplicate_clients(self):
        self.project["clients"].append(copy.deepcopy(self.project["clients"][0]))
        with self.assertRaises(d.ProjectError):
            d.validate(self.project)
    def test_boolean_is_not_integer(self):
        self.project["canvas"][0] = True
        with self.assertRaises(d.ProjectError):
            d.validate(self.project)
    def test_parent_traversal(self):
        for value in ("../x", "a/../x", "/x", "C:/x", "a\\b", "a\0b"):
            with self.subTest(value=value), self.assertRaises(d.ProjectError):
                d.logical_path(value)
    def test_empty_and_dot_path(self):
        for value in ("", ".", "//", "./"):
            with self.subTest(value=value), self.assertRaises(d.ProjectError):
                d.logical_path(value)
    def test_camera_degeneracy(self):
        self.project["scene3d"]["camera"]["up"] = [0, 0, 0]
        with self.assertRaisesRegex(d.ProjectError, "degenerate"):
            d.validate(self.project)
    def test_nonfinite_camera(self):
        self.project["scene3d"]["camera"]["eye"][0] = float("nan")
        with self.assertRaises(d.ProjectError):
            d.validate(self.project)
    def test_triangle_index_bounds(self):
        self.project["scene3d"]["meshes"][0]["geometry"] = {"kind": "triangles", "vertices": [[0, 0, 0]], "indices": [0, 0, 1]}
        with self.assertRaisesRegex(d.ProjectError, "indices"):
            d.validate(self.project)
    def test_singular_scale(self):
        self.project["scene3d"]["meshes"][0]["scale"] = [1, 0, 1]
        with self.assertRaises(d.ProjectError):
            d.validate(self.project)
    def test_translucent_mesh_rejected(self):
        self.project["scene3d"]["meshes"][0]["color"][3] = 0
        with self.assertRaises(d.ProjectError):
            d.validate(self.project)
    def test_negative_sprite_dimensions(self):
        self.project["clients"][0]["program"][0]["rect"][2] = -1
        with self.assertRaises(d.ProjectError):
            d.validate(self.project)
    def test_symlink_escape_rejected(self):
        with tempfile.TemporaryDirectory() as tmp, tempfile.TemporaryDirectory() as outside:
            root = Path(tmp)
            (root/"assets").mkdir()
            (Path(outside)/"secret.png").write_bytes(b"not an asset")
            try:
                (root/"assets"/"escape").symlink_to(outside, target_is_directory=True)
            except OSError:
                self.skipTest("host does not permit creating test symlinks")
            p = copy.deepcopy(self.project)
            p["clients"] = [p["clients"][0]]
            p["clients"][0]["asset_prefix"] = "escape"
            p["clients"][0]["program"] = [{"op": "sprite", "id": 0, "asset": "secret.png", "rect": [0, 0, 10, 10], "z": 0}]
            project = root/"project.json"
            project.write_text(json.dumps(p))
            with self.assertRaises(d.ProjectError):
                d.asset_inventory(project, p)

class PackagingTests(unittest.TestCase):
    def test_data_only_is_explicit_and_hashes_match(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)/"game.zip"
            manifest = pack.package(DEMO, output, data_only=True)
            self.assertFalse(manifest["runtime_included"])
            with zipfile.ZipFile(output) as z:
                for name, item in manifest["files"].items():
                    self.assertEqual(hashlib.sha256(z.read(name)).hexdigest(), item["sha256"])
                self.assertFalse(any(name.endswith((".ttf", ".otf", ".exe")) for name in z.namelist()))
    def test_packages_include_own_license_and_truthful_release_status(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = Path(tmp)/"data.zip"
            manifest = pack.package(DEMO, output, data_only=True)
            self.assertFalse(manifest["third_party_notices_audited"])
            self.assertFalse(manifest["runtime_execution_verified"])
            with zipfile.ZipFile(output) as z:
                self.assertEqual(z.read("licenses/CrySirror-MPL-2.0.txt"), (ROOT/"LICENSE").read_bytes())
                self.assertIn("licenses/RELEASE-CHECKLIST.txt", z.namelist())

    def test_executable_packaging_includes_engine_source_without_claiming_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            exe = Path(tmp)/"header-only-fixture"
            exe.write_bytes(b"\x7fELF" + bytes(128))
            output = Path(tmp)/"game.zip"
            manifest = pack.package(DEMO, output, executable=exe)
            self.assertTrue(manifest["runtime_included"])
            self.assertFalse(manifest["runtime_execution_verified"])
            self.assertFalse(manifest["source_matches_binary_verified"])
            with zipfile.ZipFile(output) as z:
                self.assertIn("source/crysirror/crates/crysirror-desktop/src/main.rs", z.namelist())
                self.assertIn("launch.sh", z.namelist())
                self.assertNotIn("launch.cmd", z.namelist())

    def test_reproducible_data_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp)/"a.zip", Path(tmp)/"b.zip"
            pack.package(DEMO, a, data_only=True)
            pack.package(DEMO, b, data_only=True)
            self.assertEqual(a.read_bytes(), b.read_bytes())
    def test_no_fake_runtime_when_binary_missing(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(d.ProjectError):
                pack.package(DEMO, Path(tmp)/"game.zip")
    def test_rejects_non_executable_data(self):
        with tempfile.TemporaryDirectory() as tmp:
            exe = Path(tmp)/"not-a-binary"
            exe.write_bytes(b"not an executable")
            with self.assertRaises(ValueError):
                pack.package(DEMO, Path(tmp)/"game.zip", executable=exe)
    def test_format_target_mismatch(self):
        with tempfile.TemporaryDirectory() as tmp:
            exe = Path(tmp)/"header-only-test"
            exe.write_bytes(b"\x7fELF"+bytes(100))
            with self.assertRaisesRegex(ValueError, "target"):
                pack.package(DEMO, Path(tmp)/"game.zip", executable=exe, target="windows")

class McpTests(unittest.TestCase):
    def server(self, **kwargs):
        return Server(DEMO, **kwargs)
    def ready(self, server):
        reply = server.handle({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "clientInfo": {"name": "test", "version": "1"}, "capabilities": {}}})
        self.assertEqual(reply["result"]["protocolVersion"], "2025-06-18")
        self.assertIsNone(server.handle({"jsonrpc": "2.0", "method": "notifications/initialized"}))
    def test_handshake_and_capabilities(self):
        s = self.server()
        self.ready(s)
        response = s.handle({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        names = {t["name"] for t in response["result"]["tools"]}
        self.assertEqual(names, {"project_inspect", "project_validate"})
    def test_tools_before_initialization_fail(self):
        r = self.server().handle({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
        self.assertIn("error", r)
    def test_write_not_advertised_by_default(self):
        s = self.server()
        self.ready(s)
        r = s.handle({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "project_replace"}})
        self.assertEqual(r["error"]["code"], -32602)
    def test_inspection_reports_native_gap(self):
        data = json.loads(self.server().call("project_inspect", {})["content"][0]["text"])
        self.assertIsNone(data["registered_native_adapters"])
        self.assertFalse(data["registry_verified"])
        self.assertEqual(data["source_native_adapters"], ["reallive", "siglus"])
        self.assertEqual(len(data["sha256"]), 64)
    def test_validation_calls_actual_inventory(self):
        data = json.loads(self.server().call("project_validate", {})["content"][0]["text"])
        self.assertEqual(data["asset_files"], 4)
    def test_unknown_tool_is_protocol_error(self):
        s = self.server()
        self.ready(s)
        r = s.handle({"jsonrpc": "2.0", "id": "req", "method": "tools/call", "params": {"name": "shell"}})
        self.assertEqual(r["error"]["code"], -32602)
    def test_bad_tool_arguments_are_tool_error(self):
        s = self.server()
        self.ready(s)
        r = s.handle({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "project_inspect", "arguments": {"oops": 1}}})
        self.assertTrue(r["result"]["isError"])
    def test_parse_error_does_not_crash_stream(self):
        output = io.StringIO()
        self.server().serve(io.StringIO("not json\n"), output)
        self.assertEqual(json.loads(output.getvalue())["error"]["code"], -32700)
    def test_notifications_produce_no_output(self):
        output = io.StringIO()
        self.server().serve(io.StringIO('{"jsonrpc":"2.0","method":"notifications/cancelled"}\n'), output)
        self.assertEqual(output.getvalue(), "")
    def test_real_stdio_subprocess(self):
        messages = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "clientInfo": {}, "capabilities": {}}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
            {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "project_validate", "arguments": {}}},
        ]
        result = subprocess.run([sys.executable, str(ROOT/"tools/mcp_server.py"), "--project", str(DEMO)],
                                input="".join(json.dumps(m)+"\n" for m in messages), text=True,
                                capture_output=True, timeout=10, check=True)
        lines = result.stdout.splitlines()
        self.assertEqual(len(lines), 3)
        self.assertTrue(all(json.loads(line)["jsonrpc"] == "2.0" for line in lines))
        self.assertEqual(result.stderr, "")
    def test_replace_checks_hash_and_is_validated(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp)/"example"
            shutil.copytree(DEMO.parent, dest)
            s = Server(dest/"project.json", allow_write=True)
            info = json.loads(s.call("project_inspect", {})["content"][0]["text"])
            changed = info["project"]
            changed["background"] = [10, 20, 30, 255]
            response = s.call("project_replace", {"project": changed, "expected_sha256": info["sha256"]})
            self.assertTrue(json.loads(response["content"][0]["text"])["updated"])
            self.assertEqual(d.load(dest/"project.json")["background"], [10, 20, 30, 255])
            with self.assertRaisesRegex(ValueError, "changed"):
                s.call("project_replace", {"project": changed, "expected_sha256": info["sha256"]})
    def test_invalid_replacement_does_not_touch_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp)/"example"
            shutil.copytree(DEMO.parent, dest)
            s = Server(dest/"project.json", allow_write=True)
            before = s.project.read_bytes()
            invalid = d.load(s.project)
            invalid["canvas"] = [0, 0]
            with self.assertRaises(ValueError):
                s.call("project_replace", {"project": invalid, "expected_sha256": hashlib.sha256(before).hexdigest()})
            self.assertEqual(s.project.read_bytes(), before)

class SourceBoundaryTests(unittest.TestCase):
    def test_engine_audio_and_text_are_independent_subsystems(self):
        for name in ("crysirror-text", "crysirror-audio"):
            manifest=tomllib.loads((ROOT/"crates"/name/"Cargo.toml").read_text())
            self.assertFalse({"wgpu","winit","crysirror-runtime","crysirror-siglus"} & manifest["dependencies"].keys())
        renderer=tomllib.loads((ROOT/"crates/crysirror-wgpu/Cargo.toml").read_text())
        desktop=tomllib.loads((ROOT/"crates/crysirror-desktop/Cargo.toml").read_text())
        runtime=tomllib.loads((ROOT/"crates/crysirror-runtime/Cargo.toml").read_text())
        self.assertNotIn("ab_glyph", renderer["dependencies"])
        self.assertNotIn("rodio", desktop["dependencies"])
        self.assertNotIn("crysirror-compat", runtime["dependencies"])
        self.assertEqual((ROOT/"crates/crysirror-wgpu/src/text.rs").read_text().count("impl "),0)
    def test_desktop_really_declares_wgpu_and_winit(self):
        m = tomllib.loads((ROOT/"crates/crysirror-desktop/Cargo.toml").read_text())
        self.assertIn("winit", m["dependencies"])
        self.assertIn("wgpu", m["dependencies"])
    def test_renderer_is_not_bound_to_winit(self):
        m = tomllib.loads((ROOT/"crates/crysirror-wgpu/Cargo.toml").read_text())
        self.assertNotIn("winit", m["dependencies"])
    def test_core_remains_dependency_free(self):
        m = tomllib.loads((ROOT/"crates/crysirror-core/Cargo.toml").read_text())
        self.assertNotIn("dependencies", m)
    def test_workspace_members_exist(self):
        m = tomllib.loads((ROOT/"Cargo.toml").read_text())
        for member in m["workspace"]["members"]:
            self.assertTrue((ROOT/member/"Cargo.toml").is_file())
    def test_no_font_files_shipped(self):
        self.assertFalse([p for p in (ROOT/"examples").rglob("*") if p.suffix.lower() in (".ttf", ".otf", ".ttc", ".woff")])
    def test_rust_sources_have_no_unimplemented_macros(self):
        for p in (ROOT/"crates").rglob("*.rs"):
            self.assertNotIn("todo!", p.read_text())
            self.assertNotIn("unimplemented!", p.read_text())

if __name__ == "__main__":
    unittest.main()
