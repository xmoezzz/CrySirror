# 0027: Virtual shadow maps for the sun (step 1)

Date: 2026-10-25. Status: implemented and locally validated for the
directional sun. Local lights keep the ADR 0018 atlas tiles.

## Context

Cascaded shadow maps (ADR 0010) spend a fixed 4 × 2048² on the view frustum
and end at `shadow_distance`. Large imported levels need sharp shadows near
the camera and correct shadows far away. UE5's answer is virtual shadow maps:
a very large virtual resolution of which only pages that visible pixels
sample are allocated, rendered and cached. This is our own implementation
of that idea, sized to the existing bind-group budget. The frame group was
already at 16/16 sampled textures and 4/4 fragment storage buffers under the
downlevel limits.

## Decision

`Environment.virtual_shadows = {texel_size = 0.02 m, levels = 10}`. It
replaces the sun's cascades when set and the sun casts shadows. Validated,
and mirrored in `tools/engine_schema.py`.

- **Virtual layout** (`vsm.rs`).
  - Clipmap levels: each level is 64×64 pages of 128² (8192² virtual texels),
    and level l has texel `texel_size·2^l`.
  - Light view: `look_at` from the origin along the sun. The orthographic
    depth range is the scene sphere along the light.
  - Pages are keyed by absolute light-space page coordinates. Each level's
    window is centred on the camera and addressed toroidally (slot =
    coordinate mod 64), so pages keep their keys and cache entries while the
    camera moves.
- **Physical pages.** The shared shadow atlas grows to 8192×4096. The left
  half keeps the 64 local-light tiles; the right half holds 1024 physical
  pages of 128². The atlas pass now loads instead of clearing, and tiles and
  pages are cleared individually with a depth-only full-viewport pipeline.
- **Page table.** Stored after the 64 local tile matrices in the existing
  binding-12 storage buffer: `ShadowData { tiles, pages: array<vec2<u32>> }`.
  Each entry holds the packed absolute coordinates (to verify the slot) and
  the physical page + 1.
- **Marking.** After the depth prepass, a compute pass reconstructs every
  pixel's world position, picks the finest level whose texel covers the pixel
  footprint (`dist · 2·tan(fov/2)/height`), and does an `atomicOr` on the page
  in that level's window. The marks are copied to a map-read buffer and
  mapped after submit.
- **Allocation.** Next frame, the CPU waits for that readback (one frame of
  latency; a CPU wait if the GPU is behind) and decodes keys with the window
  centres used at marking time. `PageCache` then:
  - allocates free pages, or evicts least-recently-used pages not needed
    this frame;
  - counts drops when the pool is full;
  - renders only new pages, culling casters by bounding sphere against each
    page's light-space square, with the cluster-LOD cut of ADR 0026.
  The cache key covers light view, depth range, texel, levels and a hash of
  all draws and object matrices. Any change invalidates every page. There is
  no static/dynamic split yet.
- **Sampling.** `vsm_shadow` uses the same level rule and tries up to two
  coarser levels if the page is missing. It uses 3×3 hardware-compare PCF
  clamped inside the page, with a slope-scaled bias of 1.5 texels plus
  2·tan θ in that level's texel size.
- Auto exposure does not keep history from frames without resident pages,
  because those frames are unshadowed. Captures render at least 3 frames
  with VSM.

## Validation (2026-10-25, Apple M5 / Metal)

- Unit (`vsm.rs`):
  - Toroidal slot round trip for negative and positive pages.
  - The cache renders only new pages and invalidates on geometry change.
  - LRU eviction never evicts pages needed this frame, and drops are counted.
  - Table slots and decoded requests agree; out-of-window pages are absent.
  - The page projection maps the page square and depth range to NDC.
- GPU `virtual_shadow_maps_shadow_beyond_cascade_range_and_cache_pages`:
  ground and a 3 m box, sun from (0.6, −1, 0), camera about 31 m away,
  cascades limited to 10 m.
  - VSM: the ground behind the box (+X) is below half the lit ground.
  - Cascades: no shadow at that distance.
  - A second capture renders 0 pages, keeps the same resident count and
    produces a byte-identical image.
  - Captures: `dist/vsm-test.png`, `dist/vsm-csm-test.png`.
- The full explicit GPU suite passes after the atlas and binding-12 layout
  change; local point/spot shadows are unchanged.
- Real content: Ashen Vow castle `close` view (texel 0.01, 12 levels)
  requests 55 pages with 57 resident and 0 dropped.
  - Against the cascade render: 1.3% of pixels differ by more than 8/255 with
    auto exposure off, and 0.84% with it on.
  - Before the exposure fix the difference was 99.6%: the first frame had no
    pages yet and seeded a too-bright exposure.

## Gaps (next steps)

- GPU-side allocation (no readback wait) and per-page caster culling on the
  GPU with the cluster hierarchy.
- VSM for local lights.
- Soft shadows (SMRT-style filtering).
- Page border filtering across neighbours.
- Frame-time and memory measurements. The atlas is now 8192×4096 Depth32
  (128 MiB).

## Static/dynamic split (2026-10-25)

- **Identity.** Each draw has a stable key (instance key mixed with the
  node and primitive index).
- **Dirty pages.** Each frame the caster bounding spheres are compared with
  last frame's. Added, removed, moved and skinned casters mark their old and
  new light-space footprints. Resident pages whose square overlaps a mark
  are dropped and re-rendered if requested; all other pages stay cached.
  Depth along the light does not matter for this test.
- **Depth range.** The range along the light only grows, with a 25%
  margin, so moving casters do not re-key the whole cache. Geometry
  changes no longer take part in the cache key.
- **Validation.**
  - Unit: a caster invalidates exactly the two overlapping pages (level 0
    and the coarser level 1).
  - GPU `moving_casters_rerender_only_their_virtual_shadow_pages`: after a
    box moves 6 m, 4 of 8 resident pages are re-rendered and 0 full
    invalidations occur. The unmoved pillar's shadow pixels are identical,
    the old shadow is gone and the shadow follows the box.
  - The far shadow edge is softened by the level-4 texels (0.32 m) that a
    180-px-high view selects.
