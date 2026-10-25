# 0051: Hardware ray queries with capability-based backend selection

Date: 2026-10-26. Status: accepted; implementation, compilation and physical
hardware behavior must be recorded separately in STATUS.md. This decision
introduces an initial hardware ray-query path, not a completed path tracer or
blanket compatibility with every PBR material/geometry variant.

## Decision

Use CrySirror's shared scene/material ownership and the native ray-query API
provided by wgpu. Actual intersections must traverse built BLAS/TLAS acceleration
structures with `rayQuery` shader operations. The existing compute path performs
software ray/triangle traversal and remains a separate named capability. Neither
software traversal nor raster output may be labeled hardware ray tracing.

wgpu is pinned to 29.0.3 for this migration; its compatible resolved core/HAL
packages in Cargo.lock are 29.0.4. Negotiation was checked against those exact
installed package sources and the versioned source links below, rather than
assuming the moving upstream branch has the same API. Future dependency upgrades
must revalidate the feature, limit and experimental-consent contract. The packaged public feature comment still
lists Vulkan alone, while these backend sources include runtime feature gating
for Vulkan, DX12 and Metal. Backend implementation and the actual adapter's
features/limits decide availability; a marketing name or generic feature-list
comment does not establish support or a successful rendered frame.

## Capability and device contract

`rt_capabilities` reports adapter support separately from device enablement:

- Adapter identity, backend, vendor/device IDs and device type are diagnostic
  metadata. No NVIDIA/AMD/Intel/Apple name or vendor-ID allowlist is used.
- `ray_query_advertised` is the adapter's actual
  `Features::EXPERIMENTAL_RAY_QUERY` bit. `available` additionally requires a
  supported native backend, a non-CPU adapter and usable acceleration-structure
  limits. CPU/software adapters are not described as hardware RT even if their
  API can emulate a ray-query feature.
- `ray_query_enabled` is the feature actually requested on the device. `enabled`
  additionally requires the device's enabled AS limits. A raster device can
  therefore report `available=true, enabled=false` without implicitly opting in.
- Reports retain adapter and device AS limits plus a concrete unavailable or
  disabled reason. A report is not a physical-GPU execution or performance test.
- Native API eligibility does not independently prove dedicated RT silicon.
  In particular, Metal API ray-tracing support and the hardware intersector
  described for Apple family 9 are different facts. Portable wgpu does not
  expose a dedicated-RT-unit proof flag. The machine-readable
  `dedicated_rt_hardware_verified` field therefore remains `null` (unverified),
  including when native ray queries are enabled. Record API execution separately from
  any model-specific silicon claim, and never infer hardware-unit performance
  from `supportsRaytracing`, a non-CPU adapter type or a brand name alone.
- The initial device request uses at most 1,000,000 BLAS primitives, one BLAS
  geometry descriptor, one TLAS instance and one acceleration structure per
  shader stage, bounded by the adapter's real limits. These are initial engine
  budgets, not invented hardware limits or a permanent instancing design.

Request the experimental feature only for an explicitly selected hardware path.
The wgpu 29 API requires an unsafe experimental-consent token because upstream
experimental implementations may contain unsoundness. Isolate this acknowledgement
in the renderer's opt-in boundary; it does not waive shader/AS validation or
permit raw GPU pointers in World, project data, saves, core or compatibility
crates. Core/compat retain their unsafe-code/device-dependency restrictions.

A required hardware path either obtains its negotiated device or fails with
adapter/backend/reason context. It must not silently choose compute or raster.
Any future automatic policy must disclose the selected path and the fallback
reason rather than claiming hardware RT remained active.

## Multiple hardware/backend targets

| Backend / target adapters | Runtime eligibility | Physical validation requirement |
| --- | --- | --- |
| Vulkan on Windows/Linux, including NVIDIA, AMD and Intel GPUs | Driver exposes the required acceleration-structure/ray-query API and limits | Record actual adapter, driver, negotiated limits and rendered pixels per tested device; no universal vendor claim |
| DX12 on Windows, including NVIDIA, AMD and Intel GPUs | Resolved HAL gates ray queries on DXR tier 1.1 and shader model 6.5 support | Real Windows/DX12 execution remains a separate validation task |
| Metal on macOS, including eligible Apple/other Metal devices | Resolved HAL checks OS/API availability and both `supportsRaytracing` and `supportsRaytracingFromRender` | Real Metal execution must be recorded independently; Apple branding alone is insufficient |
| CPU/software, GL/GLES, browser or noop backend | No hardware path under this contract | Visible unsupported result; raster/compute remain explicitly separate selections |

