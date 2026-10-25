# 0052: Frame extraction graph, profiling, streamed cells and mesh RT instances

Date: 2026-10-26. Status: implemented slice of ADR 0050; validation is recorded
separately in STATUS.md. This does not complete engine-wide asynchronous execution.

## Dependencies and ownership

`crysirror-tasks::graph::FrameGraph` runs independent jobs on bounded workers.
Dependencies are scoped node IDs referencing earlier nodes, preventing cycles
and cross-graph result access. Nodes consume immutable `Arc` results. Capacity
pressure retains the original closure for retry; errors cancel remaining jobs
and prevent dependent work. The graph permits at most 256 nodes. Offline tools
can explicitly join; a worker cannot wait on its own pool.

Runtime freezes logical engine state and dispatches PBR, simple geometry and
Layer/UI extraction independently, followed by a dependency join. Desktop keeps
one pending and one ready snapshot. It polls without a worker join and can keep
presenting the ready snapshot while simulation advances. Restore/resource-store
identity invalidates pending and displayed snapshots. Dynamic surface generations
and transition state are frozen with the snapshot; scoped renderer lookup restores
live resource access on error/unwind. No worker mutates World,
VM state, physics or device state. World snapshot cloning still costs owner CPU
time and is measured; this is not yet a persistent/COW scene representation.

Simulation, VM, lifecycle and fixed-step ordering remains explicit on the owner.
General simulation job dependencies, GPU upload/compilation budgets, collision
cooking, media/text migration and broader frame overlap remain ADR 0050 work.
Existing pose/RT preparation barriers are not claimed to be nonblocking.

## Profiling

The engine-owned `Profiler` class exposes `snapshot`, `trace` and `clear` to Rust,
JS/TS and tools through the common class registry. Shared instrumentation retains
at most 8192 samples, with eviction counts, thread/frame IDs and stage mean,
p50/p95/max microseconds. CPU spans use one monotonic origin; inclusive nested
spans must not be summed as independent frame costs. Runtime, worker extraction,
asset preparation, streaming commits and renderer preparation are instrumented.

Supported devices opt into `TIMESTAMP_QUERY`. A four-slot readback ring brackets
GPU work with real compute marker passes and pass-boundary timestamps. A frame-completion callback precedes a separate
resolve/readback submission; publication waits for that mapping. Markers
write storage so they cannot be elided as empty encoders. The encoder timestamp
route returned zero samples in the local Metal run; zero/non-increasing samples
are rejected rather than reported as zero-cost frames. Readbacks are polled;
ring pressure drops samples rather than stalling presentation. Explicit capture
and controlled shutdown have a bounded completion fence; live frames do not. Reports separate
enabled capability, valid sample counts, pending/dropped/failed samples.

GPU durations are bracketed by marker passes; sampling adds overhead. They
cover encoded work, not presentation,
input latency or complete application FPS. GPU and CPU clocks are not calibrated.
GPU duration summaries are therefore separate from the CPU Chrome trace. `run
--profile PATH` exports a trace plus timing diagnostics. Offscreen movie reports
retain at most 256 per-frame records; capture wall time includes warmup/readback
and is explicitly not an interactive frame-rate benchmark.

## Asynchronous streaming cells

`World.streamLevel` uses shared client-scoped resource jobs. `scene` prepares
baked entities; `level` prepares both the PBR entity hierarchy and decoded model
on background workers. The renderer reuses that decoded model. Owner commits
remap IDs/parents, preserve placement and publish the hierarchy atomically, at
most two cell commits per interactive frame (offline fences drain bounded batches). Until ready, no placeholder root is advertised
as a loaded level. Failures surface with level context. Moving away, replacing a
spec or restoring rejects stale publication.

Scoped resource leases pin pending/loaded cells. Explicit asset request IDs keep
their prior caller-owned lifetime. Managed unpinned/unshared payloads can be
collected; shared decoded models remain alive while the renderer holds them.
Unused PBR GPU model caches and RT geometry caches are released when their
scene references disappear. General managed cache entries have an idle grace
period. CPU prepared-cache accounting defaults to 256 MiB. The host can explicitly set
16–4096 MiB with `--asset-cache-mib` (or its Rust policy API), with live residency
checks and no pointers in project/save data. The budget does not bound every
transient decoder allocation, World snapshot, renderer cache or driver resource.

Offline capture has an explicit streaming readiness fence. It waits for resource
preparation and performs the normal owner commit without advancing scripts,
physics, input or clocks a second time. The interactive window does not call
this fence. Collision shape cooking and complete GPU uploads can still block
their owner; source/mip/chunk paging, world partition, an asset GUID database,
incremental cooking and fully budgeted upload/eviction remain separate work.

## Instanced native ray queries

The PBR shadow path caches local geometry by source and pose. Source Arcs prevent
pointer identity reuse; rigid transforms are excluded from geometry identity.
Native BLAS are shared by equal local positions, with full equality checks on
hash matches. TLAS instances carry affine world transforms. Static frames reuse
the TLAS; rigid motion rebuilds TLAS without rebuilding shared BLAS. Removed
meshes/instances disappear from caches and queries. Builds are committed only
after the submission containing them succeeds; abandoned encoders remain dirty.

The negotiated budget is up to 4096 TLAS instances, 1024 unique meshes and
1,000,000 unique triangles, bounded by actual adapter limits. Instance-expanded
triangle counts are diagnostic and may exceed the unique geometry budget.
The simple primary-ray path retains its existing bounded world-space profile.
Skinned pose changes still require geometry rebuilding; dynamic refit,
compaction, alpha-tested candidates, RT reflections/GI and denoising remain open.

## Demonstrations and acceptance

Use public Khronos DamagedHelmet and Sponza assets, with download manifests,
hashes and attribution kept alongside ignored local assets. They are inputs to
CrySirror's importer, World, resource service and renderer. No third-party
renderer or composed framebuffer is hosted. Recipes live in
`examples/aaa-helmet/{instances.json,streaming.ts}` and
`examples/aaa-fox-sponza/streaming.ts`; commands are in FRAME_PIPELINE.md.

Acceptance distinguishes implementation, compilation/CPU tests and actual GPU
results. Verify delayed workers with advancing simulation, immutable snapshots,
restore rejection, graph backpressure/errors, leased resource reclamation,
cell hierarchy/collision/save behavior, moving/shared/removed RT instances and
visible demo frames. Record hardware and sample loss; unavailable/ignored GPU
tests are not passes. GitHub CI remains disabled.
