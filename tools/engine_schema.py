# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
"""Strict authoring validation for CPU engine systems (no device access)."""
from desktop_project import obj, require, integer, finite, vector, color, logical_path

U32 = lambda v: integer(v, 0, 2**32-1)
I32 = lambda v: integer(v, -(2**31), 2**31-1)

def bounds(r):
    require(vector(r, 4) and 0 < r[2] <= 16384 and 0 < r[3] <= 16384, 'engine rectangle')

def flags(v, names):
    for n in names:
        if n in v:
            require(type(v[n]) is bool, f'engine boolean: {n}')

def transform(t):
    obj(t, set(), {'translation', 'rotation_degrees', 'scale'})
    for k in ('translation', 'rotation_degrees', 'scale'):
        if k in t:
            require(vector(t[k], 3), 'world transform')
    require(all(abs(x) > 1e-6 for x in t.get('scale', [1]*3)), 'singular world scale')

def geometry(g):
    require(isinstance(g, dict), 'world geometry')
    if g.get('kind') in ('model','scene'):
        obj(g,{'kind','asset'}|({'mesh'}if g['kind']=='model'else set()))
        require(logical_path(g['asset'])==g['asset'],'model asset path')
        if g['kind']=='model':require(U32(g['mesh']),'model mesh index')
        return 0
    if g.get('kind') == 'cube':
        obj(g, {'kind'})
        return 36
    obj(g, {'kind', 'vertices', 'indices'})
    require(g['kind'] == 'triangles' and isinstance(g['vertices'], list) and bool(g['vertices']) and all(vector(v, 3) for v in g['vertices']), 'world vertices')
    ix = g['indices']
    require(isinstance(ix, list) and bool(ix) and len(ix)%3 == 0 and all(integer(i, 0, len(g['vertices'])-1) for i in ix), 'world indices')
    return len(ix)

def tween(t):
    obj(t, {'property', 'from', 'to', 'duration_ms'}, {'delay_ms', 'mode', 'easing', 'real_time'})
    require(t['property'] in ('x','y','z','rotation_x','rotation_y','rotation_z'), 'tween property')
    require(integer(t['from'], -100_000_000, 100_000_000) and integer(t['to'], -100_000_000, 100_000_000), 'tween range')
    require(integer(t['duration_ms'], 1, 86_400_000) and integer(t.get('delay_ms', 0), 0, 86_400_000), 'tween time')
    require(integer(t.get('mode',0),0,2) and integer(t.get('easing',0),0,2), 'tween mode/easing')
    flags(t, ['real_time'])

def effect(e,alpha_mode):
    require(isinstance(e,dict),'shader effect object')
    kind=e.get('kind')
    ranges={'fog':{'density':(0,20),'noise_scale':(.01,128),'edge_fade':(0,.5)},'fresnel':{'power':(.01,64),'intensity':(0,100)},'dissolve':{'threshold':(0,1),'edge_width':(.0001,1)}}
    if kind=='custom':
        obj(e,{'kind','asset'},{'parameters'})
        require(logical_path(e['asset'])==e['asset'] and e['asset'].endswith('.wgsl'),'custom shader requires canonical .wgsl path')
        ps=e.get('parameters',[[0]*4]*4)
        require(isinstance(ps,list)and len(ps)==4 and all(vector(p,4,lambda x:finite(x)and abs(x)<=100000)for p in ps),'shader parameter array')
        return
    require(kind in ranges,'unknown shader effect')
    obj(e,{'kind'},set(ranges[kind])|({'speed'}if kind=='fog'else {'edge_color'}if kind=='dissolve'else set()))
    for key,(lo,hi)in ranges[kind].items():
        if key in e:require(finite(e[key])and lo<=e[key]<=hi,'shader parameter range')
    if kind=='fog':
        require(alpha_mode in ('blend','additive','premultiplied'),'fog requires transparency')
        if 'speed'in e:require(vector(e['speed'],2,lambda x:finite(x)and abs(x)<=100),'fog speed')
    if kind=='dissolve':
        require(alpha_mode!='opaque','dissolve requires mask or transparency')
        if 'edge_color'in e:require(vector(e['edge_color'],3,lambda x:finite(x)and 0<=x<=100),'dissolve edge color')

