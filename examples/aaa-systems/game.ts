// Systems showcase (ADR 0014): terrain + heightfield collision, particles,
// gameplay abilities/effects/attributes, animation state machines and
// montages, behaviour-tree AI on a baked nav grid, level streaming and a
// layer HUD. Controls: WASD, Shift run, Space attack (bite), Q/E orbit.
// With no input, an autopilot walks toward the bonfire.

const TERRAIN = { kind: "terrain", resolution: 161, size: [120, 120], height: 14, seed: 11, frequency: 2.2 };
let player: Entity, hunters: Entity[] = [];
let vy = 0, camYaw = 200, t = 0, touched = false;

function ground(x: number, z: number): number {
  const hit = world.raycast([x, 60, z], [0, -1, 0], 120, null);
  return hit ? hit.point[1] : 0;
}

function spawnFox(name: string, x: number, z: number, team: string): Entity {
  const body = world.spawn({ name, position: [x, ground(x, z) + 1.0, z],
    collider: { shape: { kind: "capsule", radius: 0.35, half_height: 0.15 }, body: "character" } });
  const fox = world.importModel("Fox.glb", { asScene: true, parent: body, prefix: name + ":fox" }).roots[0] as Entity;
  fox.position = [0, -0.5, 0];
  fox.scale = [0.012, 0.012, 0.012];
  fox.setStateMachine("FoxLocomotion");
  body.initAttributes({
    Health: { base: 100, min: 0, max_attribute: "MaxHealth" }, MaxHealth: { base: 100 },
    Stamina: { base: 100, min: 0, max: 100 },
  });
  body.addTag(team);
  body.grantAbility("Bite");
  return body;
}

