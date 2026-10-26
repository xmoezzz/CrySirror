# 0067: Shared acoustic geometry and asynchronous direct sound

Date: 2026-10-27. Status: implemented direct-sound profile; validation results
are recorded separately in STATUS.md. HRTF/reflections/reverb remain pending.

## Decision

Audio propagation is a reusable engine service, independent of native VMs,
physics collider policy and rendering geometry lifetime. A VM may translate
its sound semantics onto Audio; it must not own a second acoustic engine.
The new device-free `crysirror-acoustics` crate validates acoustic mesh/material
resources, owns immutable CPU BVHs, and computes direct-path coefficients.
The existing mixer consumes coefficients; geometry work never runs per sample.

The engine already had Rapier/Parry QBVH occlusion through physics ray queries.
The gap was a distinct acoustic scene, multi-ray volume visibility, frequency
transmission and propagation jobs. We use mature Rust BVH libraries rather than
implement a tree builder. Default Parry shares the existing geometry ecosystem;
`acoustics-obvhs` enables optional obvhs BVH2 in engine/desktop builds. Different
backends share the same validated geometry and query contract. Neither a GPU
nor a native SDK is needed; this is unrelated to renderer hardware-RT support.
Microbenchmarks choose no universal winner; defaults require correctness and
representative workload measurements. Source pins and semantic corrections are
in `crates/crysirror-acoustics/UPSTREAM.md`.

The current model provides per-triangle three-band surface transmission,
exponential air absorption, weighted-dipole directivity, deterministic volume
visibility and physical travel-time metadata. Unit conversion is explicit.
Transmission is amplitude per surface crossing; a two-sided slab incurs two
crossings. Existing mixer distance curves, stereo pan, envelopes, voice limits,
buses and ducking still apply. Low/mid/high recombination is a persistent,
allocation-free pair of one-pole crossovers (800/8000 Hz, clamped to sample rate),
with 10 ms coefficient smoothing. New acoustic voices start muted until their
first valid result; later updates retain the last ready coefficients while busy.
This is a bounded direct-path filter, not a calibrated room transfer function.

## Ownership, scheduling and completion (ADR 0050)

- `Audio.beginAcousticScene(asset)` captures the calling client's asset-storage
  capability. Bounded CPU workers read at most 32 MiB, decode JSON, validate
  geometry and build a BVH. Only the owner publishes a ready scene revision;
  failed or canceled replacements retain the previous scene. No source bytes,
  native pointers or executable modules are exposed in project state.
- `beginAcousticProbe(query)` captures an immutable scene revision and pose.
  `acousticStatus` is nonblocking, client-scoped and restore-generation checked.
  `cancelAcousticJob` cancels/releases a ticket, including completed results;
  releasing a scene build ticket does not remove the published scene.
- `prepareAcousticsAsync` and `traceAcousticsAsync` use the shared generated
  cancellable-Promise binding. Completion/error is observable, and helper-owned
  operation tickets release automatically. Scene lifetime remains explicit.
- `playSfx({...,spatial/follow,acoustics:{...}})` requests automatic propagation.
  Entity/camera positions are captured at frame boundaries. Source `forward`
  is an explicit world direction; it does not yet follow entity orientation.
  Latest poses coalesce, with round-robin dispatch and at most 32 in-flight
  jobs and 1024 queued sources across the host. Saturated CPU submission retains
  the queued pose. Only live voice generations/current scenes accept results.
- Explicit operations retain at most 16 results per client and 64 per host.
  Scenes admit conservative 128 MiB staging reservations inside a 512 MiB host
  reservation budget, reduced to a geometry-size estimate after preparation.
  Charges follow retained scene snapshots, including probes surviving scene
  clear. These are admission reservations, not measured allocator/VRAM usage.
- Geometry is limited to 4096 meshes, 256 materials, one million vertices and
  262144 triangles; one propagation query uses at most 64 visibility samples
  and 64 transmission crossings. Ignored-surface traversal has a visible limit.
  Cancellation is checked at phase boundaries; library BVH construction is a
  bounded non-preemptible phase. No unbounded child Rayon pool is enabled.
- `clearAcousticScene` removes the owner's active scene/build and automatic
  source work, and disables acoustic DSP on that client's voices. Already
  submitted explicit probes retain their snapshot and report its revision.
  Restore invalidates all derived scenes/jobs/results; they are not silently
  recreated or claimed as original-save state.

The older physics-ray occlusion path remains available. It now explicitly uses
`raycast_in(voice.client, ...)`, instead of relying on the mutable default query
client. Choosing both physics-ray and dedicated acoustic modes is rejected.

## Capability boundary and next work

Absorption/scattering are validated metadata reserved for future reflections;
no reflected sound is synthesized yet. Reported travel time does not delay PCM.
HRTF dataset loading/convolution, diffraction, geometric early reflections,
late reverb, dynamic mesh refit/instancing, pose-age quality policy and a unified
async decode/mix/output pipeline remain open. Stereo pan is not an HRTF claim.
The inspected phonon reference embeds a SOFA dataset in its binaural effect;
we do not copy that dataset or import its unfinished reverb path.

No new native VM integration is claimed by these Audio APIs. Native opcode,
wait, resource and save mappings need their own source audit and behavior tests.
