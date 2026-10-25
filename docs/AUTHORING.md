# Optional TS/JS authoring

`crysirror-author` is a Rust CLI/library that runs a trusted, single-file scene
recipe and emits a validated schema-2 project. It uses Oxc 0.75 and Boa 0.20; it
does not invoke Node, V8, Deno or the npm TypeScript compiler. These are a chosen
API baseline, not a claim of the newest versions. The dependency lock records
the resolved patch versions. Oxc's browserslist dependency is pinned to 2.0.0
because the tested 2.3.1 release changes `Version` fields from u32 to u16 and
does not compile with Oxc 0.75's generated feature table.

## Try the arena recipe

From the workspace root:

```sh
cargo run --locked -p crysirror-author -- examples/desktop/arena.ts > /tmp/crysirror-arena.json &&
    mv /tmp/crysirror-arena.json examples/desktop/arena.json
cargo run --locked -p crysirror-desktop -- validate examples/desktop/arena.json
cargo run --locked -p crysirror-desktop -- render examples/desktop/arena.json --output dist/arena.png
cargo run --locked -p crysirror-desktop -- run examples/desktop/arena.json --no-audio
```

The example creates a floor and nine pillars. It is a static arena blockout,
without character movement, physics, animation or combat. Assets are resolved
relative to the generated JSON file, not the recipe file. The shipped recipe
uses the existing empty asset namespace and needs no fonts or game data.

Define `function buildProject()` and return the project object synchronously.
`.js` follows the same contract. Type annotations/interfaces/assertions in `.ts`
are transformed away; type checking is not provided. Syntax, compiler semantic
diagnostics, execution failures, serialization failures and project validation
errors fail the build. A failed CLI writes diagnostics to stderr and no project
bytes to stdout. Shell redirection itself can truncate its destination, so generate
to a temporary file and move it only when the command succeeds.

The profile rejects static imports/exports, dynamic import expressions, TS import
equals and `.tsx`/`.jsx`. No package resolver, Node/browser APIs, async entrypoint,
Promise job pumping, runtime callbacks or game host bindings are installed. Boa's
filesystem module loader is explicitly disabled, including for imports created
through eval. Source cannot directly mutate a running CrySirror instance.

## Bounds and persistence

Source is bounded to 1 MiB; generated JSON to 4 MiB and the existing Project
limits. Boa uses a 100,000 loop-iteration guard, recursion limit 128 and stack
limit 4096. These are not a total instruction/time budget or a heap quota.
Serialization allocates before the output-size check. Only execute trusted
recipes; untrusted source needs separate process-level resource isolation.

Each build gets a new context. Persist and package the validated JSON, not the
JS heap. The normal runtime fingerprint/snapshot rules then apply. Date/random
values can change a generated project, so this tool does not promise deterministic
recipe execution. Source evaluation is intentionally absent from the MCP tool
surface. Existing MCP project inspection/validation/replacement works with the
generated JSON exactly as with hand-authored project data.

This implements authoring scripts only. Optional per-frame gameplay JS, type
declarations for a runtime scripting API and state migration remain future work
described in ADR 0002. The RFVP and Siglus VMs remain independent and unlinked.

## Engine systems data

A buildProject result may now include ClientSpec.systems with dynamic entities,
retained layers and widgets. The same Rust validation runs on that output; the
player ticks those systems after loading JSON, including when the data VM has
halted. See ENGINE_SYSTEMS.md and examples/desktop/systems.json. No per-frame
JavaScript execution is required for movement, tween animation or UI actions.
