# ADR 0079: exclusive worker DSP frames and owner publication

Date: 2026-10-27. Status: implemented bounded profile.
Follows ADR 0050 and 0069–0078. Actual validation is recorded in STATUS.md.

## Research and scope

[Unreal's Audio Mixer overview](https://dev.epicgames.com/documentation/en-us/unreal-engine/audio-mixer-overview-in-unreal-engine)
separates gameplay audio controls, the audio rendering thread and platform
callbacks, and uses task workers for decoding/generation. [Kira's public API](https://docs.rs/kira/0.12.5/kira/)
also distinguishes gameplay handles and audio-thread commands. These are design
references, not imported renderers or a claim of equivalent scheduling.

CrySirror already prepares streaming decoders, HRTF, reverb and reflection plans
on workers. Its final pooled/canonical and native/media mixers still processed
samples on the owner thread. This ADR moves final DSP and producer summation into
bounded worker jobs. It does **not** complete ADR 0050 or establish a device-clock
renderer independent of the simulation frame rate.

## State ownership and publication

`crysirror_audio::render_job::MixJob` moves one to eight same-rate CPU Mixers into
one task. No worker borrows a VM, EngineContext, Runtime, window or output device.
A fresh process-local lease identifies each ordered owner slot. The owner keeps
bounded snapshots of voice handles/specs/transport/source-buffer observations,
control groups/listeners/buses and reverb settings; it does not clone decoder,
convolution, delay-line or reverb DSP histories. Read queries retain that last
published boundary even after a worker has finished privately.

A poll returns pending or atomically restores all mixer states and returns PCM.
The single owner validates all leases before restoring any state. Every mixer
advances only its declared frame count; the combined PCM spans the maximum
count/input length. The worker renders in blocks of at most 4096 frames, checks
cancellation between blocks, sums unclipped stereo and rejects nonfinite output.
Jobs admit at most two seconds at their actual sample rate; prepared absolute
limits are 384000 stereo frames. Device clipping remains downstream.

Ordinary DSP controls explicitly fail while a slot is leased. The existing
infallible stop_client/pause_client calls defer bounded client control changes
until publication. Clear/reset invalidates a lease immediately. Matching-token
cancellation clears playback in the associated slots, while late completion
cannot replace a new voice or restored generation. Source failures and task
panics publish no PCM and release/reset the affected playback slots. Partial
lease acquisition rolls back; submission failure clears already transferred slots.

The caller must poll or cancel with the same ordered mixer slots before dropping
MixJob/MixFrame. Their must_use attributes flag unhandled values. Dropping a job
requests worker cancellation; an abandoned owner slot still needs clear/reset
before reuse. This explicit ownership contract avoids a mutex around live DSP.

## Dependencies and backpressure

`MixFrame` retains prepared PCM/counts while task admission is full. Poll retries
capacity without rerunning simulation, callbacks, input or source preparation.
After admission, its task owns the DSP until publication or cancellation. A host
must not mutate prepared frame inputs between acceptance and publication.

`Runtime.begin_tick/poll_audio_frame/audio_frame_pending` advances native/world
control once and completes native/media final mixing on workers. The Runtime
rejects a new tick while its frame is pending. Restore cancels this dependency
only after restore validation succeeds, resets playback and discards old PCM.
`tick_control` is an explicit external-coordinator boundary, not a complete frame;
its caller must enforce the outstanding audio dependency. `tick` remains the
explicit synchronous/offline path.

`App.begin_frame/poll_frame/frame_pending` runs fixed/update/runtime/late control
once, prepares pooled/canonical and native/media state and submits both mixers
in one MixFrame. Completion publishes one combined PCM product, increments the
engine frame and commits the accepted input once. Audio.rendering is true while
prepared or running; readonly playback queries retain the previous publication.
Audio controls require completion even when worker capacity has delayed admission.
Prepared playback receipts do not report ready until owner publication. Pause
hooks defer until completion; restore discards the obsolete product; shutdown
cancels before lifecycle hooks. App.frame retains synchronous offline behavior.

The default wgpu/winit desktop drives these asynchronous boundaries for scripted
and plain runtimes. Plain desktop coordinates its shared canonical SceneAudio
mixer with the Runtime's native/media mixer. Its frame gate runs before taking
elapsed time, polling gamepad navigation or consuming input edges. Pending polls
cannot accept another simulation tick; only completion advances to PCM submission
and scene extraction. The optional audio feature controls the device bridge,
not CPU frame ownership. Muted verification uses the same worker path without
opening a device. Offline movie/capture interfaces retain synchronous execution.

## Limits and follow-up

This is frame-dependent audio scheduling. It can stall simulation/render progress
when final DSP misses its dependency deadline, and a slow shared CPU pool can
cause a device underrun. It does not queue future controls sample-accurately or
run a dedicated audio clock/thread independent of visual frames. Owner snapshot
allocation/control copying, worker PCM allocation, completed DSP/resource drops,
pose/control preparation and some deferred-effect retirement still incur costs.
The change does not demonstrate improved throughput or bounded realtime latency.
The paired product retains the two existing mix instances: duck detection and
effect histories remain per mixer. Summing PCM does not create cross-mixer
sidechains or a general submix network.

PcmOutput still lazily opens its platform device on submission and uses a mutex
for its bounded queue/refill callback. Platform stream lifecycle, lock-free
sample transfer, device-loss recovery, dedicated scheduling and graceful pool
shutdown/join remain separate work. No output-device, Windows/Linux, GPU or
underrun claim follows from CPU tests. Siglus whole-ms polling still uses the
existing VM-before-mix phase; worker publication does not change native wait,
resource or save policies. No new original VM hooks are implemented by this ADR.