def material(m):
    textures={'base_color_texture','metallic_roughness_texture','normal_texture','occlusion_texture','emissive_texture','opacity_texture','specular_texture'}
    scalars={'metallic_factor','roughness_factor','occlusion_strength','alpha_cutoff','specular_factor'}
    obj(m,set(),textures|scalars|{'base_color_factor','emissive_factor','emissive_strength','normal_scale','normal_encoding','alpha_mode','double_sided','unlit','opacity_from_alpha','uv_scale','uv_offset','depth_fade','effect'})
    require(m.get('normal_encoding','rgb')in('rgb','rg','ag'),'material normal encoding')
    require(vector(m.get('base_color_factor',[1]*4),4,lambda v:finite(v)and 0<=v<=1),'material base color')
    require(vector(m.get('emissive_factor',[0]*3),3,lambda v:finite(v)and 0<=v<=65504),'material emissive')
    for k in scalars&m.keys():require(finite(m[k])and 0<=m[k]<=1,'material scalar')
    require(finite(m.get('depth_fade',0))and 0<=m.get('depth_fade',0)<=100000,'material depth fade')
    require(finite(m.get('normal_scale',1))and abs(m.get('normal_scale',1))<=100,'material normal scale')
    require(finite(m.get('emissive_strength',1))and 0<=m.get('emissive_strength',1)<=65504,'material emissive strength')
    require(m.get('alpha_mode','opaque')in('opaque','mask','blend','additive','premultiplied'),'material alpha mode')
    flags(m,['double_sided','unlit','opacity_from_alpha'])
    if m.get('effect')is not None:effect(m['effect'],m.get('alpha_mode','opaque'))
    for k in ('uv_scale','uv_offset'):
        if k in m:require(vector(m[k],2,lambda v:finite(v)and abs(v)<=100000),'material UV transform')
    for k in textures&m.keys():
        if m[k]is not None:require(logical_path(m[k])==m[k],'non-canonical material path')

def light(l):
    """Mirror of crysirror_scene::components::LightSpec::validate (ADR 0018)."""
    obj(l,set(),{'shape','color','intensity','units','falloff','range','shadows','enabled'})
    shape=l.get('shape',{'kind':'point'})
    require(isinstance(shape,dict),'light shape')
    kind=shape.get('kind')
    if kind=='point':obj(shape,{'kind'})
    elif kind=='spot':
        obj(shape,{'kind'},{'inner_degrees','outer_degrees'})
        inner,outer=shape.get('inner_degrees',20),shape.get('outer_degrees',35)
        require(finite(inner)and finite(outer)and 0<=inner<=outer and 0.1<=outer<=89,'invalid spot cone')
    elif kind=='rect':
        obj(shape,{'kind'},{'size'})
        require(vector(shape.get('size',[1,1]),2,lambda v:finite(v)and 0.001<=v<=10000),'invalid rect light size')
    elif kind in('directional','ambient'):
        obj(shape,{'kind'})
        require(l.get('units','candela')=='candela','directional/ambient light intensity is illuminance; lumens do not apply')
    else:require(False,'unknown light shape')
    falloff=l.get('falloff',{'kind':'inverse_square'})
    require(isinstance(falloff,dict),'light falloff')
    if falloff.get('kind')=='inverse_square':obj(falloff,{'kind'})
    elif falloff.get('kind')=='exponent':
        obj(falloff,{'kind','exponent'});require(finite(falloff['exponent'])and 0.01<=falloff['exponent']<=64,'invalid light falloff exponent')
    else:require(False,'unknown light falloff')
    require(vector(l.get('color',[1]*3),3,lambda v:finite(v)and 0<=v<=1000),'invalid light color')
    require(finite(l.get('intensity',10))and 0<=l.get('intensity',10)<=1e9,'invalid light intensity')
    require(finite(l.get('range',10))and 0.01<=l.get('range',10)<=100000,'invalid light range')
    require(l.get('units','candela')in('candela','lumens'),'light units')
    flags(l,['shadows','enabled'])

def entity(e):
    obj(e, {'id','name'}, {'parent','transform','geometry','color','visible','velocity','angular_velocity','tweens','material','light','reflection_probe','irradiance_volume','particles','vfx'})
    require(U32(e['id']) and isinstance(e['name'],str) and 0<len(e['name'].encode())<=256, 'entity identity')
    if e.get('parent') is not None:
        require(U32(e['parent']), 'entity parent')
    transform(e.get('transform',{}))
    if e.get('material')is not None:material(e['material'])
    if e.get('light')is not None:light(e['light'])
    if e.get('reflection_probe')is not None:reflection_probe(e['reflection_probe'])
    if e.get('irradiance_volume')is not None:irradiance_volume(e['irradiance_volume'])
    require(e.get('particles') is None or e.get('vfx') is None, 'choose either legacy particles or modular vfx')
    if e.get('vfx') is not None:
        import vfx_schema
        vfx_schema.validate(e['vfx'])
    if e.get('particles') is not None:
        import vfx_schema
        vfx_schema.legacy(e['particles'])
    flags(e,['visible'])
    require(color(e.get('color',[255]*4)) and e.get('color',[255]*4)[3]==255, 'world opaque material')
    for k in ['velocity','angular_velocity']:
        require(vector(e.get(k,[0]*3),3,lambda n:finite(n) and abs(n)<=100_000), 'world velocity')
    ts=e.get('tweens',[])
    require(isinstance(ts,list) and len(ts)<=64, 'tween count')
    for t in ts:
        tween(t)
    return geometry(e['geometry']) if e.get('geometry') is not None else 0

