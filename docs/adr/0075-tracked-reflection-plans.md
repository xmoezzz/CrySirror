# ADR 0075: tracked reflection plans and persistent sample history

Date: 2026-10-27. Status: implemented bounded first-order tracking profile.
Follows ADR 0050, 0067–0074. Actual validation is recorded in STATUS.md.

## Decision

Extend the engine-owned geometry/DSP resources from ADR 0074 with automatic
pose refresh. No native engine, renderer or new geometric dependency is added.
Static beginVoiceReflections keeps its captured-pose contract. The new
Audio.trackVoiceReflections(handle, ReflectionTrackingSpec) registers tracking;
playSfx/playSfxAsync accept the same specification as `reflections`. Playback
registration happens before any source sample is consumed. clearVoiceReflections
cancels either mode. Switching modes/re-registering settings clears old history;
ordinary pose refresh preserves it. Derived resources are invalidated on restore.

ReflectionTrackingSpec contains ReflectionOptions, a 16..1000 ms minimum refresh
interval (default 50), a 1..200 ms linear crossfade (default 20), accumulated
position threshold 0..10 metres (default .01) and forward/up angular threshold
0..180 degrees (default 1). Threshold comparisons are strict: zero means any
actual change, not repeated work at an unchanged pose. Source directivity remains
an authored world-space direction; entity rotation does not implicitly rotate it.

## Dependency, completion and ownership

Each control frame first updates listener/camera and entity-follow source poses.
The tracker captures the current immutable acoustic revision, source/listener
basis and prepared direct HRTF bridge latency. Initial tracking holds that
source's clock and explicitly waits for an enabled HRTF voice. Other voices keep
progressing. Existing HRTF preparation/failure policies are retained.

One worker query produces the initial tracked EarlyReflections or a subsequent
immutable ReflectionPlan. Initial work prepares a maximum-delay three-band ring
once; subsequent work prepares only sparse taps/metadata. The mixer owns sample
history. Even an empty path plan records input, so newly visible/delayed paths
can read retained samples. Publication validates scene revision and HRTF latency,
but allows a captured pose to lag current movement. Requiring exact pose equality
would starve continuously moving sources. Desired changes coalesce into the next
snapshot. reflectionsVoice.tracking exposes desired/submitted/published snapshots,
capture time and published age, pending/waiting/error, successful update count,
muted/draining state and engine clock. Publication means the new target is
installed; it does not mean its audible crossfade has completed.

There are at most 64 tracked voices, 32 tracked flights globally, 8 per client,
and 4 launches per frame. Round-robin admission checks the launch cap before
advancing the queue, including the five-voice boundary. Each voice retains at
most one flight or ready result. A busy crossfade retains one ready plan and
coalesces further desired movement. A full CPU pool defers work without blocking
a frame; residency exhaustion is observable waiting with interval-based retry.
No guaranteed maximum age is claimed under sustained contention.

Initial effects reserve 8 MiB; tap-only updates reserve 64 KiB against the existing
512 MiB estimated reflection host budget. Worker completion shrinks reservations
to logical ring/tap/path capacities plus 8 KiB. Reservations follow actual task,
result and installed/retired resources, including canceled work until it exits.
Scene leases similarly follow actual worker lifetime. Cancel, stop, clear and
restore prevent obsolete publication. Worker errors are observable, release the
initial source hold and preserve an existing valid effect; identical failed
snapshots are not retried every frame. Movement, scene changes or explicit
registration can submit a new snapshot.

## DSP transition policy

The fixed history capacity covers configured maximum excess delay plus up to
256 direct HRTF latency frames. Updating paths never reallocates or resets it.
The mixer renders at most current and previous plans, blending with a sample-
bounded linear fade. A publication during an unfinished fade is rejected/deferred.
Old metadata/leases are collected on the control owner after the fade; the sample
loop does not reclaim those plans. Static/channel mixer APIs remain usable without
App, VM, window, output device or script host.

Scene revision or direct HRTF latency changes mute obsolete taps immediately but
retain input history. A compatible replacement fades in from silence so the old
geometry/latency cannot leak through the transition. Failed replacement cannot
unmute obsolete paths. Clearing the acoustic scene or removing spatial state
disables tracking. Natural source EOF cancels further geometry work and freezes
the installed target while remaining echoes drain. Source completion remains
separate from effect tails, preserving original native channel waits.

Seek resets the history's filled count/splitter in constant time. Stop, stealing,
clear and explicit settings/mode replacement cut history. This snapshot/crossfade
model does not model waves already traveling through moving geometry.

## Validation and limits

Sample oracles cover changed delay reading older input, initially empty paths,
exact fade samples, busy rejection, owner-side retirement, mute-to-new-scene
fades and seek isolation. Device-free engine cases exercise continuous motion,
listener orientation, entity/camera follow, short SFX plus HRTF dependency,
blocked workers, scopes, clear/restore, scene replacement and fair admission.
Shared channel cases preserve native source-completion semantics. The authored
moving_reflections example exports stationary/moving wet-only WAVs through the
same worker-prepared plans and mixer; its explicit wait is an offline fence.
Live engine tracking never waits for geometry work in Mixer::render.

Geometry is still baked/static: no mesh refit, higher/diffuse bounce, diffraction,
reflection HRIR/Ambisonics or continuous Doppler. Updates crossfade discrete
captured delays rather than resampling a continuous physical trajectory. Ring
capacity and pending jobs bound memory, not CPU throughput for arbitrary scenes.
Final DSP/mixing, control/scratch allocation, last-reference disposal and device/
pool teardown retain synchronous owner boundaries. This is partial progress on
ADR 0050, not complete engine-wide async execution or new commercial VN opcode,
save, GPU, Windows/Linux or audio-output-device evidence.
