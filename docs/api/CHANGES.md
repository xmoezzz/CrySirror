# API change log

Newest first. Each entry is keyed by the API fingerprint printed in
[REFERENCE.md](REFERENCE.md). `tests/api_docs.rs` fails until the current
fingerprint appears here. Mark breaking changes with **Breaking**.

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