SEQUENCE_ARITY={'translation':3,'rotation':3,'scale':3,'light_color':3,'light_intensity':1,'light_range':1,'visible':1}
def sequence_noise(n):
    obj(n,{'amplitude','frequency_hz'},{'seed'});require(finite(n['amplitude'])and abs(n['amplitude'])<=1e6 and finite(n['frequency_hz'])and 0<=n['frequency_hz']<=1000 and integer(n.get('seed',0),0,2**32-1),'invalid noise')
def sequence_times(times,d,what):
    require(1<=len(times)<=4096 and all(finite(t)for t in times)and all(a<b for a,b in zip(times,times[1:]))and times[0]>=0 and times[-1]<=d,f'{what}: key times must increase inside the sequence')
def sequence(q):
    """Mirror of crysirror_scene::sequence::SequenceSpec::validate (ADR 0040)."""
    obj(q,{'duration_ms'},{'looping','speed','camera','tracks','events'})
    d=q['duration_ms'];require(finite(d)and 0<d<=86_400_000,'sequence duration');flags(q,['looping'])
    require(finite(q.get('speed',1))and 0<=q.get('speed',1)<=16,'sequence speed must be 0..16')
    modes=('linear','step','smooth','spline')
    if q.get('camera')is not None:
        c=q['camera'];obj(c,{'keys'},{'interpolation','shake'});require(c.get('interpolation','spline')in modes,'interpolation')
        sequence_times([k['time_ms']for k in c['keys']],d,'camera track')
        for k in c['keys']:
            obj(k,{'time_ms','eye','target'},{'fov_y_degrees'});require(vector(k['eye'],3)and vector(k['target'],3)and k['eye']!=k['target'],'camera key')
            if k.get('fov_y_degrees')is not None:require(finite(k['fov_y_degrees'])and 1<=k['fov_y_degrees']<=179,'camera key fov')
        if c.get('shake')is not None:sequence_noise(c['shake'])
    tracks=q.get('tracks',[]);require(isinstance(tracks,list)and len(tracks)<=512,'sequence too large')
    for t in tracks:
        obj(t,{'entity','property','keys'},{'interpolation','noise'});require(U32(t['entity'])and t['property']in SEQUENCE_ARITY and t.get('interpolation','spline')in modes,'sequence track')
        sequence_times([k['time_ms']for k in t['keys']],d,'track')
        for k in t['keys']:
            obj(k,{'time_ms','value'});require(vector(k['value'],SEQUENCE_ARITY[t['property']],lambda v:finite(v)and abs(v)<=1e7),'track keys need finite values')
            if t['property']=='scale':require(all(abs(v)>1e-6 for v in k['value']),'scale keys must be non-zero')
        if t.get('noise')is not None:sequence_noise(t['noise'])
    events=q.get('events',[]);require(isinstance(events,list)and len(events)<=4096,'sequence too large')
    for e in events:
        obj(e,{'time_ms','name'});require(finite(e['time_ms'])and 0<=e['time_ms']<=d and isinstance(e['name'],str)and 0<len(e['name'])<=128,'sequence event')

