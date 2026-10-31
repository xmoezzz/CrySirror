# API change log

Newest first. Each entry is keyed by the API fingerprint printed in
[REFERENCE.md](REFERENCE.md). `tests/api_docs.rs` fails until the current
fingerprint appears here. Mark breaking changes with **Breaking**.

## `d22690a23af1bf9f` (2026-11-01): explicit retained-canvas resource destruction

- CanvasCommand.destroy is mapped to Canvases.apply. It releases an existing
  client-owned surface only after every live presentation has been hidden.
  Missing or presented surfaces fail atomically, including in command batches.
- Frozen copies keep independent nodes and their referenced immutable image
  generations. Releasing the source surface does not mutate those copies or
  an already-admitted Canvas image capture.

## `bd71afbaa37a19da` (2026-10-31): fixed sampling, progress envelopes and shared canvas limits

- ImageEffect.piecewise_resample exposes immutable16.16 nearest sampling with
  ordered bounded regions, signed destination strides, independent /256 weights
  and explicit weighted/preserved/opaque alpha. Source/destination endpoints and
  total work are checked; shared workers cancel by row without changing input.
- ProgressCurve/ProgressKnot/ProgressShape expose reusable double-precision
  scalar envelopes, right-continuous steps and caller-supplied time. Sampling
  neither advances clocks nor owns/consumes random state.
- Canvases.getLimits() reports the shared4096-surface per-client quota used by
  Core, Scene and Runtime. Node, dimensions and byte budgets remain separate.
- InputRouteSpec.drag_enabled, InputRoutePhase.Drag and InputRouteEvent.origin
  describe explicit backend drag dispatch. This does not assert that automatic
  hardware drag detection has been connected.

## `1dbdf921cc54e571` (2026-10-31): device-free media preparation and canonical input receivers

- Media.beginProbe/probeStatus/releaseProbe exposes scoped asynchronous header
  and decoder preparation without a playback slot, Layer, picture or PCM.
  Completion is retained; Runtime restore invalidates old public tickets.
  Unknown audio identity/duration stays unknown; reflected duration_ms, when
  present, is an exact decimal string.
- InputRoutes exposes shared receiver registration, signed priority, coverage,
  leased delivery and explicit acknowledgement. Receipt epoch/event/delivery
  fields use exact decimal strings. The current profile admits one active
  receiver client and rejects interactive Widget/dialog mixing explicitly.
- Exported native contracts include MediaProbeInfo/State and the canonical
  InputReceiverSpec, InputRouteCommand, event, delivery, receipt and stats data.

## `93470c7fff47e60c` (2026-10-31): shared depth ordering, source precedence and fixed luminance

- Layers.setDepth(id, depth?) opts a subtree into finite floating-depth painter
  order. Equal depths preserve the last committed frame order, nested scopes
  keep ordinary payloads together, and null restores existing ordinary order.
  Runtime commits history once after accepting the frame; extraction is pure.
- Resources.mountDecode(ticket, target, fallback?) keeps its existing two-argument
  behavior. Explicit fallback=true puts existing overlays and real loose files
  before the mounted provider; only missing paths use it. Opened readers retain
  their source, and per-path origin queries identify actual loose bytes correctly.
- ImageEffect.fixed_luma exposes FixedLumaSpec for grouped fixed-point luminance
  reduction and Q8 mixing on the shared image worker, preserving stored alpha.

## `53762232a1524668` (2026-10-31): bounded encoded-byte tokenization

- Shared Text adds ByteSplitBoundary, ByteSplitLimits and ByteSplitOutput for
  worker-side delimiter splitting with explicit single-byte or reviewed
  Shift-JIS boundaries. Output spans/NUL bytes, input/output/token bounds and
  cancellation are independent of CatSystem2's native buffer/ABI policy.
- Core VM startup preparation can opt into a nonblocking readiness poll;
  offline hosts settle dormant preparation without replaying peers or consuming
  the first game input. The current integration does not claim that every later
  native resource wait implements an offline frame barrier.

## `a7c6ab240046fa4c` (2026-10-31): immutable raster resources, Canvas capture and precomputed fonts

- Rust HostServices adds scoped persistent pattern generations, portable
  RNG/snapshot receipts and immutable Canvas image jobs. Presenter preparation
  freezes image/font generations; GPU readback and pixel resource construction
  run on shared workers. Live mutable capture dependencies fail explicitly.
- ImageInput accepts bounded encoded memory through the same worker decoder;
  fixed-weight mixing declares integer rounding and alpha policy. GlyphReveal
  can use an exact integer per-second rate while its original interval profile
  remains the default.
- Text.beginPrecomputedFont(alias, asset, pixelSize) admits TFT v0/v1 faces
  through the ordinary FontLibrary lifecycle. Size participates in identity;
  unsupported scaling/glyph/shaping profiles fail visibly.
- The reference includes the partial System-NNN installed Config. Its dormant
  factory prepares original data on workers, with shared text/image/audio and
  clock completion. This does not claim a playable title or original saves.

## `1af7e309b18b2933` (2026-10-31): retained menu enable state and native template data

- `Ui.setMenuEnabled(id, item, enabled)` maps to the owned SetMenuEnabled
  command. Disabling an item closes invalid popup paths and cancels only
  gestures/focus made ineligible; ancestor disabling also blocks native
  activation. The logical enabled state survives engine save/restore.
- Resource PeData exposes bounded standard version-zero MENU templates as
  immutable MenuTemplate/MenuTemplateItem/PeMenuResource data. Invalid UTF-16,
  extents, budgets and unsupported MENUEX formats fail visibly. Parsing never
  creates an OS menu or executes the source module.
- The reference includes the partial first-class CatSystem2 KCS Config:
  original script, font, heap_bytes, optional read-only executable ABI evidence
  and virtual canvas. Detection can select an installed data capability root
  without extraction. A configuration or registered factory does not imply
  full native syscall, story or save compatibility.

## `0f42d774636005f8` (2026-10-31): shared font resources, retained motion and pointer coverage

- `Text.beginFont`, `fontStatus`, `resolveFont`, `releaseFontJob` and
  `releaseFont` expose bounded client-owned I/O/CPU font jobs. Ready publishes
  one immutable canonical face used by measurement, layout and rendering;
  FontInfo retains digest, revision, family and collection metadata. Native
  clients use the same HostServices FontLibrary contract. Framework dynamic
  font capture/restore rejects until resource reconstruction is available.
- Layers and Ui add `beginMotion`, `setMotionElapsed`, `endMotion` and
  `motionStatus`, backed by shared scalar tracks. Engine time is milliseconds;
  external duration/elapsed use caller units. Ownership, cancellation and
  serialized continuation belong to the shared systems. UI rejects scale tracks.
- `Ui.setHitMask` / `WidgetSpec.hit_mask` bind bounded immutable coverage to
  pointer routing; keyboard activation remains independent. WidgetHitMask
  declares dimensions, bytes, minimum threshold and optional strict edges.
  Shared ImageResource alpha-region extraction keeps pixel loops out of VMs.

## `4094eacf6dc444ee` (2026-10-31): sampled media clocks and explicit resource release

- `Media.setClock(id, nowMs)` and native `MediaRequest::SetClock` select an
  external monotonic clock before first playback and admit later samples.
  `MediaOptions.clock` defaults to Engine; `MediaClock` describes both profiles.
  Shared Media owns origin, pause compensation, seek and loop offsets. Backward
  samples fail; paused samples do not turn into a jump on resume. Decoded audio
  with an external clock is explicitly unsupported until audio-master mapping.
- `Media.remove(id)` exposes the same scoped resource/surface release used by
  native adapters. Stop remains distinct from removal.
- Retained resources are bounded at 16 per client and 32 per Runtime, while
  queued-video and retained-audio byte budgets remain 128 MiB each. Canceled
  workers keep admission leases until they actually exit.

## `497d019748fb64d4` (2026-10-31): video-header metadata and completed VFX stream receipts

- `MediaInfo`/`MediaStatus` expose optional validated `size` and declared
  `duration_ms`. Unknown duration remains null, including OMV packet/page hints.
  Native `MediaRequest::Remove` releases a scoped resource and its surface.
- `Vfx.sequenceStatus(entity, revision, completedTick, exchangeEpoch)` queries
  completed worker state. Entity lifetime, declaration fingerprint and minimum
  tick/exchange acknowledgements are checked; declared seeds are never ready
  receipts. Counter strings preserve exact u64 arguments and large results.
- VFX data adds engine/external `SimulationClock`, explicit `SequenceRng`,
  fenced `SequenceExchange`, `FallingAtlasSpec` and `SystemSequenceState`.
  Their generated names use the existing Vfx prefix. Existing systems retain
  engine clocks, with no sequence or falling node by default. CPU/screen/clock
  capability limits fail at validation and are mirrored by Python tooling.

## `9e71ed5151241ed2` (2026-10-31): CPU VFX screen-sprite publication

- Optional `VfxEmitterSpec.screen` declares a client-owned retained mesh-layer
  target with canvas, parent, painter order and anchor. Existing emitters keep
  their world-space profile when it is null. `VfxScreenSpriteTarget` and
  `VfxScreenSpriteOptions` are generated from the shared VFX crate.
- Runtime publishes completed CPU particle frames through ordinary dynamic
  PuppetFrame layers; 2D hosts need no PBR camera, VM renderer or whole-screen
  upload. World VFX drawing excludes these emitters to avoid drawing them twice.
- GPU screen simulation, conflicting layer producers and unsupported projection
  profiles fail explicitly. Active or pending screen-sprite framework snapshots
  are rejected until producer/resource ownership can be reconstructed safely.

## `d7986e975e6c66a8` (2026-10-31): reusable UI snapshots, input policy and audio metadata

- `Ui.snapshotLayers(root)` exposes independent owned UI paint/layout records;
  null selects all UI, a widget ID selects its subtree. Native clients use the
  same operation through client-checked `EngineQuery::UiSnapshot`.
- `WidgetSpec.text_color` and `text_inset` preserve their prior default text-field
  appearance. `activate_on_press` defaults false and applies only to buttons.
