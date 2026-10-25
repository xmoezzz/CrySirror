# SPDX-License-Identifier: MPL-2.0
import copy
import json
from pathlib import Path
import sys
import tomllib
import unittest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tools'))
import desktop_project as d
import package_game

class EngineSystemsTests(unittest.TestCase):
    def setUp(self):
        self.path=ROOT/'examples/desktop/systems.json'
        self.project=d.load(self.path)
        self.systems=self.project['clients'][0]['systems']
    def test_initial_environment_validation_and_scoped_asset_inventory(self):
        import tempfile
        import engine_schema as e
        good={'hdr':'sky.hdr','sun':{'direction':[0,-1,0],'intensity':5},'tonemap':'aces'}
        e.validate({'environment':good})
        e.validate({'environment':{'reflections':{}}})
        e.validate({'environment':{'reflections':{'intensity':0.5,'steps':64,'max_roughness':0.4}}})
        for bad in [{'reflections':{'steps':2}},{'reflections':{'thickness':0}},{'reflections':{'typo':1}},{'sun':{'direction':[0,0,0]}},{'intensity':-1},{'exposure':float('nan')},{'hdr':'../private.hdr'},{'typo':1},{'temporal_aa':1}]:
            with self.assertRaises(d.ProjectError):e.validate({'environment':bad})
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);(root/'assets/a').mkdir(parents=True);(root/'assets/a/sky.hdr').write_bytes(b'synthetic inventory placeholder')
            p={'schema':2,'name':'environment','content_revision':'1','assets':'assets','canvas':[64,64],'clients':[{'name':'a','adapter':'crysirror-script-v1','game':'a','asset_prefix':'a','systems':{'environment':good}}]}
            self.assertEqual(set(d.asset_inventory(root/'project.json',p)),{'a/sky.hdr'})
            (root/'outside.hdr').write_bytes(b'synthetic');(root/'assets/a/escape.hdr').symlink_to(root/'outside.hdr');good['hdr']='escape.hdr'
            with self.assertRaisesRegex(d.ProjectError,'escapes'):d.asset_inventory(root/'project.json',p)
    def test_engine_material_validation_matches_runtime_contract(self):
        import engine_schema as e
        good={'alpha_mode':'blend','base_color_factor':[.1,.2,.3,.5],'opacity_texture':'fx/opacity.png','depth_fade':4,'unlit':True,'uv_scale':[2,1]}
        e.material(good)
        for key,value in [('alpha_mode','invented'),('depth_fade',-1),('uv_scale',[float('nan'),1]),('base_color_factor',[1,1,1,2]),('opacity_texture','../secret.png'),('opacity_texture','fx//mask.png'),('unlit',1),('shader_code','unsafe')]:
            with self.subTest(key=key),self.assertRaises(d.ProjectError):e.material(dict(good,**{key:value}))
        e.command({'op':'set_material','id':1,'material':good});e.command({'op':'set_material','id':1,'material':None})
    def test_material_assets_use_client_roots_and_reject_symlinks(self):
        import tempfile
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);(root/'assets/a').mkdir(parents=True);(root/'assets/b').mkdir()
            for client in ('a','b'):(root/f'assets/{client}/opacity.png').write_bytes(client.encode())
            p={'schema':2,'name':'materials','content_revision':'1','assets':'assets','canvas':[64,64],'clients':[{'name':c,'adapter':'crysirror-script-v1','game':c,'asset_prefix':c,'systems':{'entities':[{'id':1,'name':'quad','material':{'opacity_texture':'opacity.png'}}]}} for c in ('a','b')]}
            (root/'assets/a/action.png').write_bytes(b'action')
            p['clients'][0]['systems']['widgets']=[{'id':1,'rect':[0,0,10,10],'on_activate':[{'op':'set_material','id':1,'material':{'normal_texture':'action.png'}}]}]
            d.validate(p);inventory=d.asset_inventory(root/'project.json',p)
            self.assertEqual(set(inventory),{'a/opacity.png','b/opacity.png','a/action.png'})
            (root/'assets/a/opacity.png').unlink();(root/'assets/a/opacity.png').symlink_to(root/'assets/b/opacity.png')
            with self.assertRaisesRegex(d.ProjectError,'escapes client root'):d.asset_inventory(root/'project.json',p)
    def test_shader_effect_schema_inventory_and_mcp_guard(self):
        import engine_schema as e
        from mcp_server import Server
        from unittest.mock import patch
        p=ROOT/'examples/shader-effects/project.json';project=d.load(p)
        self.assertEqual(set(d.asset_inventory(p,project)),{'quad.glb','waves.wgsl'})
        for effect in [{'kind':'unknown'},{'kind':'fog','noise_scale':0},{'kind':'dissolve','threshold':2},{'kind':'custom','asset':'../secret.wgsl'},{'kind':'custom','asset':'waves.wgsl','source':'inline'},{'kind':'custom','asset':'waves.wgsl','parameters':[[1]*4]}]:
            with self.subTest(effect=effect),self.assertRaises(d.ProjectError):e.material({'alpha_mode':'blend','effect':effect})
        server=Server(p,Path(sys.executable))
        for args in [{'client':'game','asset':'../secret.wgsl'},{'client':'other','asset':'waves.wgsl'},{'client':'game','asset':'waves.js'}]:
            with self.subTest(args=args),patch('mcp_server.subprocess.run')as run,self.assertRaises(ValueError):server.call('shader_validate',args)
            run.assert_not_called()
    def test_layer_assets_are_packaged(self):
        inventory=d.asset_inventory(self.path,self.project)
        self.assertEqual(set(inventory),{'a/panel.png'})
    def test_ui_fonts_require_explicit_licensed_asset(self):
        p=self.path.with_name('systems-text.json')
        with self.assertRaisesRegex(d.ProjectError,'licensed'):
            d.asset_inventory(p,d.load(p))
    def test_bad_cycle_and_dangling_parent_are_rejected(self):
        for parent in [1,999]:
            with self.subTest(parent=parent):
                p=copy.deepcopy(self.project);p['clients'][0]['systems']['entities'][0]['parent']=parent
                with self.assertRaises(d.ProjectError):d.validate(p)
    def test_unknown_fields_not_ignored(self):
        for section in ['entities','layers','widgets']:
            with self.subTest(section=section):
                p=copy.deepcopy(self.project);p['clients'][0]['systems'][section][0]['typo']=True
                with self.assertRaisesRegex(d.ProjectError,'unknown'):d.validate(p)
    def test_world_commands_and_materials_are_checked(self):
        for bad in [{'op':'set_velocity','id':1,'velocity':[0,1e30,0]},{'op':'native_opcode','id':1},{'op':'tween','id':1,'tween':{'property':'x','from':0,'to':1,'duration_ms':0}}]:
            with self.subTest(bad=bad):
                self.systems['widgets'][1]['on_activate']=[bad]
                with self.assertRaises(d.ProjectError):d.validate(self.project)
    def test_port_sources_match_actual_lock(self):
        imports=json.loads((ROOT/'crates/crysirror-scene/upstream-files.json').read_text());lock=json.loads((ROOT/'upstream-lock.json').read_text())
        for source,entry in imports.items():self.assertEqual(entry['commit'],lock['sources'][source]['commit'])
    def test_scene_is_safe_and_has_no_device_dependencies(self):
        manifest=tomllib.loads((ROOT/'crates/crysirror-scene/Cargo.toml').read_text())
        self.assertFalse({'wgpu','winit','kira','rodio','cpal'}&manifest['dependencies'].keys())
        self.assertIn('#![forbid(unsafe_code)]',(ROOT/'crates/crysirror-scene/src/lib.rs').read_text())
    def test_widget_actions_reject_missing_local_entities(self):
        self.systems['widgets'][1]['on_activate']=[{'op':'set_visible','id':999,'visible':False}]
        with self.assertRaisesRegex(d.ProjectError,'target'):d.validate(self.project)
    def test_mcp_input_arguments_are_checked_before_execution(self):
        from mcp_server import Server,frame_arguments
        from unittest.mock import patch
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            server=Server(self.path,Path(sys.executable),output_dir=Path(tmp))
            for tool,args in [('engine_trace',{'press_at':[2],'frames':2}),('engine_trace',{'frames':True}),('scene_capture',{'renderer':'hardware-raytrace'}),('scene_capture',{'press_at':[[0]]})]:
                with self.subTest(tool=tool,args=args),patch('mcp_server.subprocess.run') as run:
                    with self.assertRaises(ValueError):server.call(tool,args)
                    run.assert_not_called()
        self.assertEqual(frame_arguments({'frames':4,'step_ms':16,'press_at':[2]}),['--frames','4','--step-ms','16','--press-at','2'])


