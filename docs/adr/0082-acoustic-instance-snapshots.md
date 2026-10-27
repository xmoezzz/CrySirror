# ADR 0082: reusable acoustic mesh BVHs and moving instance snapshots

Date: 2026-10-27

Status: implemented profile; local validation is recorded separately in STATUS.md.

## Context and reference

The acoustic scene previously flattened all transformed triangles into one BVH.
Moving a door required preparing a complete replacement scene. The public
[Steam Audio scene contract](https://valvesoftware.github.io/steam-audio/doc/capi/scene.html)
distinguishes static geometry, transformed instances and batched scene commit.
This is a useful capability boundary, not a claim that CrySirror embeds that SDK.

We retain the existing Parry 0.17.6 / optional obvhs 0.4.0 dependencies. Exact
registry archive checksums and reviewed file hashes are in
[acoustic-instances-source-audit.json](../acoustic-instances-source-audit.json).
No BVH builder/refitter or Steam Audio implementation is copied into CrySirror.

## Geometry ownership

Each declared acoustic mesh now owns an immutable local triangle BVH. Scene
snapshots share those resources through Arc. Parry Qbvh builds the world-space
instance index in both modes; the selected Parry/obvhs backend applies to local
mesh queries. Moving a mesh transforms only its world reflection-surface page
and bounds, retains its local BVH, and rebuilds the top-level instance index.
Unchanged meshes also share their world surface pages. This is TLAS rebuild,
not incremental refit or cross-ID prototype instancing.

Queries transform the ray into local coordinates and convert local distance
back to normalized world distance. Nonuniform/negative scale and affine shear
remain valid. Normals derive from transformed triangle winding. World instance
bounds receive coordinate-scaled conservative padding. Whole ignored meshes
are excluded at the instance level, replacing repeated ignored-surface traversal.
Triangles keep their original scene offsets when meshes are disabled, so
reflection IDs and complete-scene budgets stay stable.

AcousticMeshUpdate contains an existing ID and optional transform/enabled flag.
Unknown/duplicate IDs, empty batches/changes, invalid affine transforms and
geometry outside finite +/-1e6 local/world coordinates fail visibly. Existing
4096 mesh, million-vertex and 262144-triangle limits remain. Disabled meshes
retain their memory and count against these limits. Updates change neither
topology nor material tables; prepare a new scene for those changes.

## Tasks, publication and lifetimes

- Audio.beginUpdateAcousticScene / updateAcousticsAsync validates and admits an
  atomic batch over the calling client's prepared scene. A pending scene commit
  must complete or be canceled first. Entity-bound meshes reject manual edits
  until explicitly unbound. The owner captures an immutable base revision and
  reserves staging bytes; a bounded CPU preparation worker owns all rebuilding.
- Existing operation tickets expose pending/failed/ready; ready follows owner
  publication after validating the base revision. Failure/cancellation retains
  the preceding scene. Explicit probes retain their captured immutable geometry,
  including through an update or clear. Releasing a published receipt leaves
  the scene installed; releasing a pending one publishes nothing.
- Initial scene builds reserve 128 MiB and shrink to a conservative estimate
  including 384 bytes/triangle, 16/source vertex and 512/mesh plus 8 KiB. Updates
  reserve 128 MiB and retain the preceding scene's conservative estimate. Shared
  local allocations may therefore be charged more than once across snapshots.
  The shared 512 MiB limit includes live staging and retained scene snapshots.
  These are admission reservations, not allocator/VRAM measurements.
- Cancellation is cooperative between mesh/phase boundaries. Library BVH builds
  and one mesh's surface transform are bounded non-preemptible phases. No owner
  wait, borrowed World/mixer, nested Rayon pool or device is used by the worker.

## World bindings and automatic coalescing

Audio.bindAcousticMesh accepts AcousticMeshBindingSpec: mesh, entity,
local_transform (identity) and follow_visibility (false). Scripts supply their
ordinary Entity object; Rust may supply its scoped generation handle. Engine
object resolution preserves the actual client before validating World identity.
The existing audio follow-entity path now uses that same validated resolver,
preventing a foreign object's ID from being reinterpreted in the current client.

After World/late updates, Audio captures global entity matrix times the explicit
offset. Optional visibility follows ancestors. At most 256 bindings/client and
1024/host share one automatic commit/client, with 50 ms minimum submission
interval. New poses coalesce while a captured update runs; they do not repeatedly
cancel that update. Pool/result/memory pressure stays pending with observable
waiting state. Repeated identical failed poses wait for a change/rebind instead
of continuously rebuilding. Automatic receipts release on publication/failure.

A removed entity requests mesh disable and retains a binding diagnostic. Invalid
poses retain the previous mesh and report their error. Unbind cancels a pending
automatic client batch and leaves the last published pose; other bindings
recapture next frame. Successful full scene replacement, clear and restore remove
bindings. Failed full replacement retains the old scene and its bindings.

acousticStats exposes work counters, bindings, desired/submitted poses, engine
clock/publication age, pending/waiting/error and completed updates. These are
derived engine resources, not project/native save pointers or serialized BVHs.

## Geometry age and effect completion

Every commit has a new revision. A separate scene_generation changes only on
full scene replacement. Direct queries and automatically tracked reflections
may publish a completed older instance revision within the same generation,
then coalesce the latest snapshot. This avoids starvation when geometry keeps
moving faster than a worker can finish. acousticVoice and reflection tracking
report current_revision / geometry_stale; tracked snapshots also expose capture
age. There is no claimed maximum age, deadline or motion extrapolation.

Full scene replacement/clear remains a strict generation boundary: old work is
canceled and old tracked reflection paths are suppressed. Direct HRTF latency
changes remain strict too. Explicit static voice reflections retain their exact
captured revision contract. Instance-tracked plans preserve sample history and
crossfade old/new taps; completion is not the end of the audible transition.
Direct coefficients retain their existing smoothing and source clock.

## Validation and remaining work

Local release tests cover both BVH backends, analytic affine distance/normal
oracles, shared allocation identity, stable disabled IDs, immutable old probes,
worker gates, coalescing, parent/visibility/despawn, cancellation/unbind/restore,
client object ownership, actual PCM occlusion and tracked echo changes. Slow-work
tests require completed older geometry snapshots to publish and then converge.
Real JS Promise/Entity paths and an authored moving-door WAV example use the
same shared APIs. Exact results are in STATUS.md; ignored tests are excluded.

Remaining work includes vertex deformation/skinning, sharing a prototype among
different mesh IDs, top-level refit/quality maintenance, model/collider acoustic
asset extraction, automatic material changes, geometry-age quality policy,
Doppler/diffraction and diffuse/higher-order sound. This is CPU geometry; it does
not add audio hardware ray tracing, complete Steam Audio parity, new native VM
opcodes or their HostServices facade, a standalone device render clock or the
whole ADR 0050 migration.