- `interaction_only` defaults false; true keeps normal shared input handling
  while producing no visual primitive. `keyboard_focusable` defaults true;
  false permits a pointer button to preserve an active text field's focus.
- `WidgetInteraction.activated` and `activation_serial` identify the current
  tick's activation. Repeated queries keep the same nonzero serial; a new tick
  or restored input state does not replay that activation. Native callback
  policy remains outside the generic widget system.
- Audio source receipts add `has_replay_gain`, a presence flag rather than a
  promise of applied gain. **Rust literal migration:** initialize this field in
  directly constructed `AudioResourceInfo`; existing custom sources have a
  default false metadata query. The host's client-scoped codec query consults
  the actual shared decoder registry and makes no device/format-mode promise.
- Host-only additions include live window focus, bounded integer timeline
  counters, read-only PE metadata and shared text-stream encoders. These Rust
  APIs do not themselves add reflective class signatures. UI `setText` with
  null font now preserves the existing font alias.

## `a51c7455afd639b3` (2026-10-30): shared glyph, image sampling and subtree effects

- `Layers.beginCrossfadeEffect(id, source, durationMs, effect)` and
  `begin_crossfade_effect` expose engine-owned isolated Blur/Dim transitions;
  `SubtreeTransitionEffect` and `RuleWipe` describe their data. Rendering and
  validation evidence is recorded separately in STATUS.md.
- `LayerSpec.sampling` uses `ImageSampling`/`TextureAddressMode` for independent
  clamp, repeat or mirrored addressing. Omitted sampling preserves clamp.
- `GlyphBlur.kernel` defaults to weighted blur; disk outline and square face
  coverage are shared glyph operations. `GlyphBlur`/`GlyphCoverageKernel` are
  now exported data types, including bounds and backward-compatible defaults.
- Optional first-class `circus` adapter config exports `script`/`font`. The
  automatic launcher selects only the implemented DC4PH bytecode ABI; this is
  a bounded native execution profile, not a full compatibility claim.
- **Breaking for Rust literals:** initialize `LayerSpec`/`Primitive.sampling`
  and `GlyphBlur.kernel` (normally `Default::default()`). Direct construction
  of subtree extraction/state records must also initialize their generation,
  timing and optional effect fields. Prefer the public layer commands and
  constructors. Existing serialized layer/glyph records keep their defaults.
- Rust host-only `MeshFrameHistory` capabilities and resource
  `LineCursor`/`decode_auto` additions remain outside the reflective class
  signature. Explicit native save readbacks still use a blocking fence.

## `2663a3650d0f8656` (2026-10-30): native KiriKiri startup arguments

- Optional `krkr` config `arguments` maps canonical `-name` keys to strings for
  native `System.getArgument` before the first startup instruction. Explicit
  player values override executable-embedded defaults; absent keys remain void
  and native `System.setArgument` can still mutate the table.
- Default `{}` preserves the raw adapter's previous argument policy. Rust
  `Config` literals need `arguments: BTreeMap::new()` (**Breaking** for Rust
  literal construction). At most 64 options / 64 KiB are admitted, with bounded
  canonical names and string values.
- Automatic desktop `play` supplies `-debugwin=no`, selecting the installed
  game's own production branch rather than enabling optional Windows-only
  developer consoles. This does not implement or acknowledge arbitrary
  WIN32Dialog.open calls. Player launch options do not change the installed
  game's persistent directory; framework snapshot fingerprints still cover
  the real project configuration.
- Host-only additions (reflective fingerprint unchanged): shared
  `animation::ScalarTween` owns explicit-clock interpolation/retarget state;
  `image::cursor::decode_cur16m` decodes native animated cursor resources;
  `image::crx::decode` returns bounded CRXG surfaces and native origin metadata.
  Shared AudioSource decoding now recognizes XPCM PCM/Ogg containers without
  allocating an entire decoded song. These do not establish a complete native
  Circus VM or change the script class registry.
- `image::color::Srgb8Transfer` provides batch sRGB byte decoding and linear
  byte quantization using boundaries matched to the existing f32 transfer
  expression. Shared CPU scene capture uses it; native VMs retain no colour
  conversion kernel. Exact-boundary tests and game performance measurements
  are separate evidence.

## `8ff9a0e655c6babe` (2026-10-30): reusable native sprite effects and canvas copies

- Image effects add `sprite_alpha {spec}` (preserve/opaque/corner key/colour
  key/paired side mask) and `stripe_reveal {spec}` (generic stripe/stagger
  threshold masks). `SpriteAlpha` and `StripeReveal` describe the same worker
  operations used by native adapters; existing effect declarations remain valid.
- `SheetSequence`/`SheetLoop` document device-free absolute-clock sprite sheet
  frame/UV selection. `WidgetInteraction` describes the shared hovered, focused
  and pressed state exposed by `EngineQuery::UiInteraction`.
- Canvas `AtopMode` adds `copy`; `Blend` adds `atop_copy` for RGB-only copies
  which preserve destination coverage, including hidden RGB at source alpha 0.
- Optional NScripter config `archives` lists scoped container mount prefixes
  in native lookup priority order, after loose files. Default `[]` preserves
  the previous loose-resource profile. This is not an original save schema.
- Rust-only **Breaking**: literal `ImageCompositeLayer` construction now needs
  `clip: None` (or a validated destination clip rectangle); exhaustive matches
  over the extended image/scene/query enums need the new variants. Rust
  `nscripter::Config` literals also need `archives: Vec::new()`. Serialized
  projects can omit these optional/defaulted fields.
- Installed-game CLI bootstrap now uses typed Resources worker operations,
  with `play --headless` using the same plan. It needs no external mount script;
  these host CLI options are not reflective script members.

## `8e6792f059054768` (2026-10-30): pointer cursors, media transport, krkr pixel ops

- `Ui.defineCursor(id, {frames:[{width,height,hotspot,rgba}], steps:[[frame,ms]]})`
  defines a client cursor image (still or animated, edges 1..256, straight
  RGBA8); `Ui.setCursor({kind:'standard', shape} | {kind:'custom', id})`
  selects the pointer shape. The desktop host maps them to OS cursors.
  `Ui.setCursorVisible` is unchanged.
- `Media.pause(id)`, `Media.resume(id)` and `Media.seek(id, position_ms)` drive
  the shared media clock; `Media.status` adds `paused` and `frame_ms`.
  Existing calls keep their behaviour.
- Image spec additions: effect `box_blur {rect?, area, alpha_weighted}`;
  bitmap pixel math `mask_hold_alpha_scalar256` and `ps {blend, hold_alpha}`
  (krkr Photoshop-like tables). Existing values are unchanged.
- Rust-only: `MediaRequest::{Pause, Resume, Seek}`, `MediaInfo::{paused,
  frame_ms}` (**Breaking** for code that constructs `MediaInfo` or matches
  `MediaRequest` exhaustively), `ClientStorage::serving_container`,
  `crysirror_image::cursor` (`.cur`/`.ico`/`.ani` decoding),
  `ImageCompositeMath::{MaskHoldAlphaScalar256, Ps}`, and
  AlphaMovie (AJPM) in `crysirror-video` and the media workers.

## `810d2690fcbf11b5` (2026-10-30): first-class native adapter configuration

- Public source-generated metadata now includes the optional RFVP, BGI and
  NScripter adapter configs. Existing clients retain their config shapes.
- RFVP declares its root/font/encoding/canvas mapping; BGI declares a scoped BP
  module and optional project font; NScripter declares its script, encoding,
  project font, canvas and plain text window. These describe bounded profiles,
  not complete native-game compatibility or an implicitly linked adapter.
- Rust-only shared additions: immutable leased `HostServices::media_frame`,
  reader-based ASF/WMV/WMA and OMV/Theora/Vorbis decoding, and float PCM append.
  Native VMs share Media worker, clock, frame and audio ownership.

## `26edfb80137ff54a` (2026-10-30): Siglus installed-resource boot

- Siglus config adds optional `key_file`, a capability-relative caller-owned
  decode-material file; no key bytes are added to project JSON or the repository.
- `gameexe` also accepts encoded Gameexe.dat. With Gameexe configured, boot
  respects START_SCENE/label and SCREEN_SIZE; missing entry scenes fail visibly.
  Explicit `scene_name` overrides and packs without Gameexe keep their fixture behavior.
- **Breaking** (runtime snapshots): Siglus payload schema 8 includes the new
  config field; schema-7 payloads are not automatically migrated. Original native
  save-file layouts are unchanged. Private decode material cannot be packaged.
- Shared Rust slot-text/font, NVSG, channel-conversion, borrowed bitmap blit,
  coverage/tone, output-history LZSS and cyclic-mask kernels are available without
  native VMs; they add no reflective class signatures in this fingerprint.

## `c6204fad405840ca` (2026-10-30): AVG32 installed-game adapter config

- **Breaking** (AVG32 adapter config): the `plain-adv-v1` profile fields
  (`profile`, `scenes`, `start_scene`, `version`, `message_rect`, `font_size`,
  `cell_step`, `message_interval_ms`, `resources`, `sound_effects`, `voices`,
  `message_icon`) are removed. The adapter now runs an installed game:
  `{root, font, encoding?}`, where `root` holds `Gameexe.ini` and the
  original interpreter reads its own routing, metrics, SE table and
  `SAVE.INI` through client storage.
- Rust-only: `crysirror_image::avg32` (PDT bank, copy/mask/fade/stretch
  primitives and all transitions, from siglus_rs 5f358ef).
- Also in this fingerprint, from earlier uncommitted work already in the tree:
  Canvas adds `restore` {id, surface: CanvasSurface}, replacing (or creating)
  a surface from a previously queried state.

## `0a86cec0fc22dca2` (2026-10-29): canvas region filter, RealLive images and transitions

- Canvas adds `filter` {id, rect, filter: LayerFilter, opacity?}: the region's
  existing content is redrawn through a layer filter (mono, invert,
  bright/dark, colour mix), over the original by `opacity` (default 255
  replaces the region). The region is frozen as one isolated snapshot, like
  `fade`.
