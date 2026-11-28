# Memory and resource ownership

This is the maintenance contract for changed allocation paths, not a claim that
a global allocator/budget manager or complete device telemetry is implemented.
Use the [memory skill](../.agents/skills/crysirror-memory-management/SKILL.md),
[async contracts](ASYNC_SERVICES.md) and the relevant subsystem source.

## Account for physical backing and overlapping work

| Domain | Include | Owner/release boundary |
| --- | --- | --- |
| VM/native state | Heap pages, strings, call frames, original-save staging | Session/native semantics; never evict a live native object arbitrarily. |
| Resources | Encoded buffers, archive indexes, decoded surfaces, font/audio/model caches | Client-scoped resource leases and final consumer. |
| Worker preparation | Input snapshots, decompression/decode scratch, outputs and queued reservations | Bounded task admission, cancellation and result retirement. |
| GPU resources | Textures/mips, buffers, depth/MSAA targets, history, shader code and descriptors | Device/backend owner through final submission completion. |
| Transfers/output | Upload/readback staging, queued frames, PCM/video buffers | Transfer/presentation/audio completion rather than producer completion alone. |
| Platform overhead | Heap arenas, stacks, kernel memblocks, command/display rings, driver and library allocations | Actual host/SDK allocation and shutdown contract. |

Budget distinct physical allocations, including old/new overlap during upload,
scene changes and restore. One texture with several views has one shared
backing allocation but may have separate descriptor costs. A CPU-decoded image,
GPU texture and upload staging are separate allocations even if they represent
the same pixels. On a unified-memory platform, CPU/GPU labels do not imply two
independent physical pools.

Keep requested payload size separate from committed/aligned size. Include
stride/tile padding, block compression, levels/layers/faces, multisampling,
depth/stencil, scratch and allocator granularity. Account for all simultaneously
resident generations. A new resource's admission must leave room for its input,
output, temporary copies and the old resource until safe retirement.

Budgets in individual resource/audio/image subsystems are not automatically one
host-wide physical limit. Document their overlap and untracked external/library
scratch. Do not sum estimates, reservations and measured RSS as if they were
disjoint allocations. Byte availability alone does not establish contiguous
space, an available descriptor or a kernel memblock slot.

## Allocation and lifetime review

1. Identify the owner, client/session generation, allocator/memory domain and
   budget admission point. Check arithmetic and layout before allocation.
2. Reserve bounded job/output/scratch costs. On partial initialization failure,
   release only successfully acquired objects and return their reservations.
3. Publish complete immutable results once. Replacement must preserve an old
   result that is still referenced by a VM, capture, presentation or GPU job.
4. Retire after the last relevant completion. Cancelled work can still own
   backing until its worker/device finishes. A CPU `Drop` may enqueue retirement
   rather than immediately free device storage.
5. Verify scene changes, restore, suspend, game switching, failed submissions and
   shutdown. Stop/join producers and resolve device ownership before destroying
   their resources; retained caches/arenas need an explicit lifetime and bound.

Adapter-built models (`HostServices::upload_model`) stay in the client asset
cache until the adapter calls `release_model`. The runtime drops the bytes at
the end of the first tick in which no world entity of that client references the
asset, so an adapter releases a model when it despawns or rebuilds the entity
that drew it. On 64-bit Windows, macOS, Linux, iOS and Android, the immutable
GLB backing and the prepared CPU cache each default to no aggregate cap. The
host reports their byte counts separately. An explicit prepared-cache budget
limits each pool and cannot be lowered below either pool's current residency.
Ordinary cached file reads retain their 256 MiB bound. On other targets the
encoded file cache, including generated GLBs, retains its 256 MiB limit.

The prepared CPU asset cache normally keeps recently used images for five
seconds so repeated draws can reuse the decode. When a new result would
exceed the budget, it can reclaim the least recently used automatic entries
immediately. Explicit tickets, live streaming leases and payloads still held
by consumers retain their residency. If reclaiming every eligible entry would
still leave too little space, admission fails, reports resident and incoming
byte counts, and keeps all cached entries. An automatic request denied for
this reason can decode again on a later lookup once enough space is available.
Oversized payloads and decoder errors retain their failure status.

Reuse shared caches and size classes before creating new per-VM copies. Keep
keys, saves, original input data and native policy separate from disposable
derived caches. Do not change the game's logical resolution or omit native
operations to conceal allocation pressure. Declared 3D tier reductions are
documented in [render tiers](../crates/crysirror-render-tier/README.md); they do
not authorize degrading visual-novel composition.

## Diagnostics and allocation failure

Record what the source can actually expose: allocation purpose/format, requested
and committed size, domain, live/reserved/retired bytes, peak, object counts,
queue depth and completed submission/generation. Label estimated values and
missing counters. Allocation-error reporting should have a bounded fallback
that does not allocate from the exhausted heap.

On pressure, first inspect ownership/overlap and completed retirement, then use
the documented cache trim or target-specific fallback. Recovery must be bounded
and preserve native state. Unsupported formats, unavailable GPU features and
wrong allocation size are not solved by increasing a heap. A failed fence/device
must not justify reusing storage still in use or waiting forever without a
defined error path.

The current PSV host has an allocation-free fatal path; the shared GXM driver
reports its live/retired objects and free memory domains. These are specific
diagnostics, not complete engine-wide accounting. Switch's shared backend
tracks submission retirement; inspect its actual allocator/driver diagnostics
before claiming process-wide memory measurements.

## Platform review and validation

- [PS Vita characteristics](../platform/psv/PLATFORM.md): user heap/CDRAM,
  memblocks, GXM formats, synchronous scenes and emulator retirement behavior.
- [Switch characteristics](../platform/switch/PLATFORM.md): shared process
  backing, deko3d blocks/layouts, descriptors, cache visibility and fences.
- [Native packaging](NATIVE_PACKAGING.md): session/view and device lifetime.

Compare the same profile/build/environment at boot, after load/scene change,
after retirement and after repeated replacement/close when relevant. Record
source implementation, release compilation and observed behavior separately.
Track a stable cache plateau versus monotonically retained generations. An
emulator's host memory, guest heap and guest GPU allocations are different
measurements; neither a screenshot nor one successful load proves OOM safety on
hardware. No run or new allocation instrumentation is implied by this document.
