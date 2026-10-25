# 0035 — RealLive / AVG32 source coverage and integration boundary

Status: audit completed; adapters not implemented by this change.

Date: 2026-10-25.

## Evidence

The full external `siglus_rs` checkout at commit
`5f358ef1360b0439e92aed2b749e8f25d8721954` already contains independent
`crates/reallive` and `crates/avg32` interpreters. Both local `git rev-parse HEAD`
and `git ls-remote https://github.com/xmoezzz/siglus_rs.git refs/heads/main`
returned that exact commit during this audit. The repository lock pin remains
unchanged. The source checkout was not modified.

These families are not alternate names for the Siglus VM. AVG32 runs its own
`Scenario` / `System` and TPC32 scenes. RealLive runs its own `Machine`, banked
memory, long operations, savepoints and original save formats. Both have native
software framebuffers and `game_launcher` host wrappers upstream. Neither is
vendored or registered in the current CrySirror runtime.

## Implementation status

No runtime, registry, manifest, lock file, VM or renderer integration was changed.
The accompanying `docs/native-family-audit.json` records APIs, native resource
families, migration seams and actual validation results.

The upstream implementations still own significant text and graphics behavior:
AVG32 message windows/choices, FN.DAT bitmap glyphs and PDT effects; RealLive
message windows, wrapping and punctuation rules, indentation, ruby and native
object/effect composition. Existing CrySirror text APIs do not prove these paths
have been routed through shared services.

A first adapter could preserve each original interpreter and publish its CPU
framebuffer through a client-owned dynamic surface. That would provide genuine
native execution, while shared rendering extraction would remain separate work.
Subsequent hooks should move glyph rasterization, decoded PCM/video, surfaces,
input and clock transport into shared services while preserving native layout,
opcode, wait, resource and save semantics. AVG32 exposes a `GlyphSource` seam;
RealLive currently uses a concrete `FontSet`.

The upstream desktop `game_fs` directly wraps `std::fs`. A raw root path is not
a capability provider. Adapter integration needs client-scoped storage, separate
save/cache permissions and provider propagation into movie workers. AVG32's
process-global NLS selection also needs scoping before simultaneous clients with
different encodings. Both crates compile native `kira` / `cpal` even when their
optional GPU/window player is disabled: device construction can be disabled, but
that does not satisfy CrySirror's dependency-free core/compat boundary.

## Compilation validation

Commands ran from the unchanged external checkout with `--locked`, default
features, and `CARGO_TARGET_DIR=/Users/xmoe/.cache/crysirror/native-family-target`.
`AVG32_NO_AUDIO=1` and `REALLIVE_NO_AUDIO=1` were set. No GPU/window was created
and no audio device was requested.

```text
cargo test --locked -p avg32 --lib --test synthetic --test nls -- --skip installed_game_cd_tracks_decode
cargo test --locked -p reallive --lib -- --skip derived_key_matches_known_key --skip game_tone_curves --skip game_dll_runs
```

Both commands exited 0 on the local macOS host. These are upstream builds, not
CrySirror adapter builds; other platforms and hardware are unvalidated.

## Behavior validation

AVG32: 33 library tests, 1 NLS integration test and 4 native synthetic engine
tests passed. The synthetic tests execute self-assembled TPC32 scenes for
arithmetic/conditions/strings, calls/returns across scenes, hidden/colored choices,
and scenario menu blocks, with silent audio, a virtual clock and persistence
turned off. The installed-game audio test was explicitly filtered out.

RealLive: 58 library tests passed, including native bytecode expressions/memory,
scenario build/parse, original local-memory save round trip, CPU graphics and
transitions, clock/object timing, native text wrapping/indentation and voice/WAV
decoding. Four tests requiring installed-game data were explicitly filtered out.
This audit did not execute a complete RealLive `Engine` scene or commercial game.
The RLVM and installed-game integration suites are not counted: their missing
environment variables cause early returns rather than meaningful validation.

Logs: `/tmp/codex-native-avg32-verified.log` and
`/tmp/codex-native-reallive-verified.log`. No game data, font files, credentials
or proprietary SDK files were copied into this repository.

## Conclusion

The user's statement that `siglus_rs` contains these interpreters is correct.
External native tests demonstrate useful existing implementations. CrySirror
integration and extraction of their shared services remain unfinished and must
be reported independently from those upstream results.
