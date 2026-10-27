# ADR 0084: shared direct Doppler and owner motion capture

Date: 2026-10-27

Status: implemented bounded direct model; actual validation is in STATUS.md.

## Context and inspected sources

Moving acoustic geometry and source/listener tracking did not change playback
frequency. A reusable engine needs explicit velocity, stable frame inference,
teleport/gap policy and observable worker-owned pitch progression.

Oddio 0.7.4 was inspected at its registry archive checksum and embedded Git
revision. Its complete spatial/speed modules combine movement, seek/delay queues,
per-ear output and scene signals. This phase keeps CrySirror's existing source,
HRTF, reflection, bus and worker ownership and implements the standard radial
calculation in its shared mixer. No oddio scene implementation is imported or
copied; no dependency is added. See [source audit](../doppler-source-audit.json).

The mathematical reference is OpenAL 1.1 section 3.5.2, with source and listener
velocities projected on the same source-to-listener direction. Microsoft's
X3DAudio documentation independently separates this multiplier from source
frequency. These references describe API/model conventions, not performance or
listening comparisons with those engines.

## Shared DSP contract

Spatial gains optional velocity and DopplerSettings. None doppler preserves
existing input speed; opting in defaults to factor 1, speed_of_sound 343.3 m/s,
max_ratio 4 and smoothing_ms 20. Coordinates are metres, velocities m/s in a
stationary medium. Explicit velocity is optional; standalone Mixer treats absence
as zero, while App can supply an inferred capture.

The target is (c-factor*listener_radial)/(c-factor*source_radial). Coincident
positions yield unity. Inputs must be finite and within +/-1e6 components.
Settings bound factor 0..16, speed 1..10000, max_ratio 1..4 and smoothing 0..1000
ms. Scaled radial components clamp to +/-99% of sound speed, then the ratio clamps
to [1/max_ratio,max_ratio], with separate diagnostic flags. These bilateral safety
bounds deliberately extend the specification's extreme-velocity policy; this is
not a supersonic/shock-wave simulation.

Doppler is a separate output-frame linear ramp multiplied into the existing source
resampling step, after authored rate and semitone pitch. It never rewrites user
controls or resets source position. Interpolation advances through paused input,
preparation/seek holds and starvation, but consumes no PCM during those holds.
Unchanged targets are not restarted at render-block boundaries. Removing Doppler
returns to unity on the next block. A smaller max_ratio constrains the new target;
existing interpolation can approach it from the previous wider bound.

The existing linear resampler remains the backend. Its anti-aliasing/quality
limits are unchanged; this phase does not add a band-limited or time-stretch
algorithm. HRTF/reflection/reverb receive the resampled input and retain their
existing output-rate histories/tails. Individual reflected paths do not get
independent Doppler shifts, and no direct propagation delay is introduced.

## Motion capture and ownership

Reusable MotionTracker uses caller-provided frame dt, without wall time, threads,
devices or a VM. First observation is stationary. A positive bounded interval
uses world displacement/dt; an unchanged zero-time pose retains the last velocity.
A changed zero-time pose or displacement above the teleport threshold resets
inference. Long gaps reset it too. Explicit velocity overrides inference while
updating its pose anchor. Reasons are observable MotionStatus values.

MotionSettings defaults: max_interval_ms=250, teleport_distance=50 m; limits are
1..60000 ms and 0.001..1000 m. Derived velocities fit the same vector bounds.
Reset removes inference history, not source/DSP history; the existing configured
pitch ramp can still settle toward the new stationary target.

App's owner captures the world camera or client manual listener and each opted-in
spatial attachment once after world/late updates. Entity sources use their global
parent transform plus the existing world-axis attachment offset. Automatic
velocities are captures of frame displacement; they are not a physics body's
exact velocity, nor prediction/extrapolation. Explicit velocities are available
for callers with authoritative physical motion.

Source histories use full (client,slot,generation) identities and are pruned when
voices end or lose Doppler. Listener/settings maps use registered App clients;
source storage is bounded by the existing voice pool. Restore/reset_playback
clears histories and ephemeral voices while retaining client motion settings as
host policy. Returning a manual listener to the camera resets listener history.
No history, device or worker pointer is placed in project/native save data.

Pose capture is small bounded owner control work. The existing MixFrame/MixJob
moves the mixer to a CPU worker; only that worker advances Doppler, source and DSP
samples. An outstanding final frame prevents a new App frame/control mutation.
Backpressure must not resample world velocity or advance a source twice. No new
join, task pool, device callback lock or per-sample allocation is introduced.
Existing offline synchronous execution remains available.

## Public APIs and observations

- Audio.playSfx / playSfxAsync / prepared-source voice options accept
  spatial:{position,velocity?,doppler:{...}}. setVoice replaces the supplied
  spatial record as before. follow continues to take a same-client Entity.
- Audio.setListener accepts optional explicit velocity; absent velocity is
  inferred from manual pose updates or the automatic camera.
- Audio.motionSettings / setMotionSettings expose strict client capture policy.
  Updating it resets histories. resetMotion requests new anchors at the next
  owner capture, useful for teleports; explicit velocities remain authoritative.
- Audio.dopplerVoice returns the scoped DopplerInfo plus source/listener capture
  reasons. target is desired current geometry/velocity; ratio and smoothing_frames
  describe the installed ramp. target_pending identifies a desired update not
  yet installed by render-block setup. effective_rate excludes source/output
  sample-rate conversion. Observations freeze with mixer publication while a
  worker frame runs; history/settings controls fail until completion.
- Rust clients can use calculate, MotionTracker and Mixer::doppler_info without
  App, a VM, output device or window. Public Rust Listener/Spatial literals need
  their added fields or ..Default::default(). Existing JSON fields default absent
  velocity/Doppler to None; no project/native save schema changes are made.

## Validation and remaining work

Analytic radial cases, actual source samples/clock progression, chunk equivalence,
paused smoothing, validation/scope, parent/camera motion, discontinuity/gap/explicit
capture, restore and worker backpressure have independent release tests. Real
JS/TS Promise calls exercise parsing/publication and inspections. The authored
Doppler pass-by demo exports a moving 440 Hz oscillator with independent crossing
frequency assertions, synchronously and through final DSP workers. Exact commands
and measured results are recorded in STATUS.md; no game/font/HRIR bytes are added.

Direct arrival delay/retarded-time propagation, per-path reflected Doppler,
wind/moving media, diffraction, supersonic acoustics, predictive motion, independent
device clock and band-limited resampling remain open. No native spatial opcode or
HostServices spatial facade is mapped here. Existing VN opcode/save/resource
semantics remain unchanged; this capability is not a new whole-game result or
completion of the engine-wide async/3D/VN objective.
