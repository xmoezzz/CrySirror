import hashlib
import json
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class RfvpPolicyTest(unittest.TestCase):
    def test_vendored_sources_match_pinned_hashes(self):
        base = ROOT / "crates/rfvp-upstream"
        source = json.loads((base / "SOURCE.json").read_text())
        self.assertRegex(source["commit"], r"^[0-9a-f]{40}$")
        self.assertEqual(source["license"], "MPL-2.0")
        recorded = source["source_sha256"]
        present = {p.relative_to(base).as_posix() for p in (base / "rfvp/src").rglob("*") if p.is_file()}
        self.assertEqual(present, set(recorded))
        for rel, digest in recorded.items():
            self.assertEqual(hashlib.sha256((base / rel).read_bytes()).hexdigest(), digest, rel)

    def test_no_proprietary_fonts_vendored(self):
        base = ROOT / "crates/rfvp-upstream"
        fonts = [p for p in base.rglob("*") if p.suffix.lower() in (".ttf", ".ttc", ".otf")]
        self.assertEqual(fonts, [])

    def test_core_builds_only_the_no_std_profile(self):
        manifest = tomllib.loads((ROOT / "crates/rfvp-upstream/rfvp/Cargo.toml").read_text())
        self.assertEqual(manifest["features"]["default"], ["no_std"])
        self.assertFalse(manifest["package"]["publish"])
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
        dep = workspace["workspace"]["dependencies"]["rfvp"]
        self.assertFalse(dep["default-features"])
        self.assertEqual(dep["features"], ["no_std"])

    def test_adapter_owns_no_renderer_or_mixer(self):
        manifest = tomllib.loads((ROOT / "crates/crysirror-rfvp/Cargo.toml").read_text())
        deps = set(manifest["dependencies"])
        for forbidden in ("wgpu", "winit", "rodio", "cpal", "kira", "crysirror-wgpu", "crysirror-audio"):
            self.assertNotIn(forbidden, deps)


if __name__ == "__main__":
    unittest.main()
