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
  `LayerCommand`, `CanvasCommand` and `UiCommand` to the API member that issues it.
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
