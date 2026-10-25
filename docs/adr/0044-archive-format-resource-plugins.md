# 0044 — Capability archive resources and explicit format plugins

Date: 2026-10-25. Status: implemented first archive families; remaining formats pending.

## Decision

Archive containers are engine resource providers. A native VM, script, image
loader or audio decoder can read the same entry through the shared capability
VFS without first unpacking a directory or invoking a format-tool executable.
The shared resource contract is independent from any game's VM/opcodes.

`crysirror-resource` owns `ArchiveFormat`, `Archive`, `ArchiveEntry`, bounded
`AssetReader` ranges/concatenation, registry, `ArchiveMount`, decode limits,
caller-selected transform chains and client-scoped key providers. Format
plugins own binary tables, filename codecs, segment plans and native metadata.
Transforms have explicit ordering and expected sizes; encryption keys and
per-title profiles are supplied by the caller, not serialized into projects
or copied from upstream tables. XOR/CTR output length cannot authenticate a
correct key; XP3's adlr metadata is not currently checksum-validated.

The optional `crysirror-formats-arc` crate translates concrete implementations
from the complete `vn-tools/arc_unpacker` checkout at actual commit
`456834ecf2e5686813802c37efd829310485c57d`. Its license is
GPL-3.0-or-later, preserved from upstream. The resource contract remains its
separate MPL-2.0 implementation. This arrangement does not assert that source
translation or a crate boundary removes GPL obligations for combined binaries.

## Implemented scope

Five format IDs are registered: BGI ARC, Majiro ARC, NScripter SAR/NSA and
KiriKiri XP3. Raw entries remain random-access subranges; XP3 zlib segments use
the shared transform registry, and NSA LZSS uses budgeted decoded-memory
allocation. Entry offsets/sizes and table/count limits are checked before
resource publication. Unknown formats, unsupported SPB conversion, unsupported
XP3 layouts and missing encrypted-entry profiles fail visibly.

The generated inventory contains all **289** concrete upstream decoder IDs;
only three are marked ported and two partial. The remaining 284 are pending.
The often quoted source count 290 includes the registration helper's template
definition. The inventory is a migration ledger, never a registration list.
See `crates/crysirror-formats-arc/UPSTREAM.md` for exact variants, exceptions,
metadata semantics and verification commands.

## Validation evidence

Implementation, compilation and behavior are recorded separately in the
plugin's UPSTREAM document. Current actual results: the plugin compiles;
seven self-authored behavior tests pass; an explicitly executed ignored
external-corpus test passes over 14 upstream synthetic fixtures against the
original C++ tests' expected outputs. The C++ executable was not built or run.
No game asset, font, embedded title key, proprietary SDK, device or hardware
result is distributed or claimed by these tests.

Follow-on work should port cohesive format groups and update each inventory
entry only after its actual parser/decoder and behavior checks exist. Image,
audio, file codecs and game-specific cryptographic profiles remain pending
unless explicitly named by that ledger.

## Engine integration

The reflective `Resources` class/global exposes installed formats/transforms,
bounded detection, paged metadata, client decode profiles and archive mounts.
The desktop `arc-formats` feature enables the migrated plugins explicitly.
Three engine integration tests generate SAR/PNG fixtures and pass through the
existing Bitmap, Fs, Assets and capability storage APIs. They verify decoded
pixels, scoped host keys, client isolation, mount traversal rejection and cache
invalidation while retaining generated resources. The common resource suite
passes eight tests; an engine unit test checks shared accounting across profiles
without sharing another client's budget.

VFS storage handles share each client's provider table. Handles already held
by a native VM or worker observe later mounts on subsequent operations; opened
readers retain their original source. Provider callbacks run after releasing the
mount lock. Nine runtime VFS tests pass, including late mount visibility across
a worker, isolation, priority, disk fallback and the per-client mount limit.
The TypeScript physics integration test also calls `resources.transforms()`
through the real Oxc/Boa registry binding. See [RESOURCES.md](../RESOURCES.md).
