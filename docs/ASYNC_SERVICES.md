# Asynchronous engine services

Live callers use bounded jobs, observe completion and publish at the owner
safepoint. Ordinary `App.frame` calls never join these service jobs. Rust and
MCP callers can use explicit tickets; JS/TS additionally has real Promise/await
bindings derived from the same class signatures.

| Service | Start | Observe/use | Release |
|---|---|---|---|
| Assets | `load` / JS `loadAsync` | `status`, ordinary asset APIs | `release` |
| Resources | `beginDecode` / `decodeAsync` | `decodeStatus`, `readDecode`/`mountDecode` | `cancelDecode` |
| Image | `beginImage` / `imageAsync` | `decodeStatus`, `publishImage(ticket, slot)` | `cancelDecode` |
| Probe | `beginProbe` / `probeAsync` | `decodeStatus`, `readDecode` | `cancelDecode` |
| Rich text | `beginDraw`, `beginRestore`, `beginMeasure` | `status`; draw/restore publish engine surfaces | `cancel` |
| Save | `beginSave`, `beginExists` | `status` | `cancel` |
| Load | `beginLoad` | `status`, then explicit `commitLoad` | commit consumes; otherwise `cancel` |

JS helpers additionally include `Text.drawAsync`, `restoreAsync`, `measureAsync`
and `Save.saveAsync`, `loadAsync`, `existsAsync`. Their generated return types
are `CancellablePromise<T>`, with `ticket` and `cancel()`. Promise rejection
carries service errors. Cancelling pending work discards late results; it cannot
undo an atomic file write that has already committed. Image/asset/decode awaits
return retained tickets that callers must release. Other helpers release their
temporary ticket after reading the result; load commits and consumes it.

```ts
const job = resources.imageAsync("background.png");
try {
  const ticket = await job;
  const result = resources.publishImage(ticket, 20);
  resources.cancelDecode(ticket); // surface keeps immutable pixel ownership
  layers.put({id: 20, rect: [0, 0, 640, 480],
    paint: {kind: "image", asset: result.surface}});
} catch (error) {
  engine.log(String(error));
}
```

The script host captures the initiating client. Changing the active engine
client does not redirect an await's completion. A successful restore changes
the generation; older text/resource/save jobs and script waits cannot publish
into that generation. Ready and failed tickets retain their capacity until
released, so polling cannot build an unbounded result cache. Image results
charge both the original client profile and a host-wide 256 MiB staging budget;
published dynamic surfaces have their existing independent 256 MiB budget.
Temporary native codec allocations remain separately bounded by codec limits.

Text parsing, shaping, icon reads/decoding and rasterization run on CPU workers.
The owner publishes shared immutable pixels. Typewriter marker/grapheme/finished
events accompany the published reveal frame; pending raster work never emits
events for pixels that are not visible yet. A failed replacement keeps the old
block. Text blocks remain explicit runtime products: persist `Text.state` and
rebuild with `restoreAsync` when a game needs them restored after a snapshot.

Save captures ordered native VM checkpoints and logical engine state at the
owner safepoint, then workers serialize, read/write and validate. A prepared
load leaves live state unchanged until atomic `commitLoad`. Native checkpoint
capture and VM reconstruction remain ordered owner operations, with their
existing unsupported-save cases. Media resource snapshots are still rejected.

Boa drains at most 64 future polls and 64 microtasks per turn, with a combined
1024-job capacity. `async onInit` fences `onStart`; a pending lifecycle hook is
not reentered. Uncaught hook rejection faults the App. Shutdown drains its
own async hook for at most five seconds by default, then cancels remaining
subscriptions. This shutdown fence is separate from ordinary frame work.
Arbitrary synchronous JS and native VM instructions are not preempted.

Synchronous `Resources.probe/list/mount/probeImage/decodeImage`, `Text.draw/restore`
and `Save.save/load/exists` are offline-only in a live host. Large synchronous
simple text measurements also require `beginMeasure` (live limit: 4096 bytes).
Existing offline tools remain supported. Status calls never imply GPU upload
completion or a device readback fence.

The standalone example uses engine APIs without any native VM:

```sh
cargo run --release -p crysirror-desktop -- run examples/engine/project.json \
  --script examples/engine/async-services.ts --font /absolute/runtime-font.ttc \
  --no-audio
```

