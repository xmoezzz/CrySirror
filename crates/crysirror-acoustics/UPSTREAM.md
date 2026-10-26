# Acoustic source and dependency boundary

The reference is the complete checkout of
[GitGhillie/phonon_rs](https://github.com/GitGhillie/phonon_rs) at
`87c392a3e605f9df30abe6e291f0bbc5e86f54ad`. `upstream-files.json`
records the SHA-256 inventory of all Rust library/test/benchmark sources in
`crates/phonon`. Identity inventory is separate from implementation coverage.
The original Apache license and Valve/phonon notices are preserved here.
No demo, music, measured SOFA/HRIR or font data is distributed by CrySirror.

Source review for this profile covers `simulators/direct.rs`, `models/*`,
`scene/*` and direct/panning/EQ/IIR/buffer DSP. `direct.rs` adapts exponential
per-band air attenuation, weighted dipole directivity and deterministic
Halton volume visibility. The scene, task ownership, JSON/API boundary and
persistent mixer filter are CrySirror implementations over existing libraries.
This is a direct-sound profile, not the whole Steam Audio or phonon library.
The reference binaural and reverb paths were inspected but are not imported.

Semantic differences are deliberate:

- The reference static mesh query shifts the ray origin by `min_distance`
  but reports relative hit distance and retains the unadjusted maximum.
  CrySirror preserves original-origin distance and the remaining interval.
- Acoustic materials are selected per triangle, rather than one per mesh.
  Transmission is amplitude per crossed surface, multiplied at every crossing;
  a slab with two surfaces incurs two factors. We do not use the reference's
  square-root heuristic for pairing entry and exit hits.
- Transforms are validated and baked before BVH publication, including
  nonuniform scale. Normals derive from world triangles. Mutable nested scene
  mutexes and untracked instance-transform mutation are not copied.
- Geometry/pose inputs are immutable job snapshots. Clients own independent
  scene revisions. The render/mixer thread never traverses geometry or waits
  for a propagation job. This does not make all audio decoding/mixing async.
- Absorption/scattering fields are validated metadata reserved for reflections;
  they do not currently generate reflected energy. Travel time is reported,
  not applied as playback delay. HRTF now has a separate worker-prepared module (ADR 0068);
  diffraction, reflections and reverb remain follow-up work. Existing stereo panning remains in the engine mixer.

BVH implementations are external libraries, not handwritten tree builders:

- `parry3d = 0.17.6` (already used by the engine's Rapier 0.22 physics backend)
  provides default TriMesh/QBVH queries and primitive intersection.
- Optional `obvhs = 0.4.0`, registry revision
  `2cf08de9d804196df1fc8b257326dfc35ea4d844`, provides medium-quality PLOC BVH2
  build/traversal. Spatial splitting and nested Rayon workers are disabled.
  This profile does not exercise CWBVH or GPU traversal. Its triangle primitive
  rejects signed negative zero on some shared edges; regression tests caught
  missed wall hits. The callback therefore uses Parry's triangle intersection
  for both sides and inclusive shared edges, retaining obvhs BVH traversal.

Run `cargo run --release -p crysirror-acoustics --features obvhs --example
bvh_compare` for an analytic-oracle-checked vertical-ray workload. This is a
reproducible microbenchmark, not a cross-hardware engine ranking. The optional
backend does not establish a speed advantage without workload measurements.
A full native Steam Audio binding remains an alternative for capabilities not
covered here. Steam Audio has selectable default/Embree/Radeon Rays/custom
scene backends; Embree is not an unconditional dependency of every backend.