- Snapshots may include plain project images in self-contained formats (PNG,
  JPEG, BMP, g00); each is pinned by content on first use and a later change
  fails the frame instead of reusing a stale snapshot.
- Rust-only: `crysirror_image::g00` (RealLive g00 atlas + pattern regions,
  from siglus_rs 5f358ef) and `crysirror_image::reallive_wipe::reveal_rule`
  (`#SEL` reveal styles as rule images). The shared `g00` decoders return a
  multi-pattern file as that atlas (previously refused). Raw-PCM NWA accepts
  an unset packed size (0). No breaking changes.

## `4936e7b23cc5a98a` (2026-10-29): modal dialogs, pointer warp, external crossfade clock, nested snapshots

- Ui adds `openDialog` / `closeDialog` (UiCommand `open_dialog` / `close_dialog`):
  an engine-drawn modal message dialog {id, title?, message, buttons, default,
  cancel?, font, text_size?}. While open it takes all UI input; a choice queues
  one `dialog_result` event (value = button id). EngineQuery adds
  `DialogEvents`.
- Ui adds `warpPointer` (`warp_pointer`): a one-shot request for the presenting
  host to move the OS pointer to a canvas position.
- Layers `beginCrossfade` adds `external` and `setCrossfadeElapsed`
  (`set_crossfade_elapsed`): progress set by the client instead of the engine
  clock. Rule wipes accept `vague` 0 (hard edge).
- Paint::Snapshot payloads may contain snapshot draws up to
  `MAX_SNAPSHOT_NESTING` (3) levels; renderers rasterize inner payloads first
  (previously any nesting was refused).
- Rust-only: VmAdapter::request_close, Runtime::request_close /
  vm_sessions_halted, UiSystem::drain_events takes an event kind. **Breaking**
  for Rust callers of `drain_events` (bool → Option<&str>).
- Rust-only: HostServices::release_surface drops a `present_surface` slot
  after the tick; the per-client surface slot limit is 4096 (was 16), still
  inside the 256 MiB surface budget. Not breaking (default method).
- Rust-only: the `.sli` loop-information parser moved from
  `crysirror_audio::sli` to `crysirror_resource::sli` (device-free). **Breaking**
  for Rust callers of the old path.

## `3a2df66087609087` (2026-10-29): transformed copy opacity, native menu bars

- Canvas `transform_copy` adds `opacity` (default 255): the transformed copy
  is blended over the destination (KiriKiri `operateStretch`). Copy and
  transform-copy opacity below 255 over several overlapping source draws is
  now accepted: the group is frozen into one isolated snapshot draw that
  carries the opacity (previously refused). `replace` with opacity below 255
  is refused.
- Rust-only: MenuSystem::set_native_bars / native_bars / bars /
  activate_item and UiSystem::activate_menu_item (a host presenting menu bars
  natively takes them out of engine layout, drawing and input; trees, radio
  state and events stay engine-owned); LayerSystem::transaction (atomic
  multi-batch layer changes without cloning the tree); EngineSystems::submit
  stages only the touched systems. No breaking changes.

## `5023cd13cee161c7` (2026-10-29): keep-alpha blends, rule wipes, nested crossfades

- Blend adds `atop`, `atop_multiply`, `atop_screen` (destination alpha kept:
  KiriKiri holdAlpha draws and Photoshop multiply/screen `_HDA` kernels).
- Canvas `copy` and `draw` add `atop` (`normal`/`multiply`/`screen`): the
  touched region and the draw are frozen into one isolated snapshot; earlier
  region snapshots are inlined, not nested. Copy `opacity` below 255 is now
  accepted when one source draw is copied (it folds into that draw).
- Layers `begin_crossfade` adds optional `rule` {asset, vague}: KiriKiri
  `universal` wipe (krkrz TVPInitUnivTransBlendTable). Strictly nested
  crossfades are composed inside-out; an active crossfade inside a captured
  subtree is frozen at its nearest endpoint.
- Paint::Snapshot `layers` is shared (`Arc`); serialization is unchanged.
  Snapshots admit images served by mounted containers (immutable while
  mounted); plain project files remain refused.
- Rust-only: ClientStorage::served_by_container, Assets::container_backed,
  crysirror_resource::krkr_pbd (TJS/ns0 and TJS/4s0 data packs, ported from
  xp3-brute), Renderer::read_texture. No breaking changes.

## `94359a5460ac7b99` (2026-10-28): canvas fade, copy provenance, glyph blur

- Canvas adds `fade` {id, rect, opacity}: the region's alpha is scaled by
  opacity/255, colour kept; the region is frozen as one isolated snapshot.
