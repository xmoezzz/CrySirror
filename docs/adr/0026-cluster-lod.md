# 0026: Cluster LOD (virtualized geometry, step 1)

Date: 2026-10-25. Status: implemented and locally validated, with CPU
selection (step 1) and GPU selection and frustum culling (step 2, below).

## Context

The owner asked for UE5's key capabilities, implemented independently.
Nanite's core is a crack-free cluster LOD hierarchy whose cut is chosen per
view by projected error. Imported scenes are dense: the OLY100 preview has
999,296 source triangles. This step builds that hierarchy and draws its cut
through the existing PBR passes.

## Decision

`crysirror-asset::cluster` is our own implementation (no external
simplifier).
- **Clusters.** At most 128 triangles each, grown greedily by adjacency
  toward the running centroid. Islands of ≤ 16 triangles are folded into the
  neighbour with most shared edges while it has room.
- **Simplifier.** Half-edge-collapse QEM on positions with these
  constraints:
  - Vertices on open edges (group borders) are locked.
  - Collapses are rejected if they fail the link condition, flip a triangle
    or create a degenerate one.
  - No new vertices are created.
- **DAG.**
  - Adjacent clusters are grouped (about 4; a singleton joins its best
    neighbouring group). Each group is merged and simplified to half, then
    split again.
  - All clusters made from one group share its error (max of child errors
    and its own) and an enclosing sphere. Children store them as parent
    error and bounds.
  - Groups that fail to reduce by 15% are regrouped on the next pass; they
    become roots only when a whole pass makes no progress.
- **Seams.** Topology is welded by exact position, with −0.0 equal to +0.0,
  so UV and normal seams do not lock simplification. Triangles degenerate
  after welding are dropped. Leaf clusters keep the original indices, so
  seams are exact at full detail; coarser clusters use one representative
  vertex per position.
- **Selection.** A cluster is drawn iff `error/(dist−r) ≤ t < parent_error/
  (dist_p−r_p)`, evaluated in object space with
  `t = pixel_error · 2·tan(fov/2) / height`. Adjacent selected clusters are
  merged into index ranges. The camera cut is used by every pass: shadows,
  prepass, opaque, transparent and probe captures.
- **Opt-in.** `Environment.geometry_lod = {pixel_error = 1}`. Static
  (non-skinned) model primitives above 256 triangles get a DAG at load; a
  model loaded earlier without one is reloaded. Statistics are in
  `PbrRenderer::lod_stats()` and in the desktop `render` JSON (`stats`).

## Validation (2026-10-25)

- Unit (`crysirror-asset`):
  - Split size bound and triangle conservation.
  - The simplifier keeps a closed sphere closed and manifold at half size.
  - Borders are locked on an open grid.
  - DAG on a 9k-triangle welded sphere: errors grow and spheres enclose
    upward; a zero threshold gives exactly full detail; triangle counts
    decrease with the threshold; there are no odd-count (open) edges at any
    threshold or viewpoint.
  - An unwelded UV sphere with seams simplifies to under 1/10 and has no
    position-space cracks.
- Known property: two neighbouring groups can independently create the same
  chord between locked border vertices. The cut then has an edge used 4
  times (0–2 such edges per cut in the test). The surface stays closed, with
  no open edges.
- DAG shape on a 36k-triangle seamed sphere: about 2× fewer triangles per
  level over 10 levels, with a single 76-triangle root and no premature
  roots. Before the island folding and singleton regrouping, 39% of leaves
  were premature roots.
- GPU `cluster_lod_draws_fewer_triangles_far_away_with_matching_pixels`
  (36,672-triangle sphere at 1 px):
  - Full-screen view draws 4,112 triangles; at 40 m it draws 920.
  - Without LOD, no primitive is clustered.
  - The far LOD and full images differ (>24/255) in fewer than 10% of covered
    pixels.
- OLY100 hall at 1 px: 574,234 of 999,296 source triangles drawn in 10,932
  clusters. 4.9% of pixels differ from the full-detail render by more than
  8/255, and 1.2% by more than 24/255. The whole headless render including
  DAG builds took 3.25 s. No per-pass GPU timing was measured.

## Gaps (next steps)

- GPU-driven cluster culling and selection (compute plus indirect draws).
  The cut is currently CPU-side, and each range is one draw call.
- Per-cluster frustum and occlusion culling.
- A software rasterizer for small triangles.
- Streaming of cluster pages.
- Attribute-aware simplification on seams at coarse levels.
- A DAG cache on disk.
- Skinned meshes.
- Virtual shadow maps, which pair with this hierarchy and are not started.

## Step 2: GPU selection and culling (2026-10-25)

- `geometry_lod.gpu` defaults to true; false keeps the CPU reference path.
- **Selection.** `lod_select.wgsl` runs one dispatch per clustered draw and
  one workgroup per cluster. Thread 0 evaluates the DAG cut. For the main
  view it also tests the cluster's bounding sphere against the six frustum
  planes, transformed to object space so distances stay in world units.
  It then reserves output space with `atomicAdd` on the
  `DrawIndexedIndirect` index count.
- **Copy and draw.** The workgroup copies the cluster's indices into a
  compacted per-frame index buffer. Every pass draws with
  `draw_indexed_indirect`. The CPU writes only the per-draw parameters and
  argument headers and no longer walks clusters.
- **Two cuts per draw.** The main view (prepass, opaque, transparent) gets a
  frustum-culled cut. Shadows, VSM pages and captures get an unculled cut,
  so off-screen casters still cast. Each region is sized by the leaf index
  count, which bounds any cut.
- **Validation.** GPU
  `gpu_cluster_selection_matches_the_cpu_cut_and_frustum_culls_the_main_view`:
  - At 2.5, 12 and 40 m the unculled GPU index count equals the CPU cut
    exactly.
  - From 12 m on, where the sphere is fully in view, the main cut equals the
    unculled cut; at 2.5 m it is no larger.
  - Images match the CPU path within ±2 except for at most 4 pixels.
  - In a close-up of one side, the main cut is less than half of the unculled
    cut.
  - Debug readback: `PbrRenderer::debug_gpu_lod_counts`.
- **OLY100 hall.** 0.5% of pixels differ by more than 8/255 from the CPU
  path, mostly where the triangle order of coplanar decals changed. The
  headless render drops from 3.57 s to 2.71 s, both including load, DAG
  builds and 16 TAA frames. No per-pass GPU timing was measured.
- **Remaining.** Occlusion culling (HZB), per-page culling for VSM, a
  software rasterizer for tiny triangles, and streaming.
