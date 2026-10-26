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
