# Copyright (c) 2026 CrySirror contributors.
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
import importlib.util
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("krkr_probe", Path(__file__).resolve().parents[1] / "tools/probe_krkr.py")
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


class KrkrProbe(unittest.TestCase):
    def test_native_failure_is_reported_and_temporary_project_is_removed(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            game = root / 'game'
            game.mkdir()
            (game / 'data.xp3').write_bytes(b'synthetic')
            (game / 'game.exe').write_bytes(b'synthetic')
            binary = root / 'engine'
            binary.write_bytes(b'synthetic')
            def run(cmd, **kw):
                project = json.loads(Path(cmd[2]).read_text())
                self.assertEqual(project['assets'], 'game')
                self.assertEqual(Path(cmd[2]).parent, game.parent.resolve())
                self.assertEqual(project['clients'][0]['config']['startup'], '__bootstrap/startup.tjs')
                kw['stdout'].write('CrySirror: unsupported native test\n')
                return types.SimpleNamespace(returncode=1)
            with patch.object(PROBE.subprocess, 'run', side_effect=run):
                result = PROBE.probe(game, 'game.exe', binary, root / 'proof')
            self.assertEqual(result['exit_code'], 1)
            self.assertEqual(result['native_sessions'], [])
            self.assertEqual(result['whole_game_compatibility'], 'not established')
            self.assertFalse(list(root.glob('.crysirror-krkr-*')))
            self.assertEqual((game / 'data.xp3').read_bytes(), b'synthetic')

    def test_companion_paths_cannot_escape_the_authorized_game(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / 'data.xp3').write_bytes(b'synthetic')
            with self.assertRaises(ValueError):
                PROBE.probe(root, '../elsewhere.exe', root / 'binary', root / 'output')