- Canvas copies record provenance: copying a saved region back to its unchanged
  origin composes (region over the saved surface's earlier content) instead of
  nesting clones. Observable output is unchanged; node growth becomes linear.
- Paint::GlyphRun adds optional `blur` {radius 0..=64, level}: glyph coverage
  blurred with the KiriKiri circular kernel and scaled by level/256
  (crysirror_text::glyph_blur, ported from krkrz fd5c4baa).
- Rust-only: crysirror_runtime::structured_text::parse_toml. No breaking changes.

## `224cb89811441323` (2026-10-28): keep-alpha fills, audio loop end, encoded uploads

- Canvas `fill` adds `keep_alpha` (krkr `holdAlpha`): the region's existing
  content is recoloured and its alpha kept. Not combinable with `replace`.
- AudioParameters adds `loop_end` (default None = loop at the source end), used
  for `.sli` loop links; the shared mixer already honoured LoopPoints.end.
- Rust-only additions: `HostServices::upload_encoded_image` decodes
  provider-owned encoded bytes through the engine decoder registry and
  registers them like `upload_rgba`; crysirror-audio decodes Ogg Opus
  (pure-Rust `opus-decoder`, mapping family 0) and parses `.sli` loop
  information (`crysirror_audio::sli`).
- This regeneration also records the ImageComposite math variants and the
  Canvas/Layers declarations of the previous entry, which had not been
  regenerated. No breaking changes.

## `08381e9cfceae9bf` (2026-10-28): isolated subtrees, frozen snapshots and drawing profiles

- Layers adds beginCrossfade, crossfadeStatus and endCrossfade, backed by
  engine-owned clock/state and isolated subtree rendering (ADR 0094).
- Canvas adds copy_subtree; Paint::Snapshot stores an original-size frozen static
  subtree and renders as one cached texture, so whole-snapshot opacity and scaling
  preserve overlap. Solid/immutable engine PNG dependencies are admitted; dynamic
  content, ordinary mutable files, uncaptured fonts and nesting fail explicitly.
- The reference now includes the existing retained Canvas resize/fill/affine
  copy and draw-clip declarations authored during game-kr bring-up. Canvas
  commands remain available through Canvases.apply.
- Rust-only additions: scoped storage metadata and bounded text-file encoding;
  shared Motion workers and validated dynamic PuppetFrames; native Rust MP4
  video streams (AVC/HEVC/AV1) through existing Media workers. MP4 audio remains
  unsupported. These are reusable engine capabilities, not hosted VM renderers.
- Full-engine dependency baseline becomes Rust 1.95; only local Rust 1.98.1 has
  been exercised. Source pins/codec limits are recorded in crysirror-video.


## `4fa7e1d7f355d61a` (2026-10-27): shared synth and asynchronous image resources

Rust/native extension on 2026-10-28 (reflected registry signatures unchanged):
Core adds optional read streams, container-origin queries and provider directory
listing; Archive adds authenticated requested-name resolution. Worker publication
preserves that lookup. Shared HXV4 resources, document values and vector geometry
are independently usable; native JSON/solid LayerExDraw and directory/known-archive
URI mappings are bounded facades. ADR 0091–0093 record limits. No linkZ/bootstrap,
full DLL catalog or whole-game completion is claimed.

- Bitmap adds beginEffect, beginTransition, beginRowCopy, effectStatus and
  cancelEffect; JS exposes applyEffectAsync, transitionAsync and rowCopyAsync.
  Jobs use shared CPU workers and publish only to the original client/epoch/
  destination revision. Sources and masks are immutable snapshots.
- ImageEffect, vector path/stroke/commands, ImageTransition and SineRowCopy
  specifications document gamma/gray/invert/blur/noise/flips, vector raster,
  perspective and transitions. These CPU resource operations are reusable without
  a VM. Native KiriKiri maps selected calls to the same service (ADR 0090).
- RegisterScore/RegisterWrite define finite .opna assets; existing shared Audio
  source APIs also accept bounded PMD-M .pmd/.mmm inputs. ADR 0089 records exact
  synthesis/driver limits; no UK2 VM or ROM is imported.
- Core adds device-free ImageInput/ImageResource/ImageProcess contracts,
  ImageMetadata processing/fingerprint hooks and RegionMap.flip. Default providers
  reject unsupported services. No device pointers or executable plugin payloads
  are added to portable state.

## `be619476a3bcddb4` (2026-10-27): shared KAG3 prerequisites

- Text.positionSimple and the device-free TextMeasurement::position_simple return
  scalar baselines from the engine's simple font layout, reusable by retained
  GlyphRun text including synthetic bold and unblurred offset shadows.
- Ui.setWindowPolicy and Ui.setFullscreen publish client-owned logical desktop
  requests, saved with UiSystem and consumed by the winit host.
- Core additionally exposes bounded DeferredDispatch/RegionMap kernels,
  ImageMetadata::opaque and ClientStorage::begin_write_atomic with retained poll
  completion. These Rust contracts contain no device handles. Default trait
  implementations reject unsupported providers instead of claiming success.
- KAG3 is an optional native semantic adapter; these APIs work without that VM.
  See ADR 0088 for admitted native semantics and unsupported profiles.

## `4b42007e15453574` (2026-10-27): shared logical input journals

- Input adds opt-in enableEvents/disableEvents, events/ackEvents, eventStats and
  captureEvents/restoreEvents. App accepted-frame ingress feeds independent scoped
  journals once; ordinary action bindings/queries remain independent.
- Core owns framed edge normalization, bounded FIFO admission, unleased pointer
  coalescing, immutable read prefixes and client/epoch receipts. Native Siglus and
  public Input use the same kernel and Scene's shared transport implementation.
- Journal JSON u64 masks, sequences, epochs and timestamps use exact decimal
  strings; echo receipts directly. Binary native snapshots retain u64 encoding.
  Portable schema-1 data contains no device/task handles. Runtime restore disables
  public journals; explicit restoreEvents creates a fresh epoch.
- **Breaking native runtime payload:** Siglus schema 7 adds the logical inbox and
  script-pass continuation, replacing the private latest-button baseline. Async
  sound preparation no longer drops framed input or consumes queued pulses before
  the resumed script establishes its next wait. Original file codecs are unchanged;
  original import discards pre-load intents. Limits and remaining raw-input/frame
  callback boundaries are in ADR 0087.

## `1c682b7ad1fb38cd` (2026-10-27): portable cue cursors and native PCMEVENT

- Audio.captureCueSequence returns a portable CueSequenceCursor only when no
  emission awaits a dependency. restoreCueSequence validates against its spec
  and creates a fresh scoped generation, preserving RNG/clock/order/frame budget.
  No voices, worker receipts or resource/device handles are implicitly restored.
- Core provides the same typed cursor and pure spec/cursor validation for staged
  snapshots; HostServices capture/restore access the engine-owned bank.
- Original Siglus PCMEVENT maps onto shared cue/resource/channel APIs, including
  script-global RNG, event waits and original-file restart policy. **Breaking
  runtime payload:** Siglus schema 5 -> 6, with no migration, adds portable event
  cursor and frame continuation intent. Original binary file codecs are unchanged.
  Explicit resource/default routing and remaining limits are documented in ADR 0086.

## `2077f8fd45fb4b36` (2026-10-27): owner-clock cue programs

- Audio adds createCueSequence, advanceCueSequence, pollCueSequence,
  completeCueSequence, cueSequence and releaseCueSequence. Rust HostServices and
  scripts share the same client/generation-scoped bank in ChannelAudio.
- Once/loop/weighted modes support signed interval ranges, optional decoded
  duration, repeat bias and reset/carry timing. Uniform rejection is the default;
  explicit modulo32 preserves native RNG draw order. Keys carry no asset authority.
- Idempotent emission serials hold asynchronous source/voice dependencies; owner
  clocks freeze until acknowledgement. 1024 emissions per explicit owner advance
  bound zero-interval loops. App does not implicitly advance these program clocks.
- Info is readable during pending final DSP; mutations are gated. Release cancels
  scheduling, leaving independent voices alone. Restore invalidates all handles;
  no working cursors are implicitly saved. Native PCMEVENT mapping remains open.
  Limits and lifecycle are in ADR 0085. Existing methods/save schemas are unchanged.

## `768ebdf0e5892411` (2026-10-27): direct Doppler and world motion capture

- Spatial accepts optional velocity (m/s) and DopplerSettings; the default remains
  disabled. An independent shared multiplier composes with user speed/semitone
  pitch; its interpolation progresses on output time without consuming paused
  input or restarting source/effect history. Radial and ratio clamps are visible.
- Audio.dopplerVoice exposes desired/installed target, current ratio, effective
  user rate and source/listener capture/reset reasons. motionSettings,
  setMotionSettings and resetMotion control scoped automatic frame inference.
  Entity global motion and camera/manual listener motion use teleport/gap bounds;
  explicit velocities override inference. Restore clears history while keeping
  host capture settings. Controls fence pending final DSP frames.
- Audio.setListener accepts optional velocity. **Rust source compatibility:**
  fully specified Listener/Spatial literals need the new fields or defaults.
  JSON defaults preserve existing records with Doppler absent. No native opcode,
  project or original/runtime save schema changes are made. The linear resampler,
  direct radial approximation and remaining propagation limits are in ADR 0084.

## `0d9cec36638123d1` (2026-10-27): running audio transport and original PCMCH

- Add Audio.scheduleVoiceResumeWithOptions with a fourth options map.
  Existing three-argument scheduleVoiceResume retains its signature/freeze policy.
  "freeze" freezes immediately; "preserve" continues the prior
  running/paused/fading state until the output-clock deadline. Unknown strings
  fail without changing the voice. Rust users have AudioResumePolicy and
  AudioRequest::ResumeWithOptions / Mixer::schedule_resume_with_settings.
  reset_gain:true rebuilds the gain fade toward the latest target at the deadline;
  default false retains the independent gain ramp and fades transport.
- AudioPlaybackInfo adds transport_frames, stopping and pausing. Running input
  intent counts through decoder/seek/effect dependency holds, while fully paused
  input, freezing delays and effect tails do not count. Publication age and
  decoded source position retain their existing meanings.
- **Behavior change:** a stop fade retains its completion/voice lease through
  short-source EOF, independently of a later pause envelope. Audio.voice.playing
  includes pending stop completion;
  source_playing excludes it. Fades still consume ordinary voice/duration budgets.
  Update consumers that previously treated any live voice as live source input.
- Input EOF cancels pending resumes even while HRTF/echo tails remain. Scheduling
  on an already-ended source leaves its independent tail alone. Rust exhaustive
  AudioRequest matches and AudioPlaybackInfo literals must handle the new variant
  and fields.
- Original Siglus PCMCH now translates sources, channel controls, waits and
  save semantics onto shared APIs. **Breaking for runtime snapshots:** adapter
  payload schema advances 4 -> 5 without migration. Original file codecs/layouts
  stay unchanged; empty slot volume is corrected from the false 1000 to native
  255. Native routing/discovery/PCMEVENT remain bounded or unsupported. See ADR 0083.

## `e73bda1334c12a7f` (2026-10-27): moving acoustic instances and World bindings

- Add Audio.beginUpdateAcousticScene / updateAcousticsAsync with atomic
  AcousticMeshUpdate batches. Stable topology and local mesh BVHs are retained;
  changed world surfaces and the library top-level instance BVH prepare on
  workers. Unknown/duplicate IDs, invalid poses and pending conflicting commits
  fail visibly. Old explicit probes keep their immutable snapshot.
- Add Audio.bindAcousticMesh / unbindAcousticMesh and AcousticMeshBindingSpec.
  Same-client World Entity poses, parent transforms and optional inherited
  visibility coalesce into bounded worker commits; removal disables geometry
  with a diagnostic. Binding/control state is derived and clears on scene
  replacement/restore. Manual edits require detaching bound meshes first.
- acousticStats includes AcousticGeometryStats and automatic capture/publication
  state. scene_generation identifies full scene replacement; revision also
  changes for instance movement. Automatic direct and tracked reflection results
  may publish a completed older instance snapshot, exposing current_revision /
  geometry_stale, then refresh. Full generations and latency remain strict.
- Validation is stricter for local acoustic coordinates: both local and world
  vertices must fit finite +/-1e6. Previously only transformed world vertices
  had this range bound; oversized local geometry now fails before BVH build.
- Existing audio follow-Entity resolution now validates the object's actual
  client, rejecting foreign IDs that happen to exist locally. No project or
  native save-file schema changes. See ADR 0082 for limits and remaining work.

## `24e44c371ca52eb7` (2026-10-27): prepared voice compression and native KOE

- Audio.beginPrepareSource / prepareSourceAsync options now expose
  AudioSourcePrepareSpec with optional AudioTransformSpec:
  {kind:"siglus_jitan",percent:100..400}. Frame limits apply to decoded input
  before the worker-owned numeric transform. Percent 100 preserves the original
  encoded resource; other rates bake bounded mono PCM16 WAVE at the same sample
  rate. This named splice profile is separate from transport rate/pitch and has
  native short-cue silence semantics. Existing defaults are retained.
- Add plain core AudioResourceTransform and HostServices transformed-resource
  preparation. The shared bank owns bytes, decoder, scratch, completion and
  client-scoped leases; native adapters only translate semantic calls.
- Siglus global KOE/EXKOE now uses this shared path, Voice bus, mix groups and
  source completion. **Breaking for CrySirror runtime snapshots:** its adapter
  schema is now 4; schema 3 is not migrated. Original Siglus save-file codecs and
  layouts remain unchanged. Full native voice discovery/MWND/routing remain
  incomplete; newer trailing KOE read-flag layouts fail explicitly. See ADR 0081.

## `bec05c019d833cb0` (2026-10-27): worker final audio frame completion

- Host-only follow-up (ADR 0080; reflective fingerprint unchanged): PcmOutput
  submits asynchronously and requires readiness/error polling, with a separate
  close receipt and explicit failed-session retry. Modern desktop selects
  cpal-output; rodio-output retains the optional synchronous compatibility API.
  Native/project/save schemas and Audio.rendering's DSP meaning are unchanged.

- Add readonly Audio.rendering for prepared capacity waits and running DSP.
  Playback/source/group queries retain the last owner-published state; DSP
  controls fail explicitly until the frame completes.
- Rust App.begin_frame/poll_frame/frame_pending and Runtime.begin_tick/
  poll_audio_frame/audio_frame_pending expose worker completion. Live desktop
  uses these dependencies; synchronous frame/tick remain offline paths.
- Prepared playback receipts require owner publication, even when the worker
  product is ready during an outstanding DSP frame. No project/native save or
  resource schema changes. See ADR 0079.


## `f2af02453b2f74f8` (2026-10-27): named mix groups, output age and native PCM/SE

- Add Audio.setMixGroup, mixGroup, nativeMixGroup, removeMixGroup and
  setVoiceMixGroup, with shared AudioMixGroupSettings/Info. Logical groups form
  bounded client-scoped gain hierarchies, with output-clock ramps and atomic
  validation before publication to both pooled/native mixers. They control gain,
  not arbitrary effect graphs. Parent null detaches; omitted settings are retained.
- VoiceSpec and AudioParameters add optional mix_group; new source admission
  resolves the current owner's logical name at publication. Source positions and
  independent voice envelopes remain distinct from group/bus/master gain.
- AudioPlaybackInfo adds age_frames: rendered output frames since publication,
  including pause, dependency holds, starvation and effects; seeking does not
  reset it. Native global PCM/SE use engine-measured source lengths and output
  age for original integer-millisecond wait predicates, while physical PCM may
  still be playing. API queries retain decoded position/rate as separate values.
- Rust AudioRequest gains SetMixGroup/SetMixGroupGain. **Rust source compatibility:**
  exhaustive matches and full struct literals require their new fields/variants.
  Core remains dependency free; HostServices defaults fail when unsupported.
- Siglus runtime payload schema **2 -> 3** for PCM/SE logical state; schema-2
  snapshots require migration and are rejected. Worker tickets, engine resources,
  native operation-order stamps and sample/DSP histories remain unsaved. Original
  file codecs/layouts are unchanged. Native group gain updates preserve host
  parents/mute; global PCM/SE calls use shared resource/channel APIs. ADR 0078.

## `5b75e19d7feee9bf` (2026-10-27): immutable prepared audio resources

- Add Audio.beginPrepareSource/sourcePreparation/cancelSourcePreparation,
  sourceInfo/releaseSource and beginPlaySource, plus prepareSourceAsync and
  playSourceAsync. Resource handles and playback voices have separate lifetimes;
  temporary receipt release keeps published resources, and resource release keeps
  admitted/live decoder leases. Client/restore checks reject stale capabilities.
- Core adds AudioResourceHandle/Info/Preparation, HostServices resource methods
  and AudioRequest::PlayResource. Encoded snapshots and independent decoder
  instances live in crysirror-audio; Core stays dependency/device free.
- Resource preparation and playback decode on bounded workers with explicit
  publication/cancel boundaries and residency reservations. Snapshot restores
  rebuild logical declarations, never serialized handles or encoded tracks.
- Siglus RESUME after stop/EOF now replays the retained CrySirror resource,
  without re-resolving the original asset. Stopped snapshots prepare without
  starting hidden playback. Other native audio and full async output remain open.
- Additive JS/TS API; the Rust AudioRequest enum gains a variant, so exhaustive
  downstream matches must add PlayResource. Runtime/native save layouts remain
  unchanged (Siglus runtime payload schema 2). ADR 0077.

## `4aee8cb171139420` (2026-10-27): shared audio transport and native Siglus BGM

- Add Audio.playbackInfo(handle), nativeChannelInfo(channel) and
  scheduleVoiceResume(handle, delay_ms, fade_ms), plus AudioPlaybackInfo,
  AudioParameters and PcmBus reference data. Source frames/rate/length stay
  distinct from output-clock fade/delay counters and retained effect tails.
- Shared Rust AudioParameters adds start_frame (overrides start_ms), paused,
  replace_fade_ms and clamp_range; defaults None/false/zero/false preserve old
  playback. Worker-prepared cursor publication avoids a redundant seek.
  Immediate channel replacement does not steal another channel; crossfade needs
  a spare voice, and failure preserves the old resource. Ended controls are no-op.
- AudioRequest::Resume and Mixer.schedule_resume provide sample-clock delayed
  resume with a fresh fade-in. Later pause/stop/resume replaces that schedule.
  Explicit stop/pause deadlines continue across input starvation/preparation;
  source position and ordinary fade-in remain frozen without ready PCM.
- **Breaking (Rust):** explicit AudioParameters literals must supply new fields
  or use ..Default::default(); exhaustive AudioRequest matches must handle Resume.
  Existing JS/TS methods retain signatures; new registry APIs are additive.
- **Breaking (Siglus runtime payload):** native VM schema is now 2; old schema-1
  payloads require migration. Original Siglus save-file codecs are unchanged.
  Pending resource checkpoints replay the original instruction; active audio
  restores declarations at authored start without saving worker/device history.
- Native Siglus BGM/BGMTABLE now call shared channel services with explicit
  resources aliases and bounded decoded Gameexe metadata. Original waits and
  values remain in the VM; unsupported audio/forms fail visibly. This is a
  bounded audio profile; RESUME after EOF/stop still requires retained-source
  resources and fails visibly. This is not full-game compatibility. See ADR 0076 and the pinned
  source audit for translation, commit, resource and remaining async boundaries.

## `1ff0b7ce813fb81e` (2026-10-27): automatic reflection tracking

- Add Audio.trackVoiceReflections and ReflectionTrackingSpec. SFX/random/music
  voice options also accept `reflections` through the shared playback parser.
  playSfxAsync registers tracking before consuming short sources; enabled HRTF
  prepares first. Existing captured-pose APIs keep their static contract.
- Automatic updates capture entity/listener poses after control updates,
  coalesce movement, use bounded round-robin workers and preserve fixed-capacity
  history. Immutable tap plans crossfade at most two targets. Pose snapshots may
  lag motion; scene revision and HRTF latency remain strict. Invalid old taps
  mute and new compatible plans fade from silence.
- reflectionsVoice adds optional tracking settings/snapshots, capture/published
  age, pending/waiting/error, muted/draining and successful update count.
  Publication acknowledges the target, not crossfade completion. Natural source
  EOF stops refresh and drains the installed plan; native waits remain source-
  based. clearVoiceReflections cancels either mode. Settings/mode changes cut
  history; restore invalidates derived work/resources. No Doppler, dynamic
  geometry or higher/diffuse reflections are implied. See ADR 0075 for limits.
- Shared Rust ReflectionPlan/EarlyReflections and Mixer/ChannelAudio plan
  publication are usable without App/VM/device. Sample processing retains old
  plans until control-side collection. Final mix/output is still synchronous.

## `079531ae354ff224` (2026-10-27): captured geometric early reflections

- Add Audio.beginReflectionProbe/beginVoiceReflections/reflectionStatus/
  cancelReflectionJob/clearVoiceReflections/reflectionsVoice, plus
  traceReflectionsAsync and prepareVoiceReflectionsAsync. ReflectionOptions,
  ReflectionQuery, ReflectionPath and ReflectionPaths describe bounded complete-
  scene, single-specular-bounce results. Geometry/BVH queries and delay allocation
  happen on workers; readiness follows owner publication.
- Material absorption/scattering now affect specular pressure. Direct and
  reflected legs share per-crossing amplitude transmission. Absolute physical
  travel is reported; DSP applies excess delay against the existing direct clock,
  including the prepared direct HRTF bridge latency when present.
- These are captured-pose resources. Source/listener/scene/latency changes clear
  or fail obsolete work and require explicit preparation again. Initial work
  holds the voice clock; replacement preserves the old valid effect on failure.
  Start very short async sounds paused, prepare, then unpause. No automatic
  reflection tracking, diffuse/multiple bounces, diffraction or echo HRTF/ambisonic
  rendering is claimed. Sparse DSP uses linear fractional-delay interpolation.
- Audio.voice adds source_playing. Modern playing includes processing tails;
  native ChannelAudio.is_playing now uses Mixer::source_is_playing to preserve
  original source-completion waits independently of retained echo/HRTF tails.
  Shared Rust channel/voice effect APIs remain usable without a native VM.
- OperationTable::fail invalidates queued/running/ready work without launching a
  queued task, preserving a client/generation-scoped failed receipt. See ADR 0074
  for budgets, ownership, cancellation and remaining synchronous boundaries.

## `98b40f5f4c7c26d0` (2026-10-27): shared parametric reverb buses

- Add Audio.beginReverb/reverbStatus/cancelReverbJob/clearReverb/reverbSettings
  and prepareReverbAsync. ReverbSpec, ReverbSettings and ReverbSend document
  authored late-FDN parameters and scoped bus sends/return routing.
- CPU workers prepare matched DSP resources for ordinary/canonical and native/
  media mixers. Ready means both have been published. Receipt release preserves
  installed effects; clear/replacement cuts old tails. Full pools defer work;
  canceled/superseded/restored jobs cannot publish stale rooms.
- Shared Rust LateReverb/PreparedReverb/Mixer APIs are usable without a VM/device.
  Mixer::clear now also removes derived reverb histories. No native opcode or
  save data changes. This does not add geometric reflections, measured RT60,
  material propagation or Ambisonic returns. See ADR 0073 for limits and remaining
  synchronous mix/disposal/output boundaries.

## `f55ff4c1819aaa78` (2026-10-27): worker-prepared media PCM

- Add PcmAudioInfo reference metadata. Rust clients prepare validated immutable
  PcmAudio and publish it through the shared channel mixer without a codec or
  shorter SFX Clip limit. No JS sample-array upload member is added.
- **Rust enum change:** AudioRequest gains PlayPcm. Exhaustive matches must handle
  this already prepared resource. PlayAsset keeps asynchronous preparation;
  encoded Play retains its explicit synchronous/offline contract.
- Media now opens shared scoped capabilities on workers and emits PlayPcm instead
  of encoding WAV for the owner to decode. Open failures become asynchronous
  Loading → Failed errors. PCM receipts follow queued/live mixer references and
  canceled workers retain admission leases until exit. See ADR 0072 for limits,
  full-track preparation, PTS and remaining synchronous boundaries.

## `102009774599dc55` (2026-10-27): canonical audio readiness

- Add Audio.channelStatus and SceneVoicePhase/SceneVoiceStatus data. Canonical
  SceneState playback now shares worker preparation between App and the plain
  desktop host. Logical declaration/epoch/save state remains distinct from
  queued/loading/playing/paused/ended/failed readiness.
- Gain/routing and pause preserve the source timeline. Stop/replacement/restore
  invalidate stale preparations; naturally ended epochs do not restart without
  a new Play or restore. Preparation failures remain visible frame errors.
- Runtime has independent audio_is_nonblocking/set_audio_nonblocking policy;
  desktop Run enables it before initialization, including native Assets profiles.
  **Live initialization migration:** cold synchronous pooled SFX/music must use
  the existing async helpers even in onInit or with blocking native image policy.
  Canonical play still declares logical state; inspect channelStatus for readiness.
- Trace/save formats and native opcodes are unchanged. Explicit offline playback
  retains immediate preparation, including the full canonical voice limit.
  See ADR 0071 for budgets and remaining synchronous output/media boundaries.

## `d36d62998490cf17` (2026-10-27): native channel preparation receipts

- Add core AudioAsset, AudioPreparePhase and AudioPreparation data. The typed
  Rust HostServices boundary adds begin_audio/audio_preparation/release_audio;
  admission is distinct from ready owner publication and from voice completion.
- **Rust enum change:** AudioRequest gains PlayAsset. Exhaustive matches must
  handle it; synchronous submission rejects this variant. Use preparation and
  retain the original native event until Ready. Existing encoded offline
  submission and modern Audio JS/TS helpers retain their interfaces.
- RealLive's mapped BGM/WAV and AVG32 music/wave/SE/voice events use shared
  channel workers and original pending continuations. Restore clears derived
  channel state/tickets after successful validation. This is not additional
  opcode or native-save coverage.
- AssetIo moves to crysirror-io; the resource crate preserves its reexport.
  Audio/modern preparation share decoder::open_asset over this cursor. See
  ADR 0070 for startup/intro/loop coordinates, limits and remaining sync paths.

## `2d945b181a39e802` (2026-10-27): worker audio playback and seeks

- Add Audio beginPreloadSfx/beginPlaySfx/beginPlayMusic/beginSeekVoice,
  playbackStatus/cancelPlaybackJob/sourceStatus/clearSfxCache and JS/TS
  preloadSfxAsync/playSfxAsync/playMusicAsync/seekVoiceAsync. Completed helpers
  release tickets while the published cache/voice retains its resource.
- Add StreamBufferSpec, SourceBufferStatus and SeekProgress reference data;
  TaskStats now includes reserved admission slots. Mixer stats add per-voice
  underrun_frames. Unknown source buffering returns null.
- **Breaking in realtime hosts:** cold playSfx/playRandom decoding and synchronous
  playMusic opening now reject. Preload variants asynchronously or use the
  async playback helpers. Offline blocking APIs retain their behavior.
  Worker-buffered streams require beginSeekVoice/seekVoiceAsync; synchronous
  seekVoice fails before mutation on those sources.
- Pending exact seeks freeze the voice clock. Newer seeks supersede older ones;
  cancel restores the prior logical position with background realignment.
  Failed music preparation preserves previous playback. Restore invalidates
  jobs and derived voices even when playback began before the first frame.
  See ADR 0069 for bounded ownership and remaining synchronous/native paths.

## `c619eaa9bacd0fc3` (2026-10-27): worker-prepared HRTF resources

- Add HrtfEarOrder, HrtfOptions, HrirAsset and HrirMeasurement data; Audio beginHrtf/hrtfStatus/
  cancelHrtfJob/clearHrtf/hrtfStats/hrtfVoice/setVoiceHrtf and JS/TS
  prepareHrtfAsync. Resource/filter/voice preparation is bounded and client/
  restore-scoped. A released ready ticket leaves its resource installed.
- playSfx accepts optional `binaural: true` with existing spatial/follow options.
  It requires a prepared profile. Existing calls retain stereo behavior.
  HRTF replaces spatial panning, retains the shared mixer/buses, and combines
  with direct-path acoustic bands. Pending initial preparation freezes source/
  envelope clocks; natural EOF drains filter/delay tails.
- Profiles load SOFA SimpleFreeFieldHRIR or portable explicit-rate HRIR JSON.
  Default partition latency is 64 frames. `published_direction` reports a
  prepared target, not completion of an audible crossfade. Profiles/FFT state
  are derived and must be prepared again after restore. See ADR 0068 for limits.

## `0d7b882595278ffb` (2026-10-27): shared acoustic scenes and direct sound

- Add AcousticBackend/Material/Mesh/SceneSpec/Source/Query and DirectPath data,
  and Audio beginAcousticScene/beginAcousticProbe/acousticStatus/
  cancelAcousticJob/clearAcousticScene/acousticStats/acousticVoice methods.
  Asset reads, BVH preparation and propagation execute on bounded workers;
  scene and result publication belong to the calling client/restore generation.
- Add JS/TS prepareAcousticsAsync and traceAcousticsAsync cancellable helpers.
  Add optional playSfx `acoustics` settings with existing spatial/follow options;
  a prepared scene is required. Existing calls keep their previous behavior.
- This profile applies direct three-band gains, not HRTF/reflection/reverb.
  Absorption/scattering are reserved metadata; delay_seconds is reported only.
  Scene geometry is derived state and must be prepared again after restore.
  See ADR 0067 for limits, snapshot ownership and backend selection.

## `5394949c5999cde6` (2026-10-27): awaitable services and engine menus

- Add bounded client/generation-scoped Text preparation, Resources image/probe
  jobs and phased Save jobs. `commitLoad` applies a validated ready restore at
  an owner safepoint. Native VM checkpoint capture/reconstruction remain ordered.
  Rust DecodeContext.memory_shared retains an immutable buffer's resident-byte
  charge without copying it; image staging uses the same reservation contract.
- JS/TS adds generated `CancellablePromise` helpers for Assets, Resources, Text
  and Save. Async init/start preserve lifecycle order, pending hooks cannot
  reenter, microtask turns are bounded and uncaught rejection faults the App.
  Retained image/asset/decode tickets still require explicit release.
- **Live-host migration:** synchronous Resources inspection/image decode,
  Text.draw/restore and Save.save/load/exists now reject blocking use. Replace
  these with begin/status/publish APIs or their async helpers. Large simple
  measurements use beginMeasure; offline synchronous calls remain available.
- Add Ui.setMenu/removeMenu/openMenu/closeMenu/setMenuChecked, MenuSpec and
  MenuItemSpec: engine-owned layout, radio state, input and drawing. Ui.events
  now returns only the active client's retained events. New `Ui.drainEvents`
  consumes them once without stealing other clients' events; action loops must
  consume rather than repeatedly dispatch the readonly queue. Menu activations include an
  optional `menu_bar` namespace. **Rust:** UiEvent literals require this field,
  and exhaustive UiCommand/EngineQuery matches need their new variants.
- Export OperationStatus/OperationState and UiEvent/WidgetId data contracts,
  plus the optional krkr native config and its explicit `plugin_shells` list.
  Shells register only: no native exports or implemented capability are implied.
- API task handles, Promise subscriptions and device state are not portable
  save content. Text blocks require explicit Text.state/restoreAsync handling.
  See [service boundaries](../ASYNC_SERVICES.md) and ADR 0066 for limits.

## `d4f4c6edc250ee72` (2026-10-26): shared native file/audio resources

- Add `Resources.fileFormats`, `audioFormats`, `detectFile`, `readDecoded` and
  `mountDecoded` for engine-owned converted readers. `defineProfile` additionally
  accepts native `parameters` keyed by decoder ID, with host key aliases.
- Add `beginDecode`, `decodeStatus`, `readDecode`, `mountDecode` and
  `cancelDecode`: bounded client-scoped CPU jobs retain their original profile
  and publish completed readers/archives through ordinary VFS. Live hosts must
  use these jobs instead of synchronous `detectFile`/`readDecoded`/`mountDecoded`.
- Document `DecodeLimits`, `TransformSpec`, bytewise/bitwise `LzssParams`,
  `FileKind`, `WaveSpec`/`WaveLoop` and bitmap rect/blit/stretch/pixel modes from
  their Rust declarations. Add `PixelMath.copy_non_transparent`, preserving
  source alpha as a coverage selector. Existing mode/default names are retained.
  **Breaking (Rust):** exhaustive `PixelMath` matches must handle the new variant.
- Key bytes remain host-only. Jobs, profiles and mounts are runtime capabilities;
  no portable save representation or all-variant decoder guarantee is added.

## `ff5f3d2050247846` (2026-10-26): engine-owned modular VFX

- Add the `Vfx` class and `vfx` singleton: compile/play, typed parameter updates,
  pause/stop/restart, scoped statistics and CPU event draining. Entities expose
  `vfx`, `setVfx`, `setVfxParameter`, `pauseVfx`, `stopVfx` and `restartVfx`;
  `WorldCommand.set_vfx` stores the same shared authored system (ADR 0064).
- `EntitySpec.vfx` optionally declares multiple named CPU/GPU emitters with
  schedules, shapes, typed parameters, ordered update modules, lifetime curves,
  CPU event handlers and renderer configuration. Defaults and nested types are
  generated from `crysirror-vfx`; documentation names carry a `Vfx` prefix to
  distinguish the older scene `EmitterSpec`. These aliases do not change the
  Rust names or JSON fields. Use either `vfx` or legacy `particles` per entity.
- **Behavior migration:** the renderer no longer owns a separate CPU particle
  simulation. Runtime workers publish immutable frames; GPU work advances only
  after a renderer submission receipt. Offline Rust callers must fence pending
  Runtime work with `finish_vfx_offline()` before `render_pbr()` (App offline
  preparation already does so). Existing legacy emitter data is lowered into
  the shared CPU VFX system. Runtime/device state is not project data.
- Sprites support explicit shared textures, flipbooks, facing, blend modes and
  soft depth fade; mesh particles use real static geometry; CPU ribbons form
  real strips. Sizes are full world width/height. Mesh rendering is unlit with
  an explicit VFX texture/color, without imported PBR material inheritance.
  Skinned/morphed mesh sources, GPU ribbons and GPU event sub-emitters fail
  explicitly. Sorting is per emitter plus emitter distance order, not unified
  transparency/OIT. Ribbon width does not follow size-over-life curves.
- GPU state remains resident with visibility compaction, alpha sorting and
  indirect drawing. Scoped `Vfx.stats` reports CPU counts, pending work and
  timeline pressure; GPU live count remains `null` rather than capacity.
  This is a bounded modular foundation, not a claim of full Niagara parity.
- Rust callers constructing `EntitySpec` or `PbrScene` directly must provide
  their new VFX fields; JSON projects retain optional-field compatibility.

## `1050976234c41b17` (2026-10-26): native VM services and ray lighting

- `Environment.ray_traced_gi` optionally enables per-pixel single-bounce diffuse
  GI on the hardware path. `ScreenSpaceReflections.ray_samples` defaults to 16;
  native reflections sample a GGX glossy lobe. Probe and pixel GI include local
  lights. The reflection/pixel-GI layers share the existing texture binding
  (ADR 0060). Hardware requests now require six storage-buffer bindings;
  unsupported limits fail explicitly. Existing projects retain GI-off defaults.
- `Text.measureSimple(font, size, text)` returns unwrapped simple-layout
  metrics without allocating a surface. Canvas text additionally accepts the
  `simple-horizontal-unwrapped-v1` layout profile for clipping without automatic
  wrapping (ADR 0063). KiriKiri font callbacks use engine font resources.
- `Bitmap.composite` and `Bitmap.filterRgb` expose associated stored-channel
  composition, separate coverage, explicit 0..256 weights and signed integer
  fade/multiply. These are intermediate associated-byte operations, not an
  automatic conversion to straight-alpha display pixels (ADR 0062).
- Siglus config optionally accepts `message_window` for the bounded plain
  `FRONT.MWND[0]` TEXT/NAME profile (ADR 0061). Existing configurations without
  it keep explicit rejection of unmapped native text; it cannot be combined
  with Gameexe. This is not automatic native window or complete game support.
- The desktop adds the independent `avg32` adapter/feature, using the original
  VM's pinned `portable-vm` library through scoped engine APIs (ADR 0062).
  Its configuration and shared bitmap option types are now exported.
- **Behaviour correction:** KiriKiri unsupported font styles and oversized
  nominal fonts now fail instead of silently drawing a substituted style/size.
  Single-line text clips at the canvas edge rather than wrapping. Configure a
  supported project font and style; missing native operations still fail.

## `3ff1c210d5f93836` (2026-10-26): ray-traced irradiance probes

- `IrradianceVolumeSpec.ray_traced` (bool, default `false`): on the
  hardware ray-tracing path, probes are updated with ray queries instead of
  six raster face captures (ADR 0059). Hits are lit by the sun, emission
  and the volume itself; local lights are not part of the bounce yet.
  `hardware_rt_stats.gi_probe_updates` reports ray-traced updates.
- Ray-traced volumes converge after 12 sweeps (raster: 3) before dropping
  to `idle_probes_per_frame`.
- Not breaking: optional field.

## `1c634e6ea9b9c676` (2026-10-26): ray-traced reflections

- `ScreenSpaceReflections.ray_traced` (bool, default `false`): on the
  hardware ray-tracing path, mirror reflections trace the scene and show
  off-screen and occluded objects (ADR 0058); other paths keep screen-space
  marching. `hardware_rt_stats.reflection_dispatches` reports the technique
  actually used.
- Not breaking: optional field.

## `904dab46c3443dfa` (2026-10-26): ray-traced local light shadows

- `LightSpec.source_radius` (f32, default `0`, at most `range`): emitter
  sphere radius of a point/spot light. With hardware RT the nearest two
  shadowed point/spot lights are ray traced (ADR 0057); a radius > 0
  samples the emitter for penumbrae. Raster shadows ignore it.
- Rust: `PbrLight.source_radius`; `lights::ClusterData.order`.
- Not breaking for scripts or projects: optional field.

## `bd521feef8e3756c` (2026-10-26): soft ray-traced sun shadows

- `Sun.angular_diameter_degrees` (f32, default `0`, at most 10): the sun
  disc's angular size. Hardware ray-traced shadows sample it and denoise
  over time (ADR 0056); `0` keeps one exact hard-shadow ray. Raster shadow
  paths ignore it.
- Not breaking: optional field.

## `b9821f33f25bafe8` (2026-10-26): morph targets

- `Entity.setMorph(name, weight)` and `Entity.setMorphs(weights | null)` set
  morph target (blend shape) weights by target name on whole-scene
  (`asScene`) models; `AnimationState.morphs` (map, default empty) stores
  them and saves with the entity. `stopAnimation` clears them too.
- `World.modelInfo(path).morphs` lists the model's target names (glTF
  `targetNames` extras, else `<mesh>#<index>`).
