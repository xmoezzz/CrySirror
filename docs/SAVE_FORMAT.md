# Save envelope v1

A new wrapper format, **not** the RFVP or Siglus original save format. Native
save import/export belongs to the corresponding compatibility adapter.

All integer fields are unsigned little-endian. No native pointer or usize is
serialized.

| Order | Field |
|---|---|
| 1 | Eight bytes `CRYSAVE\0` |
| 2 | u32 envelope version, currently 1 |
| 3 | u32 VM payload schema |
| 4 | u32 services payload schema |
| 5 | u32 length + UTF-8 engine ID |
| 6 | u32 length + UTF-8 game ID |
| 7 | u32 length + UTF-8 content revision |
| 8 | u32 length + opaque services payload |
| 9 | u32 length + opaque VM payload |
| 10 | u32 CRC32 over every preceding byte |

Identity strings are nonempty, at most 256 bytes, and cannot contain control
characters. Decode checks total/section bounds, checksum, format version,
expected identity and absence of trailing data before allocating payload copies.
CRC32 detects accidental damage and is not an authentication mechanism.

Both payload schemas must match the consumer or pass an explicit migration before
restore. A different engine/game/content revision is not silently accepted.
The two payload codecs and atomic whole-engine restore remain to be implemented.

The in-memory host demonstrates file namespace separation and atomic map
replacement only. Actual durable save files require a platform implementation
of `FileSystem::write_atomic`, including platform-specific failure/recovery
semantics and safe path resolution.


Native Siglus runtime payload schema 2 (ADR 0076) includes logical BGM state.
Schema-1 payloads require migration; the envelope rejects a mismatch. Pending
resource checkpoints retain the original uncommitted instruction. Active audio
restores declarations at authored range start after candidate commit; worker
receipts, decoder read-ahead, device cursors and DSP history are never serialized.
Original Siglus file codecs/header layouts stay separate and unchanged. BGM loop/
pause/delay/volume persistence uses those existing fields and shared preparation;
this bounded profile does not establish every original save variant.
