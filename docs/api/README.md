# CrySirror API documentation

CrySirror is in early development and its API changes often. The reference
here is therefore generated from the code that runs, never written by hand,
and a test keeps it current.

| File | Contents |
|---|---|
| [REFERENCE.md](REFERENCE.md) | Classes, properties, methods, script globals, command map, data specs with defaults, adapter configs |
| [crysirror.d.ts](crysirror.d.ts) | TypeScript declarations that the JS/TS host binds against |
| [api.json](api.json) | The same reference, machine-readable, for tools and MCP clients |
| [CHANGES.md](CHANGES.md) | Hand-written log of API signature changes, keyed by fingerprint |

## Sources

- **Classes.** The `ClassRegistry` (`crysirror-engine::object`): the
  `ClassInfo` docs, property types and method signatures. The same registry
  generates the JS bindings and the `.d.ts`, so the three cannot disagree.
- **Commands.** `services::COMMAND_API` maps every typed `WorldCommand`,
  `LayerCommand`, `CanvasCommand` and `UiCommand` to the API member that issues it.
  `tests/api_coverage.rs` fails on an unmapped variant.
- **Data specs.** Field names and Rust types come from the `pub struct` and
  `pub enum` declarations in `crysirror-scene`, plus shared media/asset declarations in core/runtime and task statistics in
  `crysirror-tasks`. Modular effect authoring declarations, CPU particle/event
  records and reservation limits come from `crysirror-vfx/src/schema.rs` and
  `simulation.rs`. Descriptions come from their
  `///` comments. Defaults come from serde: a minimal value is parsed and
  then serialized again.
- **Adapter configs.** The `Config` structs of the Siglus, native RealLive and
  original AVG32 VM adapters. Siglus's explicit `MessageWindow` fields are also
  exported as a data specification. Removed hosted-engine configurations are
  not accepted.

Rust crate internals are not part of this reference. For those, use
`cargo doc --workspace --no-deps`.

## Modular VFX declarations

The reference uses `Vfx`-prefixed aliases for types from `crysirror-vfx`:
`VfxSystemSpec`, `VfxEmitterSpec`, `VfxRenderSpec`, and their dependencies.
These are documentation names, not renamed Rust types or serialized tags.
The older scene `EmitterSpec` still describes `EntitySpec.particles`;
`EntitySpec.vfx` instead uses `VfxSystemSpec`. An entity cannot declare both.
`ParameterMap` is expanded to `BTreeMap<String, VfxParameterValue>` in the
reference. Tuple-enum payload types and multiline serde attributes are read
from the same sources by `tests/api_docs.rs`.

Authored parameters use tagged values such as
`{"kind":"vector","value":[0,1,0]}`. A scalar or vector module input accepts
a bare constant or a binding object such as
`{"parameter":"wind","scale":2}`. `Vfx.setParameter` accepts the raw value
matching the existing parameter type. Unknown fields, renderer kinds and
unsupported GPU combinations fail validation.

Simulation is owned by `crysirror-vfx` and scheduled by the Runtime; a renderer
does not advance a private copy. Offline Rust tools publish completed work
with `Runtime.finish_vfx_offline()` before `render_pbr()`; ordinary App offline
preparation performs that fence too. GPU simulation additionally needs real
renderer submissions to consume its bounded command timeline. GPU capacity
is never a measured live-particle count.

The current renderer supports textured/flipbook sprites, soft sprite depth
fade, static unlit mesh instances, and CPU ribbons. Mesh assets provide static
geometry and an explicit VFX texture/color, without inheriting imported PBR
materials; skinned/morphed sources are rejected. GPU ribbons and event-driven
sub-emitters are rejected. Transparent particles sort within an emitter and
emitters sort by distance; this is not unified scene transparency/OIT. Ribbon
width is explicit full world width, independent of size-over-life curves.
This API is a bounded modular VFX foundation, not full Niagara parity.

## Rules for changing the API

`crates/crysirror-engine/tests/api_docs.rs` enforces these rules in the normal
`cargo test`:

1. **Regenerate.** After changing a class, a command or a spec, run
   `CRYSIRROR_BLESS=1 cargo test -p crysirror-engine --test api_docs` and
   check the diff in `docs/api/`. A stale file fails the test.
2. **Log signature changes.** The *fingerprint* hashes the signatures only:
   names, types, writability, spec fields and defaults, command variants and
   adapter config fields. Prose changes do not affect it. When the
   fingerprint changes, add an entry to [CHANGES.md](CHANGES.md) that names
   it and says what changed and how callers migrate.
3. **Document every member.** Every class, property and method needs a
   non-empty doc string, or the test fails. Every new spec field should get
   a `///` comment; the reference shows `—` where one is missing.
4. **Make breaking changes visible.** Renaming or removing a member, a field
   or a variant, or changing a default, is a breaking change. Say so in the
   CHANGES entry. Saved projects and save files are versioned separately
   (`schema` and [SAVE_FORMAT.md](../SAVE_FORMAT.md)).
