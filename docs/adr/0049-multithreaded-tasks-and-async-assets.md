# 0049: Bounded engine workers and asynchronous resources

Date: 2026-10-26. Status: implemented **stage one only**; compilation/behavior
evidence is recorded separately in STATUS.md. [ADR 0050](0050-engine-wide-async-execution.md)
requires asynchronous/multithreaded support across the entire engine. This
worker/asset/pose implementation does not complete that requirement.

## Ownership

`crysirror-tasks` provides reusable safe CPU pools using the workspace's locked
crossbeam-channel/futures versions. Jobs own Send data/capability handles.
`Task<T>` is a real Rust Future backed by a wakeable oneshot; polling never
waits for I/O. A deliberate offline `wait` is rejected inside its own pool.
Dropping/cancelling a task requests cooperative cancellation; panics become
failed results and do not kill the worker. Shutdown cancels pending asset work
and joins workers. A blocking provider call cannot be forcibly interrupted.

Runtime Assets owns two asset workers (64 queued + running requests maximum)
and 2–4 CPU workers (256 queued + running jobs maximum). Independent character
poses use the CPU pool in bounded batches. Pose results retain instance order;
blend/hierarchy arithmetic is unchanged. GPU buffer writes and previous-frame
palettes remain on the renderer's owner thread after the pose barrier.

World, fixed-step physics, native VMs and script object state retain their
ordered owner-thread execution. Workers cannot mutate World or submit GPU
commands. This preserves the original VM opcode/wait ordering. Existing media
workers remain separate; this change does not pretend they were newly added.

## Asynchronous assets

Client-scoped bytes/image/model requests capture capability storage and a
shared decoder registry. glTF dependencies, image decoding, tangent/mip work
in the model importer execute on asset workers. Only main-thread polling or
explicit offline resolution publishes results. Renderer image and PBR model
loads consume this shared CPU cache; this is not an unused preload facade.

Desktop data-VM projects enable nonblocking loading. Projects containing native
VMs retain blocking first-use resolution until those adapters implement explicit
readiness fences; Rust hosts can select their loading policy. Pending images/models defer their
draws while already-ready content continues rendering. Loading/error state is
inspectable. Missing resources and decode failures propagate visibly; only
capacity backpressure defers submission until a later frame. Offline captures
use blocking resolution by default for complete, reproducible output.

Requests deduplicate by client, resource kind and canonical path. Tickets carry
store/client identity. Each client may have 32 loading requests; there are at
most 4096 retained entries and 256 MiB of accounted prepared CPU payloads.
Completed entries must be released when no longer needed. This accounting is
not a bound on all transient decoder allocations, exported Arc references,
existing byte caches or GPU memory.

Mount refresh/clear cancels pending work, invalidates tickets and increments
client resource revision. Late results cannot enter a new generation. Images
and model GPU cache keys include that revision. Queues, tickets and prepared
CPU/GPU resources are transient, not saved pointers or executable state.

## Public interfaces

Rust hosts can await `Assets::read_async(client, path)`. Prepared resources use
`request`, `task_status`, `cancel_task`, `release_task`, `image_for` and
`model_for`; normal runtime ticks poll completions.

The shared script class/global is `Assets` / `assets`:

```ts
const request = assets.load("models/hero.glb", "model");
// Poll from a normal update hook; this call never waits for a worker.
const status = assets.status(request);
if (status.state === "failed") throw new Error(String(status.error));
// When ready, use the same asset path in the ordinary World/Entity APIs.
// Release the request when its CPU cache entry is no longer required.
```

`assets.cancel`, `assets.release` and `assets.stats` expose cancellation,
lifetime and worker counters. Polling handles are not JavaScript Promises;
Rust task Futures are awaitable. Inspection includes worker queue/running,
completion/cancellation/panic/rejection counters and owned resource states.

## Remaining work

This does not introduce a general frame dependency graph or overlap the entire
simulation frame with rendering. GPU uploads/pipeline compilation, cluster-LOD
construction, collision cooking, HDR environment/probe loading, custom shader
reads, some material/puppet paths and native synchronous service calls still
have synchronous work. Animation jobs have an explicit frame barrier. There
is no GPU upload-time budget, task priority scheduling, broad World parallelism,
or claim of whole-engine frame-rate improvement from these structural changes.
