# SPDX-License-Identifier: MPL-2.0
from __future__ import annotations
import copy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]

def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "tools" / f"{name}.py")
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result

validator = module("validate_project")
upstreams = module("upstreams")

class ProjectTests(unittest.TestCase):
    def setUp(self):
        self.project = json.loads((ROOT / "examples/fusion.json").read_text())
    def test_valid_fixture(self):
        validator.validate(self.project)
    def test_schema_boolean_rejected(self):
        self.project["schema"] = True
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_wrong_schema(self):
        self.project["schema"] = 2
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_other_console_rejected(self):
        self.project["console_targets"] = ["switch", "another-console"]
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_duplicate_console_rejected(self):
        self.project["console_targets"] = ["switch", "switch"]
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_duplicate_client(self):
        self.project["clients"][1]["name"] = self.project["clients"][0]["name"]
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_unknown_engine(self):
        self.project["clients"][0]["engine"] = "pretend"
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_unknown_op(self):
        self.project["clients"][0]["commands"][0]["op"] = "draw.3d"
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_extra_property(self):
        self.project["shell"] = "do-not-execute"
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_path_traversal(self):
        for path in ("../escape", "..\\escape", "/etc/passwd", "C:\\data", "a/../../b", "a\0b", "."):
            with self.subTest(path=path), self.assertRaises(validator.ValidationError): validator.asset_path(path)
    def test_windows_paths_at_adapter_boundary(self):
        validator.asset_path("image\\A.png")
    def test_zero_extent(self):
        self.project["clients"][0]["commands"][0]["width"] = 0
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_invalid_color(self):
        self.project["clients"][0]["commands"][0]["rgba"][0] = 256
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_invalid_gain(self):
        self.project["clients"][0]["commands"][2]["gain_milli"] = 1001
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_invalid_boolean_as_id(self):
        self.project["clients"][0]["commands"][0]["id"] = True
        with self.assertRaises(validator.ValidationError): validator.validate(self.project)
    def test_schema_file_matches_positive_fixture(self):
        try:
            import jsonschema
        except ImportError:
            self.skipTest("optional JSON Schema cross-check requires jsonschema")
        schema = json.loads((ROOT / "schemas/project-v1.schema.json").read_text())
        jsonschema.Draft202012Validator.check_schema(schema)
        jsonschema.Draft202012Validator(schema).validate(self.project)

class UpstreamTests(unittest.TestCase):
    def setUp(self):
        self.config = json.loads((ROOT / "upstreams.json").read_text())
    def test_config(self):
        upstreams.validate_config(self.config)
    def test_changed_origin_rejected(self):
        self.config["sources"]["rfvp"]["url"] = "file:///tmp/untrusted"
        with self.assertRaises(upstreams.AuditError): upstreams.validate_config(self.config)
    def test_argument_injection_ref_rejected(self):
        self.config["sources"]["rfvp"]["ref"] = "--upload-pack=evil"
        with self.assertRaises(upstreams.AuditError): upstreams.validate_config(self.config)
    def test_path_escape_rejected(self):
        self.config["sources"]["rfvp"]["candidate_seams"].append("../outside")
        with self.assertRaises(upstreams.AuditError): upstreams.validate_config(self.config)
    def test_missing_source_errors(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(upstreams.AuditError):
                upstreams.audit_repository(Path(d)/"missing", self.config["sources"]["rfvp"])
    def fixture(self, root):
        (root/"Cargo.toml").write_text('[workspace]\nmembers=["crates/*"]\n[workspace.dependencies]\nwgpu="0.20"\nshared={path="crates/shared"}\n')
        for path, name, deps in [
            ("vm", "actual-vm-name", '[dependencies]\nshared={workspace=true}\nwgpu={workspace=true,optional=true}\n'),
            ("shared", "shared", ''),
            ("unused-console", "unused-console", '')]:
            folder=root/"crates"/path; folder.mkdir(parents=True)
            (folder/"Cargo.toml").write_text(f'[package]\nname="{name}"\nversion="0.1.0"\n'+deps)
        return {"focus_manifests":["crates/vm/Cargo.toml"],"candidate_seams":["crates/vm","absent"]}
    def test_actual_package_name_is_discovered_not_guessed(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); config=self.fixture(root)
            report=upstreams.audit_repository(root,config)
            self.assertEqual(set(report["packages"]), {"actual-vm-name","shared"})
            self.assertEqual(report["unselected_package_count"], 1)
            self.assertFalse(report["native_vm_connected"])
    def test_workspace_optional_dependency_is_preserved(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); config=self.fixture(root)
            report=upstreams.audit_repository(root,config)
            deps=report["packages"]["actual-vm-name"]["host_dependencies"]
            self.assertTrue(deps[0]["optional"])
    def test_missing_focus_manifest_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); config=self.fixture(root)
            config["focus_manifests"]=["crates/missing/Cargo.toml"]
            with self.assertRaises(upstreams.AuditError): upstreams.audit_repository(root,config)
    def test_dependency_outside_repo_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); config=self.fixture(root)
            path=root/"crates/shared/Cargo.toml"
            path.write_text(path.read_text()+'[dependencies]\nexternal={path="../../../outside"}\n')
            with self.assertRaises(upstreams.AuditError): upstreams.audit_repository(root,config)
    def test_untracked_checkout_not_overwritten(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); (root/"upstream/rfvp").mkdir(parents=True)
            with mock.patch.object(upstreams,"ROOT",root):
                with self.assertRaises(upstreams.AuditError): upstreams.fetch(self.config)
    def test_fetch_uses_explicit_source_root(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d)/"project"; root.mkdir()
            external=Path(d)/"references"
            (external/"rfvp").mkdir(parents=True)
            with mock.patch.object(upstreams,"ROOT",root):
                with self.assertRaises(upstreams.AuditError):
                    upstreams.fetch(self.config, external)
            self.assertFalse((root/"upstream").exists())
    def test_partial_fetch_keeps_previous_source_pins(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d); (root/"upstream/rfvp").mkdir(parents=True)
            (root/"upstream/siglus_rs").mkdir()
            sources={name:{"url":url,"commit":letter*40,"requested_ref":"main"}
                     for (name,url),letter in zip(upstreams.URLS.items(),("a","b"))}
            (root/"upstream-lock.json").write_text(json.dumps({"schema":1,"sources":sources}))
            def fake_run(args,*,cwd=None):
                if cwd.name=="siglus_rs": raise upstreams.AuditError("simulated interruption")
                if args[1:]==["rev-parse","HEAD"]: return "a"*40
                if args[1:]==["remote","get-url","origin"]: return upstreams.URLS["rfvp"]
                if args[1:]==["status","--porcelain"]: return ""
                raise AssertionError(args)
            with mock.patch.object(upstreams,"ROOT",root),mock.patch.object(upstreams,"run",fake_run):
                with self.assertRaises(upstreams.AuditError): upstreams.fetch(self.config)
            after=json.loads((root/"upstream-lock.json").read_text())
            self.assertEqual(after["sources"]["siglus_rs"]["commit"],"b"*40)

