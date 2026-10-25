# 0034: Verify Siglus execution separately from shared engine features

Date: 2026-10-25. Status: supported native slice executed; complete game support
and message-window integration remain incomplete.

## Implementation and source boundary

The default `SiglusFactory` in `crysirror-runtime/src/runtime.rs` creates
`crysirror_siglus_vm::adapter::SiglusVm`, the safe migrated subset of the
original interpreter. It does **not** create the complete vendored
`siglus_scene_vm`. The complete reference checkout at
`~/.cache/crysirror/upstream-20261024/siglus_rs` was checked at actual HEAD
`5f358ef1360b0439e92aed2b749e8f25d8721954`, matching `upstream-lock.json`.
The original TEXT/NAME, TIMEWAIT_KEY/down-up and SAVEPOINT paths were examined.
This change adds verification data and tests; no original opcode/value/wait/save
implementation was changed.

The shared engine's advanced text, audio, video, layers and shaders do not by
themselves complete their native adapter hooks. Current `export.rs` explicitly
rejects pending native messages/names, WIPE, masks/tone curves, child/button
objects and some multi-stage/render state. The subset's audio/movie/UI modules
also contain stubs. ADR 0009 documents the full-engine embedding work still
needed. A whole original game is not claimed runnable by this verification.

New reproducible self-authored native fixtures:

- `fixture::key_wait_scene(savepoint)`: original-ABI TIMEWAIT_KEY, optional real
  SAVEPOINT, a rectangle whose X changes after the wait, and a stack marker.
- `fixture::message_scene()`: real CD_TEXT bytecode, exposing the missing native
  message bridge rather than substituting the public Text.draw API.
- `examples/native_fixture.rs`: writes separate original `.pck` projects for
  rect, G00, key-wait/savepoint and message cases to a new external directory.
  The existing `examples/siglus/create_resource_fixture.py` remains explicitly a
  **data-VM resource** example, and is not used as native VM execution evidence.

```sh
cargo run -p crysirror-siglus-vm --example native_fixture -- \
  "$HOME/.cache/crysirror/a-new-native-fixture-directory"
target/release/crysirror trace \
  "$HOME/.cache/crysirror/a-new-native-fixture-directory/input/project.json" \
  --frames 4 --step-ms 16 --press-at 1
```

All fixture scene/image bytes are self-authored. No font or game assets are
read/copied. Outputs, captures and CLI state stay outside the source export.

## Compilation

The fixture example compiled and executed. `cargo check -p crysirror-siglus-vm
--all-targets` passed. Test/CLI logs are `/tmp/codex-siglus-run-*.log`; compact
metadata is `docs/siglus-runtime-verification-results.json`.

## Behavior validation

- Adapter verification tests: **3 passed**. A held DECIDE key does not skip
  TIMEWAIT_KEY; releasing it produces native result 1 and resumes execution.
  The adapter snapshot restores the wait/input history and repeats the same
  edge/result. A real SAVEPOINT exports/imports the original save codec;
  malformed import rolls back diagnostics, and export before SAVEPOINT fails.
  Real CD_TEXT and CD_NAME both report the precise unsupported-message error
  with no shared layer falsely published.
- Existing runtime Siglus tests: **8 passed**. Original arithmetic/control flow,
  Unicode strings, `.pck` metadata, G00 decoding, resource providers, independent
  client IDs, snapshot resource rebuilding and unknown/corrupt input rejection.
- Python native boundary tests: **6 passed**. These are schema/inventory/path
  tests, distinct from native execution and pixel validation.
- Actual release CLI: native arithmetic returns 42; the rectangle moves after
  its 100 ms wait; TIMEWAIT_KEY stays blocked without DECIDE and returns
  `[0,1,123]` with a completed Enter pulse (0 is the initial SAVEPOINT return).
- Actual Apple M5 / Metal raster capture shows the interpreted G00 sprite.
  Compute rendering of the **pure 2D** project correctly rejects absence of a
  3D scene. With a separately self-authored cube backdrop, the same native G00
  overlay renders on the compute path as well. This proves shared composition,
  not native Siglus 3D or hardware ray tracing.
- The actual CD_TEXT CLI exits nonzero with:
  `native message layout must be implemented before CD_TEXT/CD_NAME can render`.

No user game was supplied/executed by this slice. The save check is a
self-authored codec round trip, not byte/pixel equivalence against an original
game or a separately executed original host. Original text layout, audio/video,
native UI/choice behaviour and full native save compatibility are not inferred
from these passing fixtures.
