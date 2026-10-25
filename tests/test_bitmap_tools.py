# SPDX-License-Identifier: MPL-2.0
"""Reference-source checks without compiling or reading game/font data."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("compare_bitmap_upstream", ROOT / "tools/compare_bitmap_upstream.py")
bitmap = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bitmap)


class BitmapReferenceChecks(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="bitmap-tool-test-")
        self.addCleanup(self.temp.cleanup)
        self.source = Path(self.temp.name) / "upstream"
        self.source.mkdir()
        self.git("init", "--quiet")
        self.files = ["Cargo.toml", "Cargo.lock", "crates/reallive/Cargo.toml",
                      "crates/reallive/src/surface.rs", "crates/avg32/Cargo.toml",
                      "crates/avg32/src/buffer.rs", "crates/avg32/src/pdtmgr.rs"]
        for relative in self.files:
            path = self.source / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("synthetic\n")
        self.git("add", ".")
        self.git("-c", "user.name=Bitmap Test", "-c", "user.email=bitmap@example.invalid",
                 "commit", "--quiet", "-m", "synthetic reference")
        self.pin = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.source), *args], text=True)

    def test_clean_complete_source_and_untracked_files(self):
        (self.source / "unused.txt").write_text("not read by validation")
        bitmap.validate_source(self.source, self.pin)

    def test_wrong_pin(self):
        with self.assertRaisesRegex(ValueError, "HEAD differs"):
            bitmap.validate_source(self.source, "0" * 40)

    def test_dirty_source_and_staged_changes(self):
        path = self.source / "crates/reallive/src/surface.rs"
        path.write_text("changed\n")
        with self.assertRaisesRegex(ValueError, "tracked upstream source changed"):
            bitmap.validate_source(self.source, self.pin)
        self.git("add", str(path))
        with self.assertRaisesRegex(ValueError, "tracked upstream source changed"):
            bitmap.validate_source(self.source, self.pin)

    def test_incomplete_checkout(self):
        self.git("rm", "--quiet", "crates/avg32/src/pdtmgr.rs")
        self.git("-c", "user.name=Bitmap Test", "-c", "user.email=bitmap@example.invalid",
                 "commit", "--quiet", "-m", "synthetic incomplete source")
        pin = self.git("rev-parse", "HEAD").strip()
        with self.assertRaisesRegex(ValueError, "complete siglus_rs checkout required"):
            bitmap.validate_source(self.source, pin)

    def test_output_symlinks_cannot_redirect_generated_files_into_source(self):
        cache = Path(self.temp.name) / "cache"
        cache.mkdir()
        driver = cache / "bitmap-differential"
        driver.symlink_to(self.source, target_is_directory=True)
        with patch.object(bitmap, "validate_source"):
            with self.assertRaisesRegex(ValueError, "outside both source trees"):
                bitmap.run(self.source, cache)
        driver.unlink()
        driver.mkdir()
        (driver / "src").symlink_to(self.source, target_is_directory=True)
        with patch.object(bitmap, "validate_source"):
            with self.assertRaisesRegex(ValueError, "outside both source trees"):
                bitmap.run(self.source, cache)

    def test_build_outputs_must_stay_outside_sources(self):
        for path in [ROOT / "target/unwanted", self.source / "target"]:
            with self.assertRaisesRegex(ValueError, "outside both source trees"):
                bitmap.external(path, self.source)
        work = Path(self.temp.name) / "build"
        self.assertEqual(bitmap.external(work, self.source), work.resolve())


if __name__ == "__main__":
    unittest.main()
