# ADR 0005: pluggable script hosts — JS/TS by default, native VMs as peers

Status: implemented (partial) on 2026-10-24 in `crates/crysirror-script-js`.
This ADR supersedes ADR 0002's "no per-frame JS / no JS engine in the player"
for builds with the default `scripting` feature.

## Requirement

R5–R7: scripting is optional but available. The default language is TS/JS,
implemented in Rust. Other VMs, such as siglus_rs's SceneVm or RFVP, must
also plug in.

## Decision

There are two kinds of script integration. They are deliberately not merged:

| | Script host (`ScriptHost: Lifecycle`) | VM peer (`VmAdapter`, ADR 0001) |
|---|---|---|
| Examples | JS/TS (Boa + Oxc) | Siglus SceneVm (`siglus`), data VM, RFVP (pending) |
| Scheduling | Engine lifecycle hooks | Own instruction budget, waits and yields |
| Engine access | Reflective object API only | `HostServices` service batches |
| Save state | Script persists explicitly via `fs.writeSave` | Versioned private VM snapshot + original-save codecs |
| Visible to OOP API as | — | `Client` objects (`engine.clients()`) |

A native VM keeps its opcode/value/wait/save semantics because it is never
wrapped in a lifecycle host. Both kinds run in the same `App::frame`.

**JS/TS host.** Oxc lowers `.ts` to JS (it does no type checking). Boa
executes it. At `on_init` the host generates JS classes from the registry:
prototype accessors and methods route through three native functions
(`__cs_call/get/set`) with JSON values. Object references are
`{"$object": id, "$class": name}` and are revived into class instances. The
host moves the `EngineContext` into a thread-local slot during JS execution
and restores it through a drop guard, so no raw pointers or `unsafe` are
needed. Rust errors become catchable JS `Error`s. An uncaught exception
faults the App. Each JS entry has a loop-iteration limit (default 10M) and a
recursion limit (512). Read-only properties also get setters, so the engine
rejects sloppy-mode writes instead of letting JS drop them silently. Hooks:
`onInit/onStart/onFixedUpdate/onUpdate/onLateUpdate/onPause/onResume/onShutdown`.

TypeScript declarations are generated from the same registry
(`crysirror script-types`, `examples/engine/crysirror.d.ts`).

## Not yet provided (explicit)

ES modules/imports, deterministic time/RNG policy, snapshot of the JS heap,
hot reload, per-entity script components, type checking, and a VM peer
written in JS. Adding another language means implementing `ScriptHost`. Adding
another bytecode VM means implementing `VmAdapter` + `VmFactory`.

## Evidence

`crates/crysirror-script-js/tests/host.rs` covers: a TS behaviour driving the
world, caught engine errors, a sloppy-mode read-only write rejected, an
uncaught error faulting the App, a bounded runaway loop, TS parse errors, and
a TS script importing a Blender-exported GLB. The desktop
`render --script examples/engine/game.ts` run executed on Apple M5/Metal
(see STATUS.md).
