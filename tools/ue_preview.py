#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Convert the pinned UE bridge's conventional-LOD geometry into a local glTF preview.

This is a geometry/material diagnostic. Explicit material profiles use shared
engine data; UE shader graphs, landscape, skeletal objects and gameplay are not recreated.
"""
from __future__ import annotations
import argparse,json,math,struct,zlib,binascii,shutil
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]
def require(v,msg):
    if not v:raise ValueError(msg)

def png_rgba(width,height,rgba):
    require(type(width)is int and type(height)is int and 0<width<=8192 and 0<height<=8192 and len(rgba)==width*height*4, 'invalid RGBA surface')
    def chunk(kind,data):return struct.pack('>I',len(data))+kind+data+struct.pack('>I',binascii.crc32(kind+data)&0xffffffff)
    raw=b''.join(b'\0'+rgba[y*width*4:(y+1)*width*4]for y in range(height))
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>2I5B',width,height,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')

def convert(source:Path,out:Path,eye=None,target=None,omit_meshes=(),material_profile=None,project_name=None,game_id=None,data_driven=False):
    source=source.resolve();out=out.resolve()
    require(not out.is_relative_to(ROOT),'local game assets must remain outside source export')
    out.mkdir(parents=True,exist_ok=True);assets=out/'assets';assets.mkdir(exist_ok=True)
    scene=json.loads((source/'scene-source.json').read_text())
    profiles={};profile_root=None;used_profiles=set()
    if material_profile is not None:
        from engine_schema import material as validate_material
        profile_root=Path(material_profile).resolve().parent
        profile=json.loads(Path(material_profile).read_text())
        require(set(profile)=={'schema','materials'} and profile['schema']==1 and isinstance(profile['materials'],dict),'unsupported material profile schema')
        profiles=profile['materials']
        for name,entry in profiles.items():
            require(isinstance(name,str) and isinstance(entry,dict) and set(entry)=={'material','note'} and isinstance(entry['note'],str) and bool(entry['note']),'material profile needs explicit provenance/approximation note')
            validate_material(entry['material'])
    buffer=bytearray();views=[];accessors=[];meshes=[];mesh_bounds=[]
    images=[];textures=[];texture_ids={};materials=[{'name':'neutral diagnostic fallback','pbrMetallicRoughness':{'baseColorFactor':[.38,.4,.43,1],'metallicFactor':0,'roughnessFactor':.9}}]
    material_ids={};material_report=[];source_sections=[];shader_ids={}
    def material(spec):
        name=spec['source']
        if name in profiles:
            used_profiles.add(name)
            if name in material_ids:return material_ids[name]
            entry=profiles[name];native=dict(entry['material'])
            effect=native.get('effect') or {}
            if effect.get('kind')=='custom':
                path=(profile_root/effect['asset']).resolve()
                require(path.is_relative_to(profile_root) and path.is_file(),'profile shader escapes capability root or is missing')
                require(0<path.stat().st_size<=65536,'profile shader size limit')
                key=str(path)
                if key not in shader_ids:
                    uri=f'surface-{len(shader_ids)}.wgsl';shutil.copyfile(path,assets/uri);shader_ids[key]=uri
                native['effect']=dict(effect,asset=shader_ids[key])
            for field,value in list(native.items()):
                if field.endswith('_texture') and value is not None:
                    path=(profile_root/value).resolve()
                    require(path.is_relative_to(profile_root) and path.is_file(),'profile texture escapes capability root or is missing')
                    require(path.suffix.lower()=='.png','profile textures must be PNG')
                    key='profile:'+str(path)
                    if key not in texture_ids:
                        index=len(images);uri=f'texture-{index}.png';shutil.copyfile(path,assets/uri)
                        images.append({'uri':uri});textures.append({'source':index});texture_ids[key]=index
                    native[field]=texture_ids[key]
            mode=native.get('alpha_mode','opaque');pbr={'baseColorFactor':native.get('base_color_factor',[1,1,1,1]),'metallicFactor':native.get('metallic_factor',0),'roughnessFactor':native.get('roughness_factor',.6)}
            if native.get('base_color_texture') is not None:pbr['baseColorTexture']={'index':native['base_color_texture']}
            m={'name':name,'pbrMetallicRoughness':pbr,'doubleSided':native.get('double_sided',False),'alphaMode':'BLEND' if mode in ('blend','additive','premultiplied') else mode.upper(),'extras':{'crysirror_material':{'schema':1,'material':native}}}
            if native.get('unlit'):m['extensions']={'KHR_materials_unlit':{}}
            index=len(materials);materials.append(m);material_ids[name]=index
            material_report.append({'source':name,'profile':'explicit engine material','note':entry['note'],'source_scalars':spec.get('scalars',{}),'source_vectors':spec.get('vectors',{})})
            return index
        if not spec.get('texture'):return 0
        if name in material_ids:return material_ids[name]
        tex=spec['texture'];key=tex['source']
        if key not in texture_ids:
            file=Path(tex['file']);require(file.name==str(file),'non-canonical texture path')
            rgba=bytearray((source/file).read_bytes())
            require(tex['format'] in ('PF_R8G8B8A8','PF_B8G8R8A8'),'unsupported decoded pixel format')
            if tex['format']=='PF_B8G8R8A8':
                for i in range(0,len(rgba),4):rgba[i],rgba[i+2]=rgba[i+2],rgba[i]
            index=len(textures);uri=f'texture-{index}.png';(assets/uri).write_bytes(png_rgba(tex['width'],tex['height'],rgba))
            images.append({'uri':uri});textures.append({'source':index});texture_ids[key]=index
        index=len(materials);material_ids[name]=index
        materials.append({'name':name,'doubleSided':True,'alphaMode':'MASK','alphaCutoff':.5,'pbrMetallicRoughness':{'baseColorFactor':[1,1,1,1],'baseColorTexture':{'index':texture_ids[key]},'metallicFactor':0,'roughnessFactor':1}})
        material_report.append({'source':name,'texture':key,'selection':spec.get('selection'),'profile':'base color only; source alpha treated as mask; two-sided preview'})
        return index
    def accessor(values,kind,components,fmt,code,bounds=False):
        require(bool(values),'empty buffer')
        flat=[x for row in values for x in row] if components>1 else values
        require(all(math.isfinite(x) for x in flat),'non-finite geometry')
        while len(buffer)%4:buffer.append(0)
        offset=len(buffer);buffer.extend(struct.pack('<'+fmt*len(flat),*flat))
        view=len(views);views.append({'buffer':0,'byteOffset':offset,'byteLength':len(buffer)-offset})
        a={'bufferView':view,'componentType':code,'count':len(values),'type':kind}
        if bounds:
            a['min']=[min(v[i] for v in values)for i in range(components)];a['max']=[max(v[i] for v in values)for i in range(components)]
        accessors.append(a);return len(accessors)-1
    for info in scene['meshes']:
        rel=Path(info['file']);require(rel.name==str(rel),'non-canonical bridge mesh path')
        m=json.loads((source/rel).read_text());pos=m['positions'];norm=m['normals'];uv=m['uv'];ix=m['indices']
        require(len(pos)==len(norm)==len(uv) and len(ix)%3==0 and all(type(i)is int and 0<=i<len(pos) for i in ix),'mesh topology mismatch')
        p=accessor(pos,'VEC3',3,'f',5126,True);n=accessor(norm,'VEC3',3,'f',5126);u=accessor(uv,'VEC2',2,'f',5126)
        attributes={'POSITION':p,'NORMAL':n,'TEXCOORD_0':u}
        if m.get('tangents'):
            require(len(m['tangents'])==len(pos),'tangent count');attributes['TANGENT']=accessor(m['tangents'],'VEC4',4,'f',5126)
        primitives=[]
        for section in m['sections']:
            begin,count=section['start'],section['count'];require(0<=begin<=begin+count<=len(ix) and count%3==0,'section range')
            if count:primitives.append({'attributes':attributes,'indices':accessor(ix[begin:begin+count],'SCALAR',1,'I',5125),'material':material(m.get('materials',[])[section['material']]) if 0<=section['material']<len(m.get('materials',[])) else 0})
        require(bool(primitives),'no mesh primitives')
        source_sections.append([section['material']for section in m['sections']if section['count']]);meshes.append({'name':info['source'],'primitives':primitives});mesh_bounds.append((accessors[p]['min'],accessors[p]['max']))
    variants={};nodes=[];lo=[float('inf')]*3;hi=[-float('inf')]*3;omitted=[]
    for node in scene['nodes']:
        if scene['meshes'][node['mesh']]['source'] in omit_meshes:
            omitted.append({'name':node['name'],'mesh':scene['meshes'][node['mesh']]['source'],'reason':'explicitly excluded unsupported mesh in preview profile'});continue
        mat=node['matrix'];require(len(mat)==16 and all(math.isfinite(x)for x in mat),'node matrix')
        bmin,bmax=mesh_bounds[node['mesh']]
        corners=[]
        for bits in range(8):
            v=[bmax[i]if bits>>i&1 else bmin[i]for i in range(3)]
            corners.append([sum(v[k]*mat[k*4+i]for k in range(3))+mat[12+i]for i in range(3)])
        nlo=[min(v[i]for v in corners)for i in range(3)];nhi=[max(v[i]for v in corners)for i in range(3)]
        # The original sky sphere is geometry in UE. This preview uses the engine
        # sky; huge sky meshes would hide all recovered architecture.
        if max(nhi[i]-nlo[i]for i in range(3))>5000:
            omitted.append({'name':node['name'],'reason':'sky-sized geometry >5 km; engine sky used'});continue
        for i in range(3):lo[i]=min(lo[i],nlo[i]);hi[i]=max(hi[i],nhi[i])
        mesh_index=node['mesh'];overrides=node.get('overrides',[])
        if overrides:
            assignments=[]
            for slot,p in zip(source_sections[mesh_index],meshes[mesh_index]['primitives']):
                spec=overrides[slot]if 0<=slot<len(overrides)else None
                if spec and spec.get('source')!='missing material':
                    if spec.get('source') not in profiles and spec.get('blend_mode') in ('BLEND_Translucent','BLEND_Additive','BLEND_Modulate','BLEND_AlphaComposite','BLEND_AlphaHoldout'):
                        assignments.append(None)
                    else:assignments.append(material(spec))
                else:assignments.append(p['material'])
            key=(mesh_index,tuple(assignments))
            if all(a is None for a in assignments):
                omitted.append({'name':node['name'],'reason':'unsupported source transparent/additive shader; omitted from geometry preview'});continue
            if key not in variants:
                variant=[]
                for primitive,assignment in zip(meshes[mesh_index]['primitives'],assignments):
                    if assignment is not None:variant.append(dict(primitive,material=assignment))
                variants[key]=len(meshes);meshes.append({'name':meshes[mesh_index]['name']+' component material override','primitives':variant})
            mesh_index=variants[key]
        nodes.append({'name':node['name'],'mesh':mesh_index,'matrix':mat})
    require(used_profiles==set(profiles),'material profile contains unmatched source paths')
    require(bool(nodes),'no scene nodes')
    gltf={'extensionsUsed':['KHR_materials_unlit'] if any(m.get('extensions') for m in materials) else [],'asset':{'version':'2.0','generator':'CrySirror local native geometry preview'},'buffers':[{'uri':'scene.bin','byteLength':len(buffer)}],'bufferViews':views,'accessors':accessors,'meshes':meshes,'nodes':nodes,'scenes':[{'nodes':list(range(len(nodes)))}],'scene':0,'images':images,'textures':textures,'materials':materials}
    (assets/'scene.bin').write_bytes(buffer);(assets/'scene.gltf').write_text(json.dumps(gltf,separators=(',',':')))
    project={'schema':2,'name':project_name or 'Ashen Vow: original castle geometry diagnostic','content_revision':'local-geometry-1','canvas':[1280,720],'assets':'assets','clients':[{'name':'game','adapter':'crysirror-script-v1','game':game_id or 'ashen-local-preview'}]}
    center=target or [(lo[i]+hi[i])/2 for i in range(3)];span=max(hi[i]-lo[i]for i in range(3))
    eye=eye or [center[0]+span*.55,center[1]+span*.45,center[2]+span*.55]
    camera={'eye':eye,'target':center,'up':[0,1,0],'fov_y_degrees':55,'near':.1,'far':max(2000,span*4)}
    environment={'intensity':.7,'exposure':0,'tonemap':'aces','bloom':.01,'ambient_occlusion':1,'temporal_aa':True,'shadow_distance':150,'sun':{'direction':[.3,-1,.5],'color':[1,.94,.84],'intensity':5,'shadows':True}}
    if data_driven:
        project['scene3d']={'camera':camera,'meshes':[]}
        project['clients'][0]['systems']={'environment':environment,'entities':[{'id':1,'name':'native rest-space scene','geometry':{'kind':'scene','asset':'scene.gltf'}}]}
        (out/'preview.ts').unlink(missing_ok=True)
    else:
        script='function onStart(){\n world.setEnvironment('+json.dumps(environment)+');\n const result=world.importModel("scene.gltf",{pbr:true,asScene:true});\n engine.log(JSON.stringify(result.report));\n world.setCamera('+json.dumps({'eye':eye,'target':center,'fov':55,'near':.1,'far':camera['far']})+');\n}\n'
        (out/'preview.ts').write_text(script)
    (out/'project.json').write_text(json.dumps(project,indent=2))
    report={'original':scene['report'],'gltf_nodes':len(nodes),'gltf_meshes':len(meshes),'buffer_bytes':len(buffer),'rendered_triangles':sum(accessors[p['indices']]['count']//3 for node in nodes for p in meshes[node['mesh']]['primitives']),'geometry_bounds':[lo,hi],'omitted':omitted,'base_color_textures':len(textures)-sum(key.startswith('profile:')for key in texture_ids),'profile_textures':sum(key.startswith('profile:')for key in texture_ids),'engine_materials':len(used_profiles),'profile_shaders':len(shader_ids),'material_report':material_report,'original_materials':False,'original_lighting':False,'rendered':False}
    (out/'conversion-report.json').write_text(json.dumps(report,indent=2));return report

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('source',type=Path);p.add_argument('output',type=Path);p.add_argument('--eye',type=float,nargs=3);p.add_argument('--target',type=float,nargs=3);p.add_argument('--omit-mesh',action='append',default=[]);p.add_argument('--material-profile',type=Path);a=p.parse_args();require((a.eye is None)==(a.target is None),'provide eye and target together');r=convert(a.source,a.output,a.eye,a.target,a.omit_mesh,a.material_profile);print(json.dumps({k:v for k,v in r.items()if k!='original'},indent=2))
if __name__=='__main__':main()