- `PbrDrawable::Scene` gains `morphs` (Rust callers matching it exhaustively
  must add the field). ADR 0054.
- Not breaking for scripts or saved projects.

## `da2eda1e44d13887` (2026-10-26): canvas presentations inside layer trees

- `CanvasCommand::Present` gains `parent` (optional existing client layer id)
  and `position` (`[x, y]`, default `[0, 0]`). A presentation attached under a
  layer inherits its transform, visibility, opacity and clip, so retained
  layer trees can carry per-node surfaces (ADR 0053).
- The per-client canvas surface budget rises from 128 to 1024.
- Rust: `HostServices::image_metadata` returns a thread-safe
  `ImageMetadata` handle over the shared decoders (default: unsupported).
- Not breaking: both fields are optional and old snapshots deserialize with
  the defaults. The optional AGPL `krkr` adapter config is documented in
  ADR 0053, not in this generated reference, because the engine crate does
  not depend on it.

## `e3a0831acf720cb7` (2026-10-26): frame task graph, streaming resources and profiler

- Add the `Profiler` class and `profiler` singleton with `snapshot`, `trace` and
  `clear`. CPU traces include worker threads/frame IDs; GPU durations retain a
  separate uncalibrated clock and completed-sample semantics.
- Extend asset kinds with `scene` (baked entities) and `level` (PBR hierarchy and
  decoded model), backed by the same scoped background resource service.
