// Port of three.js examples/webgl_loader_gltf.html to CrySirror.
//
// three.js                                   CrySirror
// camera = PerspectiveCamera(45, ..., 0.25, 20)   world.setCamera({fov: 45, near: 0.25, far: 20})
// camera.position.set(-1.8, 0.6, 2.7)             eye: [-1.8, 0.6, 2.7]
// controls.target.set(0, 0, -0.2)                 target: [0, 0, -0.2]
// RGBELoader('royal_esplanade_1k.hdr')            hdr: "royal_esplanade_2k.hdr"
// scene.background = scene.environment = texture  show_sky: true (IBL + background)
// renderer.toneMapping = ACESFilmicToneMapping    tonemap: "aces", exposure: 0 EV (= 1.0)
// GLTFLoader('DamagedHelmet.gltf')                world.importModel(..., {pbr: true})
//
// The original has no direct light; neither do we (IBL only).
let helmet: Entity | null = null;
let t = 0;
const orbit = 0.0; // radians per second; three.js example is static

function onStart(): void {
  world.setEnvironment({
    hdr: "royal_esplanade_2k.hdr",
    intensity: 1.0,
    exposure: 0.0,
    tonemap: "aces",
    show_sky: true,
    bloom: 0.02,
    sun: null,
  });
  world.setCamera({ eye: [-1.8, 0.6, 2.7], target: [0, 0, -0.2], fov: 45, near: 0.25, far: 20 });
  const imported = world.importModel("DamagedHelmet.glb", { pbr: true });
  helmet = imported.roots[0] as Entity;
  engine.log(`helmet: ${JSON.stringify(imported.report)}`);
}

function onUpdate(dtMs: number): void {
  if (orbit === 0) return;
  t += (dtMs / 1000) * orbit;
  const r = Math.hypot(-1.8, 2.7);
  world.setCamera({ eye: [Math.sin(t) * r, 0.6, Math.cos(t) * r - 0.2], target: [0, 0, -0.2], fov: 45, near: 0.25, far: 20 });
}
