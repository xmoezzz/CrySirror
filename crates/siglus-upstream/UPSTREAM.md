# Vendored siglus_rs crates

- Source: https://github.com/xmoezzz/siglus_rs @ 5f358ef1360b0439e92aed2b749e8f25d8721954
- Crates: siglus_scene_vm, siglus_assets, shion-xfile, siglus_key_recovery,
  siglus_omv_decoder, theora-rs, na_mpeg2_decoder, na_wmv_player, and
  vendor/eluna_rs (as `eluna_rs`). License: MPL-2.0.

## Port changes (complete list)

1. Removed `siglus_scene_vm/assets/fonts/default.ttf` (MS PGothic,
   proprietary) and `src/bin/`. `text_render.rs` takes the default face from
   `install_default_font` instead of `include_bytes!`.
2. Manifests are self-contained (workspace keys inlined, edition 2024),
   auto-targets are off, lib tests are off, `[profile]` is dropped and
   upstream lints are allowed. eluna's path is `../eluna_rs`.
3. New features, all off for the CrySirror embedding:
   `native-audio` (AudioHub opens the platform device),
   `native-net` (game HTTP/Twitter syscalls; when it is off, an
   always-failing `ureq_disabled` stub is used),
   `native-render` (upstream wgpu renderer + `host`),
   `native-host` (winit hosts, desktop dialogs, IME, raw-handle surfaces).
4. `render/mod.rs`: `surface` is `Option`. `Renderer::new_offscreen` renders
   the unchanged pipelines into the internal target and reads the frame back
   into `offscreen_frame`. Window constructors require `native-host`.

5. `movie/mod.rs`: appended `pub mod omv_public`, thin public wrappers over the
   unchanged OMV header parser, Ogg extraction, plane layout and RGBA
   conversion (used by `crysirror-video`). No existing code was modified.
6. `runtime/mod.rs`: `wipe_mask` visibility `pub(crate)` → `pub`, so the
   CrySirror runtime generates the original pattern-wipe masks (ADR 0022).
   The module itself is unchanged.
7. `emote.rs`: two added lines, `mod puppet_api;` and
   `pub use puppet_api::EmoteTimelineSummary;`. The new
   `emote/puppet_api.rs` adds read/drive wrappers on `SiglusEmoteRuntime`:
   variable infos and values, timed `SetVariable`, timeline summaries,
   `play_timeline_mode`, the current scene and the decoded textures. They
   are used by `crysirror-puppet-eluna` (ADR 0029) and only delegate to
   existing player calls.

kira/cpal are still compiled. Without `native-audio` no device is opened.
No VM, form, text, wait, save or shader code was changed (changes 5 and 7 only add wrappers; change 6 only widens visibility).

## Upstream unit tests

All vendored crates set `[lib] test = false` so `cargo test --workspace`
runs CrySirror's own tests only. On 2026-10-24 the unmodified upstream
`cargo test -p eluna_rs --lib` reported 96 passed / 2 failed
(`vertex::tests::builds_single_cell_strip`,
`runtime::reverse_parity_tests::timeline_hold_markers_do_not_pollute_authored_ranges`).
The vendored copy reproduces exactly that result; it is an upstream finding,
not a port regression.


BGI shared-service integration also makes Kira CPAL explicitly opt-in through
`siglus_scene_vm/native-audio`. Decoder formats stay enabled; headless users no
longer compile the default device backend just to access CPU helpers. Borrowed
MPEG PacketRange/packet_ranges helpers from the attributed ethornell source are
added to the shared decoder demux API; BGI and Siglus use one decoder package.