- Rust callers with exhaustive `AssetKind` matches must handle the new variants.
- **Breaking behavior:** streaming volumes publish ready hierarchies on a later
  owner frame. Callers must observe readiness rather than assume immediate load
  after crossing a radius. Existing hierarchy, placement, collision and save
  data remain ordinary engine state. Resource leases/jobs are never saved.

## `df17e49431e56860` (2026-10-26): asynchronous assets and shared CPU workers

- Add `Assets.load/status/cancel/release/stats` and the `assets` script global.
  Requests are scoped to the calling client; ready image/model imports feed the
  ordinary renderer. Failures and cancellation are explicit states.
- Document `AssetKind`, `AssetState`, `AssetTicket`, `AssetStatus` and `TaskStats`.
  Tickets, jobs and CPU/GPU caches are runtime resources, never saved pointers.
- Add Rust awaitable reads and bounded worker pools. Desktop image/model loading
  is nonblocking; offline captures explicitly resolve pending work. Independent
  3D poses run on CPU workers with ordered deterministic result collection.

## `8d83111697a3c377` (2026-10-26): shared native drawing, text, media and RealLive API adapter

- Add engine-owned `Canvases.apply/get`, logical canvas surface/node/command
  declarations, positioned `Paint.glyph_run` with `GlyphReveal`, and the named
  `ordered_dither` transition. These resources work for ordinary data/JS clients.
