# ADR 0081: prepared PCM voice compression and original global Siglus KOE

Date: 2026-10-27

Status: bounded implementation; validation results are recorded in STATUS.md.

## Context

The global KOE slot was a volume-only stub. Enabling a native instruction without
implementing its resource, sound processing and observable state would not meet
the engine-first integration rule. Siglus JITAN is PCM16 splice compression with
sample-rate preservation, stereo sum/clamp, integer search/smoothing and a native
short-cue zero-buffer behavior. Multiplying a voice's playback rate is different.

Reference: siglus_rs `5f358ef1360b0439e92aed2b749e8f25d8721954`.
The exact blobs, review scope and remaining native gaps are in
[siglus-koe-source-audit.json](../siglus-koe-source-audit.json). The complete
original numeric implementation and KOE/SFX source were inspected. Relevant
global/config/context/wait/save functions retain their separately stated scope.

## Decision and ownership

- `crysirror-audio::voice_compression` owns a new safe, bounded PCM16 numeric
  profile. It is usable from Rust without a VM, device, window or game resource.
  The implementation shares no original SFX/player/timer/decoder implementation.
- `AudioResourceTransform::SiglusJitan { percent }` is a plain core declaration;
  core gains no dependencies. HostServices offers transformed source preparation
  with the existing scoped audio-resource receipt and explicit unsupported default.
- AudioResources owns input capability reads, immutable bytes, decoder/layout,
  finite PCM conversion, processing, WAVE serialization, memory charges and
  owner publication. Independent decoders/voices lease the resulting resource.
  A VM retains only logical numbers/config/wait state and scoped handles.
- Public `AudioSourcePrepareSpec.transform` accepts
  `{kind:"siglus_jitan",percent:200}` through the existing
  `Audio.beginPrepareSource` / JS `prepareSourceAsync` contract. It has the same
  ownership/cancellation/restore semantics as other prepared sources. Rust users
  can also call the pure PCM profile or resource bank directly.

## Task, dependency and completion boundaries

1. The owner validates declarations and reserves a scoped ticket/handle capacity.
   Live preparation uses the existing bounded asset/I/O preparation pool; the
   worker also performs decoding and compression. It is not a separate CPU DAG
   stage or realtime streaming time-stretcher. Offline callers may explicitly
   choose synchronous resource preparation, as in ADR 0077.
2. The worker owns the snapshot, decoder and PCM scratch exclusively. Cancellation
   is checked during reads, decode blocks and splice blocks. No owner/VM state is
   borrowed. A ready worker product does not publish a resource until owner poll.
3. Owner publication returns an immutable client-scoped resource and measured
   output frame count. Separate playback preparation creates independent decoder
   leases; ordinary shared mixing/output clocks then own sound progression.
4. Canceling an unobserved product publishes nothing. Releasing a resource blocks
   new lookup while admitted/live voices retain their lease. Restore invalidates
   derived tickets/handles; no worker pointer or PCM data enters native save bytes.

Limits: mono/stereo, 8–192 kHz, 100–400 percent and 4,194,304 decoded input frames;
the standalone profile also caps output at 8,388,608 frames. Input snapshots are
at most 128 MiB. A nonidentity transform reserves 128 MiB of additional scratch
against the existing 512 MiB shared bank budget; temporary output overlap is
charged before replacing the input. Final charge shrinks to encoded output plus
8 KiB. Existing 128 host / 32 per-client handle and bounded receipt limits apply.
Unsupported layouts, invalid rates, exhausted budgets and decoder failures fail
visibly. Codec-internal allocations retain their existing decoder limits.

Percent 100 preserves the original encoded snapshot and layout. Other percentages
produce a mono PCM16 WAVE at the same rate; `frame_limit` bounds decoded input
before processing. Cues below 100 ms become same-length silence, matching this
specific native profile. Lossy-codec float-to-PCM quantization is not claimed
identical to every original decoder. This is not a general quality WSOLA backend.

## Native global speech profile

The separate original SceneVm adapter translates KOE / EXKOE play, play-wait,
wait-key, check/metadata, stop and volume controls to the shared Voice bus,
`siglus.koe` mix group and channel 3. Normal KOE establishes the original message
voice/read-flag request for the historical no-trailing-operand ABI; EXKOE updates playback metadata without replacing that
message identity. EXKOE named flags retain native wait/key/JITAN policy. The
selected JITAN SYSCOM SET/DEFAULT/GET commands retain setter clamp 100–300 and
loaded converter clamp 100–400. Other SYSCOM calls remain unsupported.

The original slot stops before new resource preparation, including negative
numbers and failed replacement. Original instruction/stack commit waits for
resource and voice readiness. Failed work retains the original command for a
visible retry, while the already-issued stop remains effective. This is an
explicit error boundary; it does not silently swallow upstream's logged errors.

Explicit `Config.resources["koe/12"] = "voice.wav"` supplies capability-backed
resources. A 24-key `(number,percent)` cache retains engine resource handles,
never VM-private encoded/decoded tracks. It stops admitting cache entries when
full. Newer KOE trailing read-flag operands require missing MWND/read-skip
bookkeeping and are rejected before any audio admission, with a retained
unsupported diagnostic on subsequent calls. They are never marked read by a
fallback. At 100 percent the resource is encoded and is decoded on each play; this
preserves immutable file-removal/replay behavior without claiming the original
decoded-cache performance or implicit native archive discovery.

CHECK/WAIT compares integer-ms measured source duration against shared output
publication age and excludes effect tails, using ADR 0078's frame-phase model.
A completed DECIDE releases a key wait with 1 and leaves the source playing;
natural completion returns 0. It is not exact wall-clock/device timing.

Runtime adapter snapshot schema changes from **3 to 4**, with no automatic old
snapshot migration. It stores logical current cue/rate/volume, reconstructing
derived source/voice resources on restore. Active cues restart at their logical
cue start; decoder/DSP history and source availability are not serialized.
Original Siglus file layouts/codecs stay unchanged: local load stops KOE and
restores its volume, without restoring a live voice.

## Validation and remaining work

Independent tests use authored PCM/WAVE, resource worker gates, cancellation,
scope/lease oracles and the public Audio class with the data VM. A temporary
standalone harness compared complete PCM samples against the pinned original
numeric converter for 784 cases; permanent length/hash vectors retain those
results. It caught an exclusive search-window mismatch before the final pass.
Native tests execute authored original Scene.pck bytes and inspect shared sound,
PCM, waits, metadata, retry and both save paths. No game/font data is bundled.
The release `voice_compression` example exports authored original/compressed
WAVE without a VM or device. Exact commands/results are in STATUS.md.

Native archive/path/append-directory discovery, MWND/KOE_ST, backlog/menu replay,
mouth-volume tables/emote mapping and full per-character/system/BGM duck routing
remain open. Saving replay switches does not implement those consumers.
The complete Siglus engine/game integration and engine-wide ADR 0050 migration
remain incomplete. No listening, GPU, hardware or cross-platform result is implied.
