# 0043: Arcade vehicle controller

Date: 2026-10-25. Status: implemented and validated. The vehicle state is
not saved yet.

## Context

ADR 0041 listed vehicles as missing. The NFS Carbon canyon (ADR 0042) needs
cars that hold the road, slide under the handbrake and stop at barriers.
NFS-style handling is arcade: predictable grip and drift matter more than a
tyre model. Rapier offers a raycast vehicle for dynamic bodies, but the
engine has no dynamic bodies yet. A kinematic controller on the existing
character sweep is deterministic and needs no solver.

## Decision

`crysirror-engine::vehicle` with `Entity.setVehicle(spec)` and
`Entity.drive(input, dt)`:
- **Suspension.** Four wheel rays, at ±wheelbase/2 and ±track/2 around the
  body, are cast from just above it. With at least two contacts, the body
  rides at `ride_height` above the mean contact, and pitch and roll follow
  the contact plane. Otherwise gravity applies.
- **Drive.**
  - Throttle acceleration falls off toward `max_speed`.
  - Reverse input brakes before it reverses.
  - Service brake and linear drag apply.
- **Steering.** A bicycle model: yaw rate = v / wheelbase · tan δ. The
  maximum wheel angle shrinks with speed (`steer_falloff`). Positive steer
  turns toward +X of the heading (yaw 0 faces +Z).
- **Grip and drift.** The world velocity is kept through the turn, and only
  its sideways component is damped, by `exp(−grip·dt)`. The handbrake uses
  the much lower `handbrake_grip`, so drifts come out of the model rather
  than being scripted.
- **Collision.** The body moves through the existing character sweep, with
  step and slope limits. When it is blocked, the velocity keeps only the
  motion that actually happened, so walls absorb speed.
- The entity transform gets the position and an XYZ Euler rotation from
  yaw, pitch and roll. The state (velocity, yaw, pitch, roll, grounded)
  lives in `EngineState.vehicles` and is **not saved**; that is the next
  step.
- **Tool.** `tools/road_centerline.py` gives AI drivers a road centreline
  from glTF road meshes:
  - top-down rasterization with heights, so stacked roads stay apart;
  - an edge-distance transform;
  - a centring-cost Dijkstra to the geodesically farthest cell.

## Validation (2026-10-25)

- Engine
  `tests/physics.rs::arcade_vehicle_accelerates_steers_drifts_climbs_and_stops_at_walls`:
  - **Acceleration:** above 20 units/s after 3 s, straight along +X at ride
    height (±0.05).
  - **Wall:** the car stops in front of the wall at x = 149, with speed
    below 5.
  - **Steering:** positive steer raises yaw by more than 20° within 1 s.
  - **Drift:** a handbrake turn's lateral speed is at least 3× that of a
    gripping turn, and above 3.
  - **Ramp:** on a 10° ramp the body's nose-up pitch is 10 ± 1.5°.
- Python `tests/test_road_centerline.py`: on a synthetic L-shaped road the
  route stays within 1.5 m of both strip centres, reaches the far end and
  covers every road cell.
- **Real content: the NFS Carbon TNDRIFTB canyon.**
  - The centreline over the 53 decoded road chops is 4.2 km long, with a
    mean edge distance of 9.1 m.
  - Pure pursuit with bend slowdown drove from behind the "road closed"
    barrier down the mountain: 1,516 path points (about 3 km) in 142 s at
    about 24 m/s.
  - It was grounded on every frame and off the road in 0 of 8,880 frames
    (off road means more than 9 m from the centreline or more than 3 m off
    its height).
  - Movie: `images/third-party/nfsc/canyon-drive.mp4` (gitignored).
  - The car body is a box stand-in until NFS car geometry is decoded.
- An earlier centreline, made from chop centroids chained by nearest
  neighbour, folded back on itself and drove the car into a guard rail.
  That is why the rasterized Dijkstra route replaced it.

## Gaps

- Saving vehicle state.
- Dynamic bodies and car-to-car contact.
- Tyre and surface types from materials.
- Wheels as separate visuals that spin and steer.
- Audio (engine RPM).
- Dynamic bodies would also allow a Rapier raycast-vehicle variant.
