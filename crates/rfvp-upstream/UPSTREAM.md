# Vendored rfvp core

- Source: https://github.com/xmoezzz/rfvp @ 062fec82033b554b9eb3b5da069518c0c96a354e,
  crate `crates/rfvp`. License: MPL-2.0 (`LICENSE`).
- `SOURCE.json` records the SHA-256 of every vendored source file; all are
  byte-identical to the pinned commit (no source patches).

Only the portable `no_std` core is built: the HCB interpreter, syscalls, prim
tree, motions, text, sound slots and saves. It reaches the platform only
through its `host_api` traits. CrySirror's `crysirror-rfvp` adapter implements
those traits on engine services (capability storage, engine images, one
dynamic puppet mesh per frame, channel audio, save mount); it does not use
upstream's winit/wgpu/kira/std hosts.

## Port changes (complete list)

1. `rfvp/src/subsystem/resources/fonts/` is not vendored: it holds Microsoft
   fonts that must not be distributed. The `no_std` build never embeds them;
   the game's own font is served as the core's `default.ttf` (adapter config).
2. `rfvp/Cargo.toml` is new: `publish = false`, `default = ["no_std"]`, only
   the dependencies of the `no_std` profile, upstream warnings allowed.

## Known core-private algorithms (to move into CrySirror)

Image decoding of the game archives and glyph rasterization (ab_glyph) still
run inside the core, which hands the adapter finished RGBA textures. They are
recorded in STATUS.md as open extraction work.
