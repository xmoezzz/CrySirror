# Native adapter coverage and remaining integration

## Current truth (2026-10-25 audit)

The default AdapterRegistry registers `crysirror-script-v1` and `siglus`.
The latter runs the independent subset in crysirror-siglus-vm; it does not
instantiate the complete vendored siglus_scene_vm. Desktop can add the original
RFVP adapter through its `rfvp` feature and BGI through its `bgi` feature
(both desktop defaults). RealLive and AVG32 are not registered. Factory presence is not a complete-game coverage claim.

| Family | Executed coverage | Remaining boundary |
|---|---|---|
| Siglus | Native synthetic SCN/PCK control flow, input/waits, G00 and selected original-save operations in CrySirror | CD_TEXT/CD_NAME fail explicitly; full MWND/reveal/ruby/history, native audio/movie and effects export remain incomplete |
| RFVP | Original VM/software framebuffer connected through the desktop adapter | No-audio profile, no movies; runtime snapshots refused; original save slots retained; one client/process |
| RealLive | External pinned siglus_rs library tests | Independent VM exists upstream; not vendored or registered in CrySirror |
| AVG32 | External pinned siglus_rs library and synthetic Engine tests | Independent VM exists upstream; not vendored or registered in CrySirror |
| BGI | Registered desktop native adapter; original VM emits shared image/glyph plans and channel-audio commands (ADR 0039) | Supported local opening profile; snapshots, host modal requests and some native modes remain unsupported |

Evidence: ADR 0008/0009 and ADR 0034–0036; machine-readable results are
siglus-runtime-verification-results.json, siglus-service-coverage.json and
native-family-audit.json. No installed Siglus/RealLive/AVG32 game was executed
by those synthetic tests. RealLive and AVG32 already exist in the complete
siglus_rs checkout at pinned commit 5f358ef1360b0439e92aed2b749e8f25d8721954;
they are independent VMs, not Siglus aliases.

crysirror-siglus owns native G00 resource compatibility. Its decoder was ported
from the pinned implementation and compared against the original on synthetic
fixtures. Common image/endian/JPEG services live in CrySirror crates. Resource
support does not register a VM. Shared World/Layer/UI and audio/text APIs likewise
need an explicit, tested bridge preserving each original engine's policies.

ADR 0038 adds shared CPU bitmap fill/copy/mask/color-key/nearest-stretch from
the audited RealLive/AVG32 primitives. These execute in crysirror-image, also
reachable through the public engine Bitmap class and existing shared Layers.
Adapters can use the same operations without invoking another engine's renderer.
This capability extraction does not register RealLive/AVG32 VMs. Native pixel
profiles are explicit; source attribution and differential results are in
crysirror-image/UPSTREAM.md. Siglus CENTER_REP translation to shared transform
was also corrected and verified through actual native setters.

ADR 0039 supersedes the external-only BGI integration status of ADR 0037.
Storage, glyph rasterization, bitmap kernels and PCM are shared engine services.
Native layout/wait/save policy remains with the original interpreter; the adapter
does not call its whole-frame compositor. See BGI.md for setup and exact limits.

## Source acquisition

Complete checkouts and actual commit pins were obtained on 2026-10-24. The
initial seam review and manifest inventory are in SOURCE_AUDIT.md and
upstream-manifest-audit.json; full semantic source review is still pending.
Run tools/upstreams.py fetch --source-root ../CrySirror-references to reproduce
the lock outside the deliverable tree, then tools/upstreams.py audit with the
same --source-root. Preserve the actual upstream-lock.json.
Candidate paths in upstreams.json are research leads, not confirmed native API
signatures. Missing paths must be reported and the pins updated intentionally.
Keep the original repositories as reference implementations until behavior agrees.

## The implemented destination API

The new API can be inspected in these actual files:

- crates/crysirror-core/src/vm.rs: VmAdapter and VmYield.
- crates/crysirror-core/src/service.rs: HostServices, FileSystem, InputFrame.
- crates/crysirror-core/src/state.rs: namespaced command/state representation.
- crates/crysirror-runtime/src/runtime.rs: VmFactory and AdapterRegistry.
- crates/crysirror-compat/src: legacy path/profile helpers.

A VmFactory receives ClientId and ClientSpec, returns Box<dyn VmAdapter>, and
must construct a dormant machine without external side effects. engine_id()
must match ClientSpec.adapter. Each machine provides versioned private snapshot
bytes, a validated restore implementation, and budgeted stepping/yield states.
The host handles engine time, files and service command batches. Shared state
never stores original VM pointers or GPU handles.

This document does not invent RFVP/Siglus constructors or instruction formats.
Implement each factory against the real checked-out source.

## RFVP integration limits

ADR 0008 records the existing original-VM, client storage, input/time and
software-framebuffer bridge, plus the native painter fixture and upstream pixel
differential. That work does not provide audio/movie support or runtime
snapshots. Extend audited seams while retaining original save and resource
semantics; do not replace this interpreter with the data VM.

## Siglus: complete VM host and native message bridge remain pending

CrySirror owns Audio/SFX, Text, UI, Layer and World services, including the newer
APIs in ADR 0030–0033. These do not automatically connect Siglus's native calls.
The registered subset's CD_TEXT/CD_NAME operations reject execution. Preserve
MWND/page/reveal/wait, ruby, read state and original-save glyph layouts when
connecting shared services; a generic Text.draw call is insufficient.

ADR 0009 records the complete vendored source and the paused embedding work.
Inspect siglus_scene_vm and actual render/audio/window call sites. Inject audited
host seams while preserving VM/value/coroutine state and behavior. Switch the
factory only after compiled native fixtures show text, accept input and agree
with upstream. Unknown layouts/effects must remain visible errors. Do not
rewrite bytecode into RFVP or the data-program instruction set.

## Required evidence for "merged"

1. Pinned complete source, reproducible dependency lock and successful builds.
2. At least one legal fixture actually executed by each original VM.
3. Matching control flow, waits, input choices and logical resource traces.
4. Pixel/font/layout checks for supported native text and image operations.
5. Audio events, repeat/stop semantics and native-codec coverage checked.
6. Save/reload at defined safepoints, with explicit original-save importer tests.
7. Unknown operations rejected, not quietly skipped or reinterpreted.

Record coverage per engine and fixture. Neither fetching source, registering a
factory name, a hand-authored demo nor Python tests satisfies these gates.