Menus and rich text require a caller-supplied font. No font or third-party game
data is distributed. These boundaries implement a further slice of ADR 0050;
they do not establish completion of every engine subsystem's async migration.

`Ui.events` reads a client's retained event queue; it is not automatically
cleared each frame. Use `Ui.drainEvents()` to consume action events once.
Consumption leaves other clients' events in place. The UI retains the latest
256 events globally, matching the existing widget queue policy. Native menu
adapters use the same engine queue and drain only their client's menu events.
Menu trees, queued events and focus identities must belong to registered
clients when a snapshot is validated.


Audio opening/decoding (ADR 0069) now has explicit preparation and streaming
boundaries. Use `await audio.preloadSfxAsync(path)` before synchronous
`playSfx`/`playRandom`, or `await audio.playSfxAsync(options)`. Use
`await audio.playMusicAsync(name, options, crossfade_ms)` for streaming music
and `await audio.seekVoiceAsync(handle, source_frame)` for exact seeks. Helpers
release completed tickets while playback retains its resources. A pending
seek freezes that voice, and cancellation realigns to its prior logical clock.
`sourceStatus` distinguishes ready PCM, pending workers and decoder errors.

A realtime host rejects cold synchronous SFX and synchronous music opening.
Native SceneState/channel decoder paths, final mixing/output and teardown
remain synchronous; this stage does not change original VM wait semantics.


Native channel callers can submit AudioRequest::PlayAsset through
HostServices::begin_audio and poll a scoped AudioPreparation receipt. They must
keep their original event pending while Pending, acknowledge only after Ready,
and release the receipt. Ready is successful publication, separate from the
voice's current playing/ended state. RealLive and AVG32 now use this boundary
for their mapped playback operations. See [ADR 0070](adr/0070-native-channel-audio-readiness.md)
for loop/start/intro policy, cancellation and remaining synchronous paths.


SceneState/data-VM voice declarations now use the shared SceneAudio bridge in
both App and plain desktop playback. `Audio.channelStatus(channel)` exposes
queued/loading/playing/paused/ended/failed readiness and scoped errors. Logical
Play/Wait/Stop and save epochs remain separate from worker completion. A time
wait is not a readiness fence. Desktop audio runs asynchronously even when a
native image/scenario profile retains blocking Assets mode; Runtime provides
an independent audio execution override. See [ADR 0071](adr/0071-canonical-audio-worker-bridge.md).

## Prepared media PCM (ADR 0072)

Media workers open one scoped capability shared by independent audio/video
cursors, and prepare immutable core PcmAudio. PlayPcm publishes that resource
through the shared channel mixer without reopening encoded WAVE data. Opening
failures are now asynchronous Loading → Failed results. Stops and replacements
cancel workers; blocked provider calls retain their admission leases until exit.
Inspection reports decoder_worker_reservations (default limit 16). PCM storage
receipts follow queued updates and live mixer sources past Media resource Stop.
Movie audio still prepares a bounded full track; final mixing/output and
last-reference disposal remain synchronous. See ADR 0072 for sample budgets,
conversion policy and completion semantics.

## Late-reverb preparation (ADR 0073)

Audio.prepareReverbAsync uses beginReverb/reverbStatus/cancelReverbJob over
bounded CPU workers. Graph construction, rate setup and delay allocation finish
before owner publication to both ordinary and native/media mixers. Full task
pools defer admission; clear, supersession and restore invalidate pending work.
Old effects survive failed replacement. Published receipts can be released while
their DSP remains installed. A conservative 64 MiB pair reservation survives
actual worker/DSP lifetimes against a 512 MiB estimated host budget.
Per-frame DSP/mixing and last-reference disposal retain synchronous boundaries.
This is parametric late reverb, separate from geometric reflection simulation.

## Captured reflection preparation (ADR 0074)

Reflection probe/voice APIs use immutable acoustic scene/pose snapshots and
deferred CPU jobs. Image-source scans, segment transmission and delay-buffer
allocation run on workers. Initial voice work holds only that source's clock;
replacement keeps old valid playback. Changes to captured pose, scene or HRTF
bridge latency fail pending work and invalidate installed paths. Explicit
preparation is required again in static mode; opt-in tracking is described below.
OperationTable::fail preserves scoped failed receipts without launching queued
work. Cancellation releases live reservations only when the actual worker/DSP
drops. Each voice stages at 8 MiB, then shrinks to prepared storage plus an 8 KiB
allowance against the 512 MiB host estimate. Source completion stays distinct from derived echo/HRTF tails, including
native waits. Geometry/DSP pointers never enter VM saves.
Mixing, control allocations and last-reference disposal retain owner boundaries.


