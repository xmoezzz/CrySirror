# 0036: Siglus shared-service coverage must be verified through the native VM

Date: 2026-10-25. Status: audited; native text/audio/movie bridges incomplete.

The owner asked whether Siglus's details, including text drawing, have all been
extracted into the engine and whether games can now run. The answer must separate
shared mechanisms, native adapter wiring and actual native execution.

## Actual execution path

`crysirror-runtime::SiglusFactory` registers `siglus` using
`crysirror_siglus_vm::adapter::SiglusVm`. It rejects a data-VM program and executes
native SCN/PCK streams in its independent SceneVm. The separately vendored
`siglus_scene_vm` crate supplies original sources and reused codec/wipe helpers;
its complete UI/renderer is **not** the VM renderer selected by that factory.

The complete reference checkout was verified at
`5f358ef1360b0439e92aed2b749e8f25d8721954`. Relevant source paths, hashes,
coverage rows and exact command results are in
[siglus-service-coverage.json](../siglus-service-coverage.json). This audit adds
no native hook, opcode reinterpretation or renderer modification.

## Shared mechanisms versus native wiring

| Feature | Shared engine implementation | Registered native bridge |
|---|---|---|
| G00 and basic sprites | CPU image codecs, scoped uploads, retained layers | RECT/PCT objects in FRONT are wired |
| Color/filter arithmetic | LayerFilter and blend modes | mono/reverse/bright/dark/color/add and blends 0–4 are wired |
| Mask/tone curve/clip/children/buttons | Several shared mechanisms exist | Native export rejects these states |
| Dialogue/name/ruby/vertical text | New CPU text layout/raster, fallback, rich styles and reveal APIs exist | CD_TEXT/CD_NAME currently fail at native publication |
| Screen wipes | Shared transition compositor reuses original shader/masks | Active native WIPE is rejected |
| SFX/music/voice | Shared Mixer, streams, buses, fades and spatial API exist | Original audio-manager integration is absent in this port |
| OMV/Theora/MPEG/WMV | Shared decoder/frame service exists | Original MovieManager is an unavailable stub |
| Mesh/Emote | General engine glTF/animation; separate upstream source available | Current native exporter only accepts RECT/PCT |
| Snapshots/native save codec | Adapter snapshot and native import/export entry points exist | Covered rectangle/wait paths tested; text-rich native save behavior is unproven |

Unknown native layouts and unsupported states fail visibly. This is desirable
diagnostic behavior, but it does not establish that a normal VN can complete its
startup, dialogue, choices, movies and save/load flow.

## Why plain text drawing is not the missing semantic bridge

The ported `runtime/forms/stage.rs` implements
`cd_text_current_mwnd` and `cd_name_current_mwnd` as `false` returns. Its
`UiRuntime` retains a message/name and reveal fields, but `message_waiting`
always returns false. Its `render_list_with_effects` is empty. `export.rs`
explicitly refuses a nonempty message/name.

The pinned upstream has much more behavior in the corresponding paths:

- Native MWND target selection, message-block lifecycle, overflow, per-glyph
  layout, current save message, backlog, read flags and reveal-only waits.
- Name lookup and character colors from Gameexe, name glyph rebuilding and
  message-backlog entries.
- Native font selection/cache invalidation, shadow/fuchi/body sprite layers,
  vertical glyph orientation, native glyph positions, ruby layout and page state.
- Reveal clocks controlled by original SCRIPT/SYSCOM speed, async/nowait/auto
  modes, key icon state and message/page waits.

Generic `Text.draw` and its grapheme clock cannot simply replace those native
policies. Upstream reveal uses native char/reveal indices, which are not the
same as Unicode grapheme counts for combining sequences. The adapter should
retain these policies and submit their validated glyph positions/styles to a
shared font/raster service. Native font names need an explicit project-font
mapping; ambient font scanning or redistributing the old embedded font is not
an acceptable shortcut.

Current `HostServices` exposes engine batches and RGBA upload/presentation, but
does not expose the common `TextService` as a native text request interface.
That seam and a policy-preserving MWND implementation still need to be designed
and compared against the pinned implementation.

## Native save implications

The port still contains codecs for native `MwndGlyphState`, message pages,
appeared/ruby flags, cursor/indent positions, pending ruby and font/color fields.
Those structures are useful original-format work; retaining their serializer
does not supply the missing runtime layout/reveal pipeline.

The current native save tests cover a rectangle savepoint/export/import and
TIMEWAIT_KEY input-release behavior. They do not cover a real dialogue saved
mid-reveal, ruby/vertical layout, message pages, read flags, voice wait or movie
progress. The port's backend restoration only clears rectangle layers, whereas
the upstream rebuilds nested objects and MWND-derived backends. Text-rich
original-save compatibility remains a separate acceptance gate.

## Actual negative execution

Two independent, self-authored native SCN programs were generated outside the
repository using the checked interpreter ABI:

1. `PUSH STR (form 20, string index 0)`, `CD_TEXT (0x31, read-flag 0)`, EOF.
2. `PUSH STR`, `CD_NAME (0x32)`, EOF.

Both were executed through the actual release CLI with `adapter: "siglus"`,
`format: "scn"`, `string_codec: "plain"`, one trace frame and isolated external
state directories. Both exited with code **1** and the same exact diagnostic:

```text
CrySirror: VM game: I/O: native message layout must be implemented before CD_TEXT/CD_NAME can render
```

Files, stdout/stderr and source hashes are under
`~/.cache/crysirror/siglus-service-audit`; only metadata is checked into the
coverage JSON. No font/game bytes or GPU/audio device were needed. This
demonstrates a real native text boundary failure, not a malformed-container or
data-VM simulation. No new compilation or full-game validation is claimed.

The next native acceptance should be text/name → reveal wait/input → ruby/page
layout → partial-reveal save/reload, followed by native audio and movie clocks.
Engine demos alone cannot close those gates.
