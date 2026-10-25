# RealLive VM through CrySirror APIs

The current `crysirror-reallive` crate is a pure original bytecode VM plus a
semantic/syscall adapter. It replaces the hosted implementation removed under
[ADR 0047](adr/0047-engine-api-capability-extraction.md). The original RealLive
`Engine`, `System`, `Graphics`, `Sound`, `Movie`, painter and framebuffer are
not part of this bridge.

The purpose is to add capabilities to CrySirror that other VMs and ordinary
engine projects can use. The native opening profile has reached the first
story pause using engine-owned nodes; this does not declare complete-game support.

## Ownership and calls

| Operation | CrySirror state and API | What remains native |
| --- | --- | --- |
| DC allocation, fill, image draws and copies | `CanvasCommand` / `CanvasSystem`, exposed as `Canvases.apply/get` | DC numbers, native parameter forms, resource lookup and display timing |
| Screen presentation | Logical canvas nodes become ordinary retained Layer nodes | Which native DC is presented and when |
| Text | `Paint::GlyphRun`, shared glyph metrics/rasterization and engine-owned `GlyphReveal` | Native window/grid, font/color selection, markup and page/wait policy |
| Choices and name fields | Shared UI widgets, input events and scoped queries | Native result indexes, name-bank and condition semantics |
| Music, SFX and voice | Shared native audio sources, `AudioRequest`, channel mixer and bus controls | Native file/track/channel meanings and wait policy |
| Movies | `Media.play/stop/status`, `MediaManager`, incremental MPEG decode and a separate media texture Layer | Native blocking/cancellable/loop meaning and overlay lifetime |
| Files and state inspection | `ClientStorage`, engine query services | Case-insensitive names, FOLDNAME selection and native values |

A canvas is a retained drawing description, not a native framebuffer. Copies
snapshot logical draw nodes; presentation expands those nodes into the shared
Layer system. Images, glyph runs, UI and media remain engine-owned state that
can be inspected or changed while the VM waits. The adapter does not submit a
precomposed native screen through `present_surface`.

Canvas copies currently reject isolated opacity and partial replacement cases
that require a genuine shared render-target composition API. Unsupported cases
must not be approximated by reviving a private native renderer.

## VM boundary and source

The source pin is `xmoezzz/siglus_rs` commit
`5f358ef1360b0439e92aed2b749e8f25d8721954`, MPL-2.0. The retained source list,
upstream hashes, excluded modules and seam changes are recorded in
[`SOURCE.json`](../crates/crysirror-reallive/src/vm/SOURCE.json). See also
[the source/routing audit](reallive-source-audit.json).

Original bytecode parsing, NLS, expression/bank values, call/return and the
pure memory/string builtins remain in the VM. Engine operations are returned
as pending `Command` or `Text` events. Their original PC is acknowledged only
after the adapter successfully submits the corresponding CrySirror calls.
Unknown native operations fail visibly; the caller's instruction budget is
not replaced by the original player's convenience frame budget.

The complete reference inventory contains 90 Rust source files. Hashing that
inventory establishes provenance, not coverage of every native engine module
or a claim that all 90 files have been mapped into shared APIs.

## Private project setup

The desktop default features include `reallive`; minimal hosts can enable it
explicitly with `--features reallive`. The default
Runtime registry remains host controlled; it does not instantiate every native
VM merely because its source is present. A host can also call
`crysirror_reallive::register` explicitly.

Use an owner-supplied asset root and font. This example describes the project
shape; no game data or font accompanies it:

```json
{
  "schema": 2,
  "name": "Private RealLive project",
  "content_revision": "1",
  "canvas": [800, 600],
  "assets": "assets",
  "fonts": {"japanese": "fonts/owner-font.ttc"},
  "clients": [{
    "name": "game", "adapter": "reallive", "game": "owner-game-id",
    "config": {"root": "game", "font_alias": "japanese"},
    "program": []
  }]
}
```

`assets/game` contains Gameexe.ini, Seen.txt and the original resource folders.
`root` is a canonical client-relative capability path. `font_alias` selects a
project font; it is not an ambient font path. Optional `encoding` is an NLS
override and `start_scene` is a diagnostic override. Native clients execute
Seen bytecode and must not contain a data-VM `program`.