def command(c):
    require(isinstance(c,dict), 'world command')
    op=c.get('op')
    if op=='spawn':
        obj(c,{'op','entity'}); entity(c['entity']); return
    if op=='set_material':
        obj(c,{'op','id','material'});require(U32(c['id']),'world entity id')
        if c['material']is not None:material(c['material'])
        return
    if op in ('set_vfx', 'set_particles'):
        key = 'vfx' if op == 'set_vfx' else 'particles'
        obj(c, {'op', 'id', key}); require(U32(c['id']), 'world entity id')
        if c[key] is not None:
            import vfx_schema
            (vfx_schema.validate if key == 'vfx' else vfx_schema.legacy)(c[key])
        return
    if op=='set_video':
        obj(c,{'op','name','video'});require(isinstance(c['name'],str)and 0<len(c['name'])<=128,'video name')
        if c['video']is not None:
            v=c['video'];obj(v,{'asset'},{'looping','playing','time_ms','slot'});logical_path(v['asset'])
            flags(v,['looping','playing']);require(integer(v.get('time_ms',0),0,10**12),'video time')
        return
    if op=='play_sequence':
        obj(c,{'op','name','sequence'},{'start_ms'});require(isinstance(c['name'],str)and 0<len(c['name'])<=128,'sequence name')
        sequence(c['sequence']);d=c['sequence']['duration_ms'];require(finite(c.get('start_ms',0))and 0<=c.get('start_ms',0)<=d,'sequence start')
        return
    if op=='stop_sequence':
        obj(c,{'op','name'});require(isinstance(c['name'],str)and 0<len(c['name'])<=128,'sequence name');return
    if op=='set_irradiance_volume':
        obj(c,{'op','id','volume'});require(U32(c['id']),'world entity id')
        if c['volume']is not None:irradiance_volume(c['volume'])
        return
    if op=='set_reflection_probe':
        obj(c,{'op','id','probe'});require(U32(c['id']),'world entity id')
        if c['probe']is not None:reflection_probe(c['probe'])
        return
    if op=='set_light':
        obj(c,{'op','id','light'});require(U32(c['id']),'world entity id')
        if c['light']is not None:light(c['light'])
        return
    if op=='pause':
        obj(c,{'op','paused'}); flags(c,['paused']); return
    fields={'despawn':set(),'set_transform':{'transform'},'set_velocity':{'velocity'},'set_parent':{'parent'},'set_visible':{'visible'},'tween':{'tween'}}
    require(op in fields, 'unsupported world command')
    obj(c,{'op','id'}|fields[op]); require(U32(c['id']), 'world command id')
    if op=='set_transform': transform(c['transform'])
    if op=='set_velocity': require(vector(c['velocity'],3,lambda n:finite(n) and abs(n)<=100_000),'world velocity')
    if op=='set_parent': require(c['parent'] is None or U32(c['parent']),'world parent')
    if op=='set_visible': flags(c,['visible'])
    if op=='tween': tween(c['tween'])

def scalar(c):
    """Rust Unicode scalar and char::is_control parity, not isprintable()."""
    n=ord(c)
    return not (n<32 or 127<=n<160 or 0xd800<=n<=0xdfff)

def glyph_reveal(r,total):
    obj(r,{'interval_ms'},{'unit','initial_visible','elapsed_ms','forced_complete','paused'})
    require(r.get('unit','unicode_scalar')=='unicode_scalar' and integer(r['interval_ms'],0,86_400_000), 'glyph reveal interval/unit')
    require(integer(r.get('initial_visible',0),0,total) and integer(r.get('elapsed_ms',0),0,2**64-1),'glyph reveal progress')
    flags(r,['forced_complete','paused'])

def paint(p):
    require(isinstance(p,dict),'layer paint')
    kind=p.get('kind')
    if kind=='solid':
        obj(p,{'kind'},{'color'}); require(color(p.get('color',[255]*4)),'paint color')
    elif kind=='image':
        obj(p,{'kind','asset'},{'tint'}); logical_path(p['asset']); require(color(p.get('tint',[255]*4)),'paint tint')
    elif kind=='glyph_run':
        obj(p, {'kind','font','size','glyphs'}, {'color','bold','reveal'})
        require(isinstance(p['font'],str) and bool(p['font']) and len(p['font'].encode('utf-8', errors='replace'))<=256 and all(scalar(c) for c in p['font']), 'glyph font alias')
        require(finite(p['size']) and 1<=p['size']<=512 and color(p.get('color',[255]*4)), 'glyph style')
        flags(p,['bold'])
        glyphs=p['glyphs']; require(isinstance(glyphs,list) and len(glyphs)<=65536,'glyph count')
        for g in glyphs:
            obj(g,{'character','baseline'},{'size','color','bold'})
            require(isinstance(g['character'],str) and len(g['character'])==1 and scalar(g['character']) and vector(g['baseline'],2),'positioned glyph')
            if g.get('size') is not None: require(finite(g['size']) and 1<=g['size']<=512,'glyph size')
            if g.get('color') is not None: require(color(g['color']),'glyph color')
            if g.get('bold') is not None: flags(g,['bold'])
        if p.get('reveal') is not None: glyph_reveal(p['reveal'],len(glyphs))
    elif kind=='puppet':
        # Mirror of Paint::Puppet (ADR 0029): an engine rig JSON, never PSB.
        obj(p,{'kind','asset'},{'tint'}); logical_path(p['asset']); require(color(p.get('tint',[255]*4)),'paint tint')
        require(not p['asset'].lower().endswith('.psb'),'puppet layers take engine rigs, not PSB (import it first)')
    else:
        obj(p,{'kind','text','font','size'},{'color','layout'})
        require(kind=='text' and isinstance(p['text'],str) and len(p['text'].encode())<=1024*1024 and isinstance(p['font'],str) and bool(p['font']) and integer(p['size'],1,512),'text paint')
        require(color(p.get('color',[255]*4)) and p.get('layout','simple-horizontal-v1')=='simple-horizontal-v1','text paint profile')

