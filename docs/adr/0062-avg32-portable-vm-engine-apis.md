# 0062: Original AVG32 VM library with engine service commands

Date: 2026-10-26. Status: implemented, bounded `plain-adv-v1` profile.

## Decision

AVG32 is an independent native interpreter in the owner's `siglus_rs`.
Its original `Scenario` was coupled to the original `System`, message painter,
PDT manager, audio device and filesystem. Linking that full System or uploading
its composed framebuffer would violate the engine integration requirement.

The original repository now exposes a `portable-vm` feature at actual commit
`256c398e06fbcbae8e956eb046c831b3d6c8a593` on
[codex/avg32-portable-vm](https://github.com/xmoezzz/siglus_rs/tree/256c398e06fbcbae8e956eb046c831b3d6c8a593).
CrySirror depends on this exact Git revision with default features disabled.
No AVG32 interpreter source is copied into this monorepo.

The portable build compiles the **same** `scenario.rs`, native flags, header and
compact-value parser. Supported arithmetic, conditions, jumps, shared calls and
returns, waits, text packets, choices, image-load and sound opcode branches
retain their original implementations. Original input policy and RNG moved to
shared upstream modules used by both original and portable hosts. The portable
feature excludes the original System, renderer, image/font/audio/movie modules;
its direct dependency tree is anyhow, encoding_rs and serde. Original native
engine builds retain their original implementation. Features are mutually
exclusive; do not enable both with `--all-features`.

A native operation yields an owned semantic event. Events remain pending until
a successful host call; no later instruction runs while an event is pending.
`run_budget` bounds interpreter work. The adapter resumes in the same owner step
only if the original instruction did not end the native frame. The VM keeps
native scene/call/variable/wait state. CrySirror owns resources and render state:

- Image load calls engine Canvas `set` and `present`; resource metadata and
  decoding are shared services. It does not publish a private screen.
- Plain native text uses shared `crysirror-text::grid`, engine GlyphRun resources
  and the engine's paused reveal state. The original VM's `mes_print` calls
  advance one native scalar; a completion acknowledges the deferred native
  return. Baselines use the configured native font size, and cell advances
  come from explicit native `cell_step`, never measured glyph advances.
- Native click icons are explicit engine image layers; missing mappings fail.
  Native choice ordinals, including hidden entries, map to engine UI widgets
  and client-scoped activation events. Choice windows are engine panels.
- Native BGM, WAV, SE and voice commands target shared channel audio and actual
  playing-status queries. Resource aliases are capability-relative engine paths.
- Per-client encoding is scoped during interpreter runs with an unwind-safe
  thread-local guard, replacing the process-global portable NLS setting.

VM advancement and API publication run on the owner; renderer extraction and
resource products use the engine's existing worker boundaries. First-use scene
and encoded audio reads are currently synchronous and bounded. This adapter
slice does **not** finish ADR 0050's engine-wide asynchronous resource migration.

## Bounded native profile and visible failures

`plain-adv-v1` explicitly provides a non-novel, unframed message viewport, native
font size and cell pitches, resource aliases, scenes and wait-icon resource.
It is not an automatic GAMEEXE.INI/layout/route translator.

Image loads currently require an opaque full 640×480 resource. Partial loads,
associated masks, negative-buffer presentation, working-name aliases and PDT
copy/fade/effect operations need further engine command mappings and fail.
Message wrapping, page overflow, paragraphs, ruby, special name/icon/indent
controls, doubled text, colored choices and native fonts are not silently
substituted. This first profile rejects those features. Transition/movie,
calendar, native save/load and other unmapped opcodes fail visibly.

Original save formats **and** framework snapshots remain unsupported. They are
not conflated, replaced by a new format, or claimed compatible.
No commercial AVG32 game has been run by this change. A self-authored TPC32
fixture establishes the original instruction path into engine state only.

## Shared bitmap capabilities added first

`crysirror-image::associated` and public `Bitmap.composite/filterRgb` provide
VM-independent associated-RGB copy/over/under with separate coverage, explicit
0..256 weights and coverage-write policy, plus RGB-only integer fade/multiply.
Their output retains associated RGB and coverage as **intermediate data**.
Layers normally sample straight RGBA; callers must explicitly convert or choose
an appropriate display path. These primitives alone do not establish complete
AVG32 PDT rendering or mask semantics.

## Validation

Implementation, compilation and behavior are recorded separately in STATUS.md.
The original native engine's 33 library, 1 NLS and 4 synthetic tests passed;
one installed-game audio test was filtered out. The portable build's 11 tests
passed, including original arithmetic/calls/waits and event acknowledgement,
unsupported operations, native mouse expiry and encoding scopes.

An unchanged checkout at original pin `5f358ef1360b0439e92aed2b749e8f25d8721954`
was used for actual bitmap differential execution through
`tools/compare_bitmap_upstream.py`. All 1,776,384 associated copy/over/under
cases (every byte mask and 0..256 weight), 2,304 signed/clipped RGB fades and
the existing 19,200 / 9,216 / 12 cases matched original output bytes.
No device, game data or fonts were used for that comparison.

`crates/crysirror-avg32/tests/engine_bridge.rs` exercises native TPC32 into
engine Canvas/image/audio/waits/UI and rejects unsupported operations.
The licensed-font text test is ignored by default and must be explicitly run
with `CRYSIRROR_TEST_FONT`; no ignored test is counted as a pass.

Generate an owned native demo with:

```sh
cargo run --locked -p crysirror-avg32 --example avg32_fixture -- target/avg32-demo
cargo run --locked -p crysirror-desktop -- render target/avg32-demo/project.json \
  --font /absolute/path/to/licensed-system-font --frames 100 --output target/avg32.png
```

Consult the desktop `render --help` for current option spelling before running.
