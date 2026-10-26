# ADR 0072: worker-prepared media PCM and bounded decoder lifetimes

Date: 2026-10-27. Status: implemented bounded profile; validation in STATUS.md.
Follows ADR 0050 and 0069–0071.

## Decision and ownership

Media decoders previously converted their i16 audio to a WAVE byte buffer on a
worker, then submitted encoded audio for the owner to reopen and decode. Replace
that round trip with the shared core `PcmAudio` resource and
`AudioRequest::PlayPcm`. Native VMs still issue only Media/audio service requests;
no generic audio implementation moves into an adapter.

`PcmAudio::new` validates the rate (8–192 kHz), nonempty frame count (at most
16,777,216), and finite stereo samples within ±16. Validation and conversion to
shared immutable storage belong on the preparing worker. Publication clones an
Arc and installs ordinary mixer controls; it does not scan samples or open a
codec. `PcmAudioInfo` reports rate, frame count and sample bytes without exposing
samples or ownership receipts. PCM/leases are derived resources, never save data.

`crysirror_audio::pcm::PcmBuilder` incrementally converts interleaved i16 input,
validating complete frames and a caller-selected bound. Mono is duplicated;
multichannel input keeps the first two channels, matching the previous shared
WAVE decoder. This is not a surround downmix. Normalization remains /32768,
including the signed extrema. `decoder::open_pcm` supplies bounded copies and
constant-time exact seek over that immutable resource. It does not inherit the
shorter resident SFX Clip limit. These APIs work independently of any VM or movie.

`ChannelAudio::submit(PlayPcm)` uses existing scoped channels, gain, decode gain,
rate, pan, bus, looping, start offset and fades. It cancels superseded preparation
through the existing channel control path. Already prepared PCM needs no further
worker admission. Existing encoded/offline `Play` and native `PlayAsset` retain
their explicit contracts. This is a Rust enum extension, not a JS sample-array
upload API or new native opcode mapping.

## Media task boundaries

`MediaManager::play` validates logical options/capabilities and admits workers.
Workers open the client-authorized asset, query length and read through shared
AssetIo. Audio and video share one lazily opened capability/error via OnceLock,
but own independent seek cursors. Replay reuses that capability. The owner only
polls bounded queues and publishes frames/PCM. Missing/open failures now arrive
as Loading → Failed background errors, rather than synchronous open failures
from play; invalid options/authority/admission still reject immediately.

Reserve decoder-worker leases before spawning either worker. The default limit
is 16 starting/live worker reservations (8 resources × 2). Stop/replacement drops
receivers and requests cancellation without joining. A provider call already
blocked cannot be preempted: its lease remains held until the worker actually
exits. Replacements exceeding capacity fail before replacing the current slot.
This prevents repeated replacement from spawning unbounded orphan workers.
A loop-restart admission failure marks the resource Failed and retains emitted
audio controls, including the prior voice Stop, for the next owner drain.
Dedicated streaming video threads retain their existing queue backpressure;
they do not consume TaskPool jobs that would wait while sending full queues.
Inspection reports `decoder_worker_reservations`, not measured CPU utilization.

Media retains its full-track audio decode and 16,777,216 interleaved input-sample
limit. It is **not streaming movie audio**. The 128 MiB sample reservation budget
counts staging samples and an additional final allocation during Arc conversion.
The final receipt lives in PCM itself, surviving queued updates, resource Stop
and live mixer references. Receipts release only when their actual consumers
release them. Allocator spare capacity, decoder chunks/provider storage and
transient overhead are not comprehensive process-memory accounting. Some long
tracks can therefore fail the combined staging/final budget before the nominal
frame maximum. Longer movies need a separate bounded streaming design.

PTS alignment, late-start frame skipping, pause, EOF and loop clocks retain their
existing policy. `audio_started` denotes an emitted playback update, not measured
audio-device latency. Existing media snapshots remain unsupported.

## Validation and remaining boundaries

Standalone PCM tests validate conversion, rejection without partial mutation,
long-track seek, sample-level controls, client Stop isolation and lease lifetime.
Authored MPEG fixtures exercise Media → PlayPcm → shared offline mixer. Blocked
provider probes verify owner progress, shared capability opening, cancellation,
admission limits and transactional replacement. No game assets or audio device
are necessary for these tests; actual results are recorded separately.

Final mixing, owner scratch/control allocation, last-reference disposal, device
output and final pool destruction remain synchronous. No hard realtime, complete
engine async migration, original-game completion or platform/hardware result is
implied. Native discovery/boot/image preparation and acoustic reflections/reverb
remain separate work.