class BoundaryTests(unittest.TestCase):
    def test_siglus_is_a_separate_safe_cpu_frontend(self):
        import tomllib
        manifest=tomllib.loads((ROOT/"crates/crysirror-siglus/Cargo.toml").read_text())
        self.assertFalse({"wgpu","winit","kira","rodio","cpal"} & manifest["dependencies"].keys())
        self.assertIn("#![forbid(unsafe_code)]",(ROOT/"crates/crysirror-siglus/src/lib.rs").read_text())
        imported=json.loads((ROOT/"crates/crysirror-siglus/upstream-files.json").read_text())
        lock=json.loads((ROOT/"upstream-lock.json").read_text())
        self.assertEqual(imported["commit"],lock["sources"]["siglus_rs"]["commit"])
    def test_core_has_no_dependencies(self):
        import tomllib
        manifest=tomllib.loads((ROOT/"crates/crysirror-core/Cargo.toml").read_text())
        self.assertNotIn("dependencies",manifest)
    def test_core_is_no_std_and_unsafe_forbidden(self):
        text=(ROOT/"crates/crysirror-core/src/lib.rs").read_text()
        self.assertIn("#![no_std]",text)
        self.assertIn("#![forbid(unsafe_code)]",text)
    def test_compat_only_depends_on_core(self):
        import tomllib
        manifest=tomllib.loads((ROOT/"crates/crysirror-compat/Cargo.toml").read_text())
        self.assertEqual(set(manifest["dependencies"]),{"crysirror-core"})
    def test_no_upstream_glob_in_workspace(self):
        import tomllib
        manifest=tomllib.loads((ROOT/"Cargo.toml").read_text())
        self.assertTrue(all(m.startswith("crates/") and "*" not in m for m in manifest["workspace"]["members"]))
    def test_console_scope(self):
        self.assertEqual({p.name for p in (ROOT/"platform").iterdir() if p.is_dir()}, {"switch","psv"})
    def test_no_game_assets_or_fonts_distributed(self):
        forbidden={".ttf",".otf",".woff",".woff2",".hcb",".ss",".g00",".nro",".vpk",".psb"}
        # local/ and images/third-party/ hold owner-supplied data; they are
        # gitignored and never packaged (local/README.md).
        local={ROOT/"local", ROOT/"images"/"third-party"}
        ignore=(ROOT/".gitignore").read_text()
        self.assertIn("/local/",ignore)
        self.assertIn("/images/third-party/",ignore)
        # Prune private/generated trees before traversing: rustc creates and
        # removes temporary directories concurrently during workspace builds.
        excluded=local | {ROOT/"target", ROOT/".git"}
        found=[]
        for parent, dirs, files in os.walk(ROOT):
            parent=Path(parent)
            dirs[:]=[name for name in dirs if parent/name not in excluded]
            found.extend(parent/name for name in files if Path(name).suffix.lower() in forbidden)
        self.assertFalse(found)

if __name__=="__main__":
    unittest.main()
