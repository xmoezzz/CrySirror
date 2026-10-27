# ADR 0085: Owner-clock cue programs and dependency receipts

Date: 2026-10-27
Status: Accepted for shared scheduling; native mapping follows in ADR 0086

## Problem and source evidence

Original Siglus PCMEVENT is a scheduler rather than a looping voice. Its ONESHOT,
LOOP and RANDOM commands select lines, sample signed millisecond intervals, may
add the decoded source length, and wait for the last scheduling interval rather
than input EOF or DSP tails. Random selection softens repeats by doubling all
other lines' weights; it does not forbid repeats. Original native saves restart
LOOP/RANDOM declarations and discard working cursors; ONESHOT is not restored.
WAIT_KEY releases the script wait without canceling the event.

The complete 472-line PCMEVENT source at pinned siglus_rs commit
5f358ef1360b0439e92aed2b749e8f25d8721954 was read, together with the selected complete
state, wait, PCMCH event-source and save functions. Blob hashes and scope are in
[siglus-pcmevent-source-audit.json](../siglus-pcmevent-source-audit.json).
No original PCMEVENT module, sound player, decoder or renderer is imported.

A VM-private timer and random-selection loop would violate capability extraction.
A one-shot voice completion predicate also cannot express this native event.

## Decision

Core defines device-free AudioSequence declarations, handles, emissions and info,
plus six HostServices methods. crysirror-audio::sequence::Sequences owns the
bounded scheduler and its state. ChannelAudio owns a bank; SharedHost and the
public Audio class access the same bank. Native adapters can consume that
contract without depending on a decoder/device/GPU/window implementation.

Audio.createCueSequence, advanceCueSequence, pollCueSequence,
completeCueSequence, cueSequence and releaseCueSequence expose the same operation
boundaries to Rust, JS/TS and reflective tools. Keys are logical declarations,
not filesystem paths or capabilities. Caller code resolves a key to already
available/worker-prepared engine resources and controls its channel/voice.
Scheduling cancellation never implicitly stops independent voices.

Modes are once, ordered loop and weighted choice. Each line has weight and signed
[min_ms,max_ms) bounds (swapped if reversed; equal means exact, with no random
draw). include_duration adds the acknowledged source length. soften_repeat
halves the preceding line's relative probability. All-zero weights select line
zero without consuming RNG. Uniform rejection sampling is the default. The
explicit modulo32 profile preserves original xorshift32 draw and modulo order;
its modulo bias and inability to span totals above the 32-bit draw range are
legacy properties, not a recommendation for general weighted authoring.

A zero seed is mapped to 0x12345678 at the first actual draw. A caller may inject
a native shared RNG seed on a new due selection and read the updated state after
selection/acknowledgement. Repeated polls of a pending emission ignore seed
changes. Interval sampling happens at acknowledgement, after resource publication.
This prepares a future adapter to reconcile the native script RNG explicitly.

## Ownership, clocks and async dependencies (ADR 0050)

1. Owner control creates a program and explicitly advances it once per owner
   frame, supplying game and real deltas. The selected clock advances only when
   no emission awaits acknowledgement. App does not also advance it implicitly.
2. Poll selects one due cue and commits its line/RNG exactly once. The returned
   monotonically increasing serial identifies the dependency operation. Repeated
   poll returns that same emission, with no clock or budget reset.
3. Caller starts resource preparation and playback using existing bounded worker
   services. Those tasks own immutable resource/decoder work, not scheduler state.
   Pending sequences retain their time while dependencies are outstanding.
4. After successful source/voice publication, caller acknowledges that exact
   handle/serial with the known decoded duration. This installs the next interval.
   A failed cue needs an explicit caller policy: acknowledge zero duration, retry
   the same emission or release the program; failures do not silently succeed.
5. reset timing starts each selected cue at elapsed zero, discarding late time.
   carry timing retains overdue time. Poll/ack may drain immediate/catch-up cues
   up to 1024 per owner advance. A budget hit is observable and cannot be cleared
   by polling. Advancing opens the next frame budget. Negative/zero intervals
   therefore cannot spin a native/JS frame without a bound.
6. Once programs become inactive after the last interval, even if a corresponding
   voice is paused, still playing or has a processing tail. Read-only info never
   selects/acknowledges work. Final DSP-frame gating permits frozen info and
   rejects scheduler controls until owner publication, matching other Audio calls.

This bounded controller does not create a task/thread per timer or do asset I/O.
Frame/delta ownership and async readiness are separate, explicit dependencies.

## Identity and limits

Handles include client plus a monotonic bank generation; emissions have a local
serial. Foreign/stale handles, old receipts and invalid updates fail visibly.
Clear/restore drops state but keeps the generation counter. Bank capacity is 1024
programs total and 128/client; each program has at most 1024 lines, 64 KiB of key
bytes and keys up to 2048 bytes without control characters. Clock advances are
0..60000 ms. Interval bounds and acknowledged durations are bounded to seven
days. Uniform total weight, including optional repeat bias, is <= u32::MAX.
These are validated admission limits, not native format limits or measured RAM.
Inactive programs still consume capacity until explicitly released.

## Acceptance and remaining mapping

Independent Rust cases cover clocks/dependencies, repeat-biased original RNG draw
order, uniform draws, reset/carry timing, once completion, bounded malformed loops,
scope/capacity and stale receipts. Public engine cases exercise the same Host bank,
restore and frozen observations under real final-DSP capacity backpressure.
A JS Promise case resolves an emitted cue through shared resource/voice preparation.
An authored PCM demo checks the sample positions of two actual cues and exports
sync/worker output for independent decoding. Actual results are recorded separately
in STATUS.md; this ADR is not a test-pass claim.

This phase does **not** map Siglus PCMEVENT dispatch, native channel/source/routing
policy, native script-global RNG reconciliation, WAIT/CHECK/WAIT_KEY or original/
runtime save reconstruction. No working native cursor is serialized implicitly.
Native integration must translate those semantic declarations onto this bank and
acknowledge actual shared resource durations, with proper async command commit
and save boundaries. Existing unsupported native forms continue to fail visibly.
No whole-game, hardware, performance ranking or complete-engine claim follows.

Follow-up: ADR 0086 maps the stated original PCMEVENT profile and adds explicit
portable cursor capture/restore. The scope above describes this earlier phase.
