// Khronos Sponza showcase through the engine's asynchronous level service.
// Downloaded resources and attribution remain in ignored assets/.
let time = 0;
function onStart(): void {
  world.setEnvironment({hdr: "venice_sunset_2k.hdr", intensity: 0.35, rotation_degrees: 90,
    temporal_aa: false, exposure: 0, bloom: 0.03,
    sun: {direction: [0.35,-1,0.25], color: [1,0.86,0.68], intensity: 9, shadows: true}});
  world.streamLevel("sponza", {source: "sponza/Sponza.gltf", center: [0,0,0], load_radius: 100, unload_radius: 120, pbr: true});
  camera();
}
function camera(): void {
  const x = -9.5 + Math.sin(time * 0.15);
  world.setCamera({eye: [x,1.8,-0.4], target: [x+8,3.2,0.6], fov: 60, near: 0.05, far: 100});
}
function onUpdate(dtMs: number): void {time += dtMs/1000;camera();}
