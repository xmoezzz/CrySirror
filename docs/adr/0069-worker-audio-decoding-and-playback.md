# ADR 0069: Worker-owned audio decoding and owner-published playback

Date: 2026-10-27. Status: implemented profile; validation is recorded in STATUS.md.

## Ownership and completion

ADR 0050 applies to audio opening and decoding as well as acoustic/HRTF work.
The standalone decoder contracts remain in crysirror-audio, not in a VM, device
callback or legacy engine. A BufferedSource has one decoder owner: idle control
state or one bounded task. Workers return immutable PCM chunks and the decoder.
The engine owner polls results; no decoder mutex, task join or per-voice thread
is introduced on this path.

TaskPool::reserve admits a slot before moving a decoder into a closure. Capacity
includes reservations, queued tasks and running tasks. Failed admission leaves
the decoder with its caller; unused permits release their slots. This ownership
rule is reusable outside audio. Final pool destruction retains its existing
join contract and is not a realtime operation.

AudioSource distinguishes Frames, Pending and End, with separate poll_seek and
service boundaries. Existing synchronous sources keep their original exact
read/seek contracts. BufferedSource's synchronous seek fails before mutation.
Nonblocking seeking is an explicit capability; the mixer does not quietly call
synchronous codec seeks through begin_seek. This API does not make all existing
AudioSource implementations asynchronous.

## Streaming and mixing

Worker preparation opens a client-capability reader, probes the existing
standard/NWA decoder and primes PCM. Ordinary stream configuration is 4096
frames per chunk, three queued chunks. Rust callers may select 256..16384
frames and two..four chunks, with at most 65536 regular queued frames. Initial
and custom loop-head prefixes pin two chunks each so loop seeks can use ready
PCM. One source has at most one decoder task. A full task pool applies
backpressure without losing the source or falsely reporting EOF.

Mixer cursors consume chunks without decoder I/O. They retain the preceding
chunk's last sample for fractional interpolation, avoiding repeated backward
seeks at chunk edges. A starved voice contributes silence, preserves its source,
envelope and DSP clocks, and increments underrun_frames; other voices continue.
This policy favors ordered VN/music playback over dropping content to catch up
with wall time. It does not promise gap-free playback under arbitrary load.
MixStats counts voice-output underrun frames, not device callback underruns.

Exact seek sequences freeze the voice until worker PCM is ready. Newer seeks
supersede older generations; stale chunks cannot publish into the new timeline.
Cancel restores the prior logical/fractional position, with asynchronous decoder
realignment if necessary. An already committed seek remains committed when its
result ticket is released. Decoder failures are visible in source status and
render/seek errors; a failed decoder is not silently restarted.

## Public engine surface

- beginPreloadSfx / preloadSfxAsync prepares the shared resident SFX cache.
- beginPlaySfx / playSfxAsync prepares and publishes a normal mixer voice,
  including the same spatial/follow, acoustics and binaural attachment APIs.
- beginPlayMusic / playMusicAsync opens and prewarms a worker-buffered stream.
  Named music is replaced/faded only after successful preparation and admission.
  A newer request supersedes older pending preparation of the same music name.
- beginSeekVoice / seekVoiceAsync targets resident or worker-buffered sources.
- playbackStatus and cancelPlaybackJob use client/restore-scoped tickets.
  Cancel before publication prevents playback; release after publication does
  not stop the live voice. sourceStatus reports the bounded source queue.
- clearSfxCache removes only the caller's cached clips. Live sources keep PCM
  and shared residency receipts until their last reference drops.

There are at most 16 retained audio preparation/seek operations per client and
64 per host, including completed/failed tickets. Admission reserves up to 32 MiB
per cold clip or an estimated 4 MiB per stream against a shared 512 MiB audio
budget; clips shrink their receipt to resident PCM bytes. The SFX cache retains
its independent 128 MiB host limit. Receipts survive cancellation while worker
or playback references still exist and survive restore until those references
actually drop. These are retained-resource estimates, not a process-memory
sandbox: encoded provider storage, codec allocations and transient Vec growth
are not comprehensively measured. Input streams are limited to 512 MiB encoded;
individual capability providers may impose stricter limits.

Realtime hosts require async cold SFX and music preparation. playSfx and
playRandom can synchronously use preloaded clips. Offline tools retain the
previous blocking APIs. This behavior change and generated declarations belong
in docs/api/CHANGES.md. No codec algorithm, whole-frame host or VM engine is
copied for this feature.

## Remaining boundaries

The existing SceneState/native channel opening, native AudioRequest decoder
paths and synchronous native wait/checkpoint semantics are unchanged. Migrating
those requires pending native events and owner completion, not pretending that
accepting a channel request completes decoding. Original adapters remain subject
to docs/NATIVE_ADAPTERS.md acceptance rules.

Final mixing, output/device queuing, scratch allocation, voice/resource disposal
and pool shutdown retain owner/synchronous boundaries. HRTF and propagation
preparation already use separate workers, but this is not a hard realtime mixer,
complete audio async migration, reflection/reverb implementation, new VN opcode
profile or complete game boot. No GPU/device/Windows/Linux behavior is inferred
from offline PCM tests.
