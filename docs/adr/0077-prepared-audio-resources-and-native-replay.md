# ADR 0077: immutable audio resources and native retained-source replay

Date: 2026-10-27. Status: implemented bounded profile.
Follows ADR 0050 and 0069–0076. Actual local results are recorded in STATUS.md.

## Engine ownership

AudioResources in crysirror-audio owns client-scoped immutable encoded snapshots.
A resource is distinct from a playback voice, channel or decoder cursor. Core
exposes plain AudioResourceHandle/Info/Preparation data and HostServices methods;
it gains no decoder, device, window, unsafe code or external dependency.
AudioRequest::PlayResource creates a new source from a captured resource lease.
The native adapter does not own encoded buffers, decoders, mixing or envelopes.

Resource preparation opens a client capability on the I/O pool, reads a bounded
encoded snapshot with cancellation between chunks, and probes the shared
CrySirror decoder. Optional measure_frames scans/discards bounded decoded chunks
only when a header lacks a frame count. No full decoded track is retained.
A declared frame limit bounds future decoders; it does not turn an unknown
physical length into an exact measured count. Each playback opens an independent
decoder over the same encoded snapshot on a worker, then prewarms the existing
bounded BufferedSource for realtime hosts. Explicit offline hosts retain the
same preparation receipts with immediate CPU execution.

The bank admits at most 32 retained/reserved handles per client, 128 per host,
and 16/64 retained preparation receipts. Each encoded asset is at most 128 MiB.
An atomic 512 MiB bank budget reserves encoded length plus 4 MiB probe scratch,
then retains encoded length plus 8 KiB metadata estimate. These are admission
reservations, not whole-process RSS measurements or a bound on every codec's
allocator. Playback stream/mixer budgets remain separate. A Vec inside the
snapshot Arc avoids duplicating the encoded payload during publication.

## Completion and lifetime

Only observing a scoped resource preparation status publishes its completed
worker result. Cancellation never publishes an unobserved result as a side
effect. Ready/failed receipts remain bounded until released. Releasing a ready
receipt keeps its published resource; releaseSource invalidates future lookups.
Admitted playback captures its Arc lease before queueing and survives a later
release. Decoder instances explicitly retain the resource. Encoded bytes remain
charged until their actual last lease drops, including after bank clear/restore.
Clearing jobs/resources cannot reset a counter while a worker/live lease holds
storage. IDs use the existing monotonic host operation namespace, with owner
checks and no reuse during a process. Restore invalidates handles and tickets.

The snapshot freezes completed reads. Concurrent replacement during preparation
is not an atomic filesystem snapshot guarantee; size changes/truncation fail,
but same-size writes can race. Preparation validates the header/probe, not every
compressed packet when the source length is already known. Later decode errors
remain visible through existing stream status. Resource bytes and device/DSP
state are never serialized automatically into native/runtime saves.

## Public API independent of a VM

Audio.beginPrepareSource/sourcePreparation/cancelSourcePreparation prepare and
publish resources; sourceInfo/releaseSource manage their separate lifetime.
Audio.beginPlaySource uses existing VoiceSpec, spatial, follow, acoustics,
HRTF and reflection options through the same pooled owner publication path.
It rejects asset in voice options instead of ignoring a conflicting source.
Audio.prepareSourceAsync returns {resource,info}; playSourceAsync returns a
VoiceHandle. JS helpers release their temporary receipts, not their resources.
The resource bank is shared with HostServices/native channel preparation.
Direct Rust AudioResources/PreparedSound are usable without Runtime or a VM.

## Siglus semantic adapter

The original SceneVm, native save codecs and pinned upstream audit from ADR
0076 remain authoritative. Reviewed native prepare/start/stop/resume/retire call
paths retain slot bytes after stop/EOF while STOP clears outer name/loop flags.
No native player, source buffer, decoder or general engine was copied.

A native PLAY/READY now has two explicit completion stages: immutable resource
preparation, then independent channel voice preparation. Before/after VM states
keep the original instruction and arguments visible until successful publication.
Failures release new resource/voice receipts and leave the old prepared slot.
Only successful replacement releases the previous resource handle; old fading
instances keep independent leases. Existing control/wait/table semantics still
translate to shared APIs.

After EOF/immediate stop, RESUME/RESUME_WAIT submit PlayResource from the retained
binding, with initial range, actual loop region, current volume and requested
fade/delay. They never resolve/reopen the original file path. STOP's cleared
outer name/loop values remain distinct from the prepared slot's looping source.
A live paused/fading source still uses shared Resume transport directly.

The ephemeral binding is serde-skipped; runtime payload schema remains 2 and
original file layouts remain unchanged. Active runtime restores reprepare logical
declarations at authored start. Stopped/ended slot restores reconstruct only a
resource, without hidden playback; a later RESUME may use it. Reconstructing a
snapshot still requires its asset alias/files: saves do not embed encoded bytes
or preserve exact decoder/sample/DSP histories. Inactive reconstruction follows
committed VM transport state: a halted VM is not stepped merely to synchronize
subsequent audio EOF, so its snapshot can retain its last active declaration.
Rebinding a stopped resource
must not mark gallery flags as newly listened.

## Remaining boundaries

This resolves the retained-source replay gap documented in ADR 0076. Native
PCM/PCMCH/SE/voice, automatic pack/append discovery, encrypted Gameexe, complete
game compatibility, final mixing/output async ownership, teardown allocation,
dynamic acoustic geometry, Doppler and higher/diffuse reflections remain open.
No device, GPU, Windows/Linux or console behavior follows from CPU tests.
