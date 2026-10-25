# ADR 0007: modern rendering features and Blender/glTF assets

Status: glTF import implemented; upscaler contract implemented; hardware ray
tracing and vendor upscalers **not implemented**.

## Requirement

R9–R11: integrate current AAA-engine technology: ray tracing, DLSS-class
upscaling, and loading Blender models.

## Decisions

### Blender and model import (implemented)

- glTF 2.0 is the interchange format. `crysirror-asset::import_gltf` uses the
  pure-Rust `gltf` parser. It reads GLB or external-buffer glTF through the
  engine VFS and keeps the node hierarchy as entities. It converts TRS to
  engine transforms (glTF +Y up, as exported by Blender), turns multi-primitive
  meshes into child entities, and converts base-color factors from linear to
  sRGB. Features it cannot represent are listed in `report.dropped`:
  textures, animations, skins, cameras, morph targets, transparency. They are
  never claimed as imported. Non-triangle modes and data URIs are errors.
- `.blend` is not parsed in-engine, because it is an undocumented
  memory-layout dump that changes per Blender version.
  `tools/blender_export_gltf.py` runs inside Blender to convert. Run with
  `--build-fixture`, it regenerates the original lantern model in
  `examples/engine/assets/lantern.glb`.
- Scripts call `world.importModel(path, {parent, prefix})`.

### Upscalers (contract only)

`crysirror_core::graphics::{Upscaler, UpscalerSupport}` covers Native, DLSS,
FSR, XeSS and MetalFX. A required upscaler that is not implemented returns
`Unsupported`, with no silent fallback to native. Every non-native upscaler
needs motion vectors, depth and jitter from the renderer. The renderer
currently produces none of these, so no backend reports support.

Integration rules:
- DLSS requires NVIDIA's SDK and an NVIDIA RTX GPU. It runs through a
  native D3D12/Vulkan interop path that wgpu 0.20 does not expose. The SDK
  is not redistributed in this repository (AGENTS.md); a backend plugin would
  load it from the user's install. It cannot run on the macOS/M5 host used
  here.
- FSR (MIT-licensed shaders) is the portable first target. MetalFX is the
  native macOS target.
- Prerequisites, in order: a frame graph with motion vectors and jittered
  projection, then a temporal accumulation path, then the vendor plugin.

### Ray tracing

The compute ray tracer from ADR 0003 remains: primary and shadow rays with
linear traversal. It now also renders imported Blender geometry (see STATUS).
Hardware ray tracing needs a wgpu upgrade to a generation with ray-query
support, and a BVH/acceleration-structure lifecycle that is rebuilt from
logical scene data (never saved). Next steps: (1) a CPU-built BVH for the
compute path, which removes the 4096-triangle cap; (2) a wgpu upgrade spike
on a separate branch with real device tests; (3) PBR materials and textures
from glTF, shared by raster and RT.

## Evidence

- `crysirror-asset` unit tests: hierarchy/TRS/material/external buffer and
  visible failures.
- `script_imports_blender_gltf_model`: a Blender 5.2.2-exported GLB with 3
  nodes and 3 primitives, nothing dropped, hierarchy preserved, and the roof
  apex at y=1.6 after the Z-up to Y-up conversion.
- GPU captures `dist/engine-api-raster.png` and
  `dist/engine-api-compute-raytrace.png`, made on Apple M5/Metal with
  hardware_raytracing=false.
