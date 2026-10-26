# 0050: Asynchronous, multithreaded execution across the entire engine

Date: 2026-10-26. Status: **accepted architecture requirement; partially implemented**.
Owner direction: asynchronous/multithreaded support is required throughout
CrySirror. The worker pools and asset/pose work in ADR 0049 are stage one.
They do not complete this requirement.

## Decision

All engine subsystems must participate in a shared asynchronous execution model.
This includes World/gameplay, rendering, resources, physics, animation, audio,
video, text, UI, scripts/native adapters, save/restore, tooling and MCP.
New capabilities must expose scheduling, readiness and lifetime boundaries;
placing only file reads on background threads does not satisfy this decision.

Async support does not require independent operations to mutate the same state
concurrently, nor every small function to return a Future. Ordering and thread
affinity must be explicit dependencies and short commit phases. They must not
be used as a permanent reason to exclude an entire subsystem from this design.
For example, a native interpreter executes its instructions in their original
order while suspending on a shared service operation; winit owns the platform
event loop while CPU preparation, GPU submissions and completion fences form
an asynchronous pipeline around it.

## Engine execution contract

1. Use shared, bounded executors/queues with capacity backpressure and explicit
   dependencies. No per-resource thread spawning or hidden unbounded queues.
2. Give each operation a client/generation identity and observable
   pending/running/ready/failed/cancelled state, as appropriate. Rust callers can
   await asynchronous results; script/tool bindings must provide equivalent
   completion semantics. Current polling script handles are an interim binding,
   not completed Promise support.
3. Workers receive immutable snapshots or exclusively owned data. Stage changes
   as typed results/command buffers and validate ownership/generation at commit.
   Do not share mutable World/VM state behind a global lock as the architecture.
4. Build a frame dependency graph: input and logical updates, fixed simulation,
   animation/IK, spatial queries/culling, render extraction/preparation, GPU
   upload/encoding/submission and presentation. Run independent work in parallel;
   use bounded snapshots/buffers to overlap CPU preparation and GPU execution
   where dependencies permit. Preserve declared fixed-step/event ordering.
5. Keep expensive reads, decoding, compilation and cooking out of latency-critical
   event/presentation/audio paths. Use upload and completion budgets; a worker
   finishing a large model must not merely move the stall to GPU upload.
6. Cancellation, deadlines where applicable, errors, shutdown/drain and stale
   result rejection are part of the service contract. Cancellation is cooperative;
   never claim a blocking foreign/provider call can be forcibly interrupted.
7. Await dependency completion without blocking a worker on its own executor.
   Explicit synchronous adapters are limited to offline tools, initialization,
   controlled shutdown and documented compatibility transitions. A synchronous
   fallback is not completion of the asynchronous migration.
8. Keep task/executor/device handles and executable code outside project data
   and saves. Preserve capability roots, client isolation and original VM/save
   formats; unsupported operations fail visibly.

## Required subsystem migration

| Subsystem | Required asynchronous/multithreaded boundary | Current coverage / remaining work |
| --- | --- | --- |
| World, gameplay, AI, navigation | Dependency-aware simulation jobs, spatial/navigation jobs and immutable render extraction; deterministic commits | Owner-thread updates remain; general frame graph/system parallelism is outstanding |
| Physics | Background shape/mesh cooking and independent simulation/query work, with fixed-step barriers and event publication | Rapier backend exists; general scheduling/cooking migration remains |
| Animation | Parallel sampling, blending, IK/retargeting preparation and ordered event/root-motion application | Independent PBR poses run in workers; broader animation pipeline remains |
| Renderer | Parallel CPU preparation/culling, budgeted asynchronous uploads, pipeline compilation, GPU submission/completion and frame overlap | Async image/model input exists; uploads, pipelines, LOD and general frame overlap remain |
| 2D layers, Canvas, filters, transitions | Background preparation/composition work and retained scene snapshots; transitions wait on declared resource readiness | Shared layers/Canvas exist; full async preparation and transition fences remain |
| Resources | Scoped asynchronous reads, archive/decompression/decryption, decoding/import/cook, streaming/unload and revision invalidation | ADR 0049 covers initial reads/image/model imports; broader streaming/cook/lifetime pipeline remains |
| Audio | Streaming decode/DSP scheduling with bounded command/sample queues; real-time callback consumes ready data without blocking | Existing mixer/media workers are partial coverage; no whole-audio migration claim |
| Video | Asynchronous demux/decode, bounded video/audio queues, synchronized presentation, seek/cancel and teardown | Existing media workers cover a bounded profile; streaming/seek and full integration remain |
| Text and UI | Font loading, shaping/layout/raster/atlas preparation as jobs; ordered input/focus and short UI-state commits | ADR 0066 adds worker rich-text preparation/reveal and shared menus; font loading and GPU atlas boundaries remain |
| Scripts and native VMs | Ordered interpreter execution with service suspension/resumption, off-thread compilation and typed owner-thread commits | ADR 0066 adds bounded JS service Promise bindings; native execution/storage callbacks and startup compilation still have synchronous boundaries |
| Save/restore | Asynchronous staging/serialization/I/O/reconstruction followed by an atomic validated safepoint commit | ADR 0066 adds worker encoding/I/O/validation and atomic prepared commit; native checkpoint capture/reconstruction remain ordered owner operations |
| Editor, authoring, MCP | Asynchronous import/build/inspection/capture/edit operations, cancellation/progress and live-session completion handling | Existing APIs/worker inspection are partial; end-to-end async operation protocol remains |

These are acceptance requirements, not declarations that the listed work has
already been implemented. A serial owner-thread commit or fixed-step barrier
can remain after expensive/preparable work is scheduled appropriately.

## Stages and acceptance

- **Stage 1, ADR 0049:** shared bounded workers, awaitable reads, initial async
  image/model imports and independent pose jobs. Implemented and separately
  validated; this is only the starting point.
- **Stage 2:** establish the engine frame graph and snapshot/command-buffer
  boundaries; remove mandatory main-thread waits from interactive preparation
  and give GPU uploads/pipeline work explicit budgets/completion tracking.
- **Stage 3:** migrate every subsystem in the table, including native service
  readiness fences and Rust/script/tool completion semantics.
- **Stage 4:** demonstrate mixed workload behavior, sustained-load/backpressure,
  cancellation/shutdown, resource reload, VM ordering, save/restore and desktop
  platform operation with actual results.

Every migration records implementation, compilation and behavior separately.
Measure CPU/GPU frame stages, event latency and memory/queue limits on a mixed
scene while storage/decoders/cooking are deliberately delayed. Compare serial
and parallel results where order is required. Local GPU/hardware execution and
other-platform results must be reported explicitly; unrun tests and a written
plan are not passes. Do not mark the engine-wide requirement complete while
any table row lacks its declared boundary and behavioral evidence.

## Validation and automation policy

Validation is local by default. The owner has requested removal of GitHub CI.
Do not add, restore or enable GitHub Actions/CI or other remote build automation
without a new explicit owner request. This does not prohibit running appropriate
local checks/tests for actual changes.
