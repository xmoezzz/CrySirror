# CrySirror development rules

Read STATUS.md, docs/ARCHITECTURE.md and docs/NATIVE_ADAPTERS.md before editing.

- Engine integrations exist to grow CrySirror's reusable capabilities. A new VM
  must call CrySirror APIs through a thin semantic/syscall adapter; it must not
  bring a second general-purpose engine implementation or renderer with it.
- Do not host a legacy engine and upload its composed screen as an integration.
  Do not keep reusable image/text/UI/layer/effect/audio algorithms private to a VM
  merely to get a game running. Add missing capabilities to CrySirror first.
- Acceptance requires engine-owned resources/operations/state usable without
  that VM. A running game, matching screenshot or registered factory alone
  does not establish capability extraction or completed integration.

- Default host: wgpu + winit. Switch and PS Vita are deferred; no other console.
- RFVP and Siglus remain separate original VMs, never aliases for the data VM.
- Preserve opcode, value, wait, resource and original-save semantics in adapters.
- Inspect complete upstream source at pinned commits before implementing hooks.
- Do not invent commit IDs, native APIs, successful builds or hardware results.
- Core/compat must stay free of window/GPU/audio dependencies and unsafe code.
- Keep the renderer independent of winit; the desktop crate owns the event loop.
- Unknown native adapters/layouts fail visibly instead of silently succeeding.
- Use client-scoped handles and capability roots for shared-service calls.
- Keep project/VM data distinct from GPU/device pointers and executable code.
- Never distribute user game data, fonts, credentials or proprietary SDK files.
- Document implementation, compilation and behavior validation separately.
- Run Python tooling tests and cargo checks/tests when the environment permits.
- Update STATUS.md only from actual results. An ignored GPU test is not a pass.
- After API changes, regenerate docs/api and log fingerprint changes (docs/api/README.md).
