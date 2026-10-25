# ADR 0001: data-driven core, engine-specific compatibility frontends

Status: accepted; initial boundary code written; native integration pending.

## Decision

Do not merge the two interpreters into one VM, and do not make a desktop rendering
framework the canonical world/save model. Keep native instruction decoding,
values, script stacks, wait semantics and original save importers in their adapters.

Shared services provide retained visual state, audio voice state, input samples,
monotonic time, scoped files and eventual asset handles. Structured commands are
data; their interpretation and validation are compiled code. The first common
model is intentionally small and grows from tested common behavior.

The core depends on neither adapter. Adapters depend on the service API. Platform
backends depend on the API and their SDKs. CLI/editor/MCP are control surfaces,
not owners of simulation state.

## Boundary allocation

| Keep in each compatibility adapter | Share in the engine | Keep in platform backend |
|---|---|---|
| Bytecode/value/call stack semantics | Logical scene and scoped object handles | Device/window creation |
| Script scheduling and blocking calls | Explicit time/input contracts | Controller and touch sampling |
| File/archive naming and decoding | VFS routing, asset handle lifecycle | OS file operations, save locations |
| Legacy text reveal, ruby and line-break policy | Text/glyph service contract and eventual caches | GPU upload and draw submission |
| Original save parser/serializer | Versioned save envelope and transaction boundary | Durable atomic write implementation |
| Original transition/effect behavior | Capability-checked effect primitives | Target shader/pipeline variants |

Do not use each original renderer as a second authoritative world. During
migration, one original renderer may remain as a reference/comparison host, but
only one owner updates a given runtime instance's logical state.

## Text and graphics

A Win32-like abstraction is useful at the level of `layout`, `draw`, `measure`,
resource handles and results. CrySirror does not emulate Win32 or expose its C ABI.
The initial Rust traits are not a stable dynamic-library ABI; add a versioned
C-compatible ABI or serialized IPC only when separately-loaded plugins are needed.

Encoding is decoded at the legacy boundary. The core receives Unicode text and
an explicit layout-profile ID. Equal strings do not guarantee equal glyph
placement across engines. Do not replace legacy metrics until screenshot and
advance-width comparisons pass. No font binary is bundled.

Renderer commands carry logical resources and rectangles, never raw wgpu, GL or
console-native objects. Shared IR does not imply identical shaders or identical
feature availability on every target. Unsupported effects/3D requests must fail
or use an explicit, tested fallback.

## Data driving and state

Examples use JSON only to express a synthetic host-service transcript. They are
not a new scripting VM and do not replace HCB/SS. Real scenes, manifests, asset
recipes and capability profiles can use versioned schemas later.

Logical handles are scoped by an engine-host client ID. Save namespaces also
include engine and game identity. A production host should add user/profile and
content identity where required. Global deterministic ordering is explicit.

The reference `apply_batch` clones service state, validates a transaction and
commits it before any platform calls. It is not optimized for a console frame
loop. Replace this with a bounded command arena and validated diff after
profiling; preserve rollback semantics.

## Safe limitations

`no_std + alloc` means no standard-library requirement for the common crates,
not zero allocation or a proven fixed-memory budget. Reference limits are test
policy defaults, not statements about hardware memory. No thread scheduler,
filesystem syscall, graphics object or nonserializable external handle is part
of a VM snapshot by default.

The host API is a trusted embedding boundary, not a security sandbox against a
malicious native adapter. Logical path validation is not a substitute for a
symlink-safe native VFS implementation.
