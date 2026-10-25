// CrySirror engine API demo: a TypeScript behaviour driving the shared World.
// Types come from the engine class registry (see examples/engine/crysirror.d.ts).
const lanterns: Entity[] = [];
let floor: Entity | null = null;

function onStart(): void {
  floor = world.spawn({ name: "floor", geometry: "cube", position: [0, -0.05, 0], scale: [9, 0.1, 9], color: [70, 78, 92, 255] });
  for (let i = 0; i < 3; i++) {
    const anchor = world.spawn({ name: `anchor${i}`, position: [(i - 1) * 2.4, 0, 0], rotation: [0, i * 30, 0] });
    const imported = world.importModel("lantern.glb", { parent: anchor, prefix: `lantern${i}:` });
    if (imported.report.dropped.length > 0) engine.log(`dropped: ${imported.report.dropped}`);
    lanterns.push(anchor);
  }
  engine.log(`world has ${world.count} entities`);
}

function onFixedUpdate(stepMs: number): void {
  lanterns.forEach((l, i) => {
    const r = l.rotation;
    l.rotation = [r[0], r[1] + stepMs * 0.06 * (i + 1), r[2]];
  });
}
