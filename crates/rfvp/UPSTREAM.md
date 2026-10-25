# Vendored RFVP engine

- Source: https://github.com/xmoezzz/rfvp, crate `crates/rfvp`
- Commit: 062fec82033b554b9eb3b5da069518c0c96a354e (upstream-lock.json)
- License: MPL-2.0 (LICENSE copied from the repository root)

## Port changes (complete list)

1. **Removed** `src/subsystem/resources/fonts/` (MS Gothic/Mincho, proprietary,
   27 MB). `FontEnumerator::new` now reads host-installed faces from
   `install_builtin_fonts` (text_manager.rs). The one upstream test that
   embedded MSGOTHIC.TTF is `#[ignore]`d and reads `CRYSIRROR_TEST_FONT`.
2. **Removed** `src/main.rs` and `src/bin/` (desktop/GPU entry points).
3. **Added** `src/crysirror_host.rs` (+ `pub mod` in lib.rs): a synchronous
   embedding host derived from `rfvp_os_host.rs`. It uses a host
   `RfvpStorage` instead of ambient paths, an injected frame delta, and the
   synchronous VM tick of the UEFI worker. It also exposes
   `input_snapshot`/`thread_states` for diagnostics.
4. **Cargo.toml**: `default = []`; rlib only; auto-targets off; optional
   local path crates (rfvp-bitmap, na_wmv_player, na_mpeg2_decoder,
   anzu-hal, uefi) dropped; the exact `flate2 = "=1.0.28"` pin is relaxed to `^1.0.28` so it can share a lock with siglus_rs; so their features (`psx`, `old_school`,
   `native-video`, `uefi-native-video`, `anzu-audio`) are empty; `[profile]`
   removed; upstream lints allowed.

No VM, syscall, parser, motion, text, renderer or save code was changed.
Evidence that behavior is preserved: docs/rfvp-differential.json (bit-identical
framebuffers vs the unmodified upstream `rfvp_os_host`).

Residual ambient access: some upstream paths call `app_base_path()`
(`FVP_BASE_PATH`/exe dir), namely the host font-directory scan (not called by
the CrySirror host) and the threaded video player (not compiled in this
profile). They are not routed through capability storage yet.

## Upstream unit tests

`[lib] test = false` keeps them out of `cargo test --workspace`: several read
`testcase/` game files that are not in the upstream repository, and others
build `GameData`, which needs the removed fonts. Explicit run (results are not
counted as CrySirror passes):

    cargo test --manifest-path crates/rfvp/Cargo.toml --lib \
      --features runtime-core-deps,soft-render-core,no-audio,image-formats,zlib-flate2,random
