# CrySirror API documentation

CrySirror is in early development and its API changes often. The reference
here is therefore generated from the code that runs, never written by hand,
and a test keeps it current.

| File | Contents |
|---|---|
| [REFERENCE.md](REFERENCE.md) | Classes, properties, methods, script globals, command map, data specs with defaults, adapter configs |
| [crysirror.d.ts](crysirror.d.ts) | TypeScript declarations that the JS/TS host binds against |
| [api.json](api.json) | The same reference, machine-readable, for tools and MCP clients |
| [CHANGES.md](CHANGES.md) | Hand-written log of API signature changes, keyed by fingerprint |

## Sources

- **Classes.** The `ClassRegistry` (`crysirror-engine::object`): the
  `ClassInfo` docs, property types and method signatures. The same registry
  generates the JS bindings and the `.d.ts`, so the three cannot disagree.
- **Commands.** `services::COMMAND_API` maps every typed `WorldCommand`,
  `LayerCommand`, `CanvasCommand`, `UiCommand` and `InputRouteCommand` to the API member that issues it.
  `tests/api_coverage.rs` fails on an unmapped variant.
- **Data specs.** Field names and Rust types come from the `pub struct` and
  `pub enum` declarations in `crysirror-scene`, plus shared media/asset declarations in core/runtime and task statistics in
  `crysirror-tasks`. Modular effect authoring declarations, CPU particle/event
  records and reservation limits come from `crysirror-vfx/src/schema.rs` and
  `simulation.rs`. Shared native-resource limits, transforms, LZSS, file kinds, WAVE containers and bitmap operations come from `crysirror-resource` and `crysirror-image`. Descriptions come from their
  `///` comments. Defaults come from serde: a minimal value is parsed and
  then serialized again.
- **Adapter configs.** The `Config` structs of the Siglus, native RealLive and
  original AVG32 VM adapters. Siglus's explicit `MessageWindow` fields are also
  exported as a data specification. Removed hosted-engine configurations are
  not accepted.

Rust crate internals are not part of this reference. For those, use
`cargo doc --workspace --no-deps`.

The native Rust contract also documents `HostServices.begin_pattern` and
`begin_canvas_image`. Their immutable resources and nonblocking completion
receipts use the shared image workers. Canvas capture needs an enabled
presenter; live Puppet and replaceable surface/history dependencies explicitly
fail in the current immutable profile. Releasing a published image ends caller
ownership while retained scene references and already-admitted capture leases
continue to keep their actual generation alive.

`Text.beginPrecomputedFont(alias, asset, pixelSize)` uses the same FontLibrary
jobs/catalog as ordinary font loading. TFT v0/v1 stores coverage and metrics;
pixel size is explicit and participates in resource identity. Fixed-size or
missing-glyph violations fail rather than silently choosing a system face.
This resource API is usable independently of KiriKiri's native Font facade.

## Modular VFX declarations

The reference uses `Vfx`-prefixed aliases for types from `crysirror-vfx`:
`VfxSystemSpec`, `VfxEmitterSpec`, `VfxRenderSpec`, and their dependencies.
These are documentation names, not renamed Rust types or serialized tags.
The older scene `EmitterSpec` still describes `EntitySpec.particles`;
`EntitySpec.vfx` instead uses `VfxSystemSpec`. An entity cannot declare both.
`ParameterMap` is expanded to `BTreeMap<String, VfxParameterValue>` in the
reference. Tuple-enum payload types and multiline serde attributes are read
from the same sources by `tests/api_docs.rs`.

Authored parameters use tagged values such as
`{"kind":"vector","value":[0,1,0]}`. A scalar or vector module input accepts
a bare constant or a binding object such as
`{"parameter":"wind","scale":2}`. `Vfx.setParameter` accepts the raw value
matching the existing parameter type. Unknown fields, renderer kinds and
unsupported GPU combinations fail validation.

Simulation is owned by `crysirror-vfx` and scheduled by the Runtime; a renderer
does not advance a private copy. Offline Rust tools publish completed work
with `Runtime.finish_vfx_offline()` before `render_pbr()`; ordinary App offline
preparation performs that fence too. GPU simulation additionally needs real
renderer submissions to consume its bounded command timeline. GPU capacity
is never a measured live-particle count.

The current renderer supports textured/flipbook sprites, soft sprite depth
fade, static unlit mesh instances, and CPU ribbons. Mesh assets provide static
geometry and an explicit VFX texture/color, without inheriting imported PBR
materials; skinned/morphed sources are rejected. GPU ribbons and event-driven
sub-emitters are rejected. Transparent particles sort within an emitter and
emitters sort by distance; this is not unified scene transparency/OIT. Ribbon
width is explicit full world width, independent of size-over-life curves.
This API is a bounded modular VFX foundation, not full Niagara parity.

CPU sprite emitters can additionally declare `screen` with a logical `canvas`,
an owned `layer`, optional same-client `parent`, painter `order` and a fractional
particle `anchor`. Runtime publishes completed particle snapshots as ordinary
retained mesh layers, including after an explicit offline completion fence.
They require no world camera and are excluded from world-space VFX drawing.
`crysirror_vfx::screen::pack_screen_emitter` accepts explicit per-particle atlas
UVs and is usable independently of any native VM. Unsupported facing, GPU screen
simulation, producer conflicts and unsupported blends fail visibly. Framework
snapshots with active or pending screen emitters are explicitly rejected until
their derived mesh ownership can be reconstructed; this is not save support.

`VfxSystemSpec.clock` separates engine deltas from explicit integration ticks
and wall time. An external CPU system can own one `sequence_rng` shared by its
emitters in declaration order. `sequence_exchange` borrows that stream at an
already completed tick and replaces its state at a strictly increasing epoch;
admission is distinct from acknowledgement. `Vfx.sequenceStatus(entity,
revision, completedTick, exchangeEpoch)` returns pending until a worker has
completed both requested boundaries for the current entity lifetime and authored
declaration. It never returns an unprocessed seed as a completion. Native clients
use the same client-checked `EngineQuery::VfxSequence`. Decimal strings preserve
large counter arguments/results across JS/MCP transports. Worker receipts and
their host cache are derived state and are cleared at restore.
Active or queued sequence systems also reject framework snapshots until the
completed random-stream and exchange state can be reconstructed safely.

`VfxEmitterSpec.falling_atlas` is a CPU local-space screen-sprite node over the
existing particle pool. Its declarative drift, reset plane, nonuniform atlas,
wrapping/monotonic atlas clock and bounded integer stream need no VM callback or
private particle renderer. GPU, conflicting modules/events and unsupported
clock profiles reject at compilation. Multiple systems do not implicitly share
one random stream; callers must preserve their actual ownership boundary.

## Acoustic scene and direct sound

`Audio.prepareAcousticsAsync('room.json')` prepares the calling client's
`AcousticSceneSpec` asset on workers. Existing scenes remain active until a
replacement succeeds. Example script usage:

```typescript
const scene = await audio.prepareAcousticsAsync('room.json');
const voice = await audio.playSfxAsync({
  asset: 'step.wav', spatial: { position: [0, 0, -4] },
  acoustics: { radius: 0.2, samples: 16 }
});
const path = await audio.traceAcousticsAsync({
  source: [0, 0, -4], listener: [0, 0, 0]
});
```

The background voice queries follow source/listener positions. Propagation
uses low/mid/high direct-path transmission and air absorption; distance/pan
still come from the existing mixer. `acousticVoice` exposes pending/error/path
information. `clearAcousticScene` removes the active scene and disables this
processing on its voices. Async helpers release their operation tickets,
without unloading a successfully prepared scene. Scenes are derived resources
and require preparation again after restore.

Default queries use Parry; the optional host feature `acoustics-obvhs` permits
`backend: "obvhs"` in scene JSON. Unsupported backends fail visibly. The direct-path profile
does not enable HRTF by itself; prepare an HRTF resource separately for that.
The direct-path query does not simulate reflections/reverb; the optional shared
resources below provide those. Absorption/scattering affect specular reflections;
reported direct travel time is not a playback delay.
See [ADR 0067](../adr/0067-shared-acoustics-and-cpu-bvh.md) for scheduling,
limits and source attribution. The device-free synthesized demo runs with
`cargo run --release -p crysirror-audio --example acoustic_room -- /tmp/room.wav`.

## HRTF / headphone rendering

```typescript
await audio.prepareHrtfAsync('listener.sofa', { partition_frames: 64 });
const voice = await audio.playSfxAsync({
  asset: 'step.wav', spatial: { position: [2, 0, -4] }, binaural: true
});
```

Profiles are client-scoped SOFA SimpleFreeFieldHRIR assets or prepared
`HrirAsset` JSON (matching the mixer sample rate). Measured HRIR data must be
supplied separately. `HrtfOptions` controls partition size, crossfade, delay
capacity, SOFA normalization/reference distance and explicit ear mapping
(left_right by default, right_left or receiver_positions when required). The default adds 64 frames
of processing latency. Lookup follows source position and listener orientation;
`acoustics` can additionally apply the shared direct-path scene.

Initial preparation freezes a short voice until its worker buffers are ready.
Pose updates crossfade without replacing source state; `hrtfVoice` reports the
published target and pending preparation, not audible transition completion.
`setVoiceHrtf(handle,false)` restores ordinary spatial pan; `clearHrtf` also
removes the client's profile. Profiles are derived and require preparation
after restore. Natural EOF drains per-ear delays and FIR tails. See
[ADR 0068](../adr/0068-hrtf-resources-and-binaural-voices.md) for limits and
remaining async boundaries.

An offline, device-free example synthesizes its own source and reads an
explicit external profile:
`cargo run --release -p crysirror-audio --example binaural_orbit -- profile.sofa /tmp/orbit.wav`.
No measured HRIR, game sound or font is bundled with the example.

## Worker audio preparation

```typescript
await audio.preloadSfxAsync('click.wav');
const click = audio.playSfx({ asset: 'click.wav' });
const music = await audio.playMusicAsync('explore', {
  asset: 'explore.ogg', loop_points: { start: 48000, end: 960000 }
}, 500);
await audio.seekVoiceAsync(music, 96000);
```

Async helpers return shared mixer handles, release their operation tickets and
retain the playing resources. `sourceStatus(music)` reports bounded decoded
PCM, pending worker/seek state and errors. A pending seek freezes only that
voice; a failed music replacement preserves the old music. Realtime cold SFX
and music require these async methods. `playRandom` requires preloaded variants.
Offline synchronous methods remain supported. See [ADR 0069](../adr/0069-worker-audio-decoding-and-playback.md)
for cancellation, budgets and the remaining native-channel/mix/output boundaries.

## Canonical channel readiness

`Audio.play(channel, voice)` and data VM Play commands declare canonical logical
voices. `Audio.channelStatus(channel)` separately reports the desired epoch,
queued/loading/playing/paused/ended/failed phase, position and error. Stop and
restore invalidate stale jobs; natural EOF does not replay a persistent voice
declaration. Time waits do not imply audio readiness. App and plain desktop use
the same worker bridge, with explicit offline behavior retained.

Desktop audio execution is independent of native image/scenario loading policy.
Cold modern SFX/music also require the async helpers during live initialization.
See [ADR 0071](../adr/0071-canonical-audio-worker-bridge.md) for these boundaries.

## Rules for changing the API

`crates/crysirror-engine/tests/api_docs.rs` enforces these rules in the normal
`cargo test`:

1. **Regenerate.** After changing a class, a command or a spec, run
   `CRYSIRROR_BLESS=1 cargo test --release -p crysirror-engine --test api_docs` and
   check the diff in `docs/api/`. A stale file fails the test.
2. **Log signature changes.** The *fingerprint* hashes the signatures only:
   names, types, writability, spec fields and defaults, command variants and
   adapter config fields. Prose changes do not affect it. When the
   fingerprint changes, add an entry to [CHANGES.md](CHANGES.md) that names
   it and says what changed and how callers migrate.
3. **Document every member.** Every class, property and method needs a
   non-empty doc string, or the test fails. Every new spec field should get
   a `///` comment; the reference shows `—` where one is missing.
4. **Make breaking changes visible.** Renaming or removing a member, a field
   or a variant, or changing a default, is a breaking change. Say so in the
   CHANGES entry. Saved projects and save files are versioned separately
   (`schema` and [SAVE_FORMAT.md](../SAVE_FORMAT.md)).

## Parametric late reverberation

```typescript
await audio.prepareReverbAsync({
  room: { room_size_metres: 18, decay_seconds: 1.2, damping: 0.55 },
  sends: [{ bus: 'sfx', gain: 0.5 }, { bus: 'voice', gain: 0.15 }],
  return_bus: 'ambient', wet_gain: 0.5
});
// Ordinary channel, pooled and native/media playback use these shared buses.
// audio.clearReverb() removes this client's prepared room and remaining tail.
```

The optional resource applies authored parametric late reverb via FunDSP's
32-channel FDN. It does not infer geometry, materials or physical RT60. Source
sends follow current spatial/direct/HRTF processing and their bus faders, before
Master. The return uses its own bus and Master once. Empty sends disable input;
source mute blocks sends while return mute silences the wet tail. Replacing or
clearing a room cuts its old history. Effects remain derived across saves and
need preparation after restore. Shared Rust PreparedReverb/Mixer APIs work
without a native VM or output device. See [ADR 0073](../adr/0073-shared-parametric-reverb-buses.md)
for task ownership, limits, two-mixer publication and remaining async boundaries.

## Captured geometric early reflections

```typescript
await audio.prepareAcousticsAsync('room.json');
const voice = await audio.playSfxAsync({
  asset: 'step.wav', paused: true, spatial: { position: [0, 0, 2] }
});
await audio.prepareVoiceReflectionsAsync(voice, { max_delay_seconds: 0.5 });
audio.pauseVoice(voice, false);
```

