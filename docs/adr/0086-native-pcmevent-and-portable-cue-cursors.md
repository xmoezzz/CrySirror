# ADR 0086: Native PCMEVENT over shared cue programs

Date: 2026-10-27
Status: Accepted for the explicit resource/default routing profile below

## Decision and source

Build on ADR 0085's independently usable cue scheduler. Original Siglus
PCMEVENT START_ONESHOT/START_LOOP/START_RANDOM, STOP, CHECK, WAIT and WAIT_KEY now
translate to shared HostServices cue operations and existing engine PCM/PCMCH
resource/voice operations. No upstream scheduler module, sound player, renderer,
decoder, random-choice loop or interval timer is imported into the adapter.

The reviewed original PCMEVENT source is pinned at siglus_rs
5f358ef1360b0439e92aed2b749e8f25d8721954. The complete module and selected complete
state, PCMCH event-source, wait and original-save functions were read; the native
clock feed and MATH.RAND are also inspected. Scope/hashes are in
[siglus-pcmevent-source-audit.json](../siglus-pcmevent-source-audit.json).

## Shared additions: portable cursors

Core adds AudioSequenceCursor and pure declaration/cursor validation, also usable
while staging native snapshots without an audio implementation dependency.
Sequences::capture rejects an outstanding dependency. Sequences::restore validates
against the supplied declaration before admission and creates a new generation.
Cursor data preserves clock, line position, RNG state, emission count and spent
frame budget; it contains no resource, worker, mixer or device handle.

HostServices capture_audio_sequence/restore_audio_sequence and public
Audio.captureCueSequence/restoreCueSequence expose this capability independently
of Siglus. Invalid cursor data cannot partially publish a new program. Restoring
scheduling does not reconstruct its already emitted voices. Existing generic
Runtime snapshots do not automatically save arbitrary caller-owned cue programs;
users can explicitly capture/store/restore them through these APIs.

## Thin semantic mapping

- Native lines map their filename, signed/exclusive interval bounds and optional
  nonnegative probability to shared lines. Negative native probabilities become
  zero. Weighted mode opts into the original 32-bit modulo draws and softened
  repeats. Native default real_flag=true and time_type are preserved as real
  owner clock and optional source-duration inclusion. Reversed bounds are accepted.
- Native declaration allocation is bounded to 128 slots, matching shared per-client
  scheduling capacity. Line/key/interval admission uses shared Core validation;
  native keys also require explicit canonical capability aliases. Empty events
  complete at the next native frame, with no dummy sound. Unknown chains,
  opcodes, named arguments and unsupported sound targets fail visibly.
- Target -1 calls global PCM; nonnegative targets call the declared PCMCH slot.
  Each emitted voice uses CrySirror's measured resource preparation, decoder and
  channel API. Native channel/volume and persistent source declarations retain the
  existing PCMCH mapping. Global PCM ignores event-specific category/character/
  duck flags, as the original global play path does. PCMCH accepts default native
  system/category routing; nondefault character/category/BGM duck routes fail.
- START/STOP invalidate the previous shared program in the native operation
  commit batch, rather than leaving it active until a later frame.
- STOP cancels scheduling; its stop_pcm argument separately stops the selected
  voice. An inactive global event does not stop an unrelated global PCM voice.
  WAIT/WAIT_KEY block only ONESHOT programs. Natural completion returns the original
  zero for a key wait; DECIDE down/up returns one while leaving event and source
  active. The owner service keeps ticking active events after the scene script
  reaches EOF; the script itself is not replayed. Event completion is the last
  scheduling interval, independently of
  source EOF, paused transport or processing tails.
- Before each selection the adapter supplies the current script-global RNG.
  After successful resource/voice publication it acknowledges the exact emission,
  then publishes the service's final RNG back to the VM, including an immediate
  final interval that reinitializes the native declaration. MATH.RAND interleaving
  therefore shares the original stream; polling does not draw again.

## Async ownership and frame continuation

The owner captures a list/index/advance position for the native event pass. Each
program advances once, then saves a portable pre-selection cursor and native
semantic declaration. Selection emits an engine-owned dependency token; existing
resource/voice worker receipts gate the VM commit. The adapter does not step script,
consume input or tick the native frame again while resuming that event pass.

Runtime payload schema becomes **6** (no schema-5 migration). A snapshot made while
source preparation is pending contains the pre-selection cursor and frame execution
intent, not any worker or engine handle. Restore replays that cue exactly once,
with the same RNG and already-advanced owner frame. An owner clock stamp prevents
resource dependency time from being charged retroactively on the next frame.
This is the shared readiness policy: intervals start after voice publication;
wall-clock file I/O stalls in a synchronous original player are not simulated.

The VM retains only declarations, derived observations, portable scheduling data
and semantic continuation intent. Handles and outstanding receipts remain derived
adapter/host state and are invalidated by restore. Pure validation stages cursor
shape/budget/index/time consistency before live native state is replaced.

## Original files and boundaries

Original PCMEVENT binary codecs remain unchanged: LOOP/RANDOM save declarations
and restart with a fresh cursor; ONESHOT is discarded. Original PCMCH source save
rules also remain unchanged. Runtime snapshots instead retain working event
intervals/order/RNG (voices still follow existing restart/reprepare policy rather
than saving decoder/sample/DSP history). Explicit original import consumes the
load-completion marker once, avoiding a second cleanup of newly rebound events.

This profile requires explicit config.resources aliases and existing measured
source limits. Native START uses real time. Restored game-clock declarations use
the adapter's current unscaled game clock; native skip-mode 32x acceleration and
broader SCRIPT/SYSCOM control remain open. No asset discovery, nondefault native
routing, complete MWND/save layouts, sample-exact voice reconstruction, full game,
other-OS/hardware result or complete-engine claim follows. New input edges that
arrive while a native audio admission gate is pending are not yet buffered;
explicit pending-native input ownership remains further work. Exact ordering
between original frame-action callbacks and this event pass also remains open;
these straight-line native fixtures do not establish callback compatibility.
The complete upstream
engine remains a reference only; there is no hosted fallback.

## Validation

Independent shared cursor cases, public engine/JS API cases, authored original
Scene.pck cases and existing native audio/PCMCH/save regressions are kept separate
from compilation checks. They inspect actual shared scheduler identities and PCM,
including resource-capacity backpressure with elapsed host time and snapshot
replay. Actual commands/counts/failures are recorded in STATUS.md.
