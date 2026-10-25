# 0030: Shared SFX, spatial audio and streaming music

Date: 2026-10-25. Status: implemented; CPU/offline behavior verified.

The owner assigned audio and text work to Codex while another contributor owns
the renderer, scene view/components, mesh clustering, GoW tools and ADR 0025–0029.
This change stays in the audio crate and the engine Audio class, with small
host/VFS integration performed by the parent task. No title-specific renderer
or native VM is replaced.

## Implementation

`crysirror_audio::mixer::Mixer` is safe, device-free CPU code. It overwrites a
caller-owned stereo-f32 buffer at its configured output rate. Decoders, PCM,
filters and device queues are derived resources and never embedded into game
data or native VM saves. Existing `AudioSystem` / `AudioDevice` callers remain
supported.

| Capability | Executed behavior |
|---|---|
| Voice pool | Configurable host capacity (default 64, maximum 1024); per-client sublimits; lowest-priority, oldest voice is stolen first; a lower-priority incoming request fails visibly |
| Voice identity | `{client,slot,generation}`; stopped, stolen and stale handles cannot control a replacement voice; operations reject cross-client handles |
| Random containers | Seeded weighted variants, optional immediate-repeat avoidance, randomized gain and pitch; deterministic request sequence; bounded variants/history |
| Spatial audio | Listener follows world/project camera by default; equal-power stereo pan; linear/inverse/inverse-square or normalized custom attenuation; client-scoped entity attachment includes its generation |
| Occlusion | Manual 0..1 amount or optional physics line-of-sight estimate; gain reduction and a one-pole low-pass with cutoff interpolated from 20 kHz to 800 Hz |
| Buses | Per-client master/music/sfx/voice/ambient/ui gain and mute; dialogue on the voice bus ducks that client's music, with attack/release ramps |
| Music | Streaming source, named replacement/crossfade, sample-frame loop points, fade in/out, pause/resume and exact decoded-frame seek |
| Rate / pitch | Linear sample-rate conversion; playback rate and semitone pitch multiply the resampling step. Both affect duration; independent time stretching is not implemented |

Spatial coefficients and resampling factors are computed once per render block;
the inner sample loop does not allocate temporary maps or perform trigonometric
spatial calculations. The final mix is unclipped linear PCM. Peak statistics
are exposed; output limiting/mastering is a separate concern.

