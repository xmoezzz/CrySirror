# ADR 0087: Logical input journals and native audio continuations

Date: 2026-10-27
Status: Accepted for framed digital/pointer input

## Problem and source review

Native audio preparation returns before the old Siglus adapter processes the
current InputFrame. Key-down/up and short pulses during a worker wait could be
lost. Simply replaying all queued frames before resuming script is also wrong:
a pulse could arrive before the script installs its next WAIT_KEY, or multiple
clicks could collapse into one native stock. The event-pass continuation in ADR
0086 covers that pass, but ordinary audio instructions also need a continuation.

Reviewed the complete 717-line original runtime/input.rs at siglus_rs commit
5f358ef1360b0439e92aed2b749e8f25d8721954, plus the complete selected normal script
step/proc-pump and key-wait notification branches. The shared Scene InputState
already implements native held/down/up/down-up stock and flick semantics. Those
primitives are reused. Source scope/hashes are in siglus-input-source-audit.json.
No upstream input controller/window implementation is copied by this phase.

## Shared engine capability

crysirror-core::input_buffer owns a bounded logical journal. It is allocation-
capable, device/window/audio/GPU-free and has no unsafe code. Native adapters and
public Input use the same private-state kernel by value. The VM cannot implement
an alternative queue by manipulating its internals. crysirror-scene::input_buffer
provides shared serde transport and the InputInbox wrapper for embeddings, rather
than maintaining a serializer/replay algorithm per VM.

One producer submits InputFrame plus a logical timestamp. Core normalizes pressed
from explicit pulses and up->down changes, and releases from held->up changes or a
pulse not held at the end of the frame. A pulse's press precedes its release;
InputFrame has no OS sub-frame ordering information. Idle levels take no extra
slot. Consecutive edge-free pointer motion may coalesce only into an unleased
trailing record; key/button edges and their click coordinates are retained.

read publishes an ordered immutable prefix and a client/epoch/sequence receipt.
Repeated reads do not consume anything; acknowledge removes only a published
prefix after successful routing. Unpublished/stale/foreign receipts fail without
changing state. A producer cannot rewrite published pointer motion. Overflow is
visible and atomic, not silent edge loss. Capacity is 1..4096; native default is
256. Embedding owners supply unique epochs for a reconstructed/replaced stream.

Core has constant-time preflight admission. App checks every enabled client before
pushing any record, avoiding partial ingress on a later client's capacity failure
without copying pending queues each frame. Snapshot/restore validate sequence
continuity, level/edge consistency, timestamp order, capacity and schema. Ordinary
admission does no asset I/O, worker wait or full-history scan.

## Public Input API and exact transport

Input.enableEvents/disableEvents opt into a per-client journal, disabled by default.
Accepted running App frames feed enabled journals once at frame ingress, using the
current owner clock. events/ackEvents publish and acknowledge records; eventStats
is read-only. Existing action queries/rebinding remain independent and still use
current frame/device state rather than replaying the journal into the action map.

captureEvents returns schema-1 InputBufferSnapshot; restoreEvents validates it
before replacing a journal under a fresh epoch. Future observation timestamps are
rejected against the current owner clock. Runtime restore disables public journals
and invalidates their receipts; explicit restoreEvents can restore logical intents.
No device handle or action-map hardware state is implicitly restored.

Journal JSON encodes u64 masks, timestamps, sequence numbers and epochs as exact
decimal strings. Echo receipts directly; use BigInt when inspecting full masks in
JS. Safe small numeric input is accepted, but unsafe large JS numbers fail instead
of losing low button bits. Native binary snapshots keep u64 encoding. This encoding
belongs to the journal wire; it does not silently change unrelated numeric APIs.

## Native semantic mapping and asynchronous ownership

Siglus captures each incoming frame into InputInbox before polling audio readiness.
This preserves ingress while either a script instruction or PCMEVENT pass waits
for worker resource/voice publication. Native code now contains only the existing
VK/DECIDE/message/wait semantic routing in adapter/input.rs. Core owns ordering,
normalization, receipts, bounds, replay state and serialization validation.

A fresh native input pass delivers at most one journal sample and acknowledges it
once. It then runs the normal script pass to a wait/proc/budget boundary before
consuming the next sample. Thus two queued clicks remain two inputs, while one
click is not replayed into a later wait. Script semantics may still legitimately
ignore input outside a matching wait; a journal is not a command to advance every
future wait.

Runtime payload schema becomes **7** (no schema-6 migration). It stores logical
inbox data and a script-pass continuation flag, not worker/device handles. Audio
completion resumes the original script pass before a new input/frame tick. A
snapshot while preparation is pending rewinds that instruction with the same
already-applied input and queued later intents. Restore replays the command and
continues, without consuming a queued pulse before its next wait is established.
The existing PCMEVENT frame continuation remains separate; malformed simultaneous
script/event phases are rejected while staging native state.

Original file codecs remain unchanged and do not gain a journal. Successful
original import discards pre-load queued intents while retaining the observed held
baseline, so old clicks do not leak into the loaded scene or synthesize a new
press for a key already held. Runtime snapshots instead preserve logical backlog.

## Verification and remaining scope

Independent Core tests exercise inferred edges, quick pulses, immutable leases,
motion coalescing, capacity/errors, timestamps, discard and portable replay.
Public engine/real JS cases exercise client/epoch isolation, exact high mask bits,
restore, binding independence and atomic multi-client ingress. Native Scene.pck
fixtures hold real worker capacity, deliver clicks during ordinary audio and event
passes, and snapshot/replay that backlog against original WAIT_KEY behavior.
Actual results and failed attempts are recorded separately in STATUS.md.

This is a framed digital/pointer journal. Raw OS/text/IME, wheel and analog-pad
events, multiple sub-frame transitions lost before InputFrame, historical world
hit testing, hardware device restoration and additional native key mappings are
not supplied. An enabled journal can backpressure/fail its owner on overflow;
there is no unbounded queue or silent drop policy. Other native adapters have not
been migrated by this phase. Exact original frame-action/audio-pass ordering from
ADR 0086 remains open; no callback compatibility, whole-game, hardware or complete-
engine claim follows from these tests. The continuing goal retains those gaps.
