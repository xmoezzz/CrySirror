// SPDX-License-Identifier: MPL-2.0
// Resident 100k-particle GPU path. No CPU particle simulation or live readback.
function onStart(): void {
  world.setCamera({eye:[0,0.6,6],target:[0,0,0],fov:50,near:0.1,far:30});
  world.setEnvironment({show_sky:false,sun:null,temporal_aa:false,bloom:0.03});
  vfx.play({emitters:[{id:"resident-100k",target:"gpu",capacity:100000,seed:91,
    spawn:{rate:0,bursts:[{time:0,count:100000}],looping:false},
    initialize:{shape:{kind:"sphere",radius:2,surface:false},lifetime:[20,20],speed:[0.01,0.06],direction:[0,1,0],spread_degrees:180,size:[0.009,0.009],color:[0.04,0.3,0.8,0.65]},
    update:[{op:"curl_noise",strength:0.3,frequency:1.8},{op:"drag",coefficient:0.5},{op:"orbit",axis:[0,1,0],speed:0.35}],
    render:{kind:"sprite",blend:"additive"}}]},{name:"GPU resident 100k"});
}