Queries use finite single-bounce image sources with shared BVH leg queries and
per-band material pressure, air/range/directivity and crossing transmission.
traceReflectionsAsync accepts ReflectionQuery for inspection without a voice.
Voice preparation captures the current source/listener/scene and requires a
prepared HRTF voice if HRTF is enabled. Moving a pose or replacing geometry
invalidates static paths; prepare again explicitly, or opt into tracking below.
Starting short async sounds paused prevents source EOF before preparation starts.
Published receipts are separate from current effect state; use reflectionsVoice
to inspect pending work, current paths and invalidation errors. Promise.ticket
identifies its receipt; result.revision identifies acoustic geometry.

Sparse DSP uses linear fractional delays and a complementary three-band split.
Dry playback retains its clock; wet uses excess travel delay and the direct HRTF
bridge latency. Echo panning is stereo, without per-direction reflection HRIRs.
Audio.voice.source_playing excludes derived tails; playing includes them. Native
channel waits retain source completion while the engine drains echoes. Scene/path/
crossing budget overflow fails visibly. See [ADR 0074](../adr/0074-captured-geometric-early-reflections.md)
for energy convention, ownership, bounds and remaining propagation work.


## Automatic reflection tracking

```typescript
await audio.prepareAcousticsAsync('room.json');
const voice = await audio.playSfxAsync({
  asset: 'step.wav', spatial: { position: [0, 0, 2] },
  reflections: { refresh_interval_ms: 50, crossfade_ms: 20 }
});
// Or: audio.trackVoiceReflections(existingSpatialVoice, settings).
// follow: entity tracks the entity; the listener follows the camera by default.
```

ReflectionTrackingSpec adds motion thresholds and scheduling/fade controls.
Playback registers before consuming short sources; enabled HRTF prepares first.
Workers coalesce moving source/listener snapshots and publish completed paths
without resetting delay history. reflectionsVoice.tracking reports snapshots,
capture/published age, pending/waiting/error, muted/draining and update count.
Published targets can lag current motion; publication is separate from the
sample-level fade finishing. Scene/latency changes mute incompatible old paths.
Natural source EOF stops refresh and drains the installed echo. Clearing the
scene disables tracking; clearVoiceReflections cancels tracking/effects. Changing
settings or switching static/tracked mode cuts history. See [ADR 0075](../adr/0075-tracked-reflection-plans.md)
for bounds and remaining physical/async limits. An authored wet-only moving demo:
`cargo run --release -p crysirror-audio --example moving_reflections -- /tmp/echoes`.


## Source transport and native channels

```typescript
const sound = await audio.playSfxAsync({ asset: 'step.wav', paused: true });
audio.scheduleVoiceResume(sound, 200, 30);
const transport = audio.playbackInfo(sound);
// position_frames/source_rate use decoded-source units.
// resume_delay_frames/fade_out_frames use output_rate.
```

Scheduled resume freezes input, waits on the output sample clock and starts a
fresh fade-in; pause/stop/resume replaces the schedule. Explicit stop/pause fade
counters progress even when decoder input is unavailable. Source position and
ordinary fade-in do not advance through missing PCM. Preparation readiness is
separate from scheduled transport.

nativeChannelInfo(channel) queries the caller's shared native/media channel
and returns null when no voice is retained; source_playing excludes DSP tails.
It is distinct from canonical SceneState channelStatus. Rust clients use
HostServices.audio_info or Mixer/ChannelAudio.playback_info. AudioParameters can
publish paused, start at an exact source frame, clamp an authored range or fade
the replaced source. These are reusable engine operations without any VN VM.
See [ADR 0076](../adr/0076-shared-audio-transport-and-siglus-bgm.md).

### Immutable prepared audio sources

```ts
const prepared = await audio.prepareSourceAsync("music.ogg", {measure_frames: true});
const voice = await audio.playSourceAsync(prepared.resource, {looping: true});
audio.releaseSource(prepared.resource); // voice retains its admitted source lease
// A new playSourceAsync call now needs another live prepared resource.
```

prepareSourceAsync returns {resource,info}; its temporary preparation receipt is
released automatically. Each playSourceAsync creates an independent decoder on
workers and returns the ordinary VoiceHandle, supporting existing spatial/effect,
seek and transport APIs. Releasing the resource blocks future lookup, while
admitted/live playback keeps encoded bytes until the actual last lease drops.
Handles/tickets are client-scoped, ephemeral and invalid after restore. Known
source_frames include an optional frame limit; unknown physical length stays
null unless measured. Preparing may scan/discard decoded chunks when requested;
it does not retain a full PCM track. Header probing is not full-file validation.
See [ADR 0077](../adr/0077-prepared-audio-resources-and-native-replay.md) for limits.

### Hierarchical audio gain controls

```ts
audio.setMixGroup("game", {gain: 0.8});
audio.setMixGroup("dialogue", {parent: "game", gain: 0.7});
const voice = await audio.playSfxAsync({asset: "line.ogg", mix_group: "dialogue"});
audio.setMixGroup("game", {gain: 0.2}, 250); // output-clock fade of the parent
audio.setVoiceMixGroup(voice, null); // retain source position, bypass groups
```

Groups are client-scoped logical controls in both pooled and native/media mixers.
Updates retain omitted fields; parent:null detaches, and null gain/mute is invalid.
A group has its own gain/mute ramp; ancestor, voice, bus and Master gains remain
separate. The limit is 64 groups/client, 256/mixer and depth eight. Parents must
exist, cycles fail atomically, and groups with child/live voice/effect references
cannot be removed. mixGroup/nativeMixGroup exposes target settings, next-frame
own/effective gain, fade frames and output rate. Host settings survive playback
clear/restore; this is not an arbitrary effect submix or serialized DSP graph.

AudioPlaybackInfo.age_frames is output publication age, including pause/holds/
starvation/tails, independent of decoded position. New sources reset age; seeks
preserve it. Native global SFX uses it for its integer-ms end predicate, with
explicit frame-phase/timing limits in ADR 0078.

An independent offline demo exports an authored music/UI fade without a VM or
output device:

```sh
cargo run --release -p crysirror-audio --example mix_groups -- /tmp/mix-groups.wav
```

It asserts the independent UI stem during the parent music fade and compares
clear/muted/restored RMS. No fonts or game assets are bundled.

### Worker audio frame completion

Live Rust hosts use App.begin_frame(dt_ms, input), then poll_frame until it
returns true before beginning another frame. frame_pending reports the dependency;
Audio.rendering includes prepared frames awaiting task capacity. Playback, source
and group observations retain the last published boundary. DSP controls fail
while a frame is pending. Completion publishes the pooled/canonical and native/
media mix together; take_output drains it once. Pause defers to completion, and
restore/shutdown discards obsolete audio. Runtime.begin_tick/poll_audio_frame is
the standalone native/media equivalent; tick_control is for an external
coordinator that owns the dependency. Synchronous frame/tick remain available for
offline execution. Device callbacks and device-clock independence remain open:
see [ADR 0079](../adr/0079-worker-final-audio-frames.md).

The independent gain-group demo also supports final DSP workers:
`cargo run --release -p crysirror-audio --example mix_groups -- /tmp/mix-worker.wav --worker`.
It uses the same authored sample assertions as its synchronous mode, without a
VM or output device.

