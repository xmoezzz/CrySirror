# ADR 0004: engine object model, lifecycle and plugins

Status: implemented (partial) on 2026-10-24 in `crates/crysirror-engine`.

## Requirement

R1–R5 in the [backlog](README.md): CrySirror is a general-purpose engine in
the Unity sense. Shared features extracted from the VN engines (world,
transforms, scaling, text, UI) belong to CrySirror, not to any one frontend.
Every capability is reachable through an object-oriented API. The engine
lifecycle is an explicit abstraction. The engine is not script-centric, but it
can export its OOP API to scripts.

## Decision

1. **One reflective class registry is the API.** A `Class` is compiled Rust
   that describes itself through `ClassInfo` (properties with type/writability,
   methods with parameters/return types, documentation) and implements
   `get`/`set`/`call`. Objects are an `ObjectId` plus plain-data payload in an
   `ObjectTable`. Rust behaviours, script hosts, typings generation and future
   MCP/editor inspection all dispatch through `EngineContext::{get,set,call}`.
   That one path checks membership, writability and arity before reaching the
   class.
2. **Objects are handles, not owners.** The built-in `World`/`Entity`/`Fs`/
   `Client`/`Engine` classes wrap the existing runtime services. Entity objects
   carry `(id, generation)` and fail after `destroy()` or id reuse. Mutations
   are submitted as the same transactional `ServiceBatch` VMs use, scoped to
   the App's client. No second source of world truth exists.
3. **Lifecycle.** `App` owns the `Runtime` and runs
   `Created → Initialized → Running ⇄ Paused → Stopped`, with an error in any
   hook causing `Faulted`, a terminal state that is reported. Each frame runs
   `on_fixed_update` ×N (accumulated 16 ms steps, max 8 per frame; the excess
   goes into `dropped_ms`), then `on_update`, then the runtime tick (UI, World
   simulation, every VM session with its native waits), then `on_late_update`.
   Shutdown runs in reverse order and runs every hook.
4. **Plugins** package classes, behaviours, script hosts and VFS mounts. They
   are applied once before `init`. Configuration after `init` is rejected.
5. **Hosts own devices.** `crysirror-engine` has no window/GPU/audio dependency
   and forbids `unsafe`. The desktop drives `App::frame` and renders the
   runtime's extraction; `App::into_runtime` hands it back for capture.

## Rejected alternatives

- *A script-first engine (everything defined in JS).* This rejects pure-Rust
  games and makes the JS heap the source of truth for saves.
- *Exposing `World` structs directly to scripts.* This would bypass validation,
  client scoping and transactional commits.
- *Per-entity component objects that own state (Unity MonoBehaviour on a
  GameObject).* This is deferred. Behaviours are App-level for now. A
  component/query ECS needs its own ADR together with save/schema migration.

## Evidence

- Implementation: `crates/crysirror-engine/src/{object,context,lifecycle,builtin}.rs`.
- Behavior: `crates/crysirror-engine/tests/app.rs`. The tests cover hook
  order, fixed-step carry, pause, reverse shutdown, Faulted state, stale and
  cascaded handles, read-only/unknown/invalid rejection, plugin rules, a Rust
  behaviour driving world extraction, capability FS, a Siglus VM visible as a
  `Client`, and TypeScript declarations.
- Not done: component model, events/signals between objects, object
  persistence in snapshots (object ids are session-local; world state is
  saved by the runtime), MCP live inspection over this registry.
