# ADR 0010: AAA rendering: physically based HDR pipeline and showcase ports

Status: implemented (first slice) on 2026-10-24. This is the owner's
primary objective: the new engine comes first, compatibility layers second.

## Requirement

R13: build the strongest possible engine to a AAA standard, and prove it by
porting open-source showcase demos almost 1:1.

## Decision

1. **Logical scene data** (device-free, saved): `Geometry::Model { asset,
   mesh }` references glTF meshes by client-scoped path.
   `World.view.{camera, environment}` holds camera and lighting: an HDR asset
   path, intensity, rotation, exposure, tonemapper, sky, bloom, and a sun with
   shadows. Script API: `world.setCamera`, `world.setEnvironment`,
   `world.importModel(path, {pbr: true})`. When an environment is present,
   `World::pbr_scene` extracts it and the renderer selects the PBR path. When
   it is absent, the legacy path and its tests are unchanged.
2. **CPU asset stage** (`crysirror-asset::pbr`): glTF positions, normals,
   UVs and tangents (generated per vertex when missing), metallic-roughness
   materials (factors, five texture slots, alpha mask, double-sided,
   emissive strength), and images decoded with mip chains (sRGB-correct
   averaging). It also decodes Radiance HDR.
3. **GPU frame graph** (`crysirror-wgpu::pbr`):
   1. 4096² sun shadow map (orthographic fit to scene bounds, slope bias,
      5×5 PCF).
   2. 4× MSAA HDR (RGBA16F) opaque pass: Cook-Torrance GGX,
      height-correlated Smith, Schlick, normal/AO/emissive maps, alpha test.
   3. IBL: GGX importance-sampled prefiltered equirect (7 roughness mips,
      PDF-based source-mip selection), SH9 irradiance (Ramamoorthi-Hanrahan),
      512-sample split-sum BRDF LUT.
   4. Sky at the far plane, MSAA resolve.
   5. Dual-filter bloom (6 levels, Karis average).
   6. Tonemap (Khronos PBR Neutral / ACES / none) into the sRGB target, under
      the existing 2D/UI overlays.
   HDR texels are clamped to the f16 range and NaNs are filtered before
   bloom and tonemapping.
4. **Demos are ports, assets are fetched**: `tools/fetch_demo_assets.py`
   pins third-party assets by URL and SHA-256 and writes their license and
   attribution. `examples/aaa-*/assets/` is gitignored.

## Showcase ports

| Demo | Source being ported | Assets |
|---|---|---|
| examples/aaa-helmet | three.js `webgl_loader_gltf` (camera, HDR, ACES, IBL-only) | DamagedHelmet (CC-BY 4.0), Royal Esplanade (CC0) |
| examples/aaa-sponza | Filament/Bevy-style Sponza showcase | Sponza (CC-BY 3.0, Crytek), Venice Sunset (CC0) |

## Evidence (Apple M5, Metal)

- `cargo test -p crysirror-wgpu -- --ignored`: 5 passed, including
  `pbr_path_renders_sky_lit_geometry_and_sun_shadows` (sky, lit material,
  shadow difference, and compute-RT refusal of PBR scenes).
- dist/aaa-helmet.png and dist/aaa-sponza.png come from the desktop
  `render --script` path. Sponza has 103 primitives and about 70 textures;
  a release render including the asset load takes about 1.8 s wall time.
  These are not frame-time measurements.

## Added 2026-10-24 (second slice)

- 4 stable cascaded shadow maps (practical splits λ=0.75, texel-snapped
  centers, cascade blending, 2048² array), replacing the single map.
- 1× depth/view-normal/motion-vector prepass (instances carry stable keys
  and previous model matrices).
- Hemisphere SSAO (16 samples, interleaved-gradient noise) with a
  depth-aware blur, applied to ambient light.
- TAA: Halton(2,3) jitter, longest-neighbour motion reprojection, YCoCg
  neighbourhood clamp in tonemap-weighted space, ping-pong history. Still
  captures accumulate 16 frames.
- Settings: `ambient_occlusion`, `temporal_aa`, `shadow_distance`.

## Not yet done (ordered roadmap)

1. Screen-space reflections; GTAO/bent normals.
2. Transparent (alpha-blend) pass with sorting; KHR material extensions
   (clearcoat, transmission, sheen); texture transforms.
3. Skinning and animation from glTF; GPU culling, LOD and instancing;
   async asset streaming; texture compression (KTX2/BC/ASTC).
4. FSR/MetalFX/DLSS plugins on top of the TAA inputs (motion vectors,
   jitter and depth now exist; core `Upscaler` contract, ADR 0007).
5. Hardware ray tracing (wgpu upgrade) for RT shadows, reflections and GI.
   The compute ray tracer currently refuses PBR scenes visibly.
6. Frame-time profiling (GPU timestamps) and a performance budget per demo.