The CPU capability tests use synthetic backend/vendor/feature/limit inputs to
check this policy. They cannot establish execution on any of the named GPU
manufacturers. A local Metal pass would not validate Vulkan or DX12.

## Rendering and asynchronous ownership

The initial implementation uses hardware primary/hard-shadow rays for the
bounded simple scene profile and a hybrid hardware sun-shadow stage for the PBR
renderer. The latter reconstructs world positions from the jittered depth/normal
prepass and packs visibility alongside existing AO, preserving the current
sampled-texture binding budget. PBR material shading, local lights, probes and
other effects retain their engine-owned pipelines. Probe views retain their
existing shadow maps rather than sampling the main camera's visibility mask.
Source model/terrain/triangle positions use the same transforms and current
skinning palette as raster rendering. Masked/transparent casters and custom
fragment-discard materials are rejected by this first profile.
Unsupported variants fail visibly;
no private native-engine renderer or simplified silently substituted material
pipeline is introduced to make a title appear supported.

Follow [ADR 0050](0050-engine-wide-async-execution.md): scene/geometry preparation
uses immutable data and task dependencies; AS builds, queries and consumers are
ordered GPU work with explicit lifetimes and completion boundaries. CPU workers
do not mutate World or device state. Acceleration structures, buffers, device
features and executable shaders are transient renderer resources, never save
format substitutes. Cache identity includes geometry/resource generations;
changed or removed geometry must not leave stale ray intersections behind.

The initial AS contains one world-space triangle geometry and one identity TLAS
instance. Geometry positions are compared before rebuild; metadata-only changes
upload metadata without rebuilding. Cache state becomes built only after the
encoder containing the build was submitted. An abandoned encoder cannot cause a
later frame to query an unbuilt AS. Empty scenes use a clear entry point without
a dummy AS. Triangle preparation runs on the existing bounded CPU workers with
an explicit join before encoding; GPU submission remains on its renderer owner.
This is not yet a fully overlapping frame graph, per-mesh instancing, dynamic
refit or a claim of AAA-scale performance.

All query geometry is force-opaque triangles, so a single `rayQueryProceed`
completes traversal without candidate callbacks. This matters for the pinned
Naga 29.0.3 Metal translator: its synchronous intersector initializes an emulated
ready flag which `Proceed` reads without resetting, making a conventional
`while rayQueryProceed` loop non-terminating. The opaque-only single-proceed
implementation was verified on Metal. Adding alpha-tested or procedural
candidate handling requires revisiting this backend/version constraint rather
than reusing the opaque-only assumption.

## Validation and remaining boundaries

Record three distinct results: implemented source/contracts; successful
compilation/CPU tests; explicit GPU rendering on identified hardware. Tests must
cover unsupported-device rejection, adapter-versus-device capability reporting,
negotiated limits, primary hits/occlusion or hybrid shadows, changed/removed
geometry, 2D/UI overlays and the existing raster/compute regressions. An ignored
GPU test is not a pass. CPU mocks are not hardware results.

This decision does not claim full ray-tracing pipelines/SBTs, general path
tracing, denoising, unlimited scene sizes, multi-bounce GI, transparent/alpha-test
any-hit shading, complete dynamic BLAS refit/compaction or performance gains.
Each additional feature needs an engine-owned implementation and its own
behavioral evidence. GitHub CI remains disabled; run validation locally.

## Source references

- [Apple Metal ray-tracing API overview](https://developer.apple.com/documentation/metal/ray-tracing-with-acceleration-structures)
- [Apple GPU architecture talk](https://developer.apple.com/videos/play/tech-talks/111375/)
- [Apple Metal feature tables](https://developer.apple.com/metal/capabilities/)
- [wgpu 29.0.3 device API](https://docs.rs/wgpu/29.0.3/wgpu/struct.Device.html)
- [wgpu-types 29.0.3 experimental feature token](https://docs.rs/wgpu-types/29.0.3/wgpu_types/struct.ExperimentalFeatures.html)
- [wgpu HAL 29.0.4 Metal source](https://docs.rs/crate/wgpu-hal/29.0.4/source/src/metal/adapter.rs)
- [wgpu HAL 29.0.4 DX12 source](https://docs.rs/crate/wgpu-hal/29.0.4/source/src/dx12/adapter.rs)
- [wgpu HAL 29.0.4 Vulkan source](https://docs.rs/crate/wgpu-hal/29.0.4/source/src/vulkan/adapter.rs)
