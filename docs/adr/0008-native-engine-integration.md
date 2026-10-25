# ADR 0008: native engine integration: shared seams, original engines kept whole

Status: implemented for RFVP on 2026-10-24; Siglus follows ADR 0009.

## Requirement

The project owner's first priority: abstract the capabilities of the existing
engines, connect them to CrySirror, and get their games running. Siglus is the
most complex, RFVP the simplest.

## Decision

An original engine keeps its VM, syscalls, resource decoders, text/motion
logic, save format and, when needed for fidelity, its own renderer. CrySirror
replaces only the **platform seams**, and every engine reaches them through
the same core services:

| Seam | Core service | RFVP | Siglus (ADR 0009) |
|---|---|---|---|
| Files, packs, saves | `HostServices::storage()` → `ClientStorage` (thread-safe capability handle: asset read-only incl. provider mounts, save/cache atomic writes, listing) | `RfvpStorage` bridge; `save/` → save mount | resource/io seam |
| Time | `HostServices::now_ms` | `Time::set_external_delta` | platform_time |
| Input | `InputFrame` + `service::buttons` constants | `InputManager::notify_*` | input module |
| Presentation | `present_surface` (replaceable RGBA, never saved) + one retained Layer, **or** a native frame plan for a GPU plugin | original software renderer → surface | original planner/shaders (ADR 0009) |
| Lifecycle | `VmAdapter` + `VmFactory` registered by the host | `crysirror_rfvp::register` | built-in `siglus` |
| Diagnostics | `VmAdapter::diagnostics` → `Client.diagnostics` in the OOP API | title, screen, frames, input, thread states | VM diagnostics |

Engines register their factories from the host (`crysirror-desktop` features
`rfvp`, `siglus-resources`), so `crysirror-runtime` does not depend on
them.

## RFVP specifics

`crates/rfvp` vendors the original crate; the changes are listed in its
UPSTREAM.md. Proprietary fonts are removed and supplied by the host. The VM
runs synchronously inside `VmAdapter::step`. Limits, all reported: one RFVP
client per process (original process-wide `GLOBAL`); no audio (`no-audio`
profile); no movies; CrySirror runtime snapshots return `Unsupported`, and
saves go through the game's own slots on the client's save mount.

## Evidence

- `crysirror-rfvp/tests/painter.rs` (explicit, needs a caller font) runs
  rfvp's `win95_painter_demo` through the runtime. It checks boot, title,
  screen, running main context, the rendered UI pixels, mouse edges reaching
  the original InputManager, and the snapshot refusal.
- Differential: bit-identical framebuffers against the unmodified upstream
  headless host at boot and after the same input (docs/rfvp-differential.json).
- `crysirror render examples/rfvp-painter/project.json` produced
  dist/rfvp-painter.png through the shared wgpu Layer path on Apple M5/Metal.
- Upstream finding (not patched): the demo's canvas does not paint on click
  in either host. The press is observed, but `begin_canvas_action` returns
  before `PrimSetTile`.