`crysirror_audio::decoder::Stream` wraps pinned Symphonia **0.5.5**, with WAV/PCM,
MP3, Ogg/Vorbis and FLAC features. The implementation was checked against the
complete locally installed crate sources and their official example/API:
[Symphonia source](https://github.com/pdeljanov/Symphonia),
[versioned API](https://docs.rs/symphonia/0.5.5/symphonia/).
No original Siglus decoder source was copied in this step.

The decoder holds one packet (at most 65,536 decoded frames) plus the mixer's
1,024-frame source window. Long songs opened as files are not loaded or decoded
in full. Short SFX can be decoded to shared immutable clips (at most 4,194,304
frames per clip); the engine clip cache has a 128 MiB budget. Channels are
explicitly mono/stereo, rates 8–192 kHz. Unsupported/corrupt inputs fail.

Seeking rebuilds the demuxer from the same already-authorized media source,
then seeks before the target and decodes/discards preroll. This clears FLAC's
buffered parser state and primes compressed-codec history. Ogg/Vorbis encoder
delay and the empty first MDCT packet after a seek are accounted for, so sample
positions agree with sequential decoding. The compressed fixtures verify this
past the preroll window as well as at rewind.

## Engine and host integration

`crysirror-engine/src/audio_api.rs` owns the new reflective Audio class and
`AudioRuntime`. Existing `play`, `stop`, `playAt` retain their SceneState channel
commands and trace/snapshot meaning. They delegate to the previous service class
and are synchronized into the same Mixer as pooled SFX/music, using voice epochs
to distinguish gain changes from retriggers. An ended/stolen channel is not
continually restarted merely because its logical channel record still exists.

The engine updates the mixer after world/native VM and late-update work. It
uses the client capability VFS to open a regular asset file. Provider mounts
retain precedence: where a provider could own the path, the fallback uses the
provider's bounded resident bytes instead of opening an ambient file. That
provider fallback is not claimed to be encoded-file streaming.

`AudioRuntime::take_output()` drains the last App frame's 48 kHz stereo PCM.
The scripted desktop path sends this to the optional audio-owned
`pcm_output::PcmOutput` and does not also start the legacy device voices.
Standalone data/native Runtime host paths retain the prior AudioSystem bridge.
The device bridge has a bounded 250 ms queue; restore resets queued and already
buffered samples using an epoch. It opens no device for an empty/silent project.
These device hooks are compiled; no listening/hardware-audio result is claimed.

New methods are reached through the same Rust/Class/JS/TS metadata registry:

```typescript
audio.setVoiceLimit(32); // this client's limit inside the host's pool
audio.setSeed(42);
audio.setBus("music", { gain: 0.7 });
audio.setDucking({ gain: 0.25, attack_ms: 30, release_ms: 250 });
const bgm = audio.playMusic("explore", {
  asset: "music/explore.ogg", looping: true,
  loop_points: { start: 96000, end: 1920000 } // decoded source frames
}, 1000);
const footstep = audio.playRandom("footsteps", {
  variants: [{asset:"sfx/step1.wav"}, {asset:"sfx/step2.wav"}],
  gain: [0.8, 1], pitch_semitones: [-1, 1], avoid_repeat: true
}, {follow: player, spatial: {min_distance: 1, max_distance: 20},
    raycast_occlusion: true});
audio.pauseVoice(bgm, true);
audio.seekVoice(bgm, 96000);
audio.setVoice(bgm, {rate: 1, pitch_semitones: 0});
audio.stopVoice(footstep.handle, 100);
```

Other methods: `playSfx`, `voice`, `bus`, `setListener(null|{position,forward,up})`,
`stats`. Unknown fields, paths, ranges and stale/cross-client handles fail. A
destroyed/reused attached entity stops its previous voice rather than attaching
it to the replacement generation. Classic `playAt` retains its prior distance-
gain behavior; pooled `playSfx` supplies the full stereo/occlusion path.

## Persistence and current limits

- Classic SceneState channels remain in Runtime snapshots, with their previous
  semantics. After restore their derived decoders restart; an exact legacy
  device sample cursor was never part of that contract.
- Pooled SFX/music handles, their sample cursors, bus settings, random-generator
  state and manual listener overrides are playback policy in this iteration,
  **not Runtime snapshot data**. Restore stops pooled voices and rebuilds
  canonical channels. The project/VM data contains no decoder/device pointers.
- Decode, seek and mixing run synchronously on the App thread, outside the audio
  callback. Bounded buffers are not a hard-real-time guarantee. Large seek-table
  scans or expensive decoder work can still stall; no audio CPU/frame-time or
  underrun-rate result is claimed. The hardware queue uses a mutex per block.
- The resampler is linear, with no high-quality anti-alias resampling or
  independent time stretch. Spatial audio is stereo panning, not HRTF,
  diffraction, room/reverb simulation or acoustic material modeling. Raycast
  occlusion is a binary estimate; manual values can be fractional.
- Crossfades need room for both voices; pool admission/priority still applies.
  There is no codec-level codec-agnostic gapless conformance claim beyond the
  tested fixtures. Native Siglus NWA/OVK semantics were not newly integrated.

## Compilation

The audio crate and the engine API tests compiled on macOS arm64. The optional
rodio-output profile compiled without opening a device. Parent-task workspace,
desktop and no-default compilation results are recorded separately in its final
evidence; this ADR does not infer them from unit tests.

## Behavior validation

- `cargo test -p crysirror-audio --features rodio-output`: **15 passed, 0 failed,
  0 ignored** (3 prior scheduling tests, 3 device-queue tests without a device,
  9 offline decoder/mixer tests).
- The device endpoint clamps the final mixed sample to [-1,1] and substitutes
  silence for non-finite samples. The offline mixer remains unclipped and reports
  its peak. Restore epochs discard both queued and locally buffered old PCM.
- `cargo test -p crysirror-engine --test audio_mixer --test systems`: **8 passed,
  0 failed, 0 ignored** (4 new audio API tests, 4 existing systems regressions).
- Offline assertions cover priority/admission/stale and cross-client handles,
  client pool limits, all gain stages and mute, ducking/restoration, sample-exact
  ramps/crossfade/loop points, pause, seek, resampling pitch/rate, pan/orientation,
  attenuation, low-pass energy reduction and reproducible weighted variants.
- Counted streaming input verifies only a bounded prefix is read for a long WAV.
  Self-authored 0.75-second sine fixtures verify MP3/Vorbis/FLAC decoding, rewind,
  mid-stream seek agreement and decoded duration. ffmpeg is not needed to run
  the tests; fixture generation/provenance is documented beside the files.
- Engine tests exercise two client asset roots and buses, path/unknown-option
  rejection, pause/seek/partial updates, one output for canonical channels plus
  pooled voices, retrigger/restore, music crossfade, random/pool API, camera/entity
  tracking, actual physics occlusion and stale entity generations.
- The previous systems test's `RIFF-placeholder` was replaced with a runtime-
  authored valid PCM WAV: real decoding is exercised instead of bypassing errors.

No font, game asset, proprietary music, external audio recording or SDK is
included. This feature adds no GPU/window dependency to the CPU audio mixer.
