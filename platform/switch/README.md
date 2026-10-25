# Nintendo Switch backend

Status: **not implemented or tested in CrySirror**.

Source candidates to inspect after fetching RFVP:
`crates/rfvp-horizon` and `platform/switch`. These are existing directory seams,
not evidence that their current code implements CrySirror traits.

Required integration: main loop and monotonic clock, input sampling, VFS mounts,
2D/text frame submission, audio output, asset upload/lifetime, save persistence,
and a reproducible build/package recipe. Later extend the render capability set
for 3D. Do not enable desktop window/wgpu code by default in the common core.

No SDK binaries, signing material, game content, fonts or NRO artifact are
included. No claim of successful device or emulator execution is made.
