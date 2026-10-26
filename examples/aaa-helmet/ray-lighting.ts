// Public Khronos DamagedHelmet asset, rendered by CrySirror only.
// The three panels use the same metal with different roughness values.
function onStart(): void {
  world.setEnvironment({
    hdr: "royal_esplanade_2k.hdr", intensity: 0.35, exposure: 0,
    tonemap: "aces", temporal_aa: false, bloom: 0.02,
    sun: {direction: [0.7, -1, 0.4], intensity: 2, shadows: true},
    reflections: {ray_traced: true, max_distance: 30, intensity: 1, max_roughness: 1},
    ray_traced_gi: {samples: 8, max_distance: 30, intensity: 1},
  });
  world.setCamera({eye: [4.2, 3.2, 6], target: [0, -0.1, 0], fov: 48, near: 0.1, far: 60});
  world.importModel("DamagedHelmet.glb", {pbr: true});
  for (let i = 0; i < 3; i++) {
    world.spawn({name: `roughness-${i}`, geometry: "cube",
      position: [(i - 1) * 2.5, -1.15, 0], scale: [2.4, 0.15, 7],
      material: {base_color_factor: [0.7, 0.72, 0.78, 1], metallic_factor: 0.9,
        roughness_factor: [0.04, 0.3, 0.65][i]}});
  }
  world.spawn({name: "local-red", position: [-2, 1.5, 0],
    light: {color: [1, 0.15, 0.06], intensity: 35, range: 12, shadows: true}});
}
