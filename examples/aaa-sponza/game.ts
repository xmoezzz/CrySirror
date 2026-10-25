// Sponza atrium lit by a low sun with PCF shadows and HDR sky IBL,
// in the style of the Filament/Bevy Sponza showcases.
let t = 0;

function onStart(): void {
  world.setEnvironment({
    hdr: "venice_sunset_2k.hdr",
    intensity: 0.35,
    rotation_degrees: 90,
    exposure: 0.0,
    tonemap: "aces",
    bloom: 0.03,
    sun: { direction: [0.35, -1.0, 0.25], color: [1.0, 0.86, 0.68], intensity: 9.0, shadows: true },
  });
  const sponza = world.importModel("sponza/Sponza.gltf", { pbr: true });
  engine.log(`sponza: nodes=${sponza.report.nodes} primitives=${sponza.report.primitives} dropped=${JSON.stringify(sponza.report.dropped)}`);
  place(0);
}

function place(s: number): void {
  // Slow dolly along the nave.
  const x = -9.5 + 4.0 * Math.sin(s * 0.15);
  world.setCamera({ eye: [x, 1.8, -0.4], target: [x + 8, 3.2, 0.6], fov: 60, near: 0.05, far: 100 });
}

function onUpdate(dtMs: number): void {
  t += dtMs / 1000;
  place(t);
}