class SequenceSchemaTests(unittest.TestCase):
    """Mirror of SequenceSpec::validate (ADR 0040)."""
    def seq(self, **over):
        q = {"duration_ms": 1000, "tracks": [{"entity": 1, "property": "light_intensity",
             "keys": [{"time_ms": 0, "value": [5]}], "noise": {"amplitude": 1, "frequency_hz": 6}}],
             "events": [{"time_ms": 500, "name": "clank"}]}
        q.update(over)
        return {"op": "play_sequence", "name": "s", "sequence": q}
    def test_valid_and_invalid_sequences(self):
        import engine_schema as e
        e.command(self.seq())
        e.command({"op": "stop_sequence", "name": "s"})
        for bad in [self.seq(duration_ms=0), self.seq(speed=20),
                    self.seq(tracks=[{"entity": 1, "property": "rotation", "keys": [{"time_ms": 0, "value": [1]}]}]),
                    self.seq(tracks=[{"entity": 1, "property": "scale", "keys": [{"time_ms": 0, "value": [0, 1, 1]}]}]),
                    self.seq(events=[{"time_ms": 2000, "name": "late"}]),
                    self.seq(camera={"keys": [{"time_ms": 0, "eye": [0, 0, 0], "target": [0, 0, 0]}]})]:
            with self.assertRaises(Exception):
                e.command(bad)
