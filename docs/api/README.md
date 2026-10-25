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
  `crysirror-tasks`. Descriptions come from their
  `///` comments. Defaults come from serde: a minimal value is parsed and
  then serialized again.
- **Adapter configs.** The `Config` structs of the Siglus and native RealLive
  adapters. Removed hosted-engine configurations are not accepted.

Rust crate internals are not part of this reference. For those, use
`cargo doc --workspace --no-deps`.

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