function onStart(): void {
  input.bind("forward", ["w", "up"]); input.bind("back", ["s", "down"]);
  input.bind("left", ["a", "left"]); input.bind("right", ["d", "right"]);
  input.bind("run", ["shift"]); input.bind("attack", ["space"]);
  input.bind("orbitLeft", ["q"]); input.bind("orbitRight", ["e"]);

  world.setEnvironment({ hdr: "venice_sunset_2k.hdr", intensity: 0.6, rotation_degrees: 140, tonemap: "aces",
    exposure: -0.3, bloom: 0.05, ambient_occlusion: 1.0, shadow_distance: 70,
    sun: { direction: [-0.55, -0.6, 0.45], color: [1.0, 0.78, 0.55], intensity: 6, shadows: true } });

  world.spawn({ name: "valley", geometry: TERRAIN, collider: { shape: { kind: "mesh" } } });

  world.defineGameplay({
    effects: {
      BiteCost: { duration: { kind: "instant" }, modifiers: [{ attribute: "Stamina", op: "add", magnitude: -25 }] },
      BiteCooldown: { duration: { kind: "duration", seconds: 0.8 }, grant_tags: ["Cooldown.Bite"] },
      BiteDamage: { duration: { kind: "instant" }, modifiers: [{ attribute: "Health", op: "add", magnitude: -12 }], cue: "Cue.Hit" },
      StaminaRegen: { duration: { kind: "infinite" }, period: 0.25, modifiers: [{ attribute: "Stamina", op: "add", magnitude: 4 }] },
      Warmth: { duration: { kind: "duration", seconds: 2 }, period: 0.5, modifiers: [{ attribute: "Health", op: "add", magnitude: 3 }], grant_tags: ["State.Warm"] },
    },
    abilities: { Bite: { cost: "BiteCost", cooldown: "BiteCooldown", active_tags: ["State.Attacking"], duration: 0.5, cue: "Cue.Bite" } },
  });
  world.defineAnimation({
    machines: { FoxLocomotion: { entry: "Move", parameters: { speed: 0 }, states: {
      Move: { kind: "blend1d", param: "speed", samples: [
        { at: 0, clip: "Survey" }, { at: 1.6, clip: "Walk", speed: 1.6 }, { at: 6, clip: "Run", speed: 1.1 }] } } } },
    montages: { BiteLunge: { clip: "Survey", length: 3.0, rate: 4, blend_in: 0.05, blend_out: 0.1, notifies: [{ time: 0.8, name: "BiteHit" }] } },
  });
  world.defineBehaviorTree("Hunter", { kind: "selector", children: [
    { kind: "sequence", children: [
      { kind: "perceive", tag: "Team.Player", radius: 45, into: "prey" },
      { kind: "selector", children: [
        { kind: "sequence", children: [
          { kind: "near", target: "prey", less_than: 1.8 },
          { kind: "face", target: "prey", degrees_per_second: 540 },
          { kind: "cooldown", seconds: 1.2, child: { kind: "ability", name: "Bite" } } ] },
        { kind: "move_to", target: "prey", speed: 4.2, acceptance: 1.4 } ] } ] },
    { kind: "wait", seconds: 0.5 } ] });

  // Bonfire: flames, embers, smoke.
  const fx = 6, fz = 4, fy = ground(fx, fz);
  world.spawn({ name: "bonfire", position: [fx, fy + 0.2, fz], particles: { rate: 260, lifetime: [0.4, 0.9], speed: [0.8, 2.0], spread_degrees: 18,
    gravity: [0, 2.5, 0], drag: 0.6, size: [0.45, 0.08], color_start: [40, 14, 3, 1], color_end: [6, 1, 0.1, 0], max: 600, seed: 1 } });
  world.spawn({ name: "embers", position: [fx, fy + 0.4, fz], particles: { rate: 40, lifetime: [1.5, 3], speed: [1, 3], spread_degrees: 35,
    gravity: [0.3, 0.6, 0], size: [0.05, 0.02], color_start: [60, 20, 2, 1], color_end: [8, 1, 0, 0], max: 200, seed: 2 } });
  world.spawn({ name: "smoke", position: [fx, fy + 1.2, fz], particles: { rate: 25, lifetime: [3, 5], speed: [0.6, 1.2], spread_degrees: 15,
    gravity: [0.4, 0.3, 0.1], drag: 0.2, size: [0.6, 2.4], color_start: [0.12, 0.11, 0.1, 0.5], color_end: [0.3, 0.3, 0.3, 0], additive: false, max: 160, seed: 3 } });
  world.streamLevel("shrine", { source: "lantern.glb", center: [-20, 0, -18], load_radius: 45, unload_radius: 55,
    transform: { translation: [-20, ground(-20, -18), -18], scale: [2, 2, 2] }, collide: true });

  player = spawnFox("player", 30, 30, "Team.Player");
  player.applyEffect("StaminaRegen", null);
  hunters = [spawnFox("hunter1", -25, 10, "Team.Wild"), spawnFox("hunter2", 0, -30, "Team.Wild")];
  const nav = world.buildNavigation({ min: [-58, -2, -58], max: [58, 30, 58], cell: 0.5, agent_radius: 0.35, max_step: 0.6, max_slope: 50 });
  engine.log(`nav: ${JSON.stringify(nav)}`);
  for (const h of hunters) { h.applyEffect("StaminaRegen", null); h.runBehaviorTree("Hunter"); }

  layers.put({ id: 1, rect: [24, 24, 304, 22], paint: { kind: "solid", color: [10, 10, 12, 200] } });
  layers.put({ id: 2, rect: [26, 26, 300, 18], paint: { kind: "solid", color: [190, 30, 30, 255] } });
  layers.put({ id: 3, rect: [24, 50, 304, 14], paint: { kind: "solid", color: [10, 10, 12, 200] } });
  layers.put({ id: 4, rect: [26, 52, 300, 10], paint: { kind: "solid", color: [60, 170, 70, 255] } });
}

function foxOf(e: Entity): Entity { return e.children()[0]; }

