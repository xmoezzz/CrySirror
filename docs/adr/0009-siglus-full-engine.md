# ADR 0009: Siglus: full original SceneVm with a native frame-plan render plugin

Status: in progress, paused on 2026-10-24 by owner priority (the new
engine comes first). Done: the full upstream crates are vendored
(crates/siglus-upstream, UPSTREAM.md); they compile headless (no
winit/wgpu/egui/ureq in the default tree) and with `native-render` using
an offscreen original renderer. Not done: the CrySirror embedding host,
the factory switch from the subset crate, the compiled fixture game, and
differential runs.

## Context

`crysirror-siglus-vm` holds about 46k lines migrated from siglus_rs. It
omits upstream's render, text_render, audio, movie and host modules. The
upstream `siglus_scene_vm` is about 157k lines at
5f358ef1360b0439e92aed2b749e8f25d8721954. With the subset, real scenes cannot
show message windows or text, so it cannot "run a game".

Upstream already separates a backend-neutral frame plan (`RenderSprite` →
`render_plan::FramePlanner` → draw commands, wipes, techniques) from its
backends (wgpu `render/`, PS Vita GXM). siglus_rs pins **wgpu 0.20**, the
same version as CrySirror.

## Decision

1. Vendor the complete `siglus_scene_vm` (and its local path crates) the way
   `crates/rfvp` was vendored, and keep the VM/forms/text/wait/save code
   unchanged.
2. CPU side (compat): an embedding host like `crysirror_host` that pumps the
   VM with injected time and input, files through `ClientStorage`, and
   outputs the frame's plain-data `RenderSprite` list plus decoded image
   surfaces each frame. No window or device code.
3. GPU side: a renderer plugin crate that runs Siglus's **original**
   `FramePlanner` and WGSL techniques on CrySirror's `Gpu` into an offscreen
   texture, composited as a Layer. Keep backend-specific code out of
   core/compat, as AGENTS.md requires.
4. Replace the subset crate only after the full engine passes its existing
   tests and the scene fixtures. The game fixture is upstream's own
   `siglus_script_compiler` test scripts (`manual_scenario.ss`,
   `manual_menu.ss`) plus the `Gameexe.ini` fixture, compiled by upstream's
   compiler, so no user game data is involved.
5. Acceptance: the compiled fixture shows its message window and text, and
   advances on click. A differential pixel/trace comparison runs against
   upstream's own host with the same inputs.
