# SPDX-License-Identifier: MPL-2.0
"""BGI configuration and capability boundaries; no game or font bytes."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import desktop_project as d
import package_game


class BgiProjects(unittest.TestCase):
    def setUp(self):
        self.project = {
            "schema": 2, "name": "BGI fixture", "content_revision": "1",
            "canvas": [320, 180], "assets": "assets",
            "clients": [{"name": "bgi", "adapter": "bgi", "game": "fixture",
                         "asset_prefix": "game", "config": {"root": "native", "font": "font.bin"}}],
        }

    def test_strict_defaults_and_no_data_vm_substitute(self):
        d.validate(self.project)
        for key, value in [("strict", False), ("strict", 1), ("silent", "true"),
                           ("gbk", 0), ("font_face_index", -1), ("font_face_index", 2**32),
                           ("unknown", True)]:
            project = copy.deepcopy(self.project)
            project["clients"][0]["config"][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(d.ProjectError):
                d.validate(project)
        self.project["clients"][0]["program"] = [{"op": "halt"}]
        with self.assertRaisesRegex(d.ProjectError, "original bytecode"):
            d.validate(self.project)

    def test_paths_are_canonical_client_capabilities(self):
        for key in ("root", "font"):
            for path in ("../escape", "/absolute", "./relative", "a//b", "C:/host", "a\\b"):
                project = copy.deepcopy(self.project)
                project["clients"][0]["config"][key] = path
                with self.subTest(key=key, path=path), self.assertRaises(d.ProjectError):
                    d.validate(project)

    def test_same_save_namespace_cannot_have_two_native_writers(self):
        second = copy.deepcopy(self.project["clients"][0])
        second["name"] = "another"
        self.project["clients"].append(second)
        with self.assertRaisesRegex(d.ProjectError, "same game's save namespace"):
            d.validate(self.project)
        second["game"] = "independent"
        d.validate(self.project)

    def test_inventory_checks_declared_font_and_root_without_claiming_full_native_scan(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            base = root / "assets/game"
            (base / "native").mkdir(parents=True)
            (base / "font.bin").write_bytes(b"not-a-font; inventory does not execute")
            path = root / "project.json"
            path.write_text(json.dumps(self.project))
            self.assertEqual(set(d.asset_inventory(path, d.load(path))), {"game/font.bin"})
            with self.assertRaisesRegex(d.ProjectError, "complete resource inventory"):
                package_game.package(path, root / "native.zip", data_only=True)

    def test_native_root_and_font_cannot_escape_through_symlinks(self):
        for escaped in ("native", "font.bin"):
            with self.subTest(escaped=escaped), tempfile.TemporaryDirectory() as temp, tempfile.TemporaryDirectory() as outside:
                root = Path(temp)
                base = root / "assets/game"
                (base / "native").mkdir(parents=True)
                (base / "font.bin").write_bytes(b"synthetic")
                if escaped == "native":
                    (base / "native").rmdir()
                    (base / "native").symlink_to(outside, target_is_directory=True)
                else:
                    (base / "font.bin").unlink()
                    target = Path(outside) / "font.bin"
                    target.write_bytes(b"outside")
                    (base / "font.bin").symlink_to(target)
                with self.assertRaisesRegex(d.ProjectError, "escapes client root"):
                    d.asset_inventory(root / "project.json", self.project)
