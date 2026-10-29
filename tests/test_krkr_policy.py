# SPDX-License-Identifier: MPL-2.0
"""Licence and source boundary of the optional KiriKiri adapter (ADR 0053)."""
import hashlib
import json
import pathlib
import re
import tomllib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
AGPL = {"crysirror-krkr", "crysirror-tjs2", "crysirror-kag"}


def manifests():
    for path in sorted((ROOT / "crates").rglob("Cargo.toml")):
        if "target" in path.parts:
            continue
        yield path, tomllib.loads(path.read_text())


class KrkrPolicy(unittest.TestCase):
    def test_agpl_is_confined_to_krkr_crates(self):
        for path, data in manifests():
            package = data.get("package", {})
            licence = package.get("license")
            if isinstance(licence, str) and "AGPL" in licence:
                self.assertIn(package["name"], AGPL, path)
            if package.get("name") in AGPL:
                self.assertEqual(licence, "AGPL-3.0-or-later", path)
                self.assertFalse(package.get("publish", True), path)

    def test_only_optional_desktop_feature_depends_on_krkr(self):
        for path, data in manifests():
            name = data.get("package", {}).get("name")
            if name in AGPL:
                continue
            deps = data.get("dependencies", {})
            for dep in AGPL:
                if dep not in deps:
                    continue
                self.assertEqual(name, "crysirror-desktop", f"{path} depends on {dep}")
                self.assertTrue(deps[dep].get("optional"), path)
        desktop = tomllib.loads((ROOT / "crates/crysirror-desktop/Cargo.toml").read_text())
        self.assertNotIn("krkr", desktop["features"]["default"])

    def test_derived_crates_keep_their_licence(self):
        for crate in ("crysirror-tjs2", "crysirror-kag"):
            self.assertTrue((ROOT / "crates" / crate / "LICENSE").is_file(), crate)

    def test_adapter_owns_no_renderer_or_mixer(self):
        manifest = tomllib.loads((ROOT / "crates/crysirror-krkr/Cargo.toml").read_text())
        deps = set(manifest["dependencies"])
        for forbidden in ("wgpu", "winit", "rodio", "cpal", "kira", "crysirror-wgpu", "crysirror-audio"):
            self.assertNotIn(forbidden, deps)
        source = "".join(p.read_text() for p in (ROOT / "crates/crysirror-krkr/src").glob("*.rs"))
        self.assertIsNone(re.search(r"\bunsafe\s*\{", source))


if __name__ == "__main__":
    unittest.main()
