# SPDX-License-Identifier: MPL-2.0
"""Source/output capability guards only; these tests never compile Rust."""
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("compare_bgi_cpu", ROOT / "tools/compare_bgi_cpu.py")
bgi = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bgi)


class BgiCpuToolGuards(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="bgi-cpu-guard-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.source = self.base / "source"
        self.source.mkdir()
        self.git("init", "--quiet")
        for name in bgi.FILES:
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("synthetic reference\n")
        self.git("add", ".")
        self.git("-c", "user.name=BGI Guard Test", "-c", "user.email=bgi@example.invalid",
                 "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "synthetic source")
        self.pin = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.source), *args], text=True)

    def test_clean_source_and_hashes_use_verified_head_bytes(self):
        with patch.object(bgi, "PIN", self.pin):
            source, contents, metadata = bgi.source_snapshot(self.source)
        self.assertEqual(source, self.source)
        self.assertEqual(metadata["source_head"], self.pin)
        self.assertEqual(set(contents), set(bgi.FILES))
        self.assertTrue(all(len(value) == 64 for value in metadata["reference_sha256"].values()))

    def test_wrong_pin_and_dirty_or_staged_reference_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "HEAD differs"):
            bgi.source_snapshot(self.source)
        path = self.source / bgi.FILES[0]
        path.write_text("changed\n")
        with patch.object(bgi, "PIN", self.pin):
            with self.assertRaisesRegex(ValueError, "reference source changed"):
                bgi.source_snapshot(self.source)
            self.git("add", bgi.FILES[0])
            with self.assertRaisesRegex(ValueError, "reference source changed"):
                bgi.source_snapshot(self.source)

    def test_source_symlink_cannot_escape_even_if_git_assumes_unchanged(self):
        name = bgi.FILES[0]
        self.git("update-index", "--assume-unchanged", name)
        outside = self.base / "outside.rs"
        outside.write_text("synthetic reference\n")
        path = self.source / name
        path.unlink()
        path.symlink_to(outside)
        with patch.object(bgi, "PIN", self.pin):
            with self.assertRaisesRegex(ValueError, "source escapes checkout"):
                bgi.source_snapshot(self.source)

    def test_all_generated_source_and_lock_symlinks_are_rejected_without_cargo(self):
        cache = self.base / "cache"
        driver = cache / "bgi-cpu-differential"
        (driver / "src").mkdir(parents=True)
        for name in ["src/main.rs", "src/original_core.rs", "src/original_bitmap.rs",
                     "src/original_snapshot.rs", "Cargo.toml", "Cargo.lock"]:
            with self.subTest(name=name):
                path = driver / name
                path.symlink_to(self.source / bgi.FILES[0])
                with patch.object(bgi.subprocess, "run", side_effect=AssertionError("Cargo forbidden")):
                    with self.assertRaisesRegex(ValueError, "outside both source trees"):
                        bgi.output_paths(self.source, cache, None)
                path.unlink()
        escaped = self.base / "unrelated.rs"
        escaped.write_text("keep")
        (driver / "src/original_core.rs").symlink_to(escaped)
        with self.assertRaisesRegex(ValueError, "escapes the external work directory"):
            bgi.output_paths(self.source, cache, None)
        self.assertEqual(escaped.read_text(), "keep")

    def test_unrelated_cargo_manifest_is_preserved(self):
        cache = self.base / "cache"
        driver = cache / "bgi-cpu-differential"
        driver.mkdir(parents=True)
        manifest = driver / "Cargo.toml"
        original = '[package]\nname="unrelated-project"\n'
        manifest.write_text(original)
        with self.assertRaisesRegex(ValueError, "unrelated Cargo project"):
            bgi.output_paths(self.source, cache, None)
        self.assertEqual(manifest.read_text(), original)
        manifest.write_text('[package]\nname = "crysirror-bgi-cpu-differential"\n')
        self.assertEqual(bgi.output_paths(self.source, cache, None)[0], driver)

    def test_help_without_font_or_cargo_and_missing_font_argument(self):
        env = {**os.environ, "PATH": ""}
        script = str(ROOT / "tools/compare_bgi_cpu.py")
        help_result = subprocess.run([sys.executable, script, "--help"], env=env,
                                     text=True, capture_output=True)
        self.assertEqual(help_result.returncode, 0, help_result.stderr)
        self.assertIn("--font", help_result.stdout)
        missing = subprocess.run([sys.executable, script, "--source-root", str(self.source),
                                  "--work-dir", str(self.base / "unused")], env=env,
                                 text=True, capture_output=True)
        self.assertEqual(missing.returncode, 2)
        self.assertIn("--font", missing.stderr)
        self.assertFalse((self.base / "unused").exists())


if __name__ == "__main__":
    unittest.main()