```sh
cargo run -p crysirror-desktop --features reallive -- inspect /absolute/project.json
cargo run -p crysirror-desktop --features reallive -- run /absolute/project.json \
  --data-dir /absolute/private-state
```

Window creation, event processing and device output belong to the desktop
host. Retained glyph rendering is independent of winit. Desktop IME events
reach shared UI editing; no claim of every original name-entry or IME mode
follows from the presence of that route.

## Independent capability evidence

The shared services do not depend on `crysirror-reallive`. Rust and JS/TS engine
clients can invoke the same Canvas, Layer, UI, audio and Media services without
loading a native VM. Synthetic native fixtures check the reverse boundary:
Seen commands create shared nodes, and an ordinary engine command can edit
those nodes while the native PC remains in a wait.

The shared media service was explicitly tested without a VM, GPU or audio
output device against the owner-supplied RUSUR.MPG through `ClientStorage`:

- Release test command: `cargo test --release -p crysirror-runtime --lib
  media::tests -- --include-ignored --nocapture`, with the explicit private
  `CRYSIRROR_MEDIA_FIXTURE` path.
- Six tests passed. The complete file reached `Ended` without an error and
  delivered media audio to the shared mixer; source size and mtime were
  unchanged. This is media-service evidence, not a native-game execution claim.
- The run decoded 2,995 frames and published 2,985 texture updates. Those are
  codec/output counters, not a guarantee that every source picture was emitted.
  The measured elapsed time is not an engine frame-rate benchmark.

Logs and structured results are under
`target/verification/api-media-actual-rusur-{release.log,results.json}`. Private
media, fonts and game captures remain ignored and are not distributed.

## Native opening acceptance

The opt-in desktop test `reallive_engine::rusur_opening_uses_retained_engine_nodes_and_can_edit_them_without_vm`
ran owner-provided Game_RUSUR with the shared Metal renderer (Apple M5).
It traversed the title, native name fields, choice, complete opening movie and
first story pause (scene 6030, line 65). No native framebuffer was submitted.
The movie decoded 2,995 frames; the engine mixer received its audio.

At the story pause the test checked the retained `GlyphRun`, removed its main
and shadow layers through `submit_engine`, and captured changed GPU pixels.
The captured state contains 22 individually styled glyphs and five engine
layers. Native diagnostics, PC and clock remained identical: the edit required no VM
execution. The EOF media layer had already been removed. The synthetic bridge
tests separately exercise native wait deadlines, unsupported-PC failure and
shared TextField submission without game files, fonts or GPU.

Private captures and inspection data are under
`images/third-party/reallive-api/`; the test requires explicit project/input/output
environment variables. These assets and fonts remain ignored. The documented
profile is a compatibility checkpoint, not a complete playthrough or pixel-perfect
layout claim. The clean explicit test log is
`target/verification/reallive-api-acceptance.log`. The original 975 game files
were rehashed with no content, size, mtime or inventory changes.

## Current limits

- Opening-profile behavior is verified separately from compilation and from
  the rejected hosted implementation. No complete-game support is claimed.
- Native save-file import/export, whole-adapter snapshots and DLL emulation are
  not supported. VM bank/call-stack savepoint bookkeeping is not a save-file
  implementation. These capabilities must not silently serialize a different
  substitute format.
- The native syscall mapping is partial. Unsupported text/layout/graphics,
  object/animation/effect, manual-presentation and host-operation variants fail.
  No blanket compatibility with every RealLive resource or title is claimed.
- The retained streaming media path currently uses incremental MPEG video.
  Movie audio is decoded into bounded full-track PCM (16 Mi i16 samples);
  seek, pause and unlimited long-track streaming are not implemented. Active
  or retained EOF media resources cannot be silently discarded by a snapshot.
- Transparent-image non-mask copying, preserving DC growth, outgoing queued music
  replacement, unsupported window skins and advanced text/layout variants fail
  explicitly until their shared APIs are available.
- Isolated canvas opacity, partial replacement, exact group offscreen blending
  and additional native effects need shared engine APIs before adapter support.
- Audio-device behavior, Windows/Linux and hardware ray tracing require their
  own validation; no such results are claimed here.