- Add shared asynchronous `Media.play/stop/status` and document runtime media
  state/options separately from the script options `{loop, audio, gain}`.
- Add `Ui.setTitle/setCursorVisible`, Unicode `text_field` widgets with
  `max_length`, and `Ui.focusTextField/insertText/backspace` methods. `Ui.focus`
  remains the existing read-only focused-widget property.
- Add the native RealLive bytecode adapter configuration `{root, font_alias,
  encoding, start_scene}`. **Breaking migration from removed hosting:** legacy
  engine configuration fields remain rejected; register an engine font alias
  and provide assets through the client capability root. No renderer or legacy
  sound/text engine is hosted by this configuration.

## `d233217062201fa1` (2026-10-26): remove hosted engine adapters

- **Breaking:** remove RFVP, BGI and RealLive hosted packages, desktop features,
  factories and configuration schemas. Their whole-frame hosting approach does
  not meet the engine capability extraction requirement. Old projects fail
  visibly rather than silently falling back to a data VM.
- Preserve standalone shared image, glyph, audio, video and input capabilities.
  CBG resource decoding now resides in `crysirror-image::cbg` and no longer
  depends on an embedded BGI engine package.

## `f49c116e5d278370` (2026-10-25): native image resources through the engine

- `Resources.imageFormats()` reports the installed native image IDs and standard
  extensions; `probeImage(asset)` reports matching IDs; `decodeImage(slot,
  asset, {format?, profile?}?)` publishes a scoped RGBA surface.
- Archive mounts, Bitmap loads, text icons, retained layers and glTF/PBR texture
  decoding use the Runtime image registry. Source-specific base/mask/reference
  reads stay in the same client's VFS. Native password algorithms obtain raw
  caller key material through the shared key provider.
- The `arc-formats` feature registers 96 concrete native image formats, with
  variants and fidelity limits in `docs/arc-image-support.json`. A specific
  wrapper outranks its embedded codec; genuinely ambiguous detections require
  an explicit format ID. This is resource decoding, not a new VM adapter.
- Existing methods and project fields are retained. Rust image plugins can
  implement `ImageFormat`; PBR callers can inject a decoder with
  `load_pbr_with_decoder`. Native G00 surfaces do not replace a VM's own cut,
  pivot, message or save policies.

## `794e19091fcfd6ab` (2026-10-25): shared resource archives and Rapier physics

- New `Resources` class/global `resources`: installed format/transform discovery,
  bounded probing and metadata pages, immutable client decode profiles and
  archive mounts into the existing asset VFS. Migrated archive parsers are
  selected by the optional `arc-formats` host feature (ADR 0044).
- New `Physics` global `physics` and `PhysicsJoint` class: gravity, queries,
  collision events, statistics and fixed/spherical/revolute/prismatic joints.
  Runtime runs Rapier once per World fixed step, including data-only projects.
- `Entity` gains impulse, torque, persistent force, sleep/wake and kinematic
  target methods; `physicsState` and writable `angularVelocity` expose state.
  Scene angular velocity uses degrees/s; revolute joint limits/motors use radians.
- `ColliderSpec` adds mass/material/damping/CCD/sensor/group/lock settings.
  Body kinds add `dynamic` and `kinematic_position`; shapes add `convex_hull`.
  `SystemsSpec.physics` and saved physics declarations preserve logical forces,
  targets and joints. Commands add `set_physics` and `set_angular_velocity`.