### Host output lifecycle

Rust hosts use crysirror_audio::pcm_output::PcmOutput with cpal-output, or inject
an OutputBackend without a platform dependency. submit admits bounded stereo
48 kHz PCM; call poll/status/watch for opening, readiness and retained failures,
including frames with no audio. begin_close returns a receipt; closed follows
worker resource destruction. Drop requests cancellation without joining the
owner. Explicit retry requires failed/closed completion and creates a new
session. Desktop supplies startup and error polling automatically.

Audio.rendering is the final DSP frame dependency; it is distinct from device
readiness. No reflective signature/fingerprint changes accompany these Rust host
APIs. Device state is derived host state, never a VM save or resource pointer.
Run the optional silent-level device probe with
`cargo run --release -p crysirror-audio --features cpal-output --example output_probe`.
See [ADR 0080](../adr/0080-async-output-worker-and-bounded-pcm-transfer.md) for
library pins, migration and remaining clock/resampling/device-switch limits.


### Prepared voice compression

```ts
const compressed = await audio.prepareSourceAsync("voice.wav", {
  transform: {kind: "siglus_jitan", percent: 200}
});
const voice = await audio.playSourceAsync(compressed.resource, {bus: "voice"});
audio.releaseSource(compressed.resource);
```

AudioSourcePrepareSpec exposes measure_frames, an input frame_limit and an
optional AudioTransformSpec. The named PCM16 profile preserves sample rate,
folds stereo to mono at nonidentity rates and compresses through native splice
search/smoothing. It is separate from playback rate/pitch. Percent 100 retains
original encoded bytes; other rates bake bounded WAVE on the preparation worker.
A native-profile cue below 100 ms becomes same-length silence. Layout/rate/frame/
residency limits and resource ownership remain explicit; failures are retained
in the preparation receipt. No VM or audio device is required. See ADR 0081.

An authored offline demo: `cargo run --release -p crysirror-audio --example
voice_compression -- /tmp/crysirror-voice-compression`.


### Moving acoustic meshes

```ts
await audio.prepareAcousticsAsync('room.json');
const door = world.spawn({name: 'door'});
audio.bindAcousticMesh({mesh: 7, entity: door});
door.position = [3, 0, 0];
// Inspect audio.acousticStats().instances for capture/publication and errors.
// Detach before explicit editing; unbind retains the last published pose.
audio.unbindAcousticMesh(7);
await audio.updateAcousticsAsync([{id: 7, enabled: false}]);
```

Bindings use the entity's global matrix times local_transform (identity). Optional
follow_visibility includes ancestors. Pose batches retain local mesh BVHs and
immutable old query snapshots; disabled meshes retain memory and triangle IDs.
One automatic job/client coalesces at 50 ms minimum submission intervals. Missing
entities disable their mesh with a diagnostic; invalid poses keep prior geometry.
Clear/restore/full scene replacement remove derived bindings.

A revision changes for every pose commit; scene_generation changes for full
scene replacement. Automatic direct/reflection results can lag a moving instance
within the same generation. acousticVoice and reflectionsVoice.tracking report
current_revision / geometry_stale; reflection snapshots also expose capture age.
This avoids repeated cancellation while geometry moves. Full replacements and
latency changes retain strict invalidation. See ADR 0082 for bounds, source
audits and remaining deformation/prototype/refit/age-policy limitations.

The device-free worker demo exports authored closed/open/closed audio and checks
corresponding echo visibility: `cargo run --release -p crysirror-audio --example
moving_acoustic_door -- /tmp/moving-door.wav`.


### Running transport and resume policy

```ts
audio.scheduleVoiceResumeWithOptions(voice, 200, 30, {policy: "preserve", reset_gain: true});
const state = audio.playbackInfo(voice);
// transport_frames counts running input intent, excluding fully paused input.
// stopping/pausing distinguish the two fade completion policies.
```

Existing three-argument scheduleVoiceResume calls retain freezing behavior.
WithOptions takes policy:"freeze" (default) or "preserve". "preserve" continues
the prior transport/pause fade until the output-clock deadline. Input EOF cancels
the action even while effects drain; scheduling on ended input leaves tails alone. Both policies resume PCM on the next
frame after the last waiting frame. Pause/stop/resume replaces an old schedule.
Decoder starvation/seek/effect preparation count running intent but do not advance
decoded position; fully paused/delayed input and effect tails do not count.

Stop completion can retain a silent voice after input EOF until fade_out_frames
reaches zero. A later pause fade does not change that stop deadline.
Audio.voice.playing includes that state; source_playing stays false.
Existing voice and 600000 ms duration budgets apply. All inspection stays frozen
at the previous publication while a final DSP worker frame runs. Rust clients
use AudioResumeSettings and AudioRequest::ResumeWithOptions. reset_gain:true
restarts the gain ramp from zero to its latest requested target at the deadline;
false preserves the gain ramp and applies the fade to transport. Unknown options
fail. See ADR 0083.

The independent authored release demo validates every PCM sample and exports a
stereo WAVE: `cargo run --release -p crysirror-audio --example transport_timeline -- /tmp/transport.wav`.


### Moving-source Doppler

```ts
const voice = await audio.playSfxAsync({
  asset: 'engine.wav', follow: movingEntity,
  spatial: {doppler: {factor: 1, speed_of_sound: 343.3, smoothing_ms: 20}}
});
// Missing velocity is captured from entity/global or camera/manual-listener poses.
// Explicit spatial.velocity or setListener({...,velocity:[x,y,z]}) uses m/s.
audio.setMotionSettings({max_interval_ms: 250, teleport_distance: 50});
audio.resetMotion(); // next owner capture anchors automatic movement again
const state = audio.dopplerVoice(voice);
```

Doppler is opt-in, independent of authored rate/pitch and source position. World
units are metres. The direct radial target clamps scaled radial speeds to +/-99%
of sound speed, then pitch to [1/max_ratio,max_ratio], with separate diagnostics.
Output-clock smoothing continues while input is paused/starved. target is desired;
ratio/smoothing_frames describe installed interpolation and target_pending signals
render setup still needed. effective_rate excludes source/output sample-rate units.
Motion reasons distinguish inferred, explicit, initial, zero-time, gap and teleport
captures. Unchanged zero-time poses retain velocity; changed ones reset inference.
Reset/restore clears derived histories while host motion policy persists.

Pitch uses the existing linear source resampler. Downstream HRTF/echo/reverb
histories retain output-clock behavior; reflected paths do not get independent
Doppler and direct propagation delay is not added. Controls fence final DSP worker
frames; inspection stays frozen. Rust calculate/MotionTracker/Mixer APIs work
without App or any VM/device. See ADR 0084 for physical and async limits.

An authored release demo asserts crossing frequencies and exports a two-second
pass-by: `cargo run --release -p crysirror-audio --example doppler_passby -- /tmp/passby.wav --worker`.