def sprite_transform(t):
    floats={'z','scale_x','scale_y','scale_z','rotate','rotate_x','rotate_y','pivot_x','pivot_y','pivot_z','texture_center_x','texture_center_y','camera_view_angle_deg'}
    bools={'object_anchor','camera_enabled','billboard','culling'}
    vectors={'camera_eye','camera_target','camera_up'}
    obj(t,set(),floats|bools|vectors)
    for k in floats&t.keys(): require(finite(t[k]),'sprite transform')
    for k in vectors&t.keys(): require(vector(t[k],3),'sprite camera')
    flags(t,bools)
    for k in ['scale_x','scale_y','scale_z']: require(abs(t.get(k,1))>1e-6,'singular layer')
    if t.get('camera_enabled'):
        require(1<=t.get('camera_view_angle_deg',45)<179,'sprite FOV')
        d=[b-a for a,b in zip(t.get('camera_eye',[0,0,-1000]),t.get('camera_target',[0]*3))];u=t.get('camera_up',[0,1,0]);cross=[d[1]*u[2]-d[2]*u[1],d[2]*u[0]-d[0]*u[2],d[0]*u[1]-d[1]*u[0]]
        require(sum(v*v for v in cross)>1e-12,'sprite camera degeneracy')

def tree(items):
    require(isinstance(items,list) and len(items)<=4096,'engine object count')
    ids={}
    for item in items:
        require(isinstance(item,dict) and U32(item.get('id')) and item['id'] not in ids,'duplicate/invalid engine id')
        ids[item['id']]=item
    for item in items:
        seen={item['id']};p=item.get('parent')
        while p is not None:
            require(U32(p) and p in ids and p not in seen and len(seen)<=64,'engine parent missing/cycle/depth')
            seen.add(p);p=ids[p].get('parent')

def environment(e):
    obj(e,set(),{'hdr','intensity','rotation_degrees','exposure','tonemap','show_sky','sun','bloom','ambient_occlusion','temporal_aa','shadow_distance','fog','auto_exposure','reflections','geometry_lod','virtual_shadows'})
    if e.get('virtual_shadows')is not None:
        v=e['virtual_shadows'];obj(v,set(),{'texel_size','levels'})
        require(finite(v.get('texel_size',.02))and .0001<=v.get('texel_size',.02)<=10 and integer(v.get('levels',10),1,16),'invalid virtual shadows')
    if e.get('geometry_lod')is not None:
        g=e['geometry_lod'];obj(g,set(),{'pixel_error','gpu'});flags(g,['gpu']);require(finite(g.get('pixel_error',1))and 0<=g.get('pixel_error',1)<=1000,'invalid geometry LOD')
    if e.get('reflections')is not None:
        r=e['reflections'];obj(r,set(),{'intensity','max_distance','thickness','max_roughness','steps'})
        for k,default,lo,hi in [('intensity',1,0,4),('max_distance',30,.01,10000),('thickness',.3,.001,100),('max_roughness',.6,.01,1)]:
            require(finite(r.get(k,default))and lo<=r.get(k,default)<=hi,'invalid screen-space reflections')
        require(integer(r.get('steps',48),4,256),'invalid screen-space reflections')
    if e.get('fog')is not None:
        f=e['fog'];obj(f,set(),{'color','density','height_falloff','base_height','start_distance','max_opacity','sun_scattering','sun_exponent'})
        require(vector(f.get('color',[.45,.5,.6]),3,lambda x:finite(x)and 0<=x<=1000),'fog color')
        for k,default,lo,hi in [('density',.02,0,100),('height_falloff',.2,0,100),('base_height',0,-1e6,1e6),('start_distance',0,0,1e6),('max_opacity',1,0,1),('sun_scattering',0,0,1000),('sun_exponent',8,1,256)]:
            require(finite(f.get(k,default))and lo<=f.get(k,default)<=hi,'invalid fog')
    if e.get('auto_exposure')is not None:
        a=e['auto_exposure'];obj(a,set(),{'min_log2','max_log2','speed_up','speed_down','compensation'})
        lo,hi=a.get('min_log2',-8),a.get('max_log2',8)
        require(all(finite(a.get(k,0))for k in a)and -24<=lo<=hi<=24 and 0<=a.get('speed_up',3)<=100 and 0<=a.get('speed_down',1)<=100 and abs(a.get('compensation',0))<=20,'invalid auto exposure')
    flags(e,['show_sky','temporal_aa'])
    if e.get('hdr')is not None:require(logical_path(e['hdr'])==e['hdr'] and '..'not in e['hdr'],'environment asset path')
    require(e.get('tonemap','pbr_neutral')in('pbr_neutral','aces','none'),'environment tonemap')
    require(finite(e.get('rotation_degrees',0)),'environment rotation')
    for k,default,lo,hi in [('intensity',1,0,1000),('exposure',0,-20,20),('bloom',.04,0,1),('ambient_occlusion',1,0,4),('shadow_distance',60,.1,100000)]:
        require(finite(e.get(k,default))and lo<=e.get(k,default)<=hi,'environment parameter')
    if e.get('sun')is not None:
        sun=e['sun'];obj(sun,set(),{'direction','color','intensity','shadows'});flags(sun,['shadows'])
        d=sun.get('direction',[-.4,-1,-.3]);require(vector(d,3,lambda x:finite(x)and abs(x)<=1000000)and sum(x*x for x in d)>1e-8,'sun direction')
        require(vector(sun.get('color',[1]*3),3,lambda x:finite(x)and 0<=x<=1000000),'sun color')
        require(finite(sun.get('intensity',3))and 0<=sun.get('intensity',3)<=100000,'sun intensity')

