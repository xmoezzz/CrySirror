# Upstream source inventory and evidence limits

Complete Git checkouts were fetched on 2026-10-24 and their actual revisions are
saved in upstream-lock.json:

| Source | Commit |
|---|---|
| RFVP | `062fec82033b554b9eb3b5da069518c0c96a354e` |
| siglus_rs | `5f358ef1360b0439e92aed2b749e8f25d8721954` |

Both checkouts were subsequently checked against their origin URLs, clean work
trees and lock revisions by the fetch tool. No Git submodule entries were found
in either pinned tree. This replaces the prior cached-page-only inventory.
**Acquisition is complete; full semantic source review and native integration
are not complete.** No VM hooks have been implemented. The subsequent native
resource slice below was implemented after reading its complete source files.

## Reproduce acquisition and the manifest inventory

Python 3.11+ is needed (`tomllib`). Keep reference checkouts outside the deliverable
source tree because upstream includes font/game fixture files that CrySirror
must not redistribute. For example:

```sh
python3 tools/upstreams.py fetch --source-root ../CrySirror-references
python3 tools/upstreams.py audit --source-root ../CrySirror-references
```

The actual checkouts for this run are in
`/Users/xmoe/.cache/crysirror/upstream-20261024`. Neither checkout nor its fonts,
fixtures, keys or SDK files was copied into CrySirror crates or packages.
`--source-root` now applies to both fetch and audit. Fetch uses the real lock
commits on subsequent machines; no branch-tip substitution is needed.

`docs/upstream-manifest-audit.json` records the successful static inventory. It
conservatively includes optional, target and dev dependency edges. It is not a
Cargo-resolved feature graph and does not establish that a native build passes.
Its `unresolved_paths` means paths not mapped to packages by the tool's
`crates/*/Cargo.toml` scan, not necessarily absent files. In particular
`siglus_rs/vendor/eluna_rs/Cargo.toml` exists but is outside that scan.

## RFVP observations at the pinned revision

Inspected the manifest and initial host/portable entry points, including
`host_api/{mod,host,clock,event,fs,audio,render}.rs` and
`portable/{mod,runtime,native_bridge}.rs`. This is an initial seam review, not a
claim that all opcode/resource/subsystem/save implementations were audited.

- `RfvpHost` associates filesystem, renderer, audio and clock implementations.
- `PortableRuntime::boot_from_hcb_bytes`, `tick`, `handle_event` and `render_frame`
  are actual source entry points. Existing portable VM/parser/subsystem behavior
  must be compared with the historical interpreter before choosing a port seam.
- `RfvpClock::ticks_us` uses microseconds. The current CrySirror host uses
  milliseconds, so blindly forwarding clock values would change semantics.
- Audio includes encoded/PCM streams, pan, fades, pause/resume and separate BGM/SE
  slots. CrySirror's simple retained play/stop state cannot represent all of it.
- The host render API has texture and render-target resources. The current
  sprite-path contract cannot represent every native rendering operation.
- `crates/rfvp-horizon` and `crates/rfvp-psv` are absent at this revision, while
  `platform/switch` and `platform/vita` exist. Removed the absent console manifests
  from mandatory focus manifests; kept the candidate paths in the report so the
  absence remains visible. CrySirror's console scope remains Switch/PS Vita only,
  both deferred.

## Siglus observations at the pinned revision

Inspected `siglus_scene_vm/Cargo.toml` and initial `vm.rs`, `host.rs`, clock,
resource and original-save entry points. Full VM/runtime/render/text/audio/save
call-path review is still required before implementing hooks.

- The desktop dependency table directly includes wgpu, winit, kira and egui.
  Making dependencies optional alone would not establish a portable VM.
- `SiglusHost` owns `Rc<RefCell<Renderer>>`, `SceneVm`, redraw/pump flags and
  suspended waits. Replacing only the window constructor would miss observable
  redraw/resume ordering.
- The repository contains `original_save.rs` plus VM save submodules. Preserve
  these codecs and the private VM state; the CrySirror envelope is not an importer.
- `platform_time.rs` includes a virtual-clock profile, but no CrySirror clock
  injection has been implemented or validated.

## Extraction decision

Share scoped VFS/stream primitives, logical resources, rendering/audio execution,
input/time services and snapshot infrastructure where behavior is demonstrated
common. Keep each VM, syscall dispatch, decoded values, waits, resource policies,
text layout and original save semantics independent. Extend the shared contracts
with tested capabilities as needed; reject unsupported operations visibly.

Neither complete upstream engine was built, and no original game was run.
Native acceptance gates remain in NATIVE_ADAPTERS.md.

## Subsequent Siglus native-resource port

The complete runtime assets/g00.rs and assets/mod.rs, plus the format crate's
g00.rs/lzss.rs, were reviewed at the pinned revision. crysirror-siglus now carries
the runtime decoder and its public type-3 format transform with MPL provenance.
Shared checked reads, image bounds, color conversion and JPEG decoding moved to
CrySirror services. Native cut/frame/pivot/LZSS policies remain Siglus-specific.

The actual ORIGINAL runtime decoder was compiled and executed through a temporary
resource harness. Three synthetic fixtures matched the port's pixels and metadata;
see docs/siglus-resource-comparison.json. G00 type-3 is separately checked against
shared JPEG output. GPU tests exercise native resource upload and both rendering
paths. This does not establish any native opcode, wait, syscall or original-save
coverage. See ADR 0003 and crates/crysirror-siglus/UPSTREAM.md for exact changes.


## Subsequent shared CPU migration

Read the complete pinned Siglus input.rs, int_event.rs, render_math.rs, layer.rs
and RFVP exit_confirm_ui.rs before migration. Complete input, integer-event and
projection implementations and their existing tests now live in crysirror-scene;
RFVP letterbox/hit helpers and capture/release behavior support the new UI.
World/Layer/UI integration is executed new engine code, not a copy of either VM.
The unsafe/native-bound RFVP GameData and window/device-bound renderer were not
imported into shared CPU services. Exact hashes and changes are in that crate's
UPSTREAM.md and upstream-files.json.

The actual original code was compiled in tools/compare_scene_upstream.py and
175,700 observations agreed with the migration (docs/engine-systems-differential.json).
This compares the selected algorithms, not complete native games or VM behavior.


## Siglus BGM semantic bridge (2026-10-27)

The original Git tree at 5f358ef1360b0439e92aed2b749e8f25d8721954 is locally
available and verified (2519 tracked files). The BGM review covers complete
engine/form/table/container and audio module/hub source, plus selected complete
resource, wait, VM argument and native save call paths. Exact Git-blob hashes and
scope are in [siglus-bgm-source-audit.json](siglus-bgm-source-audit.json). Existing
reference kira_hub.rs opt-in-device changes remain distinct from the immutable
upstream hash. This is a scoped semantic audit, not whole-engine/game acceptance.

No native BGM engine implementation was copied into the new hook. Original VM
execution and codecs remain; new logical/syscall adapters submit CrySirror shared
channel resources/transport. Exact-frame startup, paused/crossfaded publication,
inspection, delayed resume and starvation-independent control fades are shared
engine capabilities, independently tested without the VM. Resource aliases and
bounded decoded Gameexe rows define the explicit initial profile; ADR 0076 records
unsupported resource discovery/audio/forms and remaining async boundaries.
