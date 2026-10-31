# SPDX-License-Identifier: MPL-2.0
"""Shared authoring APIs are usable without a legacy VM or device."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import desktop_project as d
import engine_schema as e

class NativeSchemaTests(unittest.TestCase):
    def test_config_is_native_and_strict(self):
        for config in ({}, {'root':'rusur','font_alias':'game','encoding':'Shift_JIS','start_scene':0}, {'encoding':'cp1252','start_scene':-1}, {'optical_disc_check':'strict'}, {'optical_disc_check':'disabled_for_native_host'}):
            self.assertEqual(d.reallive_config(config), config)
        for config in ({'font':'x.ttf'}, {'root':'../game'}, {'root':'./game'}, {'font_alias':''}, {'encoding':'unknown'}, {'start_scene':True}, {'start_scene':2**31}, {'optical_disc_check':'authenticated'}, {'optical_disc_check':None}, {'optical_disc_check':True}):
            with self.subTest(config=config), self.assertRaises(d.ProjectError):d.reallive_config(config)
    def project(self):
        return {'schema':2,'name':'native','content_revision':'1','canvas':[640,480],'assets':'assets','fonts':{'default':'licensed.font'},'clients':[{'name':'main','adapter':'reallive','game':'test','config':{'root':'game'}}]}
    def test_native_inventory_bootstrap_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); game=root/'assets/game';game.mkdir(parents=True)
            (game/'Gameexe.ini').write_bytes(b'');(game/'Seen.TXT').write_bytes(b'')
            (game/'not-declared.nwa').write_bytes(b'');(root/'assets/licensed.font').write_bytes(b'test-only inventory marker')
            project=self.project();d.validate(project)
            self.assertEqual(set(d.asset_inventory(root/'project.json',project)),{'licensed.font','game/Gameexe.ini','game/Seen.TXT'})
            project['clients'][0]['program']=[{'op':'halt'}]
            with self.assertRaises(d.ProjectError):d.validate(project)
    def glyph(self):
        return {'kind':'glyph_run','font':'default','size':24.5,'glyphs':[{'character':'あ','baseline':[0,24]}],'reveal':{'interval_ms':40}}
    def test_positioned_glyphs_and_reveal(self):
        glyph=self.glyph();e.paint(glyph)
        glyph['glyphs'][0].update(character='\u200d',size=None,color=None,bold=None);e.paint(glyph)
        for patch in ({'character':'ab'},{'character':'\n'},{'character':'\ud800'},{'baseline':[0,float('inf')]},{'size':0},{'bold':1}):
            value=self.glyph();value['glyphs'][0].update(patch)
            with self.subTest(patch=patch),self.assertRaises(d.ProjectError):e.paint(value)
        for patch in ({'interval_ms':86400001},{'initial_visible':2},{'unit':'byte'},{'paused':1},{'elapsed_ms':2**64}):
            value=self.glyph();value['reveal'].update(patch)
            with self.subTest(patch=patch),self.assertRaises(d.ProjectError):e.paint(value)
    def test_glyph_fonts_are_inventory_dependencies(self):
        project=self.project();project['clients'][0]={'name':'main','adapter':'crysirror-script-v1','game':'test','systems':{'layers':[{'id':1,'rect':[0,0,640,480],'paint':self.glyph()}]}}
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'assets').mkdir();project['fonts']={}
            with self.assertRaisesRegex(d.ProjectError,'licensed project font'):d.asset_inventory(root/'project.json',project)
    def test_shared_canvas_commands(self):
        layer={'id':1,'rect':[0,0,640,480],'paint':self.glyph()}
        commands=[{'op':'create','id':1,'width':640,'height':480},{'op':'clear','id':1,'color':[0,0,0,255]},{'op':'draw','id':1,'layer':layer},{'op':'set','id':1,'layer':layer},{'op':'copy','source':1,'destination':2,'rect':[0,0,640,480],'position':[0,0],'opacity':255},{'op':'present','id':1,'layer':1},{'op':'hide','layer':1}]
        for command in commands:e.canvas_command(command)
        for command in ({'op':'create','id':1,'width':0,'height':1},{'op':'copy','source':1,'destination':2,'rect':[0,0,1,1],'position':[0,0],'opacity':128},{'op':'draw','id':1,'layer':dict(layer,parent=1)}):
            with self.assertRaises(d.ProjectError):e.canvas_command(command)
    def test_ordered_dither_is_a_named_engine_effect(self):
        value={'duration_ms':100,'effect':{'kind':'ordered_dither','cell':[4,4],'steps':16,'diagonal':True}}
        e.transition(value)
        for patch in ({'cell':[0,4]},{'steps':65537},{'diagonal':1}):
            invalid=copy.deepcopy(value);invalid['effect'].update(patch)
            with self.assertRaises(d.ProjectError):e.transition(invalid)
    def test_media_and_ui_options(self):
        e.media_options({});e.media_options({'loop':True,'audio':False,'gain':4})
        for options in ({'looping':True},{'gain':float('nan')},{'audio':1},{'gain':5}):
            with self.assertRaises(d.ProjectError):e.media_options(options)
        for command in ({'op':'set_title','text':'native'},{'op':'insert_text','text':'名前'},{'op':'set_cursor_visible','visible':True},{'op':'focus','id':1},{'op':'backspace','id':1}):e.ui_command(command)
        for command in ({'op':'insert_text','text':'bad\nline'},{'op':'set_title','text':'あ'*342},{'op':'backspace','id':True}):
            with self.assertRaises(d.ProjectError):e.ui_command(command)
    def test_text_field_unicode_scalar_limit(self):
        widget={'id':1,'rect':[0,0,100,20],'kind':'text_field','text':'名字','font':'default','max_length':2}
        e.validate({'widgets':[widget]})
        for patch in ({'text':'名字外'},{'text':'\n'},{'text':'\ud800'},{'max_length':65537}):
            with self.assertRaises(d.ProjectError):e.validate({'widgets':[dict(widget,**patch)]})

if __name__=='__main__':unittest.main()
