# SPDX-License-Identifier: MPL-2.0
import math
from pathlib import Path
import sys
import unittest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tools'))
import desktop_project as d
import engine_schema as e
import ue_lights as u

def ident(t=(0.,0.,0.)):
    return [[1.,0.,0.,t[0]],[0.,1.,0.,t[1]],[0.,0.,1.,t[2]],[0.,0.,0.,1.]]

class LightSchemaTests(unittest.TestCase):
    def test_entity_lights_validate_like_the_engine(self):
        base={'id':1,'name':'lamp'}
        e.entity(base|{'light':{'shape':{'kind':'spot','inner_degrees':10,'outer_degrees':30},'intensity':5,'units':'lumens'}})
        e.command({'op':'set_light','id':1,'light':None})
        e.entity(base|{'reflection_probe':{'extent':[4,3,4],'blend_distance':1}})
        e.command({'op':'set_reflection_probe','id':1,'probe':{'extent':[1,1,1],'revision':2}})
        for bad in [{'extent':[0,1,1]},{'extent':[1,1]},{'extent':[1,1,1],'intensity':-1},{'extent':[1,1,1],'typo':1}]:
            with self.assertRaises(Exception):e.entity(base|{'reflection_probe':bad})
        e.entity(base|{'light':{'shape':{'kind':'directional'},'falloff':{'kind':'exponent','exponent':2}}})
        e.entity(base|{'light':{'shape':{'kind':'ambient'},'range':30}})
        for bad in [{'shape':{'kind':'spot','inner_degrees':40,'outer_degrees':20}},{'intensity':-1},
                    {'shape':{'kind':'ambient'},'units':'lumens'},{'falloff':{'kind':'exponent','exponent':0}},
                    {'falloff':{'kind':'linear'}},
                    {'range':0},{'units':'nits'},{'shape':{'kind':'tube'}},{'radius':2},{'color':[1,1]}]:
            with self.assertRaises(d.ProjectError):e.entity(base|{'light':bad})

class UeLightConversionTests(unittest.TestCase):
    def test_units_axes_defaults_and_skips(self):
        # UE spot at (100, 200, 300) cm pointing down (+X axis → -Z UE).
        down=[[0.,0.,1.,100.],[0.,1.,0.,200.],[-1.,0.,0.,300.],[0.,0.,0.,1.]]
        level={'package':'L','lights':[
            {'class':'PointLightComponent','matrix':ident(),'visible':True},
            {'class':'PointLightComponent','matrix':ident(),'visible':True,'intensity':100,'units':'Lumens','radius':0,'color':[255,128,0,255]},
            {'class':'SpotLightComponent','matrix':down,'visible':True,'intensity':50,'units':'Candelas','outer_cone':30,'inner_cone':40,'cast_shadows':0},
            {'class':'DirectionalLightComponent','matrix':down,'visible':True,'intensity':3},
            {'class':'SkyLightComponent','matrix':ident(),'visible':True},
        ]}
        out=u.convert(level)
        a,b,s=out['lights']
        # Absent units/intensity/radius: Unitless 5000 → 8 cd, 10 m range.
        self.assertAlmostEqual(a['light']['intensity'],8.0)
        self.assertAlmostEqual(a['light']['range'],10.0)
        self.assertTrue(a['light']['shadows'])
        # Explicit zero radius is kept as the minimum, not replaced by the default.
        self.assertAlmostEqual(b['light']['intensity'],100/(4*math.pi),places=5)
        self.assertAlmostEqual(b['light']['range'],0.01)
        self.assertAlmostEqual(b['light']['color'][1],u.srgb_to_linear(128),places=5)
        # cm → m with Z-up → Y-up swap; inner cone clamped to outer.
        self.assertEqual(s['position'],[1.0,3.0,2.0])
        self.assertEqual(s['light']['shape'],{'kind':'spot','inner_degrees':30,'outer_degrees':30})
        self.assertFalse(s['light']['shadows'])
        self.assertEqual(out['sun']['direction'],[0.0,-1.0,0.0])
        self.assertEqual(out['skipped'][0]['class'],'SkyLightComponent')
        for l in out['lights']:e.light(l['light'])

    def test_rotation_maps_local_minus_z_to_ue_forward(self):
        for fwd in ([0.,-1.,0.],[1.,0.,0.],[0.3,-0.8,0.52]):
            ex,ey,ez=(math.radians(x) for x in u.euler_xyz(fwd,[0.,1.,0.]))
            # glam Quat::from_euler(XYZ) = Rx·Ry·Rz applied to (0,0,-1).
            v=[0.,0.,-1.]
            v=[v[0]*math.cos(ez)-v[1]*math.sin(ez),v[0]*math.sin(ez)+v[1]*math.cos(ez),v[2]]
            v=[v[0]*math.cos(ey)+v[2]*math.sin(ey),v[1],-v[0]*math.sin(ey)+v[2]*math.cos(ey)]
            v=[v[0],v[1]*math.cos(ex)-v[2]*math.sin(ex),v[1]*math.sin(ex)+v[2]*math.cos(ex)]
            n=u.normalize(fwd)
            for i in range(3):self.assertAlmostEqual(v[i],n[i],places=5)

class Gow3LightTests(unittest.TestCase):
    def record(self,type_byte,toward=(0.,1.,0.),color=(2.,1.,0.5),intensity=0.5,radius=12.):
        import struct
        b=bytearray(156);struct.pack_into('>I',b,0,6);struct.pack_into('>I',b,16,0x02010400|type_byte)
        struct.pack_into('>3f',b,20,1.,2.,3.);struct.pack_into('>3f',b,36,*toward)
        struct.pack_into('>3f',b,124,*color);struct.pack_into('>f',b,136,intensity);struct.pack_into('>f',b,140,radius)
        return bytes(b)
    def test_type_bits_units_and_volume_shapes(self):
        import gow3_lights as g
        p=g.to_entity(g.decode(self.record(0x29),'PSpointLite1'))
        self.assertEqual(p['light']['shape'],{'kind':'point'})
        self.assertEqual(p['transform']['translation'],[1.,2.,3.])
        # Peak-normalized color, peak folded into intensity (2.0 × 0.5).
        self.assertEqual(p['light']['color'],[1.,.5,.25]);self.assertAlmostEqual(p['light']['intensity'],1.)
        # 0x2e and 0x12 both carry type 2 in the low bits.
        for byte in (0x2e,0x12):
            d=g.to_entity(g.decode(self.record(byte,toward=(0.,1.,0.)),'PSdirLite'))
            self.assertEqual(d['light']['shape'],{'kind':'directional'})
            self.assertAlmostEqual(d['transform']['rotation_degrees'][0],-90.,places=4)
        a=g.to_entity(g.decode(self.record(0x28),'PSambLite1'))
        self.assertAlmostEqual(a['light']['intensity'],math.pi)
        for ent in (p,d,a):e.light(ent['light'])
    def test_unknown_layouts_fail_visibly(self):
        import gow3_lights as g
        bad=bytearray(self.record(0x29));bad[3]=7
        for payload in (bytes(bad),self.record(0x2b),self.record(0x29)[:150],self.record(0x29,radius=0.)):
            with self.assertRaises(Exception):g.decode(payload,'x')

if __name__=='__main__':
    unittest.main()
