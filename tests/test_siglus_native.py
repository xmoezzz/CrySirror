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
    def test_message_window_is_an_explicit_bounded_profile(self):
        window = {'font':'default','rect':[12,92,296,80], 'name_rect':[12,62,296,25],
                  'size':20,'name_size':20,'spacing':[-1,10],
                  'color':[255]*4,'name_color':[250,210,70,255]}
        self.project['clients'][0]['config']['message_window'] = window
        d.validate(self.project)
        for key,value in [('font',''),('size',0),('rect',[0,0,0,2]),('spacing',[0,-20]),('color',[256,0,0,0]),('unknown',True)]:
            p=copy.deepcopy(self.project); p['clients'][0]['config']['message_window'][key]=value
            with self.subTest(key=key),self.assertRaises(d.ProjectError):d.validate(p)
        self.project['clients'][0]['config']['gameexe']='Gameexe.ini'
        d.validate(self.project)  # Explicit profile overrides native layout; Gameexe still supplies boot metadata.
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
    def test_private_decode_material_is_scoped_and_not_packaged(self):
        self.project['clients'][0]['config']['key_file'] = 'local-key.toml'
        d.validate(self.project)
        self.project['clients'][0]['config']['key_file'] = '../key.toml'
        with self.assertRaises(d.ProjectError): d.validate(self.project)
        self.project['clients'][0]['config']['key_file'] = 'local-key.toml'
        with tempfile.TemporaryDirectory() as t:
            root = Path(t); (root/'assets/game').mkdir(parents=True)
            (root/'assets/game/Scene.pck').write_bytes(b'authored scene')
            (root/'assets/game/pictures').mkdir()
            (root/'assets/game/pictures/native.g00').write_bytes(b'authored image')
            self.assertNotIn('game/local-key.toml', d.asset_inventory(root/'project.json', self.project))
            (root/'project.json').write_text(json.dumps(self.project))
            with self.assertRaisesRegex(d.ProjectError, 'complete resource inventory'):
                package_game.package(root/'project.json', root/'private.zip', data_only=True)

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
        current=json.loads((crate/'SOURCE.json').read_text())
        self.assertEqual(current['commit'],origin['commit'])
        self.assertEqual(current['snapshot_schema'],11)
        expected={str(p.relative_to(crate)) for p in crate.rglob('*.rs')}|{'Cargo.toml'}
        self.assertEqual(set(current['workspace_sha256']),expected)
        for rel,digest in current['workspace_sha256'].items():
            self.assertEqual(hashlib.sha256((crate/rel).read_bytes()).hexdigest(),digest,rel)

    def test_shared_capabilities_do_not_link_a_second_siglus_engine(self):
        # The original interpreter remains first-class; generic helpers must be
        # usable by unrelated clients without depending on that interpreter.
        self.assertFalse((ROOT/'crates/siglus-upstream').exists())
        for name in ('crysirror-runtime', 'crysirror-image', 'crysirror-audio',
                     'crysirror-video', 'crysirror-puppet-eluna'):
            manifest = (ROOT/'crates'/name/'Cargo.toml').read_text()
            self.assertNotIn('siglus_scene_vm', manifest, name)
            self.assertNotIn('siglus_assets', manifest, name)
            self.assertNotIn('siglus-upstream', manifest, name)
        provenance = json.loads((ROOT/'docs/shared-native-capability-sources.json').read_text())
        self.assertIn('crysirror-image/src/wipe_mask.rs', provenance['siglus']['engine_sources'])
        self.assertIn('crysirror-audio/src/nwa.rs', provenance['siglus']['engine_sources'])
        self.assertIn('crysirror-video/src/omv.rs', provenance['siglus']['engine_sources'])
        for name in ('mpeg', 'wmv', 'omv', 'theora', 'emote'):
            codec = ROOT/'crates'/('crysirror-codec-'+name)
            source = json.loads((codec/'SOURCE.json').read_text())
            self.assertFalse((codec/'src/bin').exists())
            for rel, digest in source['baseline_sha256'].items():
                patch = source.get('patches', {}).get(rel)
                if patch:
                    self.assertEqual(patch['origin_sha256'], digest, rel)
                    self.assertTrue(patch['reason'], rel)
                self.assertEqual(hashlib.sha256((codec/rel).read_bytes()).hexdigest(), patch['current_sha256'] if patch else digest, rel)
