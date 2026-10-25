# Physics

CrySirror uses **Rapier 3D 0.22.0** for rigid-body simulation. CrySirror owns
scene declarations, lifecycle, scoped handles and APIs; Rapier owns collision
detection, integration, contact and joint solving, CCD, sleeping and character
sweeps. See [ADR 0045](adr/0045-rapier-shared-physics.md) for the exact backend,
validation and [measured workload](physics-benchmark-results.json).

The shared physics service lives in Runtime. JSON-only projects run physics
without a script, App, window, renderer or audio device. World's fixed-step
callback advances Rapier once per 16 ms step and writes poses/velocities back
before the next substep. The ordinary World integrator skips physics-owned
bodies. Worlds and queries are client isolated; a native VM adapter can use the
same Runtime/scene service without carrying a separate physics engine.

## Data and API

Declare a collider on an entity:

```json
{
  "id": 1,
  "name": "falling box",
  "transform": {"translation": [0, 4, 0]},
  "collider": {
    "body": "dynamic",
    "shape": {"kind": "box", "half_extents": [0.5, 0.5, 0.5]},
    "mass": 2,
    "friction": 0.6,
    "ccd": true
  }
}
```

`ClientSpec.systems.physics` accepts `ClientPhysicsScene` data: gravity,
persistent forces/torques, position-kinematic targets and joint declarations.
Entity identities in these declarations carry generations. Collider settings
include density/total mass, restitution, damping, gravity scaling, enabled and
sensor states, collision membership/filter bitmasks and translation/rotation
locks. The generated [API reference](api/REFERENCE.md) gives fields and defaults.

The same service is exposed to Rust, JS/TS and tools as `Physics` (global
`physics`) and `PhysicsJoint` classes:

```ts
physics.gravity = [0, -9.81, 0];
const box = world.spawn({
  name: "box", position: [0, 4, 0],
  collider: { body: "dynamic", shape: { kind: "box", half_extents: [0.5, 0.5, 0.5] }, mass: 2 }
});
box.applyImpulse([2, 0, 0]);
const hit = physics.raycast([0, 10, 0], [0, -1, 0], 30);
const events = physics.events();
```

Linear velocity uses world units per second; scene/API angular velocity uses
degrees per second. Collider size uses entity-local units and positive scale.
Joint local frames use degrees; revolute motor/limit coordinates use radians,
and prismatic coordinates use meters. Persistent forces continue until
`clearForces`; an impulse changes velocity once. Position-kinematic targets
advance on the next fixed step and remain saved declarations. Use static
triangle/terrain meshes for level collision and convex hulls for dynamic props.

## Boundaries

Body/collider handles and cached shapes persist across unchanged scene sync.
The current bridge still traverses scene entities when the world revision
changes; backend-only timings do not establish whole-engine frame cost.
Positive nonuniform box/mesh scale is supported. Nonuniform sphere/capsule,
reflection, perspective, shear and dynamic concave meshes fail explicitly.
Automatic convex decomposition and compound-collider scene authoring remain
future work, although the backend supports multiple colliders per body.

Logical snapshots preserve declarations, poses, velocities, forces, targets and
joints. They rebuild Rapier state, so solver/contact warm starts and exact
cross-platform numerical continuation are not guaranteed. Event queues are
bounded; overflow counters are visible through `physics.stats`.

This provides a rigid-body foundation. Cloth, soft bodies, fluids, fracture,
ragdoll tooling, networking/rollback and physics debug rendering remain separate
work. It does not establish mature physics performance for a commercial level.
