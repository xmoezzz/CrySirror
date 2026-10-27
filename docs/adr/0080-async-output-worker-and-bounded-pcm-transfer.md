# ADR 0080: asynchronous output worker and bounded PCM transfer

Date: 2026-10-27. Status: implemented bounded profile.
Follows ADR 0050 and 0079; actual results are recorded in STATUS.md.

## Research and dependency choice

[CPAL](https://docs.rs/cpal/0.15.3/cpal/) provides platform devices/streams and
callback-based output. It is the platform boundary beneath the former Rodio
bridge, rather than a replacement engine mixer. [Crossbeam ArrayQueue 0.3.14](https://docs.rs/crossbeam-queue/0.3.14/crossbeam_queue/struct.ArrayQueue.html)
provides fixed-capacity storage and force_push for a bounded ring. CrySirror uses
these library APIs; no device backend or queue implementation is copied.

We also investigated current CPAL 0.18.2. Resolving it failed because the existing
WMV dependency requires CPAL 0.15/alsa-sys 0.3, while 0.18 requires the conflicting
alsa-sys 0.4 native links identity. This is not a successful latest-version
upgrade. The implemented backend pins the already-locked CPAL 0.15.3, with
Crossbeam 0.3.14. Their actual source revisions/file hashes are recorded in
[audio-output-source-audit.json](../audio-output-source-audit.json). Updating the
video/optional compatibility dependency graph is separate work.

## Reusable host boundary

crysirror-audio::pcm_output is now device-independent by default. PcmOutput owns
bounded PCM/control state and a trusted OutputBackend factory; it does not own a
platform stream. First nonsilent submit starts a dedicated device worker, which
opens, builds, starts and destroys its stream on that same thread. No borrowed
VM, EngineContext, window or project data enters it. A backend's stream need not
be Send; only the factory and PCM/control capabilities cross threads.

OpenedOutput retains the backend's stream resource in the worker. A trusted
backend must quiesce its callbacks before its stream destructor returns; close
completion observes that destructor boundary rather than inspecting an OS handle. Platform play
and destruction, including a backend's callback join, occur there. Submission
never opens/plays/destroys a platform device or joins a thread on its caller.
Initial buffer/factory allocation and OS worker creation still occur on the
owner; this is not a realtime owner-thread latency guarantee.

OutputWatch exposes serializable phase, session, pause, epoch, selected format,
queue/drop/reset-discard/render/underrun counters and retained error. Opening is
admission, not readiness. Ready follows the backend's successful stream start;
it is not proof of audible output or the first callback. closed is published
when worker resource destruction/unwind returns. Cleanup panic is a visible
failure, rather than a successful close. Watch contains no platform pointer.

poll reports failures even without another PCM submission. begin_close requests
cancellation and returns a watch immediately; Drop requests closure and detaches
the handle. A blocked platform open/build cannot be forcibly interrupted; it
observes cancellation when it returns. A three-second CPAL build timeout is only
best effort (the pinned CoreAudio implementation ignores it). No owner-side wait
or join is used. Explicit retry requires a failed session whose worker has
released its stream; it creates a fresh queue/session/epoch and opens lazily.
Old PCM, errors and close receipts cannot replace the new session.

## Callback and queue ownership

The shared queue admits 12000 complete stereo 48 kHz frames (a quarter second),
allocated once. At most two seconds of input is accepted per submit. Overflow
keeps the newest complete frames and counts discarded input. Nonfinite values
become zero; the final bridge clamps after engine mixing. The sole callback
consumer never locks the control format/error mutexes, grows a buffer, decodes
assets or executes native VM code. Source/error transfer uses preallocated
Crossbeam queues and atomics. Crossbeam is lock-free, not a promise of wait-free
execution or a realtime deadline. CPAL/OS internals may still lock or allocate,
including platform error processing; the claim is scoped to CrySirror transfer.

Pause emits silence while retaining queued/cached source state. Reset changes
an epoch and drains queued old frames on the owner. The callback discards its
cached interpolation state after observing the new epoch, and rejects stale-tag
frames. Neither operation rewrites an already filled hardware buffer. Packet
storage preserves stereo pairing under concurrent reset/overflow. Misaligned
callback buffers are silenced and return an explicit error without consuming
source frames. Error callbacks write bounded UTF-8 notes, then publish a failure
flag; workers format/store retained diagnostics outside the callback. Closing
while an error is pending must retain that error after cleanup.

Native stereo 48 kHz passes straight through. If unavailable, the device default
uses fixed-state linear interpolation; mono averages left/right, stereo maps to
channels 0/1 and further channels are silent. The backend supports the pinned
CPAL integer/float formats via its sample conversion library. Rates are bounded
at 8–384 kHz and channels at 1–32. This is not bandlimited resampling, surround
speaker layout/panning, ambisonics or device-clock drift correction.

## Default desktop integration and migration

Desktop audio now enables cpal-output directly. rodio-output remains an explicit
compatibility feature, enabling the CPAL bridge and the older synchronous
RodioDevice API; ordinary desktop PCM no longer uses a Rodio Sink. The existing
synchronous AudioSystem remains an explicitly documented compatibility path.

Both desktop core variants poll device failures independently of PCM and final
DSP completion; about_to_wait also polls when no redraw/audio is produced. Device
opening is an explicit startup dependency before another simulation tick.
Elapsed-time/gamepad/input-edge acquisition waits and opening wall time does not
become a catch-up audio frame. CPU worker completion remains ADR 0079's distinct
dependency. Exit requests device closure even if lifecycle shutdown fails.

Rust hosts migrating from the former PcmOutput must poll errors/readiness and
observe the close receipt if they need verified resource release. submit now
admits work asynchronously; it cannot synchronously report a later device-open
failure. Audio.rendering remains final DSP readiness, not hardware readiness.
Reflective Audio signatures, native opcodes/save layouts and logical audio waits
are unchanged; device pointer/state never enters project or VM serialization.

## Validation and remaining scope

Device-free tests use independent sample oracles and genuinely gated backend
opening/destruction, not a registered factory alone. They verify ownership,
readiness, bounded overflow, rate/channel conversion, pause/reset, retained
failure, panic, retry generations, control mutex independence and desktop gates.
The optional output_probe example performs a silent-level real device lifecycle
check; actual platform/callback results must be recorded separately.

Final engine DSP still follows simulation frames. This change does not establish
an independent audio render clock, timestamped command scheduling, cross-mixer
sidechains, latency tuning, high-quality device resampling, default-device switch
monitoring or automatic device-loss recovery. Explicit failure/retry is available.
Windows/Linux source support is not runtime validation; the local macOS probe
cannot establish hardware portability, listening quality or throughput. A short
intentional burst pads its exhausted input with silence; its underrun counter
is not a streaming performance benchmark.