- Existing serialized projects retain field defaults. **Rust source migration:**
  complete `ColliderSpec`/`EntitySpec` literals must initialize the added fields.
  The derived physics field moves from `EngineState.physics` to
  `EngineState.runtime.physics`; `EngineState.physics()` remains the syncing
  query accessor. Entity object identities now include client ownership.
  Physics-owned bodies now use Rapier rather than World's ordinary integrator.
  Invalid/sheared/nonuniform sphere and capsule transforms fail explicitly.
  Snapshot rebuilds do not preserve native solver warm-start caches (ADR 0045).

## `e5d2e7639aa37f1d` (2026-10-25): arcade vehicles

- `Entity.setVehicle(spec | null)`: makes a character-collider entity an
  arcade vehicle (ADR 0043), or removes it.
- `Entity.drive({throttle, steer, brake, handbrake}, dt)`: moves and
  orients the vehicle. It returns speed, lateral speed, grounded, the
  number of wheels on the ground, blocked and yaw.
- Vehicle state is not saved yet.
- Not breaking.

## `3c1011d7e4f37029` (2026-10-25): character fitting and clipping tools

- `World.sphereCast(origin, direction, radius, maxDistance, ignore?)`: a
  swept sphere. It returns the sphere centre at impact and the surface
  normal. Use it for cameras and probes that must not clip into geometry.
- `World.groundPose(position, yawDegrees, halfLength, maxDrop, ignore?)`:
  the ground under the front and back of a body, as height, pitch, normal
  and both contact points. Use it to conform quadrupeds and vehicles.
- `World.modelInfo(path).bounds`: the rest-pose `{min, max}` of the model,
  with skinned vertices posed through their joints. Use it to fit
  colliders to models.
- Not breaking.

## `68bc97f1e36d1532` (2026-10-25): puppets and level sequences

- New class `Puppets` (global `puppets`), ADR 0029: `setParameter`,
  `releaseParameter`, `play`, `stop`, `parameters`, `motions`, `value` and
  the property `count`.
- New layer paint `{kind: "puppet", asset, tint?}`. PSB assets are
  rejected.
- New layer command `{op: "puppet", id, command}`, where `command` is a
  `PuppetCommand`.
- `World.playSequence`, `World.stopSequence`, `World.sequenceStatus` and
  `World.sequenceEvents`, ADR 0040.
- New world commands `play_sequence` and `stop_sequence`.
- New data types: `PuppetRig` and its parts, and `SequenceSpec` and its
  tracks.
- Not breaking: all additions.

## `1131a6a908aacc15` (2026-10-25): irradiance visibility and idle refresh

- `IrradianceVolumeSpec.visibility` (bool, default `true`): Chebyshev
  visibility from per-probe distance moments. Set it to `false` for the old
  interpolation without occlusion.
- `IrradianceVolumeSpec.idle_probes_per_frame` (u32, default `0`, at most
  `probes_per_frame`): the refresh rate once the volume has converged.
- **Behaviour change.** Converged volumes stop refreshing until a light, a
  static object, a material override, the sun or the environment changes.
  To keep refreshing continuously, set `idle_probes_per_frame` equal to
  `probes_per_frame`.
- Not breaking: both fields are optional. ADR 0028, step 2.

## `28168b87bbb31005` (2026-10-25): first generated baseline

- Baseline of the generated reference: 13 classes, 11 script globals,
  44 mapped commands, data specs and the Siglus/RFVP/BGI adapter configs.
- Included from the latest steps:
  - `Layers.transition`, `Layers.endTransition` and the `activeTransition`
    property (ADR 0022).
  - `Entity.setReflectionProbe` and the spec fields `source` and `parallax`
    (ADR 0024, ADR 0025).
  - `Entity.setIrradianceVolume` (ADR 0028).
  - The environment fields `reflections`, `geometry_lod` (with `gpu`) and
    `virtual_shadows` (ADR 0023, ADR 0026, ADR 0027).
  - BGI adapter config (ADR 0037, ADR 0039).
- Earlier changes were not logged. ADRs 0001–0039 describe them.

### `e59e02899a2aa476` — shared release, tooltips and pixel queries

- Audio exposes begin/advance/query/release for owner-clock retired voices; counters and capability generations preserve u64 values as decimal strings.
- Ui exposes measured, delayed passive tooltips with client ownership. Canvases.readPixel exposes the bounded exact CPU sampling profile and fails on unsupported composition.
- Document immutable Mesh paint, XML documents, sprite cursor layouts and explicit RealLive optical-disc host policy; policy continuation does not authenticate a disc.

### `abf3adb03e242f64` — current shared resource and native capability reference

Regenerated against the current registry after adding the shared `windows/msi` installed-file view and its metadata types, alongside the release/tooltip/pixel-query changes above. The resource reference includes bounded worker `beginProbe` and decode completion/mount operations. MSI/CAB access is read-only and does not execute installer actions or require filesystem extraction.

### `d1da391dfe839e4a` — first-class Majiro adapter configuration

Documents the original Majiro entry script, optional read-only executable ABI metadata and native run-mode policy. This partial native profile does not imply complete syscall or game support; unsupported calls retain the original VM instruction until a real engine operation can complete.

### `61f9f936ff0989a4` — explicit Majiro ENV and portable settings policy

Adds the original ENV resource path and `MajiroSettingsProfile` to the native configuration. Portable values belong to the current client Save namespace and preserve named decimal REG_SZ semantics; original Windows registry import remains explicitly unsupported. Setter completion follows the actual shared atomic-write receipt before the native PC advances.


### `ca9c34c1e072ff84` — canonical client logs, logical locks and RGB lookup

Adds `Engine.logs`, `Input.keyboardLocks` and `Input.setKeyboardLocks`. Log
sequence/time and lock revisions use decimal strings in script snapshots.
Native `HostServices.append_log` and Engine.log use the same bounded owner ring.
Logical locks have explicit unknown physical state and no physical control.
Majiro exposes its explicit logical/physical keyboard policy; unsupported physical
requests fail at admission.

Also documents shared arbitrary RGB lookup tables and strict straight-alpha
processing, immutable ImageResource publication/release with retained leases,
and the cancellable host civil-time worker contract. These Rust host services
add no implicit device/window operation or serialized Runtime restoration of
live published resources. RealLive keeps native TCC and filename policy only.


### `f59d9a19867f34c9` — Circus opaque native save-image audit state

Adds the public NativeLiveState reference alongside the shared host additions
above. Its fields are opaque; gap/completed-opcode inspection does not establish
original save compatibility. Unreviewed live mutations, selected native capture
and restoration remain explicit admission barriers.


### `f3a197f41c653ade` — shared loop metadata and interaction region copies

Documents client-scoped AudioLoopTimebase/Range/Metadata/CatalogReceipt and the
explicit atomic host catalog. Coordinates distinguish decoded source frames
from a fixed reference sample rate; lookups never silently change PlayAsset.
Live metadata requires reconstruction and is rejected by Runtime save/restore.

RegionMap now supports bounded clipped snapshot copies, zero holes and resize.
Generic copy/clip/state live in Core, while native province allocation and the
reviewed absent-source coordinate policy stay in the KiriKiri adapter. XML
children iteration is ordered, read-only and rejects foreign/cloned nodes.
Decode-only image preparation accepts up to16MP separately from4MP effect
outputs, preserving graph/worker limits and a512MiB shared memory budget.
These host-only additions do not add a physical audio/window/device operation.


### `c00e4f6cf51eac36` — bounded encoded-byte formatting and loop-frame rounding

Adds ByteFormatValue/ByteFormatOutput for the shared Rust
crysirror_text::format::format_one_bytes kernel. It replaces exactly the first
compatible `%d` or `%s`, preserves surrounding encoded bytes and suffixes,
uses byte widths, and reports whether the argument was consumed. Width/precision
fields are bounded to510, template/output to1MiB. Integer/string precision is
ignored in this explicit profile; string-value NUL bytes are retained, template
NUL is rejected, and unsupported compatible conversions fail before publication.
VMs retain only native type, encoding and refcount policy.

Documents LoopFrameRounding and checked reference-rate conversion. This does not
add a registered System4/System-NNN factory or claim complete game support.

## 2026-11-01 — independent-product fixed-point mask blending

Fingerprint: `6781d4170ef6e9f2` (previous `d22690a23af1bf9f`).

- Added `ImageCompositeMath::MaskSeparate256` and serialized
  `PixelMath::MaskSeparate256` (`mask_separate256`): zero mask skips, full mask
  copies RGB, intermediate RGB sums two independently floored /256 products.
- Destination alpha is retained; opacity must be255. Shared graph admission
  rejects other opacities before worker allocation. Existing fixed-point
  profiles keep their arithmetic, and the same512MiB processing budget applies.
- This shared operation is usable without System-NNN. Native picture/effect
  ordering remains adapter policy; this does not imply complete game support.

## 2026-11-01 — bounded coverage clipping and two-input bitmap worker

Fingerprint: `6c31b5c44d887345` (previous `6781d4170ef6e9f2`).

- Added `AlphaClipRect` and `ImageEffect::AlphaClipRect`, with explicit source/
  destination/clip rectangles, optional exterior coverage clear, optional RGB
  multiplication, forward self-source semantics and empty-intersection profile.
- `Bitmap.beginAlphaClip(slot, source, options)` prepares that shared operation
  through existing scoped, epoch/revision-checked effect tickets. It uses
  `effectStatus`/`cancelEffect`; self_source requires identical source/target slots.
- Products are exact `(p + (p >> 7)) >> 8`. This is distinct from mask bounding
  box detection and from RGB layer opacity. Pair processing remains on shared
  ImageInput::Raster workers with existing image/graph/memory budgets.
- Layer child-index caching is private derived state, skipped by serde. It does
  not add a public or saved field.

AlphaClipRect formatting follow-up: the final source was formatted while the
first reference build was queued. Its Rust array spelling changed from
`[i32;2]`/`[i32;4]` to `[i32; 2]`/`[i32; 4]`; the current reference fingerprint
is `66c74a7ee0a69bf1` (previous `6c31b5c44d887345`). Field names, wire values,
method signatures, defaults and behavior are unchanged. The generator currently
includes that source type spelling in its fingerprint.
