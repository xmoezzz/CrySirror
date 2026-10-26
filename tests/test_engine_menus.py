# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
"""Shared menu authoring is independent of KiriKiri and native plugin files."""
import copy
from pathlib import Path
import sys
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tools'))
import desktop_project as d
import engine_schema as e

class Menus(unittest.TestCase):
    def menu(self):
        return {'id':1,'rect':[0,0,640,24],'font':'default','items':[{'id':2,'label':'File','children':[{'id':3,'label':'Fast','radio':True,'checked':True},{'id':4,'label':'Slow','radio':True}]}]}
    def test_shared_commands(self):
        for command in [{'op':'set_menu','menu':self.menu()},{'op':'open_menu','id':1,'item':2},{'op':'open_menu','id':1,'position':[10,20]},{'op':'set_menu_checked','id':1,'item':3,'checked':False},{'op':'close_menu','id':1},{'op':'remove_menu','id':1}]:
            e.ui_command(command)
    def test_bad_geometry_duplicate_ids_radio_and_boolean_rejected(self):
        original=self.menu()
        mutations=[lambda m:m.update(rect=[0,0,0,24]),lambda m:m.update(popup_width=float('inf')),lambda m:m.update(context=1),lambda m:m['items'][0]['children'][1].update(id=3),lambda m:m['items'][0]['children'][1].update(checked=True)]
        for mutation in mutations:
            menu=copy.deepcopy(original);mutation(menu)
            with self.assertRaises(d.ProjectError):e.menu_spec(menu)
        with self.assertRaises(d.ProjectError):e.ui_command({'op':'open_menu','id':1,'position':[float('nan'),0]})
