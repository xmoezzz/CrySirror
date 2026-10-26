# ADR 0070: Native channel audio readiness and shared capability streams

Date: 2026-10-27. Status: implemented profile; validation is in STATUS.md.

## Contract

RealLive's mapped BGM/WAV operations and AVG32's mapped music, wave, SE and
voice events previously read complete encoded assets and opened decoders on
the owner. Replace that bulk work with engine-owned preparation, not an old
engine host or a second VM-specific audio implementation.

The dependency-free core declares AudioAsset (capability path and optional
decoded-frame boundary), AudioRequest::PlayAsset, AudioPreparePhase and
AudioPreparation. HostServices::begin_audio admits a request and returns a
client-scoped ticket. Admission is not readiness. audio_preparation separately
reports Pending, Ready or Failed. Ready means the owner published playback;
a short voice can already have ended when its receipt is consumed. A caller
releases receipts explicitly. Release before publication cancels that request;
release after publication leaves the voice playing.

ChannelAudio owns bounded preparation and one channel mixer, independently of
any VM. It opens source/intro capabilities on the configured I/O pool, uses the
existing standard/NWA decoders, validates declared limits/rates and primes
BufferedSource. Worker preparation pins initial, loop and exact startup
prefixes. Intro/tail frame coordinates, native loop offsets, start_ms,
decode gain, playback gain/rate/pan and native ducking policy use the same
source wrappers/mixer as synchronous channel playback. No sample composition,
codec or output algorithm is added to an adapter.

There are at most 16 retained preparation receipts/client and 64/host, including
ready/failed receipts. Input EncodedAudio is capped at 64 MiB per segment;
capability streams are capped at 512 MiB encoded per segment, with provider
limits possibly stricter. Each real-time request reserves encoded resident
bytes, when supplied, plus an estimated 4 MiB queue/decoder allowance against
a separate 512 MiB channel budget. The receipt follows worker/stream ownership.
Provider allocations, codec peaks and process memory are not comprehensively
measured. Offline direct decoder playback retains its prior residency boundary;
this is not a global memory sandbox.

Latest accepted preparation takes precedence for a channel. A stopped or
superseded pending request cannot publish; its receipt reports failure when
collected. A failed replacement leaves the existing live voice intact. Stop
invalidates unpublished replacement work. Synchronous gain/pan/pause/loop
controls require published playback; pending preparation is reported as busy
instead of silently applying controls only to the outgoing voice. restart:false
can return an immediate successful receipt without reopening an active channel.
The old submit_audio remains explicitly synchronous for encoded requests;
PlayAsset requires the preparation API.

## Native continuations

RealLive retains the original pending VmEvent and does not call Machine::ack
until preparation succeeds. Playback waits are installed after publication.
The adapter polls its ticket before interpreting later bytecode, so it does not
repeat the resource request or count the opcode again on every pending frame.

AVG32 retains its original portable AudioEvent. A pending preparation yields
before more Scenario instructions execute; repeated polling reuses its ticket.
Wave-channel mappings commit only when ready. Preparation failure leaves the
native event/instruction continuation unacknowledged, rather than running later
arithmetic or skipping its sound wait. No original parser/opcode source changed.
The previously unsupported native operations/save formats remain unsupported.

Runtime configures the shared I/O pool and realtime/offline mode before VM
steps, polls completions, and renders ready channels afterward. Full shared worker pools retain bounded
Pending requests in OperationTable::submit_deferred; owner polling reserves a
slot before moving queued inputs into tasks. Releasing a queued request runs
no I/O. Retained-result and memory admission limits can still fail explicitly.
Offline callers
can receive immediate prepared products through OperationTable::insert_ready,
which enforces the same owner/admission/retention rules. Runtime restore clears
channel playback, PCM and pending receipts only after validation commits; late
worker results cannot revive the previous runtime's channels. Cancellation does
not join a worker; final task-pool destruction retains its existing join boundary.

## Shared I/O layering

The existing AssetIo read-at to standard Read/Seek cursor moved into a small
crysirror-io crate over core and std. Resource reexports its existing AssetIo
path; audio wraps the same cursor for MediaSource. Modern Audio preparation
also uses decoder::open_asset. This avoids making resource depend on audio or
maintaining two independent cursor algorithms. Core/compat gain no std, device,
GPU, window or unsafe dependency.

## Evidence and limits

Source review revalidated all 90 reference RealLive Rust hashes at
5f358ef1360b0439e92aed2b749e8f25d8721954 and inventoried 31 AVG32 Rust sources at
256c398e06fbcbae8e956eb046c831b3d6c8a593. Native sound/event/wait branches were
inspected at those pins. These inventories are not blanket opcode coverage.

Independent PCM tests and original native bytecode fixtures cover worker gates,
retained receipts, failures, cancellation, startup/intro/loop coordinates and
native sound waits. No commercial game, additional native opcode, original save
compatibility, subjective audio quality, device, GPU or other platform result
is established by those fixtures.

Native resource-name discovery/directory listings, VM boot/scenario reads and
other unmigrated image/video operations remain synchronous. SceneState/data-VM
channel mirroring and encoded media audio requests retain their previous
synchronous opening paths. Final mixing/output, scratch/voice allocation and
disposal are still owner work. This advances ADR 0050, without claiming full
engine/audio async or filling the outstanding room-acoustics capabilities.
