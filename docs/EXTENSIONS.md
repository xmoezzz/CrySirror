# 3D and MCP: implemented subset versus remaining scope

## 3D implementation subset

Schema-2 projects can describe an opaque static scene with a perspective camera,
transformed cubes and indexed triangles. crysirror-wgpu draws it with directional
lighting and a depth buffer, underneath 2D sprite/text overlays.

The renderer also provides a bounded compute-raytrace path using actual GPU
ray/triangle intersections and directional shadow rays. The same overlays follow
that pass. It was executed on Metal. Hardware acceleration remains unsupported
and an explicit hardware-raytrace request errors. ADR 0003 records limits and
desktop-platform scope; no cross-platform hardware RT claim is made.

This does not add a complete 3D command service to the original no_std core.
Require3d in that older interface still returns Unsupported. Dynamic scene
resources, glTF, PBR, animation, terrain, physics and vehicle simulation are not
implemented. Avoid pretending the static fixture is a general 3D engine or a
racing-game vertical slice.

The next architecture and action-game milestones are specified in
[ADR 0002](adr/0002-modern-engine-and-optional-typescript.md). Optional offline
TS/JS authoring now generates validated static scene data through Rust Oxc/Boa;
see [AUTHORING.md](AUTHORING.md). Runtime JS callbacks are still planned.

## MCP implementation subset

The stdlib Python server implements stdio JSON-RPC protocol 2025-06-18,
initialization, tool enumeration/calls and error responses. Actual handshake
and project operations were exercised in Python tests. Runtime screenshot
calls still require a successfully built/tested desktop executable.

Tools inspect/validate project data, optionally replace that selected project,
and optionally call a configured host to capture or package it. This is not a
live scene debugger, asset-generation model provider or arbitrary command shell.
It starts a fresh rendering process on capture and does not hot-reload a window.

Future authoring recipes should be structured data plus checked operations.
Keep high-frequency simulation and rendering local; do not send one MCP request
per draw call or physics step. Preserve original VM behavior behind its adapter.