function onFixedUpdate(stepMs: number): void {
  const dt = stepMs / 1000;
  t += dt;
  let fwd = input.axis("back", "forward"), side = input.axis("left", "right");
  let run = input.isDown("run");
  if (fwd || side || input.isDown("attack")) touched = true;
  if (!touched) { fwd = t > 1 ? 1 : 0; run = t > 3; }
  camYaw += input.axis("orbitLeft", "orbitRight") * 90 * dt;
  let yaw = player.rotation[1];
  if (!touched) {
    yaw = Math.atan2(6 - player.position[0], 4 - player.position[2]) * 180 / Math.PI;
    camYaw += ((yaw - camYaw + 540) % 360 - 180) * Math.min(1, dt * 2);
  }
  const cy = camYaw * Math.PI / 180;
  let dx = Math.sin(cy) * fwd + Math.cos(cy) * side, dz = Math.cos(cy) * fwd - Math.sin(cy) * side;
  if (!touched) { dx = Math.sin(yaw * Math.PI / 180) * fwd; dz = Math.cos(yaw * Math.PI / 180) * fwd; }
  const len = Math.hypot(dx, dz);
  const speed = len ? (run ? 6 : 1.8) : 0;
  if (len) { dx /= len; dz /= len; yaw = Math.atan2(dx, dz) * 180 / Math.PI; }
  vy -= 20 * dt;
  const m = player.moveCharacter([dx * speed * dt, vy * dt, dz * speed * dt], dt, { step: 0.6, maxSlope: 50 });
  if (m.grounded) vy = -1;
  player.rotation = [0, yaw, 0];

  if (input.pressed("attack") && player.activate("Bite").ok) foxOf(player).playMontage("BiteLunge", null);

  for (const e of [player, ...hunters]) {
    const fox = foxOf(e);
    const prev = (e as any).__p ?? e.position;
    const p = e.position;
    const v = Math.hypot(p[0] - prev[0], p[2] - prev[2]) / dt;
    (e as any).__p = p;
    fox.setAnimParam("speed", v);
    for (const ev of e.drainEvents()) {
      if (ev.kind === "ability_activated" && e !== player) {
        fox.playMontage("BiteLunge", null);
        const target = e.ai?.blackboard?.prey;
        const victim = [player, ...hunters].find(x => x.id === target);
        if (victim) victim.applyEffect("BiteDamage", e);
      }
    }
  }
  if (Math.hypot(player.position[0] - 6, player.position[2] - 4) < 3 && !player.hasTag("State.Warm")) player.applyEffect("Warmth", null);
}

let nextLog = 0;
function onLateUpdate(): void {
  if (t >= nextLog) {
    nextLog += 3;
    const d = hunters.map(h => Math.hypot(h.position[0] - player.position[0], h.position[2] - player.position[2]).toFixed(1));
    engine.log(`t=${t.toFixed(1)} hp=${player.attribute("Health")?.toFixed(0)} stamina=${player.attribute("Stamina")?.toFixed(0)} hunterDist=${d} hunterAI=${hunters.map(h => h.ai?.status)}`);
  }
  const hp = (player.attribute("Health") ?? 0) / 100, st = (player.attribute("Stamina") ?? 0) / 100;
  layers.put({ id: 2, rect: [26, 26, Math.max(1, 300 * hp), 18], paint: { kind: "solid", color: [190, 30, 30, 255] } });
  layers.put({ id: 4, rect: [26, 52, Math.max(1, 300 * st), 10], paint: { kind: "solid", color: [60, 170, 70, 255] } });
  const p = player.position, cy = camYaw * Math.PI / 180;
  const head: Vec3 = [p[0], p[1] + 0.8, p[2]];
  const dir: Vec3 = [-Math.sin(cy) * 0.94, 0.34, -Math.cos(cy) * 0.94];
  const hit = world.raycast(head, dir, 7, player);
  const d = hit ? Math.max(1, hit.distance - 0.3) : 7;
  world.setCamera({ eye: [head[0] + dir[0] * d, head[1] + dir[1] * d, head[2] + dir[2] * d], target: head, fov: 55, near: 0.05, far: 400 });
}
