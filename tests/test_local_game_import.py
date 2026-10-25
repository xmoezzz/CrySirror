# SPDX-License-Identifier: MPL-2.0
"""Only synthetic resources. User game files are never part of this suite."""
import json,struct,sys,tempfile,unittest,zlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'tools'))
from psarc import Archive,ArchiveError
import gow3_wad
import ue_preview

def archive_bytes(payload):
    names=b'/test.bin\n';raw=[names,payload];blocks=[];encoded=[];entries=[];offset=0
    for data in raw:
        index=len(blocks);stored=[]
        for at in range(0,len(data),256):
            b=data[at:at+256];compressed=zlib.compress(b)
            if len(compressed)<len(b):blocks.append(len(compressed));stored.append(compressed)
            else:blocks.append(0 if len(b)==256 else len(b));stored.append(b)
        encoded.append(b''.join(stored));entries.append((index,len(data),offset));offset+=len(encoded[-1])
    toc=32+60+len(blocks)
    header=struct.pack('>4sI4s5I',b'PSAR',0x10004,b'zlib',toc,30,2,256,2)
    for index,length,offset in entries:header+=bytes(16)+index.to_bytes(4,'big')+length.to_bytes(5,'big')+(toc+offset).to_bytes(5,'big')
    return header+bytes(blocks)+b''.join(encoded)

class PsarcTests(unittest.TestCase):
    def test_compressed_raw_partial_and_empty_payloads(self):
        for data in (b'',b'a'*1400,bytes(range(256))*3+b'partial'):
            with self.subTest(size=len(data)),tempfile.TemporaryDirectory() as t:
                p=Path(t)/'fixture.psarc';p.write_bytes(archive_bytes(data));a=Archive(p)
                self.assertEqual(a.read(a.by_name['/test.bin']),data);self.assertEqual(a.prefix(1,3),data[:3])
    def test_invalid_header_and_ranges_rejected(self):
        for data in (b'bad',archive_bytes(b'a')[:40],archive_bytes(b'a')[:32]+bytes(60)):
            with self.subTest(size=len(data)),tempfile.TemporaryDirectory() as t:
                p=Path(t)/'bad.psarc';p.write_bytes(data)
                with self.assertRaises(ArchiveError):Archive(p)
    def test_decompression_integrity_and_limit(self):
        with tempfile.TemporaryDirectory() as t:
            p=Path(t)/'fixture.psarc';p.write_bytes(archive_bytes(b'a'*600));a=Archive(p)
            with self.assertRaises(ArchiveError):a.read(1,100)
            with self.assertRaises(ArchiveError):list(a.iter_entry(-1))
            b=bytearray(p.read_bytes());b[-1]^=1;p.write_bytes(b)
            with self.assertRaises((ArchiveError,zlib.error)):a.read(1)

def record(kind,name,payload=b''):
    b=struct.pack('>II',kind,len(payload))+name.encode().ljust(56,b'\0')+payload
    return b+bytes((-len(b))%16)
class WadTests(unittest.TestCase):
    def test_named_group_end_and_exact_framing(self):
        b=record(0x140000,'WAD_fixture',bytes(8))+record(0x20000,'GroupStart')+record(0x10077,'opaque',b'payload')+record(0x30000,'MAT_named_group')
        r=gow3_wad.inspect(b);self.assertEqual(r['consumed'],len(b));self.assertEqual(r['record_count'],4);self.assertFalse(r['mesh_decoder'])
    def test_unknown_layout_truncation_and_groups_fail(self):
        for data in (bytes(64),record(0x140000,'fixture')+record(0x30000,'end'),record(0x140000,'fixture')+record(0x20000,'start'),record(0x140000,'fixture',bytes(8))[:-2]):
            with self.subTest(length=len(data)),self.assertRaises(gow3_wad.WadError):gow3_wad.inspect(data)

    def test_entity_count_is_metadata_without_payload(self):
        metadata=struct.pack('>II',0,1053)+b'EntityCount'.ljust(56,b'\0')
        b=record(0x140000,'WAD_level')+metadata+record(0x10077,'next',b'payload')
        r=gow3_wad.inspect(b)
        self.assertEqual(r['records'][1]['metadata_value'],1053)
        self.assertEqual(r['records'][1]['size'],0)
        self.assertEqual(r['records'][2]['offset'],128)
        for name,count in [('UnknownMetadata',1),('EntityCount',1000001)]:
            b=record(0x140000,'WAD_level')+struct.pack('>II',0,count)+name.encode().ljust(56,b'\0')
            with self.assertRaises(gow3_wad.WadError):gow3_wad.inspect(b)