def irradiance_volume(v):
    """Mirror of crysirror_scene::components::IrradianceVolumeSpec::validate (ADR 0028)."""
    obj(v,{'extent'},{'counts','intensity','probes_per_frame','idle_probes_per_frame','visibility','revision'})
    counts=v.get('counts',[8,4,8])
    require(vector(v['extent'],3,lambda x:finite(x)and .01<=x<=100000)and vector(counts,3,lambda c:integer(c,2,64))and counts[0]*counts[1]*counts[2]<=4096
            and finite(v.get('intensity',1))and 0<=v.get('intensity',1)<=16 and integer(v.get('probes_per_frame',8),1,256)and integer(v.get('idle_probes_per_frame',0),0,v.get('probes_per_frame',8))and isinstance(v.get('visibility',True),bool)and integer(v.get('revision',0),0,2**32-1),'invalid irradiance volume')

def reflection_probe(p):
    """Mirror of crysirror_scene::components::ReflectionProbeSpec::validate (ADR 0024)."""
    obj(p,{'extent'},{'intensity','blend_distance','revision','source','parallax'});flags(p,['parallax'])
    if p.get('source')is not None:require(isinstance(p['source'],str)and p['source'].endswith('.hdr')and logical_path(p['source'])==p['source']and'..'not in p['source'],'reflection probe source')
    require(vector(p['extent'],3,lambda v:finite(v)and .01<=v<=100000)and finite(p.get('intensity',1))and 0<=p.get('intensity',1)<=16
            and finite(p.get('blend_distance',.5))and 0<=p.get('blend_distance',.5)<=100000 and integer(p.get('revision',0),0,2**32-1),'invalid reflection probe')

def layer_filter(f):
    """Mirror of crysirror_scene::layer::LayerFilter::validate (ADR 0021)."""
    obj(f,set(),{'mono','invert','bright','dark','color_rate','color','add','tone_curve','mask','alpha_test','distortion'})
    unit=lambda v:finite(v)and 0<=v<=1
    require(all(unit(f.get(k,0))for k in('mono','invert','bright','dark','color_rate'))and vector(f.get('color',[0]*3),3,unit)
            and vector(f.get('add',[0]*3),3,lambda v:finite(v)and -1<=v<=1),'invalid layer filter color parameters')
    flags(f,['alpha_test'])
    if f.get('tone_curve')is not None:
        t=f['tone_curve'];obj(t,{'asset'},{'row','saturation'});logical_path(t['asset'])
        require(unit(t.get('row',0))and unit(t.get('saturation',0)),'invalid tone curve')
    if f.get('mask')is not None:
        m=f['mask'];obj(m,{'asset'},{'gray_alpha'});logical_path(m['asset']);flags(m,['gray_alpha'])
    d=f.get('distortion')
    if d is not None:
        rng=lambda k,lo,hi,default=None:finite(d.get(k,default))and lo<=d.get(k,default)<=hi
        kind=d.get('kind')
        if kind=='mosaic':obj(d,{'kind','cut'},{'aspect'});ok=rng('cut',1e-5,1)and rng('aspect',1e-3,1000,1)
        elif kind=='raster':obj(d,{'kind','fractions','waves','power','progress'},{'vertical'});flags(d,['vertical']);ok=rng('fractions',1,10000)and rng('waves',-1000,1000)and rng('power',-1000,1000)and rng('progress',0,1)
        elif kind=='radial_blur':obj(d,{'kind','center','power','coeff'});ok=vector(d['center'],2,lambda v:finite(v)and -10<=v<=10)and rng('power',0,10000)and rng('coeff',0,1000)
        elif kind=='threshold_fade':obj(d,{'kind','fade','threshold'},{'invert'});flags(d,['invert']);ok=rng('fade',0,1)and rng('threshold',0,1)
        else:ok=False
        require(ok,'invalid layer distortion')

