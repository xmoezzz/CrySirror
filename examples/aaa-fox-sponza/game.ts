// Third-person locomotion slice: the systems a Tomb Raider-style game
// needs (ADR 0011/0012): skinned animation with cross-fades, a kinematic
// capsule character with autostep/slopes, gravity and jumping, an orbit
// camera that avoids walls by raycasting, input action maps, and PBR
// lighting with cascaded shadows, SSAO and TAA.
//
// Controls: WASD move (camera-relative), Shift run, Space jump,
// Q/E or right-mouse drag orbit. With no input an autopilot runs the nave.

const WALK = 1.6, RUN = 5.0, GRAVITY = 18.0, JUMP = 6.5;
let player: Entity, fox: Entity;
let vy = 0, yaw = 90, camYaw = 90, camPitch = 14, camDist = 3.6;
let gait = "";
let t = 0, touched = false;

function onStart(): void {
  input.bind("forward", ["w", "up"]);
  input.bind("back", ["s", "down"]);
  input.bind("left", ["a", "left"]);
  input.bind("right", ["d", "right"]);
  input.bind("run", ["shift"]);
  input.bind("jump", ["space"]);
  input.bind("orbitLeft", ["q"]);
  input.bind("orbitRight", ["e"]);
  input.bind("look", ["mouse_right"]);

  world.setEnvironment({
    hdr: "venice_sunset_2k.hdr", intensity: 0.35, rotation_degrees: 90, tonemap: "aces", bloom: 0.03,
    ambient_occlusion: 1.0, temporal_aa: true, shadow_distance: 40,
    sun: { direction: [0.35, -1.0, 0.25], color: [1.0, 0.86, 0.68], intensity: 9.0, shadows: true },
  });

  const level = world.importModel("sponza/Sponza.gltf", { pbr: true });
  for (const r of level.roots as Entity[]) r.setCollider({ shape: { kind: "mesh" } });

  player = world.spawn({
    name: "player", position: [-9.5, 1.0, -0.4],
    collider: { shape: { kind: "capsule", radius: 0.3, half_height: 0.15 }, body: "character" },
  });
  fox = world.importModel("Fox.glb", { asScene: true, parent: player }).roots[0] as Entity;
  fox.position = [0, -0.45, 0];
  fox.scale = [0.01, 0.01, 0.01];
  fox.play("Survey");
  engine.log(`fox clips: ${JSON.stringify(world.modelInfo("Fox.glb").clips.map((c: any) => c.name))}`);
}

function wrap(a: number): number {
  while (a > 180) a -= 360;
  while (a < -180) a += 360;
  return a;
}

function onFixedUpdate(stepMs: number): void {
  const dt = stepMs / 1000;
  t += dt;
  let fwd = input.axis("back", "forward");
  let side = input.axis("left", "right");
  let running = input.isDown("run");
  if (fwd !== 0 || side !== 0 || input.isDown("jump") || input.isDown("look")) touched = true;
  if (!touched) {
    // Autopilot: idle, walk, then run down the nave and turn back.
    const phase = t % 14;
    fwd = phase < 1.5 ? 0 : 1;
    running = phase > 5;
    camYaw = 90 + (phase > 9.5 ? 180 : 0);
  }
  camYaw += (input.axis("orbitLeft", "orbitRight") * 90 + (input.isDown("look") ? input.pointerDelta[0] * 12 : 0)) * dt;

  // Camera-relative desired direction on the XZ plane.
  const cy = camYaw * Math.PI / 180;
  const fx = Math.sin(cy), fz = Math.cos(cy);
  let dx = fx * fwd + fz * side, dz = fz * fwd - fx * side;
  const len = Math.hypot(dx, dz);
  const speed = len > 0 ? (running ? RUN : WALK) : 0;
  if (len > 0) {
    dx /= len; dz /= len;
    const target = Math.atan2(dx, dz) * 180 / Math.PI;
    yaw += wrap(target - yaw) * Math.min(1, dt * 10);
  }

  const before = player.position;
  vy -= GRAVITY * dt;
  const moved = player.moveCharacter([dx * speed * dt, vy * dt, dz * speed * dt], dt, { step: 0.35, maxSlope: 50 });
  if (moved.grounded) {
    vy = input.pressed("jump") ? JUMP : -1;
  }
  player.rotation = [0, yaw, 0];

  const planar = Math.hypot(moved.translation[0], moved.translation[2]) / dt;
  const next = !moved.grounded ? gait : planar > 3 ? "Run" : planar > 0.3 ? "Walk" : "Survey";
  if (next !== gait) {
    fox.play(next, { fade: 0.25, speed: next === "Run" ? 1.1 : next === "Walk" ? 1.6 : 1 });
    gait = next;
  }
  if (player.position[1] < -20) player.position = [-9.5, 2, -0.4];
  void before;
}

function onLateUpdate(): void {
  // Orbit camera behind the player; pull in when a wall is in the way.
  const p = player.position;
  const head: Vec3 = [p[0], p[1] + 0.6, p[2]];
  const cy = camYaw * Math.PI / 180, cp = camPitch * Math.PI / 180;
  const dir: Vec3 = [-Math.sin(cy) * Math.cos(cp), Math.sin(cp), -Math.cos(cy) * Math.cos(cp)];
  const hit = world.raycast(head, dir, camDist, player);
  const d = hit ? Math.max(0.6, hit.distance - 0.25) : camDist;
  world.setCamera({
    eye: [head[0] + dir[0] * d, head[1] + dir[1] * d, head[2] + dir[2] * d],
    target: head, fov: 60, near: 0.05, far: 200,
  });
}
