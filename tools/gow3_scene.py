#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Bounded GoW III PS3 WAD geometry frontend. Own implementation, no game assets.

This observed layout profile uses declared group/config offsets, not feature scans.
EDGE count/range/stream checks are mandatory. Unknown layouts fail explicitly.
"""
from __future__ import annotations
import json,struct,math,argparse
from pathlib import Path
from collections import Counter
from gow3_wad import inspect,WadError
ROOT=Path(__file__).resolve().parents[1]
def check(v,msg):
    if not v:raise WadError(msg)
class Reader:
    def __init__(self,data):self.data=data
    def raw(self,at,n):
        check(type(at)is int and type(n)is int and 0<=at<=at+n<=len(self.data),f'range 0x{at:x}+{n}')
        return self.data[at:at+n]
    def get(self,fmt,at):return struct.unpack(fmt,self.raw(at,struct.calcsize(fmt)))
    def u16(self,at):return self.get('>H',at)[0]
    def u32(self,at):return self.get('>I',at)[0]
    def rel(self,at):return at+self.get('>i',at)[0]
class Bits:
    def __init__(self,data):self.data=data;self.at=0
    def get(self,width):
        check(0<=width<=16 and self.at+width<=len(self.data)*8,'truncated bit stream')
        v=0
        for _ in range(width):v=(v<<1)|((self.data[self.at//8]>>(7-self.at%8))&1);self.at+=1
        return v

def edge_indices(data,index_count,vertex_count):
    check(0<index_count<=65535 and index_count%3==0 and 0<vertex_count<=65535,'EDGE count range')
    r=Reader(data);reused,bias,mask_bytes,width,version=r.get('>HHHBB',0)
    check(version==0 and width<=16 and (width>0 or reused==0),'unsupported EDGE version/width')
    code_bytes=(index_count//3+3)//4;packed_bytes=(reused*width+7)//8
    used=8+mask_bytes+code_bytes+packed_bytes
    check(used<=len(data)<=((used+15)&~15),'EDGE stream length/padding')
    flags=Bits(r.raw(8,mask_bytes));codes=Bits(r.raw(8+mask_bytes,code_bytes));packed=Bits(r.raw(8+mask_bytes+code_bytes,packed_bytes))
    values=[]
    for i in range(reused):
        v=packed.get(width)-bias+(values[i-8]if i>=8 else 0)
        check(0<=v<vertex_count,'EDGE delta index out of range');values.append(v)
    cursor=0;sequential=0
    def vertex():
        nonlocal cursor,sequential
        if flags.get(1):
            check(cursor<len(values),'EDGE reuse flag count');v=values[cursor];cursor+=1
        else:v=sequential;sequential+=1
        check(v<vertex_count,'EDGE sequential index out of range');return v
    previous=None;out=[]
    for _ in range(index_count//3):
        code=codes.get(2)
        if code==3:t=[vertex(),vertex(),vertex()]
        else:
            check(previous is not None,'EDGE reuse before first triangle')
            a,b=((0,2),(2,1),(1,0))[code];t=[previous[a],previous[b],vertex()]
        previous=t;out.extend([t[0],t[2],t[1]]) # native clockwise -> glTF CCW
    check(cursor==reused and sequential==vertex_count,'EDGE vertex/reuse totals mismatch')
    check((flags.at+7)//8==mask_bytes,'EDGE flag stream length mismatch')
    return out

def mesh(data,name,decoder=edge_indices):
    r=Reader(data);check(r.u32(0)==0x1000d and r.u32(4)==len(data),'unknown MG resource profile')
    top=0x10+r.u32(0x18);groups=r.u16(top);check(0<groups<=4096,'MG surface group limit');table=r.rel(top+12)
    parts=[]
    for group in range(groups):
        base=r.rel(table+group*4);check(r.u32(base)==23 and r.u32(base+4)==1,'MG surface version/layout')
        material_slot=r.u16(base+8);check(r.u16(base+10)==1,'MG surface material slot profile')
        geo=base+r.u32(base+28);configs=r.u16(geo+2);check(0<configs<=4096,'MG config limit');start=r.rel(geo+28)
        positions=[];indices=[];attributes=[];normals=[];tangents=[];nonunit_normals=0
        declarations=[];bindings=r.rel(geo+20)
        for j in range(r.u16(geo)):
            at=bindings+j*12;declarations.append((r.rel(at),r.get('>4B',at+4),r.u32(at+8)))
        for number in range(configs):
            c=start+number*96;vn,ic=r.get('>HH',c+8);vb=r.rel(c+32);vs=r.u16(c+40);desc=r.rel(c+68)
            count,stride,duplicate,reserved=r.get('>4B',desc)
            check(count==duplicate and reserved==0 and 0<count<=16 and 12<=stride<=256,'vertex stream declaration')
            fields=[r.get('>8B',desc+8+i*8)for i in range(count)]
            layouts={16:[(0,2,3,1,12,0,0,0),(12,6,1,2,4,2,0,0)],24:[(0,2,3,1,12,0,0,0),(12,1,4,3,8,14,0,0),(20,6,1,4,4,15,0,0)]}
            check(fields==layouts.get(stride),'unsupported vertex attribute layout')
            check(vs==((vn*stride+15)&~15),'vertex buffer size/stride mismatch');r.raw(vb,vs)
            p=[list(r.get('>3f',vb+i*stride))for i in range(vn)]
            normal_fields=[f for f in fields if f[1]==6 and f[2]==1 and f[4]==4]
            check(len(normal_fields)==1,'unsupported normal declaration')
            for i in range(vn):
                w=r.u32(vb+i*stride+normal_fields[0][0])
                def snorm(v,n):return max(-1.,(v-(1<<n)if v&(1<<(n-1))else v)/((1<<(n-1))-1))
                n=[snorm(w&2047,11),snorm(w>>11&2047,11),snorm(w>>22&1023,10)]
                if not .98<sum(x*x for x in n)<1.02:nonunit_normals+=1
                length=math.sqrt(sum(x*x for x in n));normals.append([x/length for x in n]if length>1e-12 else[0.,1.,0.])
                tf=[f for f in fields if f[1]==1 and f[2]==4 and f[3]==3 and f[4]==8]
                if tf:
                    t=r.get('>4h',vb+i*stride+tf[0][0]);v=[max(-1.,x/32767)for x in t[:3]];length=math.sqrt(sum(x*x for x in v));check(length>1e-9,'zero tangent');tangents.append([x/length for x in v]+[1. if t[3]>=0 else -1.])
                else:tangents.append(None)
            check(all(math.isfinite(v)and abs(v)<1e7 for vtx in p for v in vtx),'non-finite/excessive position')
            ix=decoder(r.raw(r.rel(c+20),r.u16(c+24)),ic,vn);offset=len(positions);positions.extend(p);indices.extend(offset+i for i in ix);attributes.append(fields)
        uv_bindings=[row for row in declarations if row[1][0:2]==(15,241) and row[1][3]==2 and row[2]>0]
        check(len(uv_bindings)<=1,'ambiguous UV0 binding')
        uv=[[0.,0.]]*len(positions)
        uv_supported=bool(uv_bindings)
        uv_note=None
        if uv_bindings:
            at,param,size=uv_bindings[0];step=param[2]*4
            check(8<=step<=256 and len(positions)*step<=size,f'UV0 buffer size/stride ({len(positions)}, {step}, {size})')
            if size>((len(positions)*step+15)&~15):uv_supported=False;uv_note='UV0 source remap/cardinality unresolved; material omitted'

            uv=[list(r.get('>2f',at+i*step))for i in range(len(positions))]
            check(all(math.isfinite(x)and abs(x)<100000 for v in uv for x in v),'UV0 finite range')
        parts.append({'positions':positions,'normals':normals,'tangents':tangents,'uv':uv,'indices':indices,'group':group,'material_slot':material_slot,'configs':configs,'vertex_attributes':attributes,'nonunit_normals':nonunit_normals,'uv_supported':uv_supported,'uv_note':uv_note})
    return parts

def resources(data):
    info=inspect(data);records=info['records'];groups=[{'parent':None,'records':[],'children':[]}];stack=[0]
    for i,r in enumerate(records):
        kind=int(r['kind'],16)
        if kind==0x20000:
            group=len(groups);groups.append({'parent':stack[-1],'records':[],'children':[]});groups[stack[-1]]['children'].append(group);stack.append(group)
        elif kind==0x30000:stack.pop()
        else:groups[stack[-1]]['records'].append(i)
        r['group']=stack[-1]
    return info,groups

def extract(wad,out):
    out=out.resolve();check(not out.is_relative_to(ROOT),'game data must remain outside source export');out.mkdir(parents=True,exist_ok=True)
    data=wad.read_bytes();info,groups=resources(data);meshes={};errors=[]
    for record in info['records']:
        if record['kind']=='0x00010052' and record['name'].startswith('MG_'):
            try:
                start=record['offset']+64;meshes[record['name']]=mesh(data[start:start+record['size']],record['name'])
            except (WadError,struct.error)as e:errors.append({'mesh':record['name'],'offset':record['offset'],'error':str(e)})
    report={'wad_bytes':len(data),'records':len(info['records']),'mesh_groups':len(meshes),'surface_groups':sum(len(m)for m in meshes.values()),'vertices':sum(len(p['positions'])for m in meshes.values()for p in m),'triangles':sum(len(p['indices'])//3 for m in meshes.values()for p in m),'errors':errors,'rendered':False}
    (out/'mesh-source.json').write_text(json.dumps({'report':report,'meshes':meshes,'groups':groups,'records':info['records']},separators=(',',':')))
    (out/'extraction-report.json').write_text(json.dumps(report,indent=2));return report

def links(data,info,groups):
    records=info['records'];models={};nodes=[];missing=[]
    for r in records:
        if r['kind']=='0x00010052'and r['name'].startswith('MDL_'):
            refs=[records[i]['name']for i in groups[r['group']]['records']if records[i]['kind']=='0x00010000'and records[i]['size']==0]
            models[r['name']]={'meshes':[n for n in refs if n.startswith('MG_')],'materials':[n for n in refs if n.startswith('MAT_')]}
    def refs(group,outer):
        g=groups[group]
        if not outer and any(records[i]['kind']=='0x0001001f'and records[i]['name']for i in g['records']):return []
        result=[records[i]['name']for i in g['records']if records[i]['kind']=='0x00010000'and records[i]['name'].startswith('MDL_')and records[i]['size']==0]
        for child in g['children']:result.extend(refs(child,False))
        return result
    for r in records:
        if r['kind']!='0x0001001f'or not r['name']:continue
        if r['size']!=148:
            at=r['offset']+64;check(r['name']=='ContextNames'and Reader(data[at:at+r['size']]).u32(0)==0x2a0001,'unknown named scene metadata layout');continue
        p=r['offset']+64;d=Reader(data[p:p+r['size']]);check(d.u32(0)==0x30001,'unknown scene instance version');a=d.get('>12f',64)
        matrix=[*a[:3],0.,*a[3:6],0.,*a[6:9],0.,*a[9:12],1.]
        check(all(math.isfinite(x)for x in matrix),'non-finite instance transform')
        for model in refs(r['group'],True):
            if model not in models:missing.append({'entity':r['name'],'model':model});continue
            for mg in models[model]['meshes']:nodes.append({'name':r['name'],'model':model,'mesh':mg,'matrix':matrix})
    return models,nodes,missing

def dds_preview(payload,max_size=256):
    check(type(max_size)is int and 0<max_size<=8192,'DDS preview size')
    r=Reader(payload);check(r.u32(0)==0x10007 and r.raw(76,4)==b'DDS ','native DDS wrapper')
    header=bytearray(r.raw(76,128));get=lambda at:struct.unpack_from('<I',header,at)[0]
    check(get(4)==124 and get(76)==32,'DDS header version');width,height=get(16),get(12);total=max(1,get(28));inline=r.u32(64)
    check(width>0 and height>0 and max(width,height)<=8192 and 0<total<=14,'DDS dimensions/mips')
    check(get(112)&(0x200|0x200000)==0,'cubemap/volume texture needs explicit material import')
    inline=total if inline==0xffffffff else inline;check(0<inline<=total,'native inline mip count')
    four=bytes(header[84:88]);depth=get(88)
    check(four in (b'DXT1',b'DXT3',b'DXT5',bytes(4))and(four!=bytes(4)or depth in(8,32)),'unsupported native DDS format')
    def level(w,h):return max(1,(w+3)//4)*max(1,(h+3)//4)*(8 if four==b'DXT1'else 16)if four!=bytes(4)else w*h*(depth//8)
    for _ in range(total-inline):width=max(1,width//2);height=max(1,height//2)
    pixels=payload[204:];expected=sum(level(max(1,width>>i),max(1,height>>i))for i in range(inline))
    check(len(pixels)==expected,'native DDS inline data length')
    while max(width,height)>max_size and inline>1:pixels=pixels[level(width,height):];width=max(1,width//2);height=max(1,height//2);inline-=1
    struct.pack_into('<I',header,8,0x1007|(0x20000 if inline>1 else 0)|(0x80000 if four!=bytes(4)else 8))
    for at,value in [(12,height),(16,width),(20,level(width,height)if four!=bytes(4)else width*(depth//8)),(28,inline)]:struct.pack_into('<I',header,at,value)
    return bytes(header)+pixels

def dds_full(payload,ts,max_size=1024):
    """Complete mip chain: the WAD keeps the small inline mips; the matching TS
    stream (byte offset u32 at +60) holds the larger ones, largest first; the
    streamed sizes of all OLY100 textures sum exactly to the TS length."""
    r=Reader(payload);check(r.u32(0)==0x10007 and r.raw(76,4)==b'DDS ','native DDS wrapper')
    header=bytearray(r.raw(76,128));get=lambda at:struct.unpack_from('<I',header,at)[0]
    width,height=get(16),get(12);total=max(1,get(28));inline=r.u32(64)
    if ts is None or inline==0xffffffff or inline>=total:return dds_preview(payload,max_size)
    four=bytes(header[84:88]);depth=get(88)
    check(four in (b'DXT1',b'DXT3',b'DXT5',bytes(4))and(four!=bytes(4)or depth in(8,32)),'unsupported native DDS format')
    check(get(112)&(0x200|0x200000)==0,'cubemap/volume texture needs explicit material import')
    def level(w,h):return max(1,(w+3)//4)*max(1,(h+3)//4)*(8 if four==b'DXT1'else 16)if four!=bytes(4)else w*h*(depth//8)
    sizes=[level(max(1,width>>i),max(1,height>>i))for i in range(total)]
    streamed=sum(sizes[:total-inline]);at=r.u32(60)
    check(0<=at and at+streamed<=len(ts),'TS stream range')
    inline_pixels=payload[204:];check(len(inline_pixels)==sum(sizes[total-inline:]),'native DDS inline data length')
    pixels=bytes(ts[at:at+streamed])+inline_pixels;levels=total
    while max(width,height)>max_size and levels>1:pixels=pixels[level(width,height):];width=max(1,width//2);height=max(1,height//2);levels-=1
    struct.pack_into('<I',header,8,0x1007|(0x20000 if levels>1 else 0)|(0x80000 if four!=bytes(4)else 8))
    for k,value in [(12,height),(16,width),(20,level(width,height)if four!=bytes(4)else width*(depth//8)),(28,levels)]:struct.pack_into('<I',header,k,value)
    return bytes(header)+pixels

# Sampler slots whose role was identified from content: 0 base color
# (colored DXT1), 5 DXT5nm normal (R=255, B=0, X in alpha, Y in green),
# 9 grayscale specular. Other slots stay unmapped and are reported.
ROLE_SLOTS={'base':0,'normal':5,'specular':9}

def preview(wad,out,codec,eye=None,target=None,ts=None,base_size=256,detail_size=256):
    import subprocess,hashlib
    from ue_preview import convert
    out=out.resolve();check(not out.is_relative_to(ROOT),'game assets must remain external');out.mkdir(parents=True,exist_ok=True)
    data=wad.read_bytes();info,groups=resources(data);rdir=out/'source';rdir.mkdir(exist_ok=True);jobs=[];stream=bytearray();reference=[]
    tsdata=ts.read_bytes()if ts else None
    def collect(raw,ic,vn):
        jobs.append({'file':'indices.bin','offset':len(stream),'length':len(raw),'encoding':'edge','index_count':ic,'vertex_count':vn,'reverse_winding':True});stream.extend(raw);values=edge_indices(raw,ic,vn);reference.append(values);return values
    selected=[r for r in info['records']if r['kind']=='0x00010052'and r['name'].startswith('MG_')]
    for r in selected:
        at=r['offset']+64;mesh(data[at:at+r['size']],r['name'],collect)
    (rdir/'indices.bin').write_bytes(stream);(rdir/'index-jobs.json').write_text(json.dumps({'schema':1,'indices':jobs}))
    subprocess.run([str(codec.resolve()),str(rdir/'index-jobs.json'),str(rdir/'decoded-indices.json')],check=True)
    decoded=json.loads((rdir/'decoded-indices.json').read_text())['indices'];check(decoded==reference,'engine/Python index differential mismatch');iterator=iter(decoded)
    def shared(raw,ic,vn):return next(iterator)
    meshes={}
    for r in selected:
        at=r['offset']+64;meshes[r['name']]=mesh(data[at:at+r['size']],r['name'],shared)
    models,nodes,missing=links(data,info,groups);check(not missing,'unresolved model references')
    texrecords={r['name']:r for r in info['records']if r['kind']=='0x0001000d'and r['name'].endswith('.dds')}
    materials={};texture_jobs=[];texpaths={};unsupported=[];texture_names=[];unmapped=Counter()
    def texture(name,size):
        key=(name,size)
        if key in texpaths:return texpaths[key]
        if name not in texrecords:unsupported.append({'texture':name,'error':'missing texture resource dependency'});return None
        tr=texrecords[name];start=tr['offset']+64
        try:pixels=dds_full(data[start:start+tr['size']],tsdata,size)
        except WadError as e:unsupported.append({'texture':name,'error':str(e)});return None
        i=len(texture_jobs);file=f'texture-{i}.dds';png=f'texture-{i}.png';(rdir/file).write_bytes(pixels)
        (rdir/png).unlink(missing_ok=True);texture_jobs.append({'file':file,'output':png,'max_size':size});texpaths[key]=png;texture_names.append(key);return png
    for r in info['records']:
        if r['kind']!='0x00010056':continue
        at=r['offset']+64;rd=Reader(data[at:at+r['size']]);check(rd.u32(0)==8,'native material version');n=rd.raw(104,1)[0];check(n<=32,'material texture count');by_slot={}
        for i in range(n):
            base=208+i*104;name=rd.raw(base+48,56).split(b'\0')[0].decode('ascii');slot=rd.raw(base,1)[0]
            by_slot.setdefault(slot,[]).append(name)
        for slot in by_slot:
            if slot not in ROLE_SLOTS.values():unmapped[slot]+=1
        roles={}
        for role,slot in ROLE_SLOTS.items():
            names=by_slot.get(slot,[])
            if len(names)==1:
                png=texture(names[0],base_size if role=='base'else detail_size)
                if png:roles[role]=png
        materials[r['name']]=roles if 'base' in roles else None
    (rdir/'texture-jobs.json').write_text(json.dumps({'schema':1,'textures':texture_jobs}));subprocess.run([str(codec.resolve()),str(rdir/'texture-jobs.json'),str(rdir/'decoded-textures.json')],check=True)
    decoded_textures=json.loads((rdir/'decoded-textures.json').read_text())['textures'];sizes={job['file']:(job['width'],job['height'])for job in decoded_textures}
    mesh_entries=[];material_profile={'schema':1,'materials':{}};uv_unsupported=[]
    for name,parts in meshes.items():
        pos=[];norm=[];uv=[];tan=[];ix=[];sections=[];mats=[]
        # Material order is declared by the referencing native MDL resource.
        mnames=next((m['materials']for m in models.values()if name in m['meshes']),[])
        for part in parts:
            offset=len(pos);pos.extend(part['positions']);norm.extend(part['normals']);uv.extend(part['uv']);tan.extend(part['tangents']);sections.append({'start':len(ix),'count':len(part['indices']),'material':len(mats)});ix.extend(offset+i for i in part['indices'])
            slot=part['material_slot'];check(slot<len(mnames),'MG material slot outside MDL table');mname=mnames[slot];roles=materials.get(mname)or{}
            spec={'source':mname,'texture':({'png':roles['base'],'width':sizes[roles['base']][0],'height':sizes[roles['base']][1]}if'base'in roles else None),'roles':sorted(roles),'selection':'native sampler slots 0/5/9 identified from texture content'}
            if not part['uv_supported']:
                spec['texture']=None;spec['source']+=' [UV0 unavailable]' 
                if part['uv_note']:uv_unsupported.append({'mesh':name,'group':part['group'],'error':part['uv_note']})
            mats.append(spec)
            if spec['texture']:
                m={'alpha_mode':'opaque','double_sided':True,'roughness_factor':.6,'base_color_texture':roles['base']}
                if 'normal' in roles:m|={'normal_texture':roles['normal'],'normal_encoding':'ag'}
                if 'specular' in roles:m|={'specular_texture':roles['specular']}
                material_profile['materials'][mname]={'material':m,'note':'Slots 0/5/9 as base/DXT5nm normal/specular; native shader, alpha and baked lighting not evaluated'}
        file=f'mesh-{len(mesh_entries)}.json';d={'source':name,'positions':pos,'normals':norm,'uv':uv,'indices':ix,'sections':sections,'materials':mats}
        if all(t is not None for t in tan):d['tangents']=tan
        (rdir/file).write_text(json.dumps(d,separators=(',',':')));mesh_entries.append({'source':name,'file':file})
    # Some MG positions are already authored in level coordinates (e.g. tiny FX
    # vertices coincide with entity translations). Applying entity matrices again
    # double-transforms them. Retain those matrices as diagnostic metadata until
    # native bind/animation composition is decoded; display encoded rest space.
    ids={m['source']:i for i,m in enumerate(mesh_entries)};identity=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]
    scene_nodes=[{'name':n['name'],'mesh':ids[n['mesh']],'matrix':identity,'native_entity_matrix':n['matrix']}for n in nodes]
    report={'source':wad.name,'records':len(info['records']),'native_mesh_groups':len(meshes),'surface_groups':sum(len(m)for m in meshes.values()),'meshlets':len(jobs),'vertices':sum(len(p['positions'])for m in meshes.values()for p in m),'triangles':sum(len(p['indices'])//3 for m in meshes.values()for p in m),'native_instances':len(nodes),'index_decoder':'crysirror-asset::indices','index_differential_matches':len(jobs),'texture_decoder':'crysirror-image::decode_standard','decoded_texture_previews':len(decoded_textures),'unsupported_textures':unsupported,'unsupported_uv':uv_unsupported,'source_normals_normalized':sum(p['nonunit_normals']for m in meshes.values()for p in m),'transform_policy':'encoded_rest_space; native entity matrices retained as metadata, not composed','original_materials':False,'original_animation_pose':False,'original_lighting':False,'complete_game_scene':False,'limitations':['First MG surface table; alternate native tables not selected','Native bind/entity/animation transform composition unresolved; encoded rest geometry only','Native shader semantics, baked vertex lighting and source punctual lights not reproduced','Source coordinate units preserved; physical scale not calibrated']}
    textured=sum(bool(mat['texture'])for entry in mesh_entries for mat in json.loads((rdir/entry['file']).read_text())['materials'])
    report['texture_sources']={'ts_stream':ts.name if ts else None,'base_max_size':base_size,'detail_max_size':detail_size,'unmapped_sampler_slots':{str(k):v for k,v in sorted(unmapped.items())}}
    report['material_summary']={'source_records':len(materials),'records_without_unique_sampler_0':sum(v is None for v in materials.values()),'textured_surface_groups':textured,'neutral_surface_groups':report['surface_groups']-textured,'surfaces_without_verified_uv':sum(not p['uv_supported']for parts in meshes.values()for p in parts),'neutral_policy':'Explicit diagnostic fallback for missing verified UV or unique base-color sampler; native shader may need no base texture'}
    profile=rdir/'native-preview-materials.json';profile.write_text(json.dumps(material_profile))
    (rdir/'scene-source.json').write_text(json.dumps({'report':report,'meshes':mesh_entries,'nodes':scene_nodes}));conversion=convert(rdir,out/'preview',eye=eye or[156,-18,-287],target=target or[156,-16,-345],material_profile=profile,project_name='God of War III: Olympus OLY100 rest-space preview',game_id='gow3-local-preview',data_driven=True)
    result={'native':report,'conversion':{k:v for k,v in conversion.items()if k not in('original','material_report')}};(out/'import-report.json').write_text(json.dumps(result,indent=2));return result

if __name__=='__main__':
    a=argparse.ArgumentParser(description=__doc__);a.add_argument('wad',type=Path);a.add_argument('output',type=Path);a.add_argument('--codec',type=Path);a.add_argument('--eye',type=float,nargs=3);a.add_argument('--target',type=float,nargs=3);a.add_argument('--ts',type=Path,help='matching .TS stream for full mip chains');a.add_argument('--base-size',type=int,default=256);a.add_argument('--detail-size',type=int,default=256);args=a.parse_args()
    print(json.dumps(preview(args.wad,args.output,args.codec,args.eye,args.target,args.ts,args.base_size,args.detail_size)if args.codec else extract(args.wad,args.output),indent=2))
