# 0055: RT from renderer GPU streams; dynamic BLAS; alpha-tested casters

Date: 2026-10-26. Status: implemented (Metal validated). Extends ADR 0051 and
the instancing of ADR 0052.

## Context

The PBR hardware path (ADR 0051/0052) copied every primitive's triangles to
the CPU, skinned them on workers per pose, keyed BLAS by position content,
created a new BLAS and buffer for each new pose, and rejected masked or
transparent casters. Gaps listed by Codex: efficient dynamic scenes (BLAS
caching, instancing, dynamic update/refit), alpha-tested occluders.

## Decision

1. **BLAS inputs are the renderer's own GPU streams.** With the ray-query
   feature, primitive vertex/index/skin buffers additionally carry
   `STORAGE | BLAS_INPUT`. Static BLAS read the 48-byte PbrVertex stream
   (positions at offset 0) with the u32 index buffer; nothing is copied to
   the CPU. Static geometry is identified by its source primitive
   (model key, mesh, primitive); per-frame flat primitives by content.
2. **Dynamic BLAS.** Skinned or morphed instances own a resident BLAS and
   a position buffer. A compute pass applies the frame's joint palette to
   the (morphed) vertex stream; the BLAS is rebuilt in place only when the
   hash of that instance's palette and morph weights changes. Flags:
   `PREFER_FAST_BUILD | ALLOW_UPDATE`, `PreferUpdate`. **wgpu 29.0.x
   implements `PreferUpdate` as a full rebuild** (wgpu-core logs this), so
   this is a fast rebuild, not a refit; it becomes a refit when wgpu does,
   without engine changes. Budget: 256 dynamic BLAS.
3. **TLAS** carries one instance per draw with the draw index as custom
   data and is rebuilt when an instance moved or any BLAS was (re)built.
4. **Alpha-tested casters.** Naga 29's Metal backend uses a synchronous
   intersector and does not implement candidate confirmation, so any-hit
   style candidate loops are not available. Instead:
   - per-corner UVs of alpha-tested geometry are gathered on the GPU from
     the same streams into one storage buffer;
   - each MASK material is baked once into a 128² tile of a 2048² R8 alpha
     atlas by a render pass that binds the material's own bind group and
     runs the raster coverage function (`sample_surface(..).color.a`);
   - the shadow ray re-traces past a hit whose interpolated, UV-transformed
     atlas alpha is below the material cutoff, up to 8 layers (deeper
     stacks count as occluded). Every query still proceeds once on
     force-opaque geometry.
   - Blended (transparent) materials do not cast, matching the raster
     shadow pass. Custom fragment-discard shaders and effect presets whose
     coverage depends on position/time fail visibly.
5. **Abandoned work.** If a frame's encoder is not submitted, the next
   frame forgets its BLAS builds, UV gathers and atlas tiles and redoes them.

## Not done

Soft shadows, local-light RT shadows, RT reflections/GI and denoising
(the next steps); compaction; per-instance LOD for RT.

## Evidence

- CPU: shader validation test (one `rayQueryProceed` per query, no loops on
  proceed flags).
- Apple M5 / Metal, explicit `cargo test --release -p crysirror-wgpu --test
  hardware_rt -- --ignored --test-threads=1`: 4 passed — shared BLAS for
  identical cubes and TLAS-only rebuild on rigid motion; uniform-alpha mask
  (alpha 1 casts, 0.2 relights the floor); a 2×2 checker alpha texture
  halves the shadow (410 → 205 pixels); a skinned FBX bar reuses its
  dynamic BLAS for an unchanged pose and rebuilds it when bent.
- Khronos Sponza (local, not distributed), release desktop render with
  `--renderer hardware-raytrace`: 103 instances, 14 alpha-tested (3 baked
  materials), static BLAS 103, TLAS builds 1.
