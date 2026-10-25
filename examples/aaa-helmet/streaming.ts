// Public Khronos DamagedHelmet sample used as an asynchronous streaming cell.
// Reuse the separately downloaded assets/ and its THIRD_PARTY.json attribution.
let elapsed = 0;
function camera(): void {
  const phase = elapsed % 6000;
  const near = (phase >= 1000 && phase < 3000) || phase >= 4000;
  world.setCamera({eye: near ? [-2.8, 1.2, 4.8] : [0, 3, 20], target: [0, 0, 0], fov: 45, near: 0.1, far: 100});
}
function onStart(): void {
  world.setEnvironment({hdr: "royal_esplanade_2k.hdr", intensity: 0.5, temporal_aa: false, bloom: 0.02,
    sun: {direction: [0.7, -1, 0.4], intensity: 4, shadows: true}});
  world.spawn({name: "floor", geometry: "cube", position: [0,-1.1,0], scale: [12,0.2,12], color: [160,165,175,255]});
  world.streamLevel("helmet-cell", {source: "DamagedHelmet.glb", center: [0,0,0], load_radius: 8, unload_radius: 12, pbr: true});
  camera();
}
function onUpdate(dtMs: number): void {elapsed += dtMs; camera();}