Cue sequence declarations/defaults come from audio_sequences_api.rs; device-free
handle/emission/info fields come from core/service/audio_sequence.rs. Audio's six
cue-sequence operations share the native HostServices bank. Clocks are explicitly
advanced by one caller; poll/ack model asynchronous resource dependencies and do
not automatically start/stop voices. ADR 0085 documents limits and restore policy.

captureCueSequence returns CueSequenceCursor only after dependency completion;
restoreCueSequence validates it against the declaration and creates a new identity.
Cursor fields come from the public engine wire declaration and Core DTO. Native
snapshot staging uses the same pure validation without an audio implementation
in the VM. Cursor restore does not implicitly recreate voices; see ADR 0086.

Input's opt-in journal methods share Core InputSample/InputReceipt/InputBufferSnapshot
and Scene's transport. Journal JSON uses decimal strings for all u64 fields,
including masks, times, sequences and epochs; native binary snapshots use u64.
Echo receipts directly and use BigInt for full masks in JS. This does not change
unrelated numeric APIs. The schema and ingress/acknowledgement boundaries are in
ADR 0087. Current-frame action bindings are independent of journal consumption.

## Worker image effects and synthesized audio

Use `await bitmap.applyEffectAsync(slot, effects)` for a validated `ImageEffect[]`
chain, `transitionAsync(slot, other, options, mask?)` for snapshot transitions, or
`rowCopyAsync(slot, source, options)` for preserved-edge sine row copies. The
underlying begin/effectStatus/cancel methods expose the same owner-polled jobs
without JS. Destination edits invalidate an older pending result. See
`examples/shared-effects/showcase.ts`; its checker, mask and vector geometry are
self-authored through public APIs and need no game assets. These are CPU image
products usable by ordinary retained layers, not GPU shader-pass descriptors.

Existing `Audio.prepareSource`/`playSource` also accept schema-1 `.opna` register
scores and bounded PMD-M `.pmd`/`.mmm` streams. `RegisterScore`/`RegisterWrite`
describe exact output-frame events. `cargo run --release -p crysirror-audio
--example chiptune -- /tmp/crysirror-chiptune` generates its own register asset and
renders two seconds through the shared worker mixer. No ROM/sample bank is
included. Unsupported chip/driver modes and bounded seek rules are in ADR 0089.


## Authenticated container resources

ADR 0091 documents the optional HXV4 profile in `kirikiri/xp3`, with an explicitly
scoped `hxv4_executable` companion and optional `hxv4_startup_target`. Live callers
use `beginDecode(...,"archive",options)`, poll readiness and then `mountDecode`;
requested-name hash lookup survives that worker publication. Cipher state and
keys are not serialized into project/save JSON. Native archive URI handling only
refers to already installed client mounts.

ADR 0096 documents the CXDEC profile in the same format: `cxdec_modules` names the
capability-relative EXE/DLL/TPM companions from which content-filter and Special
filename parameters are recovered statically at mount time and validated against
the archive's own checksums. No parameter is passed by the caller or persisted.

`tools/probe_krkr.py` (`--protection hxv4|cxdec`, `--module` for CXDEC companions)
is an offline diagnostic over the same Resources API, using
a temporary project and explicit local inputs. It preserves normal asset-root
containment and records native failures. A zero renderer exit, blank frame or
plugin file inventory is not a whole-game result. JSON document grammar and
solid vector geometry also have public Rust entry points in Runtime, independent
of the optional native VM; their bounded native facades are in ADR 0092/0093.

## Isolated layer transitions and native video

`Layers.beginCrossfade(id, source, durationMs)` isolates two equally sized sibling
subtrees, including their ordinary image/text/canvas/puppet draws.
`crossfadeStatus(id)` reports `{source,duration_ms,elapsed_ms}` through the engine
clock; `endCrossfade(id)` releases the effect without exchanging the source data.
See ADR 0094 for validation and the native adapter's separate exchange policy.

Rust callers can decode MP4 through `crysirror_video::isobmff::Mp4Stream`; the
existing Media API routes AVC, HEVC and AV1 to the same bounded video worker.
This is CPU software video decode. MP4 audio tracks require audio to be disabled
until an audio decoder is connected; no HDR tone mapping or hardware video decode
is claimed. Exact dependencies, supported sample profiles and reference-pixel
verification are recorded in `crates/crysirror-video/README.md` and SOURCE.json.

Additional shared Rust APIs include `runtime::text_storage::encode_lines`,
`ClientStorage::metadata`, `puppet_eluna::motion` model/clock/worker interfaces,
validated `PuppetFrame` publication and `LayerSystem::set_dynamic_puppet`.
Native plugins translate their arguments and completion semantics to these
services; they do not own a second general-purpose renderer.

`Canvases.apply([{op:"copy_subtree", source, destination, rect, position, clip}])`
freezes a static layer subtree as one `Paint::Snapshot {size,layers}` draw. The
renderer caches an offscreen product at the source resolution, so outer opacity
and scaling affect the whole snapshot. This first profile admits solid drawing
and verified immutable engine PNG resources. Bounded clipped groups can use
nested isolation snapshots so overlapping children receive group opacity once.
Uncaptured fonts and unsupported dynamic content still fail. See ADR 0094 for resource readiness
and lifetime requirements; it is not a general CPU pixel-readback service.


### Device-free engine mesh captures

`crysirror_puppet::cpu_snapshot::{MeshSnapshot, SnapshotTexture}` captures ordinary
engine `PuppetFrame` declarations from immutable RGBA resources. `render(size,
check_cancelled)` is a pure worker job; the owner publishes the result for that
immutable input. `capture(size)` is a blocking offline convenience with one cached
output. Replacing an Arc-backed texture never changes an already captured input.

This bounded CPU profile supports indexed triangles, interpolated UV/color,
clamped bilinear sRGB texture sampling, linear premultiplied blending and the
engine Normal/Add/Subtract/Multiply/Screen modes. Stencil masks fail explicitly.
Target working memory, source residency and triangle coverage have explicit limits.
The standalone tests in crysirror-puppet need no VM, device, font or game asset.
General GPU-reference parity has not been established. The RFVP facade uses the
presenter's client-scoped GPU mesh history when available, with this CPU capture
as an explicit reference fallback; native thumbnails remain explicit readbacks.
These Rust host APIs do not add reflective script signatures or change the
`26edfb80137ff54a` fingerprint.

The shared CPU image API also owns `wipe_mask::generate` (deterministic grayscale
transition masks); its output can feed `associated::map_coverage_alpha` without
a native VM. `video::omv::{header,packed_len,to_rgba}` validates native OMV plane
layouts and converts them to ordinary engine RGBA. NWA remains available through
the validated Audio decoder/source API, with lazy capability-reader access and
bounded blocks. These implementations no longer link the Siglus interpreter.
Native form/syscall mappings and real-game compatibility are separate concerns;
these Rust additions do not change the reflective fingerprint.


### Shared native movie resources

