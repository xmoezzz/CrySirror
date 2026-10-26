// SPDX-License-Identifier: MPL-2.0
// All effect algorithms are engine modules. TS only authors data and controls
// instances: no per-particle loop, private image compositor or VM renderer.
let trail: Entity;
let fire: Entity;
let time = 0;
const flame = { kind: "sprite", texture: "flame-sheet.png", columns: 4, rows: 2,
  frames: 8, fps: 12, soft_distance: 0.3, blend: "additive" };
const colorCurve = (start: number[], end: number[]) => ({ keys: [{time:0,value:start},{time:1,value:end}] });
const sizeCurve = (start: number, end: number) => ({ keys: [{time:0,value:[start,start]},{time:1,value:[end,end]}] });

function onStart(): void {
  world.setCamera({ eye:[5.5,3.4,7.6], target:[0,0.6,0], fov:48, near:0.1, far:80 });
  world.setEnvironment({ hdr:"royal_esplanade_2k.hdr", intensity:0.4, show_sky:false,
    bloom:0.07, temporal_aa:true, sun:{direction:[-0.5,-1,0.2],color:[0.55,0.7,1],intensity:2,shadows:true},
    fog:{color:[0.015,0.025,0.05],density:0.012} });
  world.spawn({name:"floor",position:[0,-0.2,0],scale:[12,0.2,10],geometry:{kind:"cube"},
    material:{base_color_factor:[0.055,0.07,0.1,1],roughness_factor:0.5,metallic_factor:0.45}});
  world.spawn({name:"plinth",position:[0,0.1,0],scale:[1.7,0.4,1.7],geometry:{kind:"cube"},
    material:{base_color_factor:[0.1,0.12,0.16,1],roughness_factor:0.3,metallic_factor:0.7}});
  const hero = world.importModel("DamagedHelmet.glb", {pbr:true}).roots[0] as Entity;
  hero.position=[0,1.2,0];hero.scale=[0.9,0.9,0.9];

  fire=vfx.play({parameters:{wind:{kind:"vector",value:[0.15,0,0]}},emitters:[
    {id:"fire",target:"gpu",capacity:4096,seed:12,
      spawn:{rate:350},initialize:{shape:{kind:"sphere",radius:0.22,surface:false},lifetime:[0.6,1.3],speed:[0.6,1.4],size:[0.4,0.5],color:[5,1.7,0.2,0.8]},
      update:[{op:"acceleration",value:[0,0.8,0]},{op:"acceleration",value:{parameter:"wind"}},
        {op:"curl_noise",strength:0.6,frequency:1.7},{op:"drag",coefficient:0.4},
        {op:"color_over_life",curve:colorCurve([5,1.6,0.15,0.8],[0.8,0.06,0.005,0])},
        {op:"size_over_life",curve:sizeCurve(0.55,0.05)}],render:flame},
    {id:"smoke",capacity:256,seed:14,spawn:{rate:30},
      initialize:{shape:{kind:"sphere",radius:0.15,surface:false},lifetime:[2,4],speed:[0.6,0.9],size:[0.3,0.3],color:[0.15,0.18,0.22,0.2]},
      update:[{op:"acceleration",value:{parameter:"wind"}},{op:"curl_noise",strength:0.2,frequency:1},
        {op:"color_over_life",curve:colorCurve([0.12,0.15,0.18,0.23],[0.06,0.08,0.12,0])},
        {op:"size_over_life",curve:sizeCurve(0.3,1.6)}],render:{...flame,blend:"alpha",fps:5}},
  ]},{name:"GPU fire + CPU smoke",position:[-2,0.15,0]});

  // CPU collision events drive the engine's deferred, bounded child emitters.
  vfx.play({emitters:[
    {id:"shards",capacity:128,seed:6,spawn:{rate:10},
      initialize:{shape:{kind:"box",half_extent:[0.5,0.2,0.5]},lifetime:[3,4],speed:[2,4],direction:[0,1,0],spread_degrees:30,size:[0.07,0.07],color:[2,0.8,0.1,1]},
      update:[{op:"acceleration",value:[0,-5,0]},{op:"rotate",radians_per_second:2},
        {op:"collision_plane",normal:[0,1,0],offset:0.02,restitution:0.45,friction:0.1,kill:false}],
      events:[{event:"collision",emitter:"impact",count:8,inherit_velocity:0.15}],render:{kind:"mesh",mesh:{kind:"cube"},blend:"additive"}},
    {id:"impact",capacity:512,seed:9,spawn:{rate:0,auto_start:false},
      initialize:{lifetime:[0.15,0.45],speed:[0.4,2],direction:[0,1,0],spread_degrees:80,size:[0.035,0.035],color:[8,1,0.1,1]},
      update:[{op:"acceleration",value:[0,-3,0]},{op:"color_over_life",curve:colorCurve([8,1,0.1,1],[1,0.02,0,0])}],render:{kind:"sprite",blend:"additive",facing:{kind:"velocity"}}},
  ]},{name:"collision sub emitters",position:[2,0.5,0]});

  trail=vfx.play({emitters:[{id:"orbit-trail",capacity:256,seed:22,spawn:{rate:90},
    initialize:{lifetime:[1.6,1.6],speed:[0,0],color:[0.02,1.2,4,0.8],ribbon_id:1},
    update:[{op:"color_over_life",curve:colorCurve([0.02,1.2,4,0.8],[0,0.03,0.2,0])}],
    render:{kind:"ribbon",width:0.065,max_segments:255,uv_scale:2,blend:"additive"}}]},
    {name:"world space trail",position:[2.5,2.2,0]});
  input.bind("pauseFx",["q"]);input.bind("restartFx",["e"]);
  engine.log("VFX lab: GPU fire/flipbook, CPU smoke/soft alpha, collision child sparks, instanced mesh, ribbon; Q pause / E restart.");
}
let paused=false;
function onUpdate(dtMs: number): void {
  time+=dtMs/1000;
  trail.position=[Math.cos(time*1.8)*2.5,2.2+Math.sin(time*2.4)*0.35,Math.sin(time*1.8)*2.5];
  vfx.setParameter(fire,"wind",[Math.sin(time)*0.2,0,0.08]);
  if(input.pressed("pauseFx")){paused=!paused;vfx.pause(fire,paused);}
  if(input.pressed("restartFx"))vfx.restart(fire);
}