class PreviewTests(unittest.TestCase):
    def fixture(self,path):
        mesh={'source':'self-authored triangle','positions':[[0,0,0],[1,0,0],[0,1,0]],'normals':[[0,0,1]]*3,'uv':[[0,0],[1,0],[0,1]],'indices':[0,1,2],'sections':[{'start':0,'count':3,'material':0}]}
        scene={'report':{},'meshes':[{'source':mesh['source'],'file':'mesh.json'}],'nodes':[{'name':'triangle','mesh':0,'matrix':[1,0,0,0,0,1,0,0,0,0,1,0,4,5,6,1]}]}
        (path/'mesh.json').write_text(json.dumps(mesh));(path/'scene-source.json').write_text(json.dumps(scene));return mesh,scene
    def test_geometry_conversion_preserves_indices_and_placement(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);self.fixture(root);r=ue_preview.convert(root,root/'preview')
            self.assertEqual(r['rendered_triangles'],1);self.assertEqual(r['geometry_bounds'],[[4,5,6],[5,6,6]])
            g=json.loads((root/'preview/assets/scene.gltf').read_text());a=g['accessors'][g['meshes'][0]['primitives'][0]['indices']];v=g['bufferViews'][a['bufferView']];b=(root/'preview/assets/scene.bin').read_bytes()
            self.assertEqual(struct.unpack_from('<3I',b,v['byteOffset']),(0,1,2));self.assertFalse(r['rendered'])
    def test_tangents_and_non_ue_project_identity(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);mesh,_=self.fixture(root);mesh['tangents']=[[1,0,0,-1]]*3
            (root/'mesh.json').write_text(json.dumps(mesh))
            ue_preview.convert(root,root/'preview',project_name='native diagnostic',game_id='native-local',data_driven=True)
            project=json.loads((root/'preview/project.json').read_text())
            self.assertEqual(project['name'],'native diagnostic');self.assertEqual(project['clients'][0]['game'],'native-local')
            self.assertIn('environment',project['clients'][0]['systems']);self.assertFalse((root/'preview/preview.ts').exists())
            import desktop_project
            desktop_project.validate(project)
            g=json.loads((root/'preview/assets/scene.gltf').read_text());a=g['accessors'][g['meshes'][0]['primitives'][0]['attributes']['TANGENT']]
            v=g['bufferViews'][a['bufferView']];b=(root/'preview/assets/scene.bin').read_bytes()
            self.assertEqual(struct.unpack_from('<4f',b,v['byteOffset']),(1,0,0,-1))
            mesh['tangents'].pop();(root/'mesh.json').write_text(json.dumps(mesh))
            with self.assertRaisesRegex(ValueError,'tangent count'):ue_preview.convert(root,root/'bad')
    def test_material_override_and_bgra_png(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);mesh,scene=self.fixture(root)
            tex={'source':'synthetic pixel','file':'pixel.rgba','width':1,'height':1,'format':'PF_B8G8R8A8'};(root/'pixel.rgba').write_bytes(bytes([10,20,30,255]))
            scene['nodes'][0]['overrides']=[{'source':'pixel material','texture':tex,'blend_mode':'BLEND_Masked'}];(root/'scene-source.json').write_text(json.dumps(scene))
            r=ue_preview.convert(root,root/'preview');self.assertEqual(r['base_color_textures'],1)
            png=(root/'preview/assets/texture-0.png').read_bytes();length=struct.unpack_from('>I',png,33)[0];self.assertEqual(zlib.decompress(png[41:41+length]),bytes([0,30,20,10,255]))
    def test_explicit_engine_material_profile_keeps_transparent_instances(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);mesh,scene=self.fixture(root)
            scene['nodes'][0]['overrides']=[{'source':'self-authored soft surface','blend_mode':'BLEND_Translucent','scalars':{'opacity':.5}}]
            (root/'scene-source.json').write_text(json.dumps(scene))
            (root/'opacity.png').write_bytes(ue_preview.png_rgba(1,1,bytes([128,128,128,255])))
            (root/'waves.wgsl').write_text('fn crysirror_surface(i:SurfaceInput)->SurfaceOutput{return surface_default(i);}')
            profile={'schema':1,'materials':{'self-authored soft surface':{'material':{'unlit':True,'alpha_mode':'blend','depth_fade':2,'opacity_texture':'opacity.png','effect':{'kind':'custom','asset':'waves.wgsl'}},'note':'Self-authored test approximation'}}}
            path=root/'profile.json';path.write_text(json.dumps(profile))
            report=ue_preview.convert(root,root/'preview',material_profile=path)
            self.assertEqual(report['gltf_nodes'],1);self.assertEqual(report['engine_materials'],1);self.assertFalse(report['omitted'])
            gltf=json.loads((root/'preview/assets/scene.gltf').read_text());m=gltf['materials'][1]
            self.assertEqual(m['alphaMode'],'BLEND');self.assertEqual(m['extras']['crysirror_material']['material']['depth_fade'],2)
            self.assertEqual(m['extras']['crysirror_material']['material']['opacity_texture'],0)
            self.assertIn('KHR_materials_unlit',m['extensions'])
            self.assertEqual(report['profile_shaders'],1)
            self.assertEqual(m['extras']['crysirror_material']['material']['effect']['asset'],'surface-0.wgsl')
            self.assertEqual((root/'preview/assets/surface-0.wgsl').read_text(),(root/'waves.wgsl').read_text())
            profile['materials']['wrong source']=profile['materials'].pop('self-authored soft surface');path.write_text(json.dumps(profile))
            with self.assertRaisesRegex(ValueError,'unmatched'):ue_preview.convert(root,root/'bad-profile',material_profile=path)
    def test_mesh_path_escape_and_bad_indices_rejected(self):
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);mesh,scene=self.fixture(root);scene['meshes'][0]['file']='../outside.json';(root/'scene-source.json').write_text(json.dumps(scene))
            with self.assertRaisesRegex(ValueError,'path'):ue_preview.convert(root,root/'a')
            mesh,scene=self.fixture(root);mesh['indices']=[0,1,100];(root/'mesh.json').write_text(json.dumps(mesh))
            with self.assertRaisesRegex(ValueError,'topology'):ue_preview.convert(root,root/'b')