def layer(l):
    obj(l,{'id','rect','paint'},{'parent','transform','order','clip','opacity','visible','blend','uv','filter'})
    require(U32(l['id']) and (l.get('parent') is None or U32(l['parent'])), 'layer identity')
    if l.get('filter')is not None:layer_filter(l['filter'])
    bounds(l['rect']);paint(l['paint']);sprite_transform(l.get('transform',{}));flags(l,['visible'])
    require(vector(l.get('order',[0,0]),2,I32) and integer(l.get('opacity',255),0,255) and l.get('blend','normal') in ['normal','add','subtract','multiply','screen'],'layer composition')
    if l.get('clip') is not None: bounds(l['clip'])
    if l.get('uv') is not None:
        uv=l['uv'];require(vector(uv,4,lambda n:finite(n) and 0<=n<=1) and uv[0]<uv[2] and uv[1]<uv[3],'layer UV')


def validate(s):
    obj(s,set(),{'entities','layers','widgets','environment'})
    if s.get('environment')is not None:environment(s['environment'])
    entities=s.get('entities',[]);layers=s.get('layers',[]);widgets=s.get('widgets',[])
    for items in (entities,layers,widgets): tree(items)
    require(sum(entity(e) for e in entities)<=1_000_000,'world vertex budget')
    for l in layers: layer(l)
    for w in widgets:
        obj(w,{'id','rect'},{'parent','kind','flow','padding','gap','z','visible','enabled','clip_children','modal','color','text','font','text_size','on_activate','value','max_length'})
        bounds(w['rect']);flags(w,['visible','enabled','clip_children','modal'])
        require(w.get('kind','panel') in ['panel','label','button','toggle','slider','text_field'] and w.get('flow','absolute') in ['absolute','row','column'],'widget type/flow')
        require(integer(w.get('padding',0),0,8192) and integer(w.get('gap',0),0,8192) and I32(w.get('z',0)) and color(w.get('color',[52,66,88,255])),'widget appearance')
        require(isinstance(w.get('text',''),str) and all(not 0xd800<=ord(c)<=0xdfff for c in w.get('text','')) and len(w.get('text','').encode())<=1024*1024 and integer(w.get('text_size',18),1,512),'widget text')
        require(w.get('font') is None or isinstance(w['font'],str),'widget font')
        require(U32(w.get('max_length',1024)), 'widget max length')
        if w.get('kind')=='text_field': require(w.get('max_length',1024)<=65536 and len(w.get('text',''))<=w.get('max_length',1024) and all(scalar(c) for c in w.get('text','')), 'text field length or committed text')
        require(not w.get('text') or bool(w.get('font')),'widget requires font')
        require(finite(w.get('value',0)) and 0<=w.get('value',0)<=1,'widget value')
        actions=w.get('on_activate',[]);require(isinstance(actions,list) and len(actions)<=128,'widget action count')
        for c in actions: command(c)
        validate_action_targets(entities,actions)


def validate_action_targets(entities,commands):
    import copy
    state={e['id']:copy.deepcopy(e) for e in entities}
    for c in commands:
        op=c['op']
        if op=='pause': continue
        if op=='spawn':
            e=c['entity'];require(e['id'] not in state,'duplicate spawned entity');state[e['id']]=copy.deepcopy(e);continue
        require(c['id'] in state,'widget target entity missing')
        if op=='despawn':
            removed={c['id']}
            while True:
                n=len(removed);removed|={i for i,e in state.items() if e.get('parent') in removed}
                if len(removed)==n: break
            state={i:e for i,e in state.items() if i not in removed}
        elif op=='set_parent': state[c['id']]['parent']=c['parent']
        elif op=='set_vfx': state[c['id']]['vfx']=c['vfx']
        elif op=='set_particles': state[c['id']]['particles']=c['particles']
        elif op=='tween':
            ts=state[c['id']].setdefault('tweens',[]);ts.append(c['tween']);require(len(ts)<=64,'widget target tween limit')
    tree(list(state.values()))
    for e in state.values():
        require(e.get('particles') is None or e.get('vfx') is None, 'choose either legacy particles or modular vfx')

def transition(t):
    """Shared transition spec. Native raw Siglus codes remain adapter policy."""
    obj(t,{'duration_ms','effect'},{'easing'})
    require(integer(t['duration_ms'],1,600000) and t.get('easing','linear') in ('linear','ease_in','ease_out','smooth'),'transition timing')
    e=t['effect']; require(isinstance(e,dict),'transition effect'); kind=e.get('kind')
    if kind=='ordered_dither':
        obj(e,{'kind','cell','steps'},{'diagonal'});flags(e,['diagonal'])
        require(vector(e['cell'],2,lambda x:integer(x,1,8192)) and integer(e['steps'],1,65536),'ordered dither limits')
    elif kind=='crossfade':obj(e,{'kind'})
    elif kind=='mask':
        obj(e,{'kind','asset'},{'fade'});logical_path(e['asset']);require(integer(e.get('fade',0),0,8),'mask fade')
    elif kind=='slide':obj(e,{'kind','direction'});require(integer(e['direction'],0,3),'slide direction')
    elif kind=='mosaic':obj(e,{'kind','size'});require(integer(e['size'],1,1024),'mosaic size')
    else:require(False,'unsupported authoring transition effect')

