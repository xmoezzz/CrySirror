# 0016: Preset effects and developer surface shaders

Date: 2026-10-25. Status: implemented and locally validated (surface ABI v1).

The owner requested reusable shader presets, including fog, and the ability to
run developer-authored shaders. Extend the engine material system (ADR 0015)
with shared preset data and a client-scoped code-asset reference. Source-game
material names and graphs do not belong in the renderer.

## Implementation

`crysirror-scene::effect::ShaderEffect` is validated safe CPU data. The optional
`Material.effect` field preserves the earlier material/snapshot identity when
absent. Presets have bounded numeric parameters and reject unknown fields/kinds:

| Preset | Behavior | Parameters |
|---|---|---|
| fog | Moving two-octave UV noise, density coverage, smooth sheet edges | density, noise_scale, speed, edge_fade |
| fresnel | View-dependent edge emission | power, intensity |
| dissolve | World-space noise coverage and glowing surviving edges | threshold, edge_width, edge_color |

Fog requires blend/additive/premultiplied; dissolve requires mask or transparency.
Unlit, base RGBA, independent opacity and opaque-depth fade compose with presets
through the same material contract. Fog needs authored UVs and is a sheet/surface
shader, not global height/volumetric fog or an original UE shader reconstruction.

Custom data is `{kind:"custom",asset:"fx/effect.wgsl",parameters:[[...],...]} `:
a canonical WGSL path and four vec4 values. No inline source, GPU object, compiled
pipeline or executable pointer enters project/VM/save data. The source is a
separate developer code asset. All reads and GPU cache identities include the
client; the same filename can refer to different code in two client roots.

The hook is `fn crysirror_surface(i: SurfaceInput) -> SurfaceOutput`. ABI v1
supplies sampled linear material inputs, transformed UV0, world position/normal,
view direction, simulation seconds and parameters. It returns color/opacity,
emission, roughness and metallic. Textures are sampled by the engine beforehand;
lighting, alpha mode, cutoff, skinning and blending stay engine-owned. Unlit
materials can supply direct custom colors. See the self-authored
[example and ABI guide](../../examples/shader-effects/README.md).

Color, depth prepass and shadow call the **same** sampling/hook implementation,
using the main camera and world time. Custom alpha/dissolve masks therefore also
cut holes in depth and shadows. Shadow uniforms now carry camera/time alongside
the cascade matrix. Static/skinned and single/two-sided pipeline variants share
one builder. Transparent draw order and HDR/TAA policies from ADR 0015 remain.

`crysirror-wgpu::effect` parses and validates WGSL using pinned Naga 0.20.0
(the existing wgpu generation), validates the public ABI and all three assembled
stages, and returns source/stage diagnostics. Entry points and global variables/
resource bindings are rejected in custom modules. Pure helper functions and
constants are supported. Wrong signatures and engine-name collisions fail the
assembled-stage check. Renderer pipeline creation uses a wgpu validation error
scope, reporting failures with the client and source path.

Limits: 65,536 source bytes, 64 functions and 256 types (including ABI types/
helpers), 32 custom modules per renderer. Modules and source bytes are cached
for the host session; restart to pick up edits. There is no hot reload, shader
graph, arbitrary vertex/fullscreen/compute replacement or custom resource binding
interface. The resource/ABI validation does not impose a GPU execution-time budget.
Initial compilation is synchronous; no asynchronous compilation/performance claim.

Rust, JSON projects, `Entity.setMaterial`, JS/TS and snapshots use the same
material data. Save/restore tests now include preset/custom parameters and paths.
Python validation/inventory supports effect assets, widget material changes and
declared model/scene assets. The sample GLB is self-contained; inventory does
not recursively discover arbitrary glTF dependencies. The UE preview profile
also explicitly copies developer shader assets into the output capability root.
No shader source is embedded into runtime world data.

New developer tools:

- `crysirror shader-check <project> --client <name> --asset <file.wgsl>` resolves
  the same project/client capability roots and checks all three stages without
  creating a GPU, window or VM/save directories.
- MCP `shader_validate` exposes that read-only check. `project_validate` still
  validates data/declared files; shader compilation is a separate operation.
  Captures perform the actual device compilation and rendering.

## Compilation validation

On macOS arm64 / Apple M5 / Rust 1.98.1:

- Workspace/all-targets check and desktop release build passed.
- Desktop no-default-features check passed with its existing `unused_mut` warning.
  Cargo retains the existing `block 0.1.6` future-incompatibility warning.
- No Windows/Linux, hardware RT or other console execution is claimed.

## Behavior validation

- Workspace: **166 passed / 0 failed / 13 ignored**. Ignored = eleven GPU,
  one caller-font and one RFVP-game test, excluded from workspace passes.
- Explicit Metal GPU suite: **11 passed**. New tests verify custom code and
  parameters reach pixels; same-named code stays client-scoped; world time
  animates effects; invalid code returns diagnostics. Authored-UV Fog and
  developer waves animate while static Fresnel stays stable. Mask coverage tests
  include base alpha, dissolve and custom code in the shadow pass.
- CPU tests compile presets/custom code for all stages; reject malformed code,
  wrong ABI/resource declarations, source limits and invalid effect data. CLI
  tests validate actual WGSL without a GPU and reject root escapes/unknown clients.
  Rust and TS tests preserve effect data through save/restore and rollback.
- Python 3.11: **110 passed**, including shared schema, asset inventory, MCP
  argument guards and external-profile shader copying. Fixtures are self-authored.
- Actual MCP stdio validated the project, compiled waves.wgsl without a GPU,
  rejected an escaping shader path, traced four effects at simulation 1.920 s
  and captured actual PNG pixels. `docs/shader-effects-mcp.json` records results.
- Actual native demo: `dist/shader-effects.png` and the later-time MCP capture.
  No script is required. Both geometry and code are self-authored.
- The local castle retained **42 fog sheets / 7,709 nodes / 13,426,500 scene
  triangles**, now using the Fog preset and recorded source color/opacity/depth
  parameters, without the prior authored mask texture. Its new procedural noise,
  flow and edge fade are explicit approximations. Only the original sky is omitted;
  full material/lighting/gameplay fidelity remains absent. User game assets,
  source-specific profile and capture remain outside this source export.

Logs: `docs/shader-effects-{cargo-check,no-default,release,rust-tests,gpu-tests,python-tests}.txt`.
Metadata and native capture paths: `docs/shader-effects-results.json`.

No complete AAA feature set, original UE shader execution, volumetric fog,
GPU frame-time, or original-image-equivalence result is established here.
