# ADR 0089: Shared OPNA synthesis and bounded PMD sources

Date: 2026-10-27
Status: Accepted for the explicit profile below

## Decision

Extract the pure YM2608 model and PMD v4.8 driver from the user's pinned UK2
implementation into crysirror-audio, preserving its MPL notice and provenance.
No UK2 VM, Kira output/mixer, device layer, font or game resource is imported.
The source audit records the exact revision and file hashes. This capability is
usable without any native VM through Audio.prepareSource/playSource and the
ordinary decoder, buffered worker, voice, bus and final mix pipeline.

`.opna` is an engine schema-1 JSON register score: ordered writes at exact output
sample positions, explicit sample rate and finite duration. Simultaneous writes
retain input order. Limits are 8 MiB encoded input, 262144 writes, 8000–192000 Hz,
one hour and 4096 output frames per read. The chip uses the exact 7987200/144
clock ratio rather than a rounded internal rate. Independent decoder leases own
independent oscillator/driver state. Partitioning reads does not change samples.

Explicit `.pmd`/`.mmm` names select the bounded PMD-M profile, not heuristic
matching of arbitrary audio. Directory entries and voice reads are validated;
zero directory entries mean absent parts. The shared safe driver retains original
FM/SSG/rhythm event, Timer-A fade and Timer-B tempo behavior. Finished parts keep
release clocks for a one-second tail; looping scores remain live. Seek recreates
the same source and replays on a worker, limited to 120 seconds. This is not a
constant-time random-access or whole-music PCM allocation strategy.

Unknown registers, active ADPCM, unsupported FM3/SSG-EG modes, unsupported driver
commands/modes, malformed offsets and zero-time command loops fail visibly.
Per-read command budgets prevent a malformed score from trapping a worker.
The model covers six FM channels, SSG and procedural rhythm; its rhythm is not a
Yamaha ROM recording. No ROM, external sample bank or proprietary music is shipped.
Silicon-accurate synthesis, every PMD dialect/command and UK2 VM integration are
not claimed. Further native engines must only translate their music requests to
these shared source resources.

## Verification

Implementation, release compilation and behavior evidence are recorded separately
in STATUS.md. Independent tests exercise register timing, partition invariance,
seek replay, PMD output/end, malformed streams and shared Audio resource ownership.
The self-authored `crysirror-audio/examples/chiptune.rs` writes a register score and
renders 96000 stereo frames through the existing worker mix path to WAV; its input
and output are generated outside tracked source assets.