## Tracked reflection updates (ADR 0075)

Audio.trackVoiceReflections or playback `reflections` opts into coalesced pose
refresh after camera/entity updates. Initial source playback waits for HRTF and
reflection resources. Workers prepare paths/taps, while the mixer retains its
maximum-delay history and crossfades at most two plans. At most four jobs launch
per frame with round-robin admission, 32 flights globally and eight/client.
Full pools defer; residency waiting/errors and captured/published age are visible
in reflectionsVoice.tracking. Scene revision/HRTF latency remain strict even when
poses advance during worker execution. Invalid old geometry/latency is muted.
Natural source EOF cancels new updates and drains the installed plan; native
waits still use source completion. Clear/restore invalidate tasks and resources.
The sample loop does not reclaim retired plans; control-side disposal and final
mix/output remain synchronous. See ADR 0075 for exact limits and lifetime policy.


## Audio transport and Siglus command publication (ADR 0076)

Exact startup frames, paused publication and optional replacement crossfade are
shared channel parameters. Workers seek/prewarm once; publication sets the logical
cursor without another seek. Shared inspection distinguishes source units from
output-clock stop/pause fades and delayed resume. Control deadlines progress
across decoder starvation while source and ordinary fade-in clocks remain held.

Siglus BGM keeps before/after original VM states around admitted audio work and
commits only on Ready. Failed receipts release without acknowledging the native
instruction; unexpected status-query errors retain their existing ticket. Pending
checkpoints contain the original instruction, not tickets; active restores queue
logical audio declarations after candidate validation. Boot/Gameexe reads, bounded
metadata translation, VM ordering, final mixing/output and teardown remain owner
boundaries. This is partial progress on ADR 0050, not full engine-wide async.

## Prepared audio resources (ADR 0077)

Audio.beginPrepareSource(asset, {measure_frames?, frame_limit?}) returns a
client/restore-scoped ticket. sourcePreparation observes Pending/Ready/Failed;
only this observation publishes completed resources. cancelSourcePreparation
cancels/releases a receipt without publishing an unobserved result. Ready result
is {resource:{client,id},info:{source_rate,source_frames,encoded_bytes}}.
Audio.prepareSourceAsync returns that result and releases the temporary receipt.
Audio.sourceInfo checks a live handle; releaseSource invalidates future lookup.

Audio.beginPlaySource(resource, voiceOptions)/playSourceAsync capture a source
lease at admission and create an independent buffered decoder on workers. They
use existing playbackStatus/cancelPlaybackJob and scoped VoiceHandle completion.
Voice options match beginPlaySfx, including spatial/effect dependencies, excluding
asset. Releasing the resource after admission cannot cancel that captured lease;
new lookups fail. Each voice keeps its own seek/read cursor. Restore invalidates
resources/jobs and old instances release storage only on actual completion/drop.

Native HostServices adds the same resource preparation/info/release contract and
AudioRequest::PlayResource. Siglus resource and voice completion stages keep the
original command uncommitted until Ready. Stopped/EOF RESUME uses the retained
resource instead of path lookup. Stopped snapshot reconstruction starts no voice.
Budgets and remaining synchronous boundaries are in ADR 0077.

## Named audio gain groups and output age (ADR 0078)

Audio.setMixGroup performs bounded owner validation/publication in both mixers;
no decoder I/O or worker wait occurs in its control DAG update. Active group
fades advance per output frame even with no voices or paused/starved inputs.
Publication revalidates the logical VoiceSpec.mix_group dependency; removed
names fail rather than starting an ungrouped voice. Cycles, depth>8 and capacity
fail before mutation; both mixers validate before either changes. Group removal
requires no live voice/effect references or children. Parent/mute host controls
persist through playback reset; their current graph is not a VM save payload.

VoiceSpec/AudioParameters accepts mix_group for pooled/native/resource playback.
Audio.playbackInfo/nativeChannelInfo adds age_frames since owner publication,
including pause/dependency/starvation/tail output frames; seek leaves age alone.
This remains distinct from decoded source position/rate and pending readiness.

