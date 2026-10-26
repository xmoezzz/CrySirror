# ADR 0074: captured geometric early reflections

Date: 2026-10-27. Status: implemented point-source, single-specular-bounce profile.
Follows ADR 0050, 0067–0073. Actual validation is recorded in STATUS.md.

## Model and source boundary

Add reusable first-order image-source queries and sparse echo DSP to
crysirror-acoustics. Triangle intersection and segment traversal use the existing
pinned Parry/optional obvhs scene; no new BVH builder, native VM or renderer is
imported. Image-source geometry is an engine implementation of the standard
mirror construction, not a port of another engine. Source/algorithm references:

- [Pyroomacoustics room simulation](https://pyroomacoustics.readthedocs.io/en/pypi-release/pyroomacoustics.room.html)
  distinguishes image-source early paths from a ray-traced diffuse tail.
- [Steam Audio materials](https://valvesoftware.github.io/steam-audio/doc/unreal/material.html)
  distinguishes absorption, scattering and transmission.
- Complete pinned phonon_rs source remains at 87c392a3e605f9df30abe6e291f0bbc5e86f54ad.
  Its reverb estimator is a parameter placeholder, not an implementation imported
  here. Existing directivity/transmission source notices are preserved. No
  Pyroomacoustics or Steam Audio runtime is linked by this change.

For each baked world triangle, mirror the point source across its plane, intersect
listener-to-image with the finite triangle through Parry, and trace both legs
against the shared BVH. Opposite-side/near-plane candidates are rejected. Shared
coplanar edge hits are deduplicated by bounce/normal, choosing the first audible
triangle in scene order. This is not a multi-bounce or diffuse path solver.

AcousticMaterial absorption is energy, transmission remains amplitude per crossed
surface, and scattering removes the diffuse fraction from this specular result.
Our explicit pressure convention is:

    sqrt(max(0, 1 - absorption - transmission²) * (1 - scattering))
      * exp(-air_coefficient * path_metres)
      * min(1, reference_distance_metres / path_metres)
      * source_directivity_along_source_to_bounce
      * transmission_on_both_legs

Existing material tables are accepted even when absorption + transmission² > 1;
the reflected remainder clamps to zero. This is CrySirror's convention, not a
claim of exact Steam Audio material equivalence. Leg crossings reuse the direct
path kernel and multiply transmission once per crossing; no entry/exit pairing.
Opaque/absorbing intervening surfaces can remove the path, while transmissive
surfaces attenuate it per band. Crossing-budget errors fail visibly.

Physical path travel and excess travel relative to the straight source/listener
path are reported separately. Dry playback retains its existing clock. Echo DSP
applies the excess delay so relative arrivals are correct against an undelayed
dry path. The effect aligns its output to an already prepared direct HRTF bridge
latency if present. Echo arrivals use stereo equal-power panning; they do not
receive per-direction HRIR/ITD filtering or Ambisonic rendering.

## Bounds and DSP ownership

ReflectionOptions validates a point source (radius zero), physical/directivity
settings, excess delay 0..2 seconds, 1..64 unique audible paths, and a complete-
scene budget up to 262144 triangles. Defaults cover the scene triangle limit.
Exceeding path/triangle/crossing limits returns an error instead of a truncated
apparently complete result. Arrival, point, pressure and delay data remain
inspectable through ReflectionQuery/Path/Paths. Queries check cancellation every
64 triangles. Scan cost is O(scene triangles); BVH queries accelerate leg tests.

EarlyReflections prepares taps, complementary-band splitting and a mono three-
band history ring on a worker. Fractional delays use linear interpolation and
three bands use the existing one-pole 800/8000 Hz split (now shared BandSplitter).
This is a bounded approximation, not a bandlimited wave solver. Sample processing
performs no geometry, I/O or ring allocation. A filled-history guard makes seek
reset constant-time without clearing the delay buffer. Natural EOF drains the
maximum echo delay plus 10 ms of band-filter history; tiny IIR residue beyond
that horizon is truncated. Explicit stop/steal/clear/replacement cuts old history.

Echo input is captured before direct distance/occlusion/HRTF processing, so an
occluded direct path does not automatically suppress a separately visible echo.
The wet result uses the voice gain/envelope and bus/Master once, independently
of the dry spatial curve; its own range attenuation is already in path pressure.
It can feed the optional late-reverb send with the rest of the voice output.

## Engine APIs and snapshot semantics

Audio.beginReflectionProbe(query), beginVoiceReflections(handle, options),
reflectionStatus, cancelReflectionJob, clearVoiceReflections and reflectionsVoice
use shared worker-prepared data. traceReflectionsAsync and
prepareVoiceReflectionsAsync wrap these in the existing Promise system and release
receipts. Result revision is the acoustic scene revision, not the operation
receipt; promise.ticket identifies the latter. Rust channel/mixer APIs work
without an engine App, native VM, window or output device.

Voice preparation captures source position, listener pose, acoustic revision and
current direct HRTF bridge latency. Initial preparation holds only that source's
clock so a one-shot cannot finish while work is pending. Replacement keeps a
valid old effect/source running. If HRTF is still preparing, wait for it before
requesting reflections. Very short sounds started through playSfxAsync should be
created paused, prepared, then unpaused; requesting an effect after source EOF
fails. This API does not retroactively recover consumed PCM.

These are **captured-pose resources**, with no automatic tracking yet. Source or
listener pose changes clear installed paths; scene revision and HRTF latency
changes invalidate them too. Pending obsolete work becomes Failed and releases
its initial clock hold. Callers explicitly prepare again. Failed replacement
keeps the old still-valid effect. Current state and invalidation/failure reasons
are distinct from a receipt acknowledging earlier successful publication.

OperationTable::fail now marks queued/running/ready work failed without starting
queued work, preserving scoped observable errors. Dropping a task requests
cancellation; captured inputs and charges survive until its actual worker ends.
Reflection tables retain 16 jobs/client, 64/host. Per-voice preparation/installed
DSP initially reserves an estimated 8 MiB against 512 MiB/host. A worker then
shrinks it to logical ring/tap/path storage plus an 8 KiB allowance; actual owners,
including canceled live work, retain that charge. Probes retain at most 64 paths plus their captured scene lease. Effects and
tasks are derived resources, never VM/project save pointers. Restore invalidates
jobs, paths, handles and histories.

Modern Mixer::is_playing/Audio.voice.playing include natural processing tails;
source_is_playing/Audio.voice.source_playing exclude them. Shared channel/native
wait queries use source completion, so original VM instruction waits do not grow
with the echo/HRTF tail. Original RealLive/AVG32 bytecode fixtures verify this
boundary while engine-owned echoes remain active. Adapter implementations and
native opcodes/save formats do not change.

## Validation and remaining work

Analytic finite-wall oracles cover both BVHs, shared edges/backfaces, units,
material energy, leg transmission/crossing limits, direct occlusion with a
visible echo, cancellation and budget failures. Sample-level tests cover
fractional delays, pan, seek/reset, initial clock holds, EOF tails, native source
completion and HRTF bridge alignment. Engine/JS/native fixtures and authored
sound demos exercise publication, scopes, invalidation and actual PCM output.
The dense-grid benchmark checks the analytic mirror oracle near the scene limit;
it is not a cross-hardware ranking. See STATUS.md for measured results.

Automatic pose refresh, dynamic acoustic geometry/refit, higher-order/diffuse
reflections, diffraction/pathing, reflection HRIR/ambisonic rendering, Doppler and
seamless filter transitions remain open. Final mixing, control/scratch allocation,
last-reference disposal and device/pool teardown retain synchronous boundaries.
This does not complete engine-wide async execution or establish new commercial-
game, original-save, GPU or other-platform behavior.
