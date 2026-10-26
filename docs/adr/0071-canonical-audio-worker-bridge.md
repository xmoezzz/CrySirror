# ADR 0071: Canonical SceneState audio worker bridge

Date: 2026-10-27. Status: implemented profile; validation is in STATUS.md.

## Shared ownership

SceneState AudioVoice declarations and their logical epochs remain canonical.
Paths, gain and loop flags retain their existing trace/save format; resource,
decoder, task and mixer handles are derived host state. A new SceneAudio bridge
in crysirror-audio reconciles those declarations against a caller-owned Mixer.
The engine App and plain desktop host now call this same implementation. The
engine retains one mixer for its canonical and pooled voices, so their existing
voice limits, buses, ducking and final PCM composition keep their ownership.

SceneAudio uses the native channel service's shared capability opener and
source preparation functions. Realtime opening, probing and decode/priming run
on bounded I/O workers, returning the existing BufferedSource. No new codec,
private VM audio implementation or separate legacy engine is introduced. The
plain desktop path no longer calls AudioSystem::sync/read_for to open complete
assets on the owner. The library's older explicit synchronous AudioSystem and
MixerDevice APIs remain available to offline callers.

## Epochs, completion and limits

Reconciliation processes Stop, replacement and restore before collecting ready
products. A stale result cannot play after its declaration disappeared or a new
Play command replaced its epoch. Repeated Play of the same asset remains a real
restart; gain-only changes preserve the epoch and sample clock. The old voice
can continue while its replacement prepares. Failed preparation retains that
old voice, records a scoped error and returns a visible sync/frame failure.
Existing App frame failures remain terminal; this does not invent silent recovery.

Published epochs are tracked independently of desired epochs. Natural EOF or
voice stealing does not implicitly replay a declaration that is still present.
A new Play or restore is required to restart it. Host routing remains derived
policy across restore; route/gain updates and pause preserve source position.
Pause also applies to products that publish after the pause request.

At most 16 jobs/client and 64/bridge are retained. Extra logical declarations
remain Queued and retry on later syncs. Full shared task pools use the bounded
deferred admission in ADR 0070. A separate estimated 512 MiB bridge budget
reserves 4 MiB per realtime source; leases survive cancellation/reset until the
worker/source actually releases storage. This is not a process-memory limit or
a measurement of codec/provider/transient allocations. Offline preparation
publishes directly, preserving the same-frame canonical limit of 128 voices
without imposing the realtime job-table limit on offline playback.

Audio.channelStatus(channel) reports the caller's desired epoch and derived
stopped/queued/loading/playing/paused/ended/failed state, position and error.
A stopped declaration takes precedence even before the next audio sync. Worker
products publish during frame synchronization; reading status is not a fence.
Decoder failures remain visible even while a valid ready PCM prefix can drain.

Data VM timers keep their original logical clock semantics. A time wait is not
an audio-readiness fence. Clients needing readiness inspect channelStatus; this
addition does not alias any original VM to the data VM or change native opcodes.
Original RealLive/AVG32 preparation continuations remain in ADR 0070.

## Independent execution policy

Runtime::audio_is_nonblocking defaults to the existing Assets policy for
explicit offline/test hosts. set_audio_nonblocking overrides audio independently.
The desktop Run path sets audio async before script initialization and VM
execution, including native projects whose image/scenario strategy still blocks.
The desktop lifecycle reasserts that policy on resume. Runtime inspection reports
the effective audio_nonblocking value. This fixes the native window profile
otherwise opting out of audio workers through the global assets flag.

Realtime cold synchronous modern SFX/music reject through the audio policy,
including onInit. Callers use preloadSfxAsync/playSfxAsync/playMusicAsync as
already specified in ADR 0069. Canonical play remains a logical declaration;
channelStatus distinguishes it from completed loading. Offline render/trace
hosts retain their explicit blocking policy unless they override audio.

## Validation and remaining boundaries

Independent authored PCM tests exercise full-pool pressure, cancellation,
replay/restore epochs, routing/gain, pause, natural EOF, failed replacement,
streaming failure and delayed admission beyond the per-client job limit.
Engine tests execute data VM Play/Wait/Stop and public Audio APIs, including
per-client roots and save reconstruction. Native bytecode fixtures verify that
audio workers remain active with native Assets mode still blocking.

Final mixing, owner scratch/allocation/disposal, output/device queues and final
pool destruction remain synchronous boundaries. Encoded media AudioRequest
submission still opens its decoder on the owner. Native directory discovery,
boot/scenario and unmigrated image/video operations remain open. No complete
engine/audio async migration, commercial-game compatibility, device/GPU or
other-platform behavior is claimed by the offline tests. Reflection/reverb,
diffraction and near-field propagation remain separate acoustic work.