`HostServices::media_frame(client,id)` returns an immutable engine-published
`MediaFrame` with RGBA, dimensions, PTS and revision. Its clones retain the
engine memory lease; foreign/missing identities fail, loading has no picture,
and Stop releases the resource's local picture. Native adapters do not create
decoders or playback clocks. Reader-based ASF/WMV/WMA and OMV/Theora/Vorbis run
on the existing bounded Media workers, including cancellation and error
publication. Audio uses the common float PCM builder/mixer. Native resource
lookup, frame placement, return values and wait/skip semantics stay in adapters.

Current reflective fingerprint: `61f9f936ff0989a4`. The latest additions include
owner-clock voice release, measured passive tooltips and exact bounded canvas
pixel queries. Shared data also documents frozen Mesh paint, XML and sprite
cursors. RealLive exposes its portable-host optical-disc policy explicitly;
continuation under that policy is not disc authentication. Earlier additions
include menu enable state, native menu templates, font jobs and Layer/UI motion.
Media picture publication itself remains a Rust host API. See the adapter
READMEs and STATUS for supported native profiles and real-game results.

Earlier Rust-only additions include
`crysirror_scene::animation::ScalarTween`: validated finite endpoints,
explicit monotonic millisecond clock, shared `Easing`, `sample`, `complete`
and continuous `retarget`. Logical interpolation state is serializable and
retains no host/VM/device reference. `image::cursor::decode_cur16m` feeds
ordinary client-scoped animated cursor resources. `image::crx::decode` returns
straight RGBA and native origin metadata; the default image registry also
recognizes CRXG content. Audio `decoder::open_memory/open_reader` recognize
XPCM mode 0 PCM and mode 5 Ogg, using a bounded seekable payload view and the
existing packet decoder. XPCM spectral transforms 1/3 remain unsupported.
`crysirror-circus::Program` preserves native MES operands and command boundaries.
The registered Circus adapter executes a bounded original DC4/DC4PH native
profile using shared engine image/layer/UI/audio/VFX/storage services. Unmapped
instructions stop at their original PC; registration is not full compatibility.

`crysirror_image::color::{srgb8_transfer, Srgb8Transfer}` exposes immutable
batch colour transfer data; convenience `srgb8_to_linear` and
`linear_to_srgb8` use the same checked byte-quantization boundaries. Shared
CPU scene capture reuses this independently tested module. Pixel parity
against the prior transfer expression is separate from game performance.


### Shared sprite resources and installed-game bootstrap

`ImageEffect::SpriteAlpha` uses `SpriteAlpha` to normalize a decoded sprite
resource (preserved/opaque coverage, exact corner/colour key or paired mask).
`ImageEffect::StripeReveal` generates generic stripe/stagger threshold masks;
native effect numbers and their policy remain adapter data. Both run on the
existing bounded image workers and publish ordinary engine resources.
`crysirror_scene::sprite::SheetSequence` selects a horizontal cell and UVs from
an absolute engine clock, independently of any VM or renderer.

Rust native clients can query `EngineQuery::UiInteraction` for the shared
widget's hovered/focused/pressed state. `ImageCompositeLayer.clip` constrains
a destination rectangle while retaining source sampling phase. Canvas RGB-only
copy preserves destination alpha and must not discard hidden source RGB. These
capabilities do not require an NScripter or KiriKiri instance.

Desktop `play <game-folder>` builds its typed native project and archive plan
internally. `--detect-only` prints that plan; `--headless --frames N --capture
image.png` uses the same native adapter and worker-mounted archives without a
window or audio device. No temporary bootstrap TypeScript or bulk extraction is
required. See docs/NATIVE_ADAPTERS.md for profiles and validation limits.

Rust hosts also have `crysirror_resource::sniff::FileDetector`: `new()` acquires
the verified immutable embedded `pure-magic`/`magic-db` rules, and `identify`
reads a caller's `AssetReader` prefix of at most 64 KiB. `identify_bytes` takes
an already-owned slice with the same bound. The detector has no filename hint,
external rules dependency or libmagic FFI. Clones share read-only rules across
workers; each call owns its reader/slice and result. `None` means no specific
rule matched, and a positive MIME/description remains identification evidence,
not file validation or a decoder-support claim. `prefix_truncated` records a
bounded sample of a larger input. Desktop engine detection uses this API on
its boot worker in addition to native metadata and archive/profile validation.
The pure Rust library API does not add a reflective script signature.

### Client-scoped mesh frame history