Native PCM/SE preparation uses engine resource measurement then independent
voice preparation. Its temporary resource handle releases after admission,
while worker/live leases retain bytes. Before/after VM states and ordered
semantic operations preserve original instruction and cross-category ordering;
only completion acknowledges following native instructions. Original SFX
integer-ms end policy observes shared output age, with current frame-phase
limits described in ADR 0078. Final DSP/mixing/output ownership is still a
synchronous ADR 0050 boundary.

## Persistent raster resources and immutable Canvas image jobs

Offline native startup also has an explicit readiness boundary. A dormant
adapter can opt into `VmAdapter.preparation_pending`; Runtime's nonblocking
`poll_vm_preparations` steps only those preparing VMs with empty input, without
advancing the clock/world or replaying already-started peers. Headless hosts
settle this bounded startup phase before counting simulated game frames. This
prevents an empty capture from succeeding merely because a fast owner loop
finished thousands of simulated frames before cold resource I/O completed.
Ordinary game execution/input begins after readiness; this does not make every
later VM resource wait part of that startup fence.

`HostServices.begin_pattern` admits Create/Advance/Render/Restore work on the
shared image pool. Portable specification, RNG and snapshot bytes are data;
the engine owns the immutable pattern generation and its memory reservation.
Advance returns a candidate generation plus the caller's next RNG state. Only
the native semantic receipt commits that RNG; failure cannot advance it. Source
resources check both client identity and the creating runtime capability.

`HostServices.begin_canvas_image(client, canvas)` freezes a retained engine
Canvas, published image leases and font catalog membership at admission. It
reserves output/scratch memory and a worker slot before enqueueing presentation.
Desktop/offscreen hosts explicitly enable this presenter capability. A CPU host
without one fails visibly. `Runtime.prepare_canvas_images` scopes renderer lookup
to those immutable generations; pending asset dependencies retry next frame.
The existing wgpu snapshot compositor prepares the texture. Its lazy reader
retains that texture; GPU completion waiting, row depadding, channel conversion
and immutable ImageResource publication run on an image worker. Polling the
ImageJob never joins a GPU or a worker, and completion remains observable.

This is a bounded immutable capture profile: live Puppet sources, replaceable
`__surface/` dependencies and frame-history paths are rejected until they have an
admission-time freeze contract. Ordinary immutable images, nested snapshots,
completed meshes, text, clipping and isolated group composition reuse the normal
engine compositor. No native VM renderer or composed legacy screen participates.
Existing automatic Canvas compaction still uses its synchronous PNG helper;
these explicit ImageJobs do not claim to have made that separate path async.

Releasing a published image ends caller ownership exactly once. Retained
Canvas/Layer/transition/filter/mesh, World material/terrain/environment and queued
VFX dependencies keep its path valid. Owner collection removes retired paths
after their final logical reference disappears; already-frozen render/capture
leases remain usable afterward. Open Layer transactions defer collection to
preserve rollback. Dependency visiting does not serialize geometry/particle data.

## Media header preparation without playback

`HostServices.begin_media_probe` and `Media.beginProbe/probeStatus/releaseProbe`
prepare scoped video metadata on the shared workers. Admission reserves worker
and header/decoder scratch budgets. Polling never joins, completion is retained,
and dropping a pending job cancels it. This creates no Media playback slot,
clock, decoded picture, Layer or PCM output. The public ticket belongs to its
client and Runtime restore generation; restoring invalidates old tickets.

MPEG uses the sequence parser shared with playback; MP4 prepares the selected
stream decoder; ASF and Theora stop after decoder metadata is available. A
validated header does not establish that every packet is decodable. Unknown
audio identity and duration remain unknown, and AlphaMovie header-only
preparation fails explicitly until it has an appropriate constructor boundary.

## Canonical input receiver delivery

`InputRoutes` calls the same Scene-owned receiver registry and delivery queue
as native adapters. Signed priority, pointer coverage, capture and hover live
in that shared service. A poll leases one event; its client/epoch/event/delivery
receipt must be acknowledged before advancing. Unhandled delivery can continue
to the next eligible receiver. Restoring preserves the receiver definitions
and discards transient physical input, captures and old receipts.

This bounded profile accepts one active receiver client. Interactive Widget or
dialog mixing fails explicitly. Controller events require a backend to submit
real controller identities; their presence in the data model does not simulate
hardware support. VM callback dispatch remains native adapter policy.
