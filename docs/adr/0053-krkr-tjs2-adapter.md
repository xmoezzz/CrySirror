# 0053: KiriKiri (TJS2) adapter over engine Layer/Canvas services

Date: 2026-10-26. Status: implemented (first slice). Implementation,
compilation and behaviour evidence are recorded separately in STATUS.md.

## Context

The owner asked for KiriKiri (吉里吉里, KRKR2/KRKRZ) support next, built on
an existing Rust implementation, and chose Kirakira
(https://github.com/liulifox233/Kirakira) over krkrz_rs. Kirakira is
AGPL-3.0-or-later; CrySirror is MPL-2.0. Its E-mote support is not needed:
CrySirror already owns puppets (ADR 0029).

AGENTS.md and NATIVE_ADAPTERS.md require a thin semantic/syscall adapter:
original VM semantics, engine-owned resources, no second renderer.

## Decision

1. **Vendor only the VM.** `crates/krkr-upstream/krkr-tjs2` (TJS2 frontend,
   compiler, bytecode loader, VM, builtins) and `krkr-kag` (KAG parser) are
   copied byte-for-byte from commit
   `c451918023477620ff9055a2eb8a6714b947d9c1`; `SOURCE.json` records every
   file's SHA-256. Kirakira's engine, renderer, audio, font, video, plugin,
   XP3 and E-mote crates are not used.
2. **Licence boundary.** These two crates and `crysirror-krkr` are the only
   AGPL code. They are `publish = false`, no MPL crate depends on them, and
   the desktop exposes them only through the non-default `krkr` feature (a
   binary built with it is AGPL). `tests/test_krkr_policy.py` enforces this,
   following the optional-GPL precedent of `crysirror-formats-arc`.
3. **Native classes translate to engine services** (`crysirror-krkr`):
   - `Layer` → one engine Layer node (position, size, clip to bounds,
     visibility, opacity) plus one engine Canvas surface for its image,
     presented beneath the node. Painter order is a depth-first walk of the
     KiriKiri tree, sorted by `absolute`, written as engine layer order.
   - `loadImages` → a Canvas `set` of an image paint decoded by the shared
     decoder registry (TLG through `arc-formats`); `fillRect`, `colorRect`,
     `copyRect`, `operateRect`, `assignImages`, `drawText` → Canvas
     `clear`/`draw`/`copy` where the engine operation is the same operation.
   - `Timer` → engine clock, KiriKiri defaults (interval 1000, capacity 6,
     overridable `onTimer`, action owner/name); `System` continuous handlers.
   - `Window` → the project canvas; mouse/key input → `onMouseDown/Up/Move`,
     `onClick`, `onKeyDown` on the window and the front-most hit layer.
   - `Storages` → KiriKiri auto-path, case-insensitive naming over the
     client's capability storage; `Scripts` → the TJS2 compiler/VM.
4. **Two engine capabilities were added first**, usable without KiriKiri:
   - `CanvasCommand::Present { parent, position }`: a canvas presentation
     can live inside a layer tree and inherit its transform, visibility,
     opacity and clip. The per-client surface budget is 1024.
   - `HostServices::image_metadata`: a thread-safe image metadata handle,
     because KiriKiri scripts read `imageWidth` synchronously after
     `loadImages`.
5. **Unsupported means visible failure.** Members whose meaning the engine
   cannot reproduce throw a TJS exception naming the missing capability:
   partial translucent `fillRect`, partial `copyRect` onto drawn pixels,
   non-default draw faces/`holdAlpha`, `operateRect` modes other than
   `omAlpha/omAuto` or opacity ≠ 255, text shadows, `Font.getTextWidth`,
   stretch/affine/piled copies, pixel access, transitions, `Plugins.link`
   (no DLL is ever loaded), `showModal`, a second `Window`, and a window size
   different from the project canvas. Kirakira's practice of silently
   counting stub calls is not adopted.

## Rejected alternatives

- **Vendor Kirakira's `krkr-engine`.** It is a second general-purpose engine
  (layer blending, renderer, mixer, video); prohibited by AGENTS.md.
- **krkrz_rs (MIT).** Offered; the owner chose Kirakira.
- **Unsafe borrowed host pointer inside TJS natives.** Replaced by the
  long-lived `image_metadata` handle, the same pattern as `storage()`.

## Not done yet (in order)

1. XP3 archive mounts (`data.xp3`, `foo.xp3>` auto paths) through the shared
   resource registry, and KiriKiri's TJS2 bytecode-in-archive startup path.
2. `KAGParser` native class over `krkr-kag`, then the stock KAG3
   `system/*.tjs` boot sequence (MainWindow, MessageLayer, Conductor).
3. Synchronous engine font metrics (`Font.getTextWidth`), glyph shadows and
   draw faces/region replace on canvases, so message layers work.
4. `WaveSoundBuffer`/`VideoOverlay` over engine audio/Media services,
   `beginTransition` over engine screen transitions (ADR 0022).
5. Saves (`.ksd` via `Dictionary.saveStruct`), VM snapshots.

## Acceptance evidence (this slice)

- `crysirror-scene` `presentation_attaches_under_parent_layer_and_follows_it`.
- `crysirror-krkr` tests: a self-authored script with a script subclass of
  `Layer`, a Timer on the engine clock and a click produce engine Layer and
  Canvas state inspectable without the VM; unsupported members fail with
  their capability names; Shift_JIS scripts and case-insensitive storage.
- `examples/krkr-demo` (self-authored script, generated images) renders
  through the desktop renderer with `--features krkr`.
- No commercial KiriKiri title was run. A real KAG3 game does not boot yet
  (items 1–3 above).