`HostServices::mesh_frame_history(client)` optionally leases a
`MeshFrameHistory` capability from the presenter's provider. `texture(layer,
size)` reserves the layer's logical previous-picture resource and returns a
canonical engine path; publication rejects an unreserved reference from that
client. `readback(layer, size)` is an explicit blocking capture fence for native
save/thumbnail jobs, and fails before a completed picture exists. Device handles,
capability leases and captured GPU state are never native save data.

`Renderer::mesh_frame_history_provider` installs the presenter before native boot.
`advance_frame_histories` processes immutable mesh declarations once per logical
tick, including headless ticks. Referenced sources are frozen by queue-ordered
texture copies before any new target is drawn. Re-extracting the same retained
`Arc<PuppetFrame>` does not recursively apply feedback. First reads of a reserved
but never drawn source sample transparent black; explicit CPU readback still
fails until that source has been drawn. Layers remain engine resources usable
without RFVP or another VM. Last client lease disposal clears its slots; limits
are eight slots per client, 64 leased clients and 128 MiB of logical target/history
storage. Renderer cache and transient mesh targets retain their existing limits.

The desktop host enables GPU history for windowed and `play --headless` runs,
and for single-image `render` runs. Movie warm-up currently retains the CPU
reference path. `CRYSIRROR_CPU_FRAME_HISTORY=1` selects that reference path for
diagnostics. Normal GPU feedback never maps a buffer; only explicit captures do.
Native thumbnail capture currently remains a blocking fence, rather than an
asynchronous save-completion API. Renderer profiles expose history
`renders`, `copies`, and explicit `readbacks`. These Rust host capabilities add no
reflective JS/TS signature; generation still checks the documented fingerprint.

### Retained glyph coverage effects

`Paint.glyph_run.blur.kernel` defaults to `weighted_blur` for existing records.
`disk_outline` produces circular maximum coverage around a glyph;
`square_face` combines offset glyph faces by alpha union. Outline/face kernels
admit radius at most 16 and level exactly 256; weighted blur retains its existing
radius at most 64 and gain behavior. Each effect is an ordinary retained GlyphRun
with independent color/placement and the same reveal timing as its body run.
`crysirror_text::glyph_coverage` owns the bounded device-independent coverage
operations; the renderer caches individual glyph resources. The public data
example `examples/desktop/glyph-effects.json` uses this without a native VM.

### Image backing extents and addressing

`crysirror_image::padding::pad_to_extent` preserves an image's pixels at the
origin and fills a larger checked extent with an explicit color. Native policies
may choose a power-of-two backing while retaining the image's logical size.
`LayerSpec.sampling` selects independent `address_u`/`address_v` modes:
`clamp_to_edge` (default), `repeat`, or `mirror_repeat`. Clamp preserves the
existing [0,1] source-UV contract. Explicit repeat/mirror axes admit increasing,
finite source bounds with absolute value at most 4096. Non-default addressing
currently requires an image paint; unsupported paints fail validation.

The renderer creates independent sampling aliases over the same admitted GPU
image allocation. Two layers using one asset with different addressing never
mutate one another's sampler. Nested snapshot/composite renderers use the same
image-key and sampling contract. These additions extend public LayerSpec data;
Rust struct literals must initialize `sampling` (normally `Default::default()`).

Rust hosts expose `HostServices::window_focused(client)` as current,
client-checked presenter state: `Some(false)` is an unfocused window,
`Some(true)` is focused and `None` means no window-focus model. Desktop updates
it from winit's initial focus query and subsequent owner events; suspension
clears it. Runtime snapshots do not overwrite current host focus. Native adapters
choose an explicit offscreen policy when no window exists; they must not report
that choice as an observed OS focus result.

`Layers.beginCrossfadeEffect(id, source, durationMs, effect)` starts an isolated
subtree effect using the engine clock. For example,
`{kind:"blur",before:[4,4],after:[8,8],dynamic:false}` freezes its two inputs
for the transition; `dynamic:true` renders live inputs each tick. Radius bounds
are 0..128 per axis, arithmetic is in encoded byte colors and alpha, and one
quantization follows the two-dimensional box sum. A `dim` effect processes a
rule image with grayscale, box blur and optional inversion before a rule wipe.
Progress/parameters, input snapshots, temporary textures and completion belong
to the engine. `crossfadeStatus`/`endCrossfade` inspect/release its state.

Rust `crysirror_scene::timeline_counter::Counter` supplies integer clocks and
range/loop timelines from explicit game/real deltas (`update_time`). Start,
pause/resume, count/range queries and `save_parts`/`from_save_parts` have no VM,
window or device dependency. Native counter IDs, callback invocation and choice
of clock remain adapter policy; the shared arithmetic uses checked-width i128
intermediates for i64 ranges.

`Ui.snapshotLayers(root)` returns independent logical `LayerSpec` records on the
project canvas. A widget ID selects its subtree; null selects the owner's current
widgets, engine menus and dialogs. Current text, state, layout, order and clipping
are frozen in the returned data, with IDs remapped for independent installation.
Window decorations and OS cursors are outside this capture. Native hosts use the
same implementation through client-scoped `EngineQuery::UiSnapshot {canvas,root}`;
unsupported non-axis-aligned or derived GPU state fails explicitly. These layers
can become a shared Canvas snapshot without a VM rasterizer or a GPU readback.

`HostServices::audio_codec_supported(client, codec)` queries canonical names such
as `opus` or `vorbis` in the actual shared decoder registry. Availability does not
imply every channel map or metadata option is supported. Prepared audio resource
receipts expose `has_replay_gain`: this reports source tags, without claiming a
gain policy was applied. An adapter must apply its native policy through shared
audio capabilities or reject tagged material when that policy is unavailable.

The default `Resources` archive registry also exposes `circus/pck`, independently
of a Circus VM or the optional arc-formats package. `Resources.beginDecode`
with archive kind/that format and `mountDecode` use the shared bounded PCK index
and range readers. Native VOICE name/slot policy remains in its adapter.

Widget text-field `text_color`/`text_inset` keep the existing default appearance.
`activate_on_press` is opt-in for buttons, and avoids a second activation at
release. `interaction_only` keeps shared input handling without visual paint;
`keyboard_focusable:false` keeps a pointer button from taking an edit field's
keyboard focus. `WidgetInteraction.activated` and its nonzero
`activation_serial` describe one current-tick activation. Repeated native pumps
can remember the serial instead of inferring clicks from a bounded event queue;
new ticks and input restoration clear the edge.

Rust `resource::text_stream::encode(text, mode, max_bytes)` produces bounded,
immutable text bytes using `TextWriteMode::{PlainUtf16,Cipher1,Deflate{level}}`.
It preserves UTF-16LE BOM, FEFE bit swapping, or mode-2 zlib and its two u64LE
lengths; paths, offsets and atomic publication remain storage-owner policy.
`resource::pe::PeData::parse` validates caller-owned PE32/PE32+ data up to 64 MiB.
`image_base`, `sections`, `sha256`, `rva_slice` and `va_slice` expose validated,
file-backed ranges and identity. Virtual-only padding is not readable file data.
No executable is mapped or run by this parser; native build/table policy belongs
to its adapter.

Shared `Media.status` and native `HostServices::media_info` expose validated
`size` before the first picture is presented, once decoder-header work finishes.
`duration_ms` is a declared container duration when available; null remains
unknown. OMV page/packet hints never substitute for a total-frame count. The
incremental ASF/Theora `decode_video_with_metadata` callbacks publish those
header facts before their first picture over the same caller-owned reader.
Metadata preparation leaves the presentation clock and published-frame count
unchanged. `MediaRequest::Remove` releases the scoped decoder/resource and
surface; `Stop` retains a stopped resource and its reconstruction obligation.

`Media.setClock(id, nowMs)` / `MediaRequest::SetClock` select an external clock
before playback is first anchored, then admit monotonically nondecreasing
samples. The engine computes position immediately from its retained origin and
pause/seek offsets; frame publication has a separate PTS and remains tick work.
Native adapters select an existing game/real clock without maintaining another
video timer. Pause overlays compensate external elapsed time on resume. Unknown
audio streams are prepared normally: a decoded audio track with an external
clock fails before PCM/picture publication until audio-master mapping exists.
Silent streams work even when the caller requested audio. Clock arguments can
be exact decimal strings. `Media.remove` releases media and surface after callers
detach image uses. Up to 16 resources per client and 32 per Runtime can be kept,
with unchanged 128 MiB video/audio byte budgets and bounded decoder leases.


## Client-owned dynamic fonts

`Text.beginFont(alias, asset, faceIndex?)` submits a capability-scoped font job.
Poll `Text.fontStatus(ticket)` from the ordinary host update; pending work never
blocks the engine owner. I/O reads and CPU outline/shaping preparation are
separate bounded worker stages. Ready means the immutable face has been
published into the same catalog used by text measurement, CPU rasterization,
GlyphRun and GPU text cache keys. It does not mean a frame has been presented.

The returned `font.alias` pins the content/revision; use it in text/layer specs.
`Text.resolveFont(logicalAliasOrFamily)` resolves a current owned face.
`Text.releaseFontJob(ticket)` releases its retained completion, and
`Text.releaseFont(handle)` separately removes its published resource. Existing
measurement capabilities see catalog publication, while a different client's
view rejects the canonical alias. Replacing a logical alias retains the old
canonical face until explicitly released; it cannot silently change old text.

Rust native clients use `HostServices::font_library` with the same
FontLoadSpec/FontJob/FontLoadState/FontInfo contracts. A source is a scoped
storage path or immutable byte buffer, and a selected collection index is
explicit. Encoded sources are bounded to32MiB; job and retained-face admission
are additionally bounded per client and Runtime. Framework capture/restore
explicitly rejects pending or retained dynamic fonts until resource
reconstruction is implemented. Original native script saves are separate.
No font binary is distributed with these interfaces.

## Retained Layer and UI motion

`Layers.beginMotion(id, track)` and `Ui.beginMotion(id, track)` retain a shared
MotionTrackSpec over the engine ScalarTween. Layers support x/y/scale_x/scale_y;
UI supports x/y. An omitted `from` uses the current property. Automatic duration
uses engine milliseconds; `external:true` uses caller units and advances only
through `setMotionElapsed(id, property, elapsed)`. Repeated elapsed samples are
idempotent and backward samples fail. Zero duration applies the endpoint
immediately. External units can be native frames without a private VM tween.

`motionStatus(id)` reports engine-sampled values and completion. An existing
target with no tracks returns an empty completed status; a missing target fails.
`endMotion(id, propertyOrNull, finish)` cancels one or all tracks and optionally
applies their endpoints. Replacing/removing the target cancels its tracks.
UI motion advances before pointer hit testing. Track state is serialized with
the owned Layer/UI logical state and validated on restore. Native callers use
LayerCommand/UiCommand and EngineQuery::LayerMotion/UiMotion, with the same
client ownership checks as other engine resources.


`Ui.setHitMask(id, maskOrNull)` (also WidgetSpec.hit_mask) binds a reusable,
immutable one-byte coverage grid matching the widget's unscaled logical extent.
A pointer sample is eligible at coverage >= threshold; threshold1 therefore
selects nonzero alpha. Optional strict_bounds additionally excludes the local
top/left edges. Transparent samples permit pointer input to reach lower widgets;
keyboard focus/activation does not sample the mask. Ordinary unmasked widgets
retain their existing rectangle behavior. Local bytes, dimensions and aggregate
owner/Runtime memory are bounded, and masks are logical saved UI state.
`ImageResource::alpha_region(rect)` extracts checked stored source alpha in the
shared image contract; native adapters do not implement their own pixel loops.

Shared resource format `windows/msi` exposes embedded-cabinet installed files
through the normal archive/worker/mount APIs. Directory components and media
ranges are validated; external cabinets, unknown layouts and decoder failures
remain explicit errors. Individual decoded members retain common memory leases.
The host VFS keeps a hard limit of 256 provider mounts per client, including
RealLive voice banks. No MSI actions or original executable code are run.

The partial first-class Majiro adapter configuration is documented alongside the
other native configurations. System 4 and System-NNN currently expose native
program readers, not runnable VM factories. Their current boundaries are explicit
in the crate READMEs and STATUS.


The current fingerprint is `1dbdf921cc54e571`. `Engine.logs()` reads the same
client ring used by native Output and Engine.log; completion records an owner
append, not file/device output. `Input.setKeyboardLocks` patches only logical
Caps/Num/Scroll state. Physical state is null and control unsupported unless a
future host explicitly supplies that capability. Logical state is reset only
after successful Runtime restore.

`ImageEffect.rgb_lookup` accepts three256-byte tables and `straight_rgba`.
The recognized premultiplied profile is rejected because ImageSurface carries
no association metadata. ImageJobs transform immutable inputs on workers.
`HostServices.publish_image_resource` publishes a completed resource without
owner encoding/copying; paths are unique, scoped and explicitly releasable.
Frozen frames retain their resource lease. Further processing can use
`ImageInput::Resource`; these paths are not encoded VFS files. Runtime saves
with live publications explicitly require unsupported reconstruction.
`HostServices.begin_civil_time` samples actual host local civil time on a
cancellable worker, separately from virtual simulation time.


Shared Rust interaction maps support `RegionMap.copy_from` and `resize`, with
client validation and atomic bounded copies. `clip_region_copy` preserves
source/destination correspondence; zero source holes overwrite old labels.
These are logical interaction identifiers, independent of visible RGBA pixels.
`XmlDocument.children` returns children in document order and checks that its
input node is borrowed from that document; cloned/foreign nodes are rejected.

Host audio loop metadata explicitly declares decoded-frame or reference-rate
coordinates. Publication is atomic and client scoped; playback consults it only
when the caller explicitly requests lookup. Live catalogs reject Runtime
capture/restore until reconstruction is supported. Decode-only shared image
jobs allow16MP resources/compositions; actual image-effect outputs retain their
4MP limit. Jobs retain graph limits and the common512MiB reservation budget;
decode-only scratch omits Gaussian planes that are never used.


`crysirror_text::format::format_one_bytes` is a shared bounded Rust byte formatter,
independent of VM/font/device state. It handles one compatible integer `%d` or
byte-string `%s` at a time. Encoded text is preserved; widths count bytes,
precision is ignored in this profile, and string zero-padding is ignored.
String-value embedded NUL bytes are retained; NUL templates are unsupported.
Width/precision fields are limited to510 and template/output to1MiB. Supported
number zero-padding places a negative sign before its zeros. Unsupported
compatible conversions fail without changing caller state. ByteFormatOutput's
matched field lets a VM preserve its own argument/refcount policy. This is a
Rust host API, with no automatic script formatter or native-adapter registration.


`ImageEffect.piecewise_resample` samples immutable input using explicit16.16
source origins/steps and signed destination steps. Regions compose in order,
with independent /256 weights and an explicit alpha policy. The shared worker
checks both source/destination endpoints, at most64 regions and4Mi pixel writes;
row cancellation never mutates the input. This operation supports quadrant zoom,
mirrored tiles and fixed raster sampling independently of any native adapter.

`ProgressCurve` is a device-free normalized envelope with at most64 ordered
knots and double-precision shapes. Repeated times form right-continuous steps;
sampling reads caller-supplied time and never advances a clock or consumes RNG.
`Canvases.getLimits()` reports the same per-client4096-surface budget enforced
by Core, Scene and Runtime; existing canvas size, node and byte limits remain.


`InputRouteSpec.drag_enabled`, `InputRoutePhase.Drag` and the event's `origin`
model explicit backend drag delivery and bubbling. Hardware input does not yet
automatically synthesize drag events; callers supplying Drag must include origin.


`Canvases.apply([{op: "destroy", id}])` releases an existing owned surface after
all live presentations have been hidden. Destroying a still-presented or absent
surface fails atomically. Previously frozen copies retain their independent
nodes and admitted image/resource leases; destroying the source does not alter
those copies. The same command is available to native adapters and ordinary
engine clients for resource retirement.

`ImageCompositeMath::MaskSeparate256` and bitmap `mask_separate256` expose
independent /256 product truncation with zero/full mask endpoints, preserving
destination alpha. They require opacity255; graph admission rejects unsupported
opacity before starting a worker. This is an explicit stored-byte arithmetic
profile, usable independently of any VM, with existing scope and memory limits.

`Bitmap.beginAlphaClip(slot, source, options)` is a shared two-input image worker
operation described by `AlphaClipRect`. It multiplies stored coverage using exact
integer endpoint rules and optionally clears the exterior or multiplies RGB.
The `self_source` profile deliberately reads the destination in forward order,
including overlap feedback, and requires identical bitmap slots. It shares
client/epoch/revision checks and status/cancellation with other bitmap jobs. It
is a coverage operation, rather than an alpha bounding-box query.