def canvas_command(c):
    require(isinstance(c,dict),'canvas command');op=c.get('op')
    fields={'create':({'id','width','height'},set()),'clear':({'id','color'},set()),'draw':({'id','layer'},set()),'set':({'id','layer'},set()),'copy':({'source','destination','rect','position','opacity'},{'replace'}),'present':({'id','layer'},{'order'}),'hide':({'layer'},set())}
    require(op in fields,'canvas operation');required,optional=fields[op];obj(c,required|{'op'},optional)
    for key in ('id','source','destination'):
        if key in c:require(U32(c[key]),'canvas id')
    if op=='create':require(integer(c['width'],1,8192) and integer(c['height'],1,8192),'canvas dimensions')
    elif op=='clear':require(color(c['color']),'canvas clear color')
    elif op in ('draw','set'):
        layer(c['layer']);require(c['layer'].get('parent') is None,'canvas draw cannot have layer parent')
    elif op=='copy':
        require(vector(c['rect'],4) and c['rect'][2]>0 and c['rect'][3]>0 and vector(c['position'],2) and c['opacity']==255 and type(c['opacity']) is int,'canvas copy bounds/opacity')
        flags(c,['replace'])
    elif op in ('present','hide'):require(U32(c['layer']) and I32(c.get('order',0)),'canvas presentation')

def media_options(options):
    """Options accepted by the public Media.play script API."""
    obj(options,set(),{'loop','audio','gain'});flags(options,['loop','audio'])
    require(finite(options.get('gain',1)) and 0<=options.get('gain',1)<=4,'media gain')

def menu_spec(menu):
    obj(menu, {'id','rect','font','items'}, {'text_size','cell_width','popup_width','row_height','visible','context'})
    require(integer(menu['id'],1,65535), 'menu id')
    bounds(menu['rect'])
    require(isinstance(menu['font'],str) and 0<len(menu['font'].encode())<=128, 'menu font alias')
    require(integer(menu.get('text_size',18),1,512), 'menu text size')
    flags(menu,['visible','context'])
    for key,default in [('cell_width',100),('popup_width',240),('row_height',24)]:
        value=menu.get(key,default);require(finite(value) and 8<=value<=8192,'menu row/popup limits')
    seen=set()
    def items(values, depth):
        require(isinstance(values,list) and depth<=16, 'menu nesting/items')
        checked=set()
        for item in values:
            obj(item, {'id','label'}, {'visible','enabled','checked','radio','group','separator','children'})
            require(integer(item['id'],1,65535) and item['id'] not in seen,'menu item id')
            seen.add(item['id']);require(len(seen)<=1024,'menu item count')
            require(isinstance(item['label'],str) and len(item['label'].encode())<=4096,'menu label')
            flags(item,['visible','enabled','checked','radio','separator'])
            group=item.get('group',0);require(U32(group),'menu radio group')
            if item.get('radio',False) and item.get('checked',False):
                require(group not in checked,'multiple checked radio siblings');checked.add(group)
            items(item.get('children',[]),depth+1)
    items(menu['items'],0)

def ui_command(c):
    require(isinstance(c,dict),'UI command');op=c.get('op')
    if op in ('set_title','insert_text'):
        obj(c,{'op','text'});limit=1024 if op=='set_title' else 1024*1024
        require(isinstance(c['text'],str) and len(c['text'].encode('utf-8', errors='replace'))<=limit and all(scalar(x) for x in c['text']),'UI single-line Unicode text')
    elif op=='set_cursor_visible':obj(c,{'op','visible'});flags(c,['visible'])
    elif op in ('backspace','focus'):obj(c,{'op','id'});require(U32(c['id']),'UI widget id')
    elif op=='set_menu':obj(c,{'op','menu'});menu_spec(c['menu'])
    elif op in ('remove_menu','close_menu'):obj(c,{'op','id'});require(U32(c['id']),'menu id')
    elif op=='open_menu':
        obj(c,{'op','id'},{'item','position'});require(U32(c['id']),'menu id')
        require(c.get('item') is None or U32(c['item']),'menu item')
        require(c.get('position') is None or vector(c['position'],2),'menu position')
    elif op=='set_menu_checked':
        obj(c,{'op','id','item','checked'});require(U32(c['id']) and U32(c['item']),'menu id');flags(c,['checked'])
    else:require(False,'unsupported authoring UI operation')
