// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
async function onStart(): Promise<void> {
    const original = bitmap.create(1,160,160,[20,35,65,255]);
    for(let y=0;y<8;y++) for(let x=0;x<8;x++)
        bitmap.fill(1,{x:x*20,y:y*20,width:20,height:20},
            (x+y)%2===0 ? [220,60+x*8,35+y*14,255] : [20+x*10,80+y*12,180,255]);
    const titles=["Original","Gamma / RGB","Gaussian blur","Vector path",
                  "Perspective","Row distortion","Mask transition","Blur transition"];
    const slots=[1,2,3,4,5,6,7,9];
    for(const slot of [2,3,5,6,7,9,12]) bitmap.load(slot,original);
    await bitmap.applyEffectAsync(2,[{kind:'gamma',gamma:[2,0.8,1.3],floor:[0,0,0],ceiling:[255,255,255]}]);
    await bitmap.applyEffectAsync(3,[{kind:'gaussian_blur',sigma:4}]);
    bitmap.create(4,160,160,[20,35,65,255]);
    await bitmap.applyEffectAsync(4,[{kind:'vector',path:{
        commands:[{kind:'move',to:[20,135]},{kind:'cubic',control1:[20,5],control2:[140,5],to:[140,135]},
                  {kind:'quad',control:[80,90],to:[20,135]},{kind:'close'}],
        fill:[45,210,180,230],stroke:{color:[255,220,90,255],width:5},
        transform:[1,0,0,1,0,0],even_odd:false}}]);
    await bitmap.applyEffectAsync(5,[{kind:'perspective',quad:[[20,0],[140,20],[160,145],[0,160]]}]);
    await bitmap.applyEffectAsync(6,[{kind:'raster',amplitude:12,frequency:0.1,phase:0.8}]);
    await bitmap.applyEffectAsync(12,[{kind:'invert'}]);
    bitmap.create(8,160,160,[0,0,0,255]);
    for(let x=0;x<8;x++) { const v=Math.floor(x*255/7); bitmap.fill(8,{x:x*20,y:0,width:20,height:160},[v,v,v,255]); }
    await bitmap.transitionAsync(7,12,{progress:0.55,softness:0.12,blur:0},8);
    await bitmap.transitionAsync(9,12,{progress:0.5,softness:0,blur:10});
    for(let i=0;i<slots.length;i++) {
        const x=20+(i%4)*190, y=i<4?80:300;
        layers.put({id:100+i,rect:[x,y,160,160],paint:{kind:'image',asset:`__surface/${slots[i]}`}});
        layers.put({id:200+i,rect:[x,y+170,180,28],paint:{kind:'text',font:'default',size:17,text:titles[i],color:[225,232,245,255]}});
    }
    layers.put({id:1,rect:[20,12,740,34],paint:{kind:'text',font:'default',size:26,text:'CrySirror / shared image pipeline',color:[245,248,255,255]}});
    layers.put({id:2,rect:[20,47,740,24],paint:{kind:'text',font:'default',size:15,text:'Worker processing > revision-checked resources > engine renderer',color:[140,170,210,255]}});
    engine.log('shared-effects-showcase-ready');
}
