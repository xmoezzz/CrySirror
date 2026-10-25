#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Compile actual pinned CPU upstream algorithms and compare the scene port.
Reference checkouts must be external; no native game data or device required.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

DRIVER = r'''
#![allow(dead_code)]
use crysirror_scene::{input::{InputState,VmKey,VmMouseButton},animation::IntEvent,layer::SpriteTransform};
mod layer {pub type Sprite=crysirror_scene::layer::SpriteTransform;}
ORIGINAL_MODULES
fn main(){
 let mut checks=0u64;
 for mode in 0..3{for easing in 0..3{for delay in [0,37,1000]{for real in [0,1]{
  let mut a=original_animation::IntEvent::new(-3250);let mut b=IntEvent::new(-3250);
  match mode{0=>{a.set_event(1900,751,delay,easing,real);b.set_event(1900,751,delay,easing,real);},1=>{a.loop_event(-3250,1900,751,delay,easing,real);b.loop_event(-3250,1900,751,delay,easing,real);},_=>{a.turn_event(-3250,1900,751,delay,easing,real);b.turn_event(-3250,1900,751,delay,easing,real);}}
  for i in 0..400{let game=if i%7==0{0}else{16};a.update_time(game,19);b.update_time(game,19);a.frame();b.frame();
   assert_eq!([a.def_value,a.value,a.cur_time,a.end_time,a.delay_time,a.start_value,a.cur_value,a.end_value,a.loop_type,a.speed_type,a.real_flag],[b.def_value,b.value,b.cur_time,b.end_time,b.delay_time,b.start_value,b.cur_value,b.end_value,b.loop_type,b.speed_type,b.real_flag]);checks+=1;
  }
 }}}}
 let(mut a,mut b)=(original_input::InputState::default(),InputState::default());
 for i in 0..600 {a.next_frame();b.next_frame();match i%6{
  0=>{a.on_key_down(original_input::VmKey::Enter);b.on_key_down(VmKey::Enter);},
  1=>{a.on_mouse_down(original_input::VmMouseButton::Left);b.on_mouse_down(VmMouseButton::Left);},
  2=>{a.on_mouse_move(i,i/2);b.on_mouse_move(i,i/2);},
  3=>{a.on_key_up(original_input::VmKey::Enter);b.on_key_up(VmKey::Enter);},
  4=>{a.on_mouse_up(original_input::VmMouseButton::Left);b.on_mouse_up(VmMouseButton::Left);},
  _=>{a.on_key_down(original_input::VmKey::Space);b.on_key_down(VmKey::Space);a.on_key_up(original_input::VmKey::Space);b.on_key_up(VmKey::Space);},
 }
 for vk in 0..=255{assert_eq!((a.vk_is_down(vk),a.vk_down_stock(vk),a.vk_up_stock(vk)),(b.vk_is_down(vk),b.vk_down_stock(vk),b.vk_up_stock(vk)));checks+=1;}
 }
 for i in 0..480 {let s=SpriteTransform{rotate:i as f32*0.01,rotate_x:if i%2==0{0.2}else{0.},rotate_y:if i%3==0{-0.3}else{0.},scale_x:0.5+(i%7)as f32/5.,pivot_x:32.,pivot_y:16.,camera_enabled:i%4==0,billboard:i%5==0,culling:i%7==0,object_anchor:i%8==0,..SpriteTransform::default()};
 let a=original_projection::sprite_quad_geometry_rect(&s,120.,90.,0.,0.,64.,32.,640.,360.);let b=crysirror_scene::projection::sprite_quad_geometry_rect(&s,120.,90.,0.,0.,64.,32.,640.,360.);
 assert_eq!(a.is_some(),b.is_some());if let(Some(a),Some(b))=(a,b){for(a,b)in a.projected.iter().zip(&b.projected){assert_eq!([a.x.to_bits(),a.y.to_bits(),a.depth.to_bits(),a.clip_w.to_bits()],[b.x.to_bits(),b.y.to_bits(),b.depth.to_bits(),b.clip_w.to_bits()]);}assert_eq!(a.world,b.world);}checks+=1;
 }
 for window in [(800,800),(1920,1080),(300,900),(0,0)]{for point in [(-1.,-1.),(0.,0.),(100.,200.),(400.,400.),(1920.,1080.)]{let(a,rect)=original_ui::probe(point,window);let b=crysirror_scene::ui::map_window_to_virtual(point.0,point.1,window,(640,360));let transform=crysirror_scene::ui::UiSurfaceTransform::new((640,360),window);assert_eq!(a,b);assert_eq!(rect,transform.map_rect(17,23,80,24));checks+=1;}}
 println!("{{\"algorithm_comparisons\":{},\"native_vm_verified\":false}}",checks);
}
'''

def run(source_root: Path):
    manifest=json.loads((ROOT/'crates/crysirror-scene/upstream-files.json').read_text())
    paths={}
    for name, info in manifest.items():
        checkout=source_root/name
        revision=subprocess.check_output(['git','-C',str(checkout),'rev-parse','HEAD'],text=True).strip()
        if revision!=info['commit']:
            raise ValueError(f'{name}: revision differs from pinned source')
        for relative,digest in info['files'].items():
            path=checkout/relative
            if hashlib.sha256(path.read_bytes()).hexdigest()!=digest:
                raise ValueError(f'source changed: {path}')
            paths[Path(relative).name]=path
    with tempfile.TemporaryDirectory(prefix='crysirror-scene-differential-') as folder:
        temp=Path(folder);(temp/'src').mkdir()
        modules=[]
        for module,filename in [('input','input.rs'),('animation','int_event.rs'),('projection','render_math.rs')]:
            modules.append(f'#[path={json.dumps(str(paths[filename]))}] mod original_{module};')
        source=paths['exit_confirm_ui.rs'].read_text();helpers=source[source.index('struct UiSurfaceTransform'):source.index('fn fill_rect(')]
        (temp/'src/original_ui.rs').write_text(helpers+'''\npub fn probe(p:(f64,f64),w:(u32,u32))->(Option<(i32,i32)>,(i32,i32,i32,i32)){(map_window_to_virtual(p.0,p.1,w,(640,360)),UiSurfaceTransform::new((640,360),w).map_rect(17,23,80,24))}\n''')
        modules.append('mod original_ui;')
        (temp/'src/main.rs').write_text(DRIVER.replace('ORIGINAL_MODULES','\n'.join(modules)))
        (temp/'Cargo.toml').write_text('[package]\nname="crysirror-scene-differential"\nversion="0.0.0"\nedition="2021"\n[dependencies]\ncrysirror-scene={path='+json.dumps(str(ROOT/'crates/crysirror-scene'))+'}\n')
        result=subprocess.run(['cargo','run','--quiet','--offline','--manifest-path',str(temp/'Cargo.toml'),'--target-dir',str(ROOT/'target/scene-differential')],capture_output=True,text=True)
        if result.returncode:
            raise RuntimeError(result.stderr)
        print(result.stdout.strip())

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--source-root',type=Path,required=True)
    run(parser.parse_args().source_root)
