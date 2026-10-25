# SPDX-License-Identifier: MPL-2.0
"""Native project boundary tests; execution/pixels are exercised by Rust tests."""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch
import subprocess

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'tools'))
import desktop_project as d
import package_game
from mcp_server import Server

class NativeProjects(unittest.TestCase):
    def setUp(self):
        self.project = {'schema':2,'name':'native fixture','content_revision':'1','canvas':[320,180], 'assets':'assets', 'clients':[{'name':'siglus','adapter':'siglus','game':'fixture','asset_prefix':'game','config':{'scene':'Scene.pck','string_codec':'plain','resources':{'native.g00':'pictures/native.g00'}}}]}
    def test_native_config_is_strict_and_does_not_accept_data_program(self):
        d.validate(self.project)
        for key,value in [('format','guess'),('string_codec','auto'),('unknown',True),('scene_name','')]:
            p=copy.deepcopy(self.project);p['clients'][0]['config'][key]=value
            with self.subTest(key=key),self.assertRaises(d.ProjectError):d.validate(p)
        self.project['clients'][0]['program']=[{'op':'halt'}]
        with self.assertRaisesRegex(d.ProjectError,'original bytecode'):d.validate(self.project)
    def test_native_config_paths_are_canonical_and_scoped(self):
        for path in ('../Scene.pck','./Scene.pck','a//Scene.pck','/Scene.pck','a\\b'):
            p=copy.deepcopy(self.project);p['clients'][0]['config']['scene']=path
            with self.subTest(path=path),self.assertRaises(d.ProjectError):d.validate(p)
        self.project['clients'][0]['config']['resources']={'../native.g00':'safe.g00'}
        with self.assertRaises(d.ProjectError):d.validate(self.project)
    def test_native_inventory_includes_declared_scene_and_alias_sources(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);base=root/'assets/game';(base/'pictures').mkdir(parents=True)
            (base/'Scene.pck').write_bytes(b'synthetic');(base/'pictures/native.g00').write_bytes(b'synthetic')
            path=root/'project.json';path.write_text(json.dumps(self.project))
            self.assertEqual(set(d.asset_inventory(path,d.load(path))),{'game/Scene.pck','game/pictures/native.g00'})
            result=json.loads(Server(path).call('project_validate',{})['content'][0]['text'])
            self.assertEqual(result['asset_scope'],'declared');self.assertFalse(result['native_vms_verified'])
            with self.assertRaisesRegex(d.ProjectError,'complete resource inventory'):
                package_game.package(path,root/'native.zip',data_only=True)
    def test_native_alias_symlink_escape_fails(self):
        with tempfile.TemporaryDirectory() as t,tempfile.TemporaryDirectory() as outside:
            root=Path(t);base=root/'assets/game';base.mkdir(parents=True);(base/'Scene.pck').write_bytes(b'synthetic')
            (Path(outside)/'native.g00').write_bytes(b'outside');(base/'pictures').symlink_to(outside,target_is_directory=True)
            with self.assertRaisesRegex(d.ProjectError,'escapes client root'):d.asset_inventory(root/'project.json',self.project)
    def test_mcp_inspects_configured_executable_registry(self):
        path=ROOT/'examples/desktop/project.json'
        server=Server(path,Path(sys.executable))
        output=json.dumps({'registered_adapters':['crysirror-script-v1','siglus']}).encode()
        with patch('mcp_server.subprocess.run',return_value=subprocess.CompletedProcess([],0,output,b'')) as run:
            result=json.loads(server.call('project_inspect',{})['content'][0]['text'])
            self.assertTrue(result['registry_verified']);self.assertEqual(result['registered_native_adapters'],['siglus'])
            self.assertEqual(run.call_args.args[0][1],'inspect')
    def test_native_vm_is_safe_cpu_and_pinned_original_source(self):
        crate=ROOT/'crates/crysirror-siglus-vm';manifest=tomllib.loads((crate/'Cargo.toml').read_text())
        self.assertFalse({'wgpu','winit','rodio','cpal','ash'} & manifest['dependencies'].keys())
        self.assertIn('#![forbid(unsafe_code)]',(crate/'src/lib.rs').read_text())
        origin=json.loads((crate/'upstream-files.json').read_text())
        lock=json.loads((ROOT/'upstream-lock.json').read_text())
        self.assertEqual(origin['commit'],lock['sources']['siglus_rs']['commit'])
        self.assertIn('crates/siglus_scene_vm/src/vm.rs',origin['files'])
        self.assertNotIn('crates/siglus_assets/src/keys.rs',origin['files'])
        self.assertIn('pub struct SceneVm',(crate/'src/vm.rs').read_text())
