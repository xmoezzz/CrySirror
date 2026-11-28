# API change log

## `856a153d826df6b5` (2026-11-27) — stored-byte rectangle resampling

`ImageEffect.rect_resample_byte255` accepts a source rectangle and an output
size. It uses 8.8 coordinates, 255-sum weights and one truncating shift for
opaque RGB output. Existing JSON effects retain their meaning. Rust
exhaustive matches on `ImageEffect` must handle the new variant.

The regenerated reference also includes the existing `rgba_color_matrix`
variant for straight RGBA 4×5 matrices. The prior generated fingerprint was
`6bfb007b98404827`.

## `6bfb007b98404827` (2026-11-27) — particle coverage and layered Blinn materials

World particle sprites accept optional `triplanar_coverage` with a scoped
texture, world-coordinate scale and scrolling rates. The 3D blend
`additive_coverage_layer` accumulates emitted colour and coverage before
one composite over the opaque scene. Existing serialized sprite definitions
retain their blend. Rust callers spelling out a full `RenderSpec::Sprite`
literal must initialize `triplanar_coverage`; use `None` for ordinary sprites.
Exhaustive matches on `crysirror_vfx::BlendMode` must handle
`AdditiveCoverageLayer`.

`ShaderEffect::LayeredBlinn` adds two masked specular lobes with material
intensity, softness, mask power, specular scale and specular exponent.
Rust exhaustive matches for material effects must handle this variant.
The prior fingerprint was `0d7e3b8a087f4655`.

## `0d7e3b8a087f4655` (2026-11-27) — baked vertex lighting and additive particle colour

`Environment.baked_vertex_scale` multiplies the diffuse irradiance stored on
vertices. It defaults to 1. Existing serialized scenes retain their lighting.
Rust literals constructing every environment field must supply the new value.
`VfxRenderSpec.blend` accepts `additive_color` on 3D renderers for independent
addition of emitted RGB and coverage. Existing blend names keep their semantics.
Rust exhaustive matches on `VfxBlendMode` (`crysirror_vfx::BlendMode`) must
handle `AdditiveColor`. The preceding generated fingerprint was
`a8c01b42c4a50deb`.

## `2ac32143b330d5a1` (2026-11-26) — Gaussian fog model

`Fog.model` selects the opacity model: `exponential` (default, unchanged)
or `gaussian`, a per-fragment `max_opacity·(1 − e^(−(d·b)²))` with a
linear height ramp `b` that may thicken upward (negative
`height_falloff`, allowed only for this model). Existing environments
omit the field and keep their fog. Rust code building `Fog` with a struct
literal must add `model` (or use `..Default`-style construction). The
previous fingerprint was `848a0fe0b5b5bfd9`.

## `2c1cf545aaec7f6f` (2026-11-26) — raster quality and capture magnification

`World.setEnvironment` accepts optional `raster_quality` with
`samples` (1, 2, 4, or 8), `shadow_map_size` (512–4096, power of two)
and `anisotropy` (1–16). Omitting it preserves the previous raster defaults.
`Ui.setCaptureScale` and `ui.set_capture_scale` accept 1, 2, or 4.
Rust exhaustive matches on `UiCommand` and `AudioRequest` must cover
`SetCaptureScale` and `SetChannelSpatial`, respectively. Positional
source playback is opt-in through `AudioParameters.spatial_position`.
The previous fingerprint was `625a9fe62f18f709`.

## `625a9fe62f18f709` (2026-11-25) — soft additive blending and RGB lightmaps

`AlphaMode.soft_additive` adds screen blending with an alpha-weighted
source. `ShaderEffect.rgb_lightmap` multiplies the base color by the RGB
texture in the emissive slot, sampling secondary UVs from `_AUX.xy`.
Rust callers with exhaustive matches on either enum need a new arm.
The previous fingerprint was `3832b7ea7ef5e8ae`.

## `848a0fe0b5b5bfd9` (2026-11-25) — VFX local-plane sprites

`FacingMode::Local` draws a sprite fixed in its emitter's local XY plane
(+X width, +Y height), for native particle quads oriented by their
spawner frame. Rust callers matching `FacingMode` exhaustively must handle
the new variant. The previous fingerprint was `3832b7ea7ef5e8ae`.

## `3832b7ea7ef5e8ae` (2026-11-25) — KiriKiri text selection

`Ui.setTextSelection` adds UTF-16 caret and range editing. Plain fields
keep their existing editing behavior. Rust callers matching `UiCommand`
exhaustively must handle `SetTextSelection`. The previous fingerprint
was `fea5db2c50352d1d`.

## `fea5db2c50352d1d` (2026-11-25) — KiriKiri launcher overrides

The combined reference adds the optional KiriKiri `launcher_overlay`
configuration to the rendering API at `1dc311dc1a85ca09`. Existing projects
can omit the field and keep their current launcher and archive handling.

## `1c2e078251b5e7a6` (2026-11-22) · Text field selection

`Ui.setTextSelection(id, start, end)` enables caret editing for an owned
editable field. The two offsets are UTF-16 code units. End `-1` extends
the selection to the end, and start `-1` collapses it at the caret.
`Ui.insertText` replaces that range or inserts at the caret. Existing
fields keep their original editing behavior until selection is requested.
The corresponding `UiCommand::SetTextSelection` is appended to the enum.

## `a6235515c431567d` (2026-11-22) · KiriKiri launcher overrides

KiriKiri project configuration gains an optional `launcher_overlay`. Explicit
launcher selection supplies an ordered set of mounted XP3 archives and the
accepted protocols from the launcher's manifest. Existing project files omit
the field. Rust callers can use `detect_source_with_entry` to preserve the
selected executable. Directory callers continue to use `detect_source`.

## `1dc311dc1a85ca09` (2026-11-25) — textured toon outlines

`ToonShading.outline_texture` adds an optional raw hull colour texture with
an independent lighting ramp in lookup row 2. Existing serialized materials
use the default `null` and require no migration. Rust struct literals that
enumerate every field need `outline_texture: None`. The previous fingerprint
was `8206decff09b50e6`.

## `8206decff09b50e6` (2026-11-25) — filmic tonemap curve

`Environment.filmic_curve` (`FilmicCurve`) sets the `filmic` tonemap's Hable constants (shoulder, linear strength and angle, toe strength, numerator and denominator), white point and exposure bias in stops; omitted, the published curve with its 2× bias applies. Rust struct literals of `Environment` need `filmic_curve: None`. The previous fingerprint was `7a50ef09e7ce0526`.

## `7a50ef09e7ce0526` (2026-11-25) — height-mask layers

`baked_vertex_irradiance` takes an optional `mask_layer` (`MaskLayer { tint, softness }`): the emissive-slot texture × `tint` covers the base where the vertex mask in `_AUX` w rises above the occlusion-slot texture's green channel, with the base weighted `saturate((mask − (1 + s)·m + 2s) / 2s)`. The occlusion slot then gives no occlusion. Omitted, materials are unchanged; struct literals of `BakedVertexIrradiance` need `mask_layer: None`. The previous fingerprint was `653c879dbcdae8ea`.

## `653c879dbcdae8ea` (2026-11-25) — slope layer blend

`ShaderEffect.slope_layer_blend` mixes the base texture (walls) with the emissive-slot texture × `tint` (upward-facing surfaces) by `saturate(1 − n.y)` of the interpolated world normal. `baked_vertex_irradiance` takes an optional `slope_layer` tint for the same layering; omitted, materials are unchanged. Rust callers with exhaustive `ShaderEffect` matches need the new arm, and struct literals of `BakedVertexIrradiance` need `slope_layer: None`. The previous fingerprint was `9640ededc06f0fce`.

## `9640ededc06f0fce` (2026-11-24) — filmic tonemap

`Environment.tonemap` accepts `filmic`: Hable's curve as published for Uncharted 2 (GDC 2010), with a fixed 2× exposure bias and white point 11.2. Existing environments keep their tonemap. Rust callers with exhaustive `Tonemap` matches need the new arm. The previous fingerprint was `01a2119f5ee7031e`.

## `01a2119f5ee7031e` (2026-11-24) — vertex layer blend and baked vertex irradiance effects

Regeneration adds the `vertex_layer_blend` and `baked_vertex_irradiance` `ShaderEffect` variants to the reference. Both read the primitive's `_AUX` vertex stream; materials without them are unchanged. Rust callers with exhaustive `ShaderEffect` matches need the new arms. The previous fingerprint was `c95d822ee88e0793`.

## `c95d822ee88e0793` (2026-11-23) — authored material raster queues

Optional `Material.raster_queue` specifies the draw order, Forward depth-write setting and front-face culling. Geometry and late transparent queues can both use straight-alpha colour and raster shadow casting. Existing materials retain their alpha mode's scheduling when the field is omitted. The previous fingerprint was `7f97588e7266cc17`.

## `7f97588e7266cc17` (2026-11-23) — placed face lines and cloth coverage

`ToonShading.line_model` selects the interpretation of a surface line texture. Existing material files default to `gated`. The `face` variant adds a nose tint and a view-dependent line on two coverage channels. The `cloth` variant adds distance-dependent alpha expansion, material cutout and screen dither. The previous fingerprint was `079f68e6848a1b48`.

## `079f68e6848a1b48` (2026-11-23) — model instance dither

`AnimationState.dither` accepts coverage from 0 to 1 for the rasterized model. Omitting it preserves full coverage and existing project data. The previous fingerprint was `d38cf2cc97b5f3af`.

## `d38cf2cc97b5f3af` (2026-11-23) — toon surface lines and directional lightmaps

`ToonShading.shadow_tone_override` adds a weighted linear shadow colour after lighting. The optional `surface_line_texture` and `line_color` apply a masked tint to the lit surface. Their defaults preserve existing materials without migration. The generated reference now includes `ToonShading`, `ToonShadeKind` and `ShaderEffect`.

`ShaderEffect.normal_shell` displaces a front-culled shell after skinning and uses its texture mask and screen dither for coverage. `directional_lightmap` samples paired linear baked textures on independently transformed UV0. Its two texture slots hold radiance and world direction/normalization. Rust callers with exhaustive `ShaderEffect` matches need the new variants. The previous fingerprint was `b37c677029d16b90`.

## `b37c677029d16b90` (2026-11-23) — stored RGBA mask interpolation

`Bitmap.blit` and `Bitmap.stretch` accept `mask_rgba_scalar256`. The mode
uses the source alpha as a /256 interpolation weight for all four stored
channels. Existing serialized modes and defaults retain their meanings.
Rust consumers matching every `PixelMath` variant need a new arm.
Regeneration also picked up the workspace's optional `Environment.toon_light`.
The previous fingerprint was `e849c4b9bdfabb09`.

## `e849c4b9bdfabb09` (2026-11-21) — integer-clock pixel stretch

`TransitionEffect.pixel_stretch` scales the incoming raster from its top-left corner on selected axes using logical pixel coordinates. It can also shrink the outgoing raster into the opposite corner. The previous fingerprint was `8a73f065cc0a1a90`.

## `8a73f065cc0a1a90` (2026-11-21) — byte-clock rule transitions

`TransitionEffect.byte_rule` provides integral rule thresholds and byte-weighted blending. Callers specify the mask, polarity and discrete phase clock. Existing transition specifications need no migration.

The previous signature fingerprint was `53a766c230fb9b32`.

## `53a766c230fb9b32` (2026-11-21) — environment colour grading

`Environment.grading` (`ColorGrading`: `color_filter`, `contrast` around middle grey, `saturation`) adjusts colour before tonemapping; absent or neutral values leave output unchanged. Rust callers that construct every `Environment` field must supply `grading` (`None`). Models may also carry spring-bone chains (`crysirror_springs` glTF document extras), which the renderer simulates after animation; no reflected signature covers them.

The previous signature fingerprint was `60772aceebd8f060`.

## `60772aceebd8f060` (2026-11-20) — PCM16 percentage faders

The Rust audio service accepts `Pcm16GainScale::Percent` on `AudioPcm16Gain`.
`AudioPcm16Gain::percent()` sets both sides and the secondary multiplier
to 100. Q8 remains the default. Existing Rust struct literals that list
every field can add `..Default::default()` to keep Q8 arithmetic.
The previous signature fingerprint was `0c844a1be2219ddd`.

## `0c844a1be2219ddd` (2026-11-20) — gradients on glyph faces

`PositionedGlyphStyle.gradient` accepts two RGB endpoints, a cell size and a vertical, horizontal or diagonal axis. Shadow and outline passes use their own colours. Existing serialized glyphs default to a uniform face. Rust callers that construct every style field must supply `gradient`. Regeneration changed the fingerprint from `274d1ddae2c9986c`.

## `274d1ddae2c9986c` (2026-11-20) — generated model release and the shared KAGParser store

- Rust-only `HostServices::release_model(client, path)` ends an adapter's use of an `upload_model` path (default: unsupported). The runtime keeps the bytes while any world entity of the client has `Geometry::Scene`/`Model` with that asset and drops them at the end of the first tick without one; an identical later upload registers again. Existing callers are unaffected; adapters that rebuild models should release the old paths (see [memory management](../MEMORY_MANAGEMENT.md)).
- `crysirror_kag::krkr_store` (new; `crysirror-kag` now depends on `crysirror-tjs2`) holds the native `KAGParser.store()` dictionary capture/decode moved from `crysirror-krkr`, with `Profile::Kag3` (condition stacks, active frames refused, the previous behaviour) and `Profile::Cm3d2` (levels only, call frames restored at their label, offset and position). `crysirror-krkr` keeps its messages and behaviour.
- No reflected signature changes; the fingerprint stays `274d1ddae2c9986c`.

## `274d1ddae2c9986c` (2026-11-20) — toon materials, attached parts and Unity-style lighting

- `Material.toon` (`ToonShading`) selects a cel shading model evaluated on display-referred values: base × colour plus rim, a shadow layer gated by a 256×3 lookup texture (diffuse ramp, shadow rate, optional colour table), Blinn highlight/specular, cast-shadow colour and an optional inverted-hull outline. Toon materials bind their lookup/shadow/highlight textures in the metallic-roughness/occlusion/specular slots, which must be empty, and take no effect or unlit flag. `Material::all_textures` lists every texture. Absent by default; existing materials keep their meaning. Complete Rust struct literals of `Material` supply the field or use `..Default::default()`.
- `AnimationState.attach` rides a follower's root nodes on a named leader node at their rest offset (parts parented to a bone); it requires `follow`. Exhaustive matches on `PbrDrawable::Scene` gain `attach`.
- `Environment.ambient` replaces the environment's diffuse spherical harmonics with uniform linear radiance; `Sun.shadow_strength` (default 1) blends cast-shadow visibility as `mix(1, shadow, strength)`. Both keep existing environments unchanged.
- The material uniform is visible to the vertex stage (outline extrusion); the unlit flag uses code 1 and toon code 2 in `Material.alpha.z`.
- Regeneration changed `224f952a71accc52` to `274d1ddae2c9986c`. Captures of the installed CM3D2 opening are recorded in docs/validation/cm3d2.md; no other game or device result is implied.

## `224f952a71accc52` (2026-11-20) — external clips, followed skeletons and client models

- `AnimationLayer.source` names a client asset whose clip drives the entity, bound to its nodes by node name (for example a converted `.anm`). `AnimationState.follow` copies the evaluated local translation/rotation of another entity's same-named nodes; `AnimationState.nodes` replaces named nodes' local components (`NodePose`). All three default to absent, so existing project, command and snapshot data keep their meaning. Rust struct literals of `AnimationLayer`/`AnimationState` must supply the fields or use `..Default::default()`; exhaustive matches on `PbrDrawable::Scene` gain `follow` and `nodes`.
- Rust-only `HostServices::upload_model` registers an adapter-built GLB under an immutable client-scoped `__crysirror/` path (default: unsupported). `crysirror_kag::ParserOptions` gains `storage_attribute`/`target_attribute` (defaults `storage`/`target`). `crysirror_asset::interchange` recognises CM3D2 `.model`/`.anm`; the image registry adds `kiss/cm3d2-tex`.
- The regenerated reference keeps the fingerprint `224f952a71accc52`.

## `224f952a71accc52` (2026-11-19) — outline glyph coverage tables

- `PositionedGlyphStyle.raster` accepts `supersample_alpha4` with a 17-entry byte table. The renderer maps rounded 4×4 coverage through the table before colouring the glyph. Existing glyph records retain their defaults. Rust callers that exhaustively match `GlyphCoverageQuality` must account for the new variant.
- The generated reference now describes `PositionedGlyphStyle` and `GlyphCoverageQuality`. The fingerprint changes from `737bf04f8bfeec00` to `224f952a71accc52`.

## `737bf04f8bfeec00` (2026-11-19) — retained audio transport

- Rust `HostServices` can schedule scoped channel retirement and resume a retained voice at its existing decoder cursor. A retiring voice remains allocated until the next nonempty mix publishes its removal.
- A retained rewind holds the voice and its fade until the scoped seek receipt publishes playback at frame zero with the requested loop mode. The script bindings and reflected fingerprint remain the same.

## `737bf04f8bfeec00` (2026-11-19) — reusable resident audio from a reader

- Rust-only `HostServices::begin_resident_audio` opens and decodes a scoped reader on a worker. The `ResidentAudioState` receipt publishes immutable PCM. The slot and playback instances share the reservation through the last PCM clone.
- The generated reference adds `ResidentAudioState`. The fingerprint changes from `a5986f521965c214` to `737bf04f8bfeec00`. Script bindings and TypeScript declarations retain their signatures.

## `a5986f521965c214` (2026-11-19) — PCM16 device attenuation and embedded audio

- Rust-only `HostServices::set_audio_pcm16_device_gain` sets a retained client Q8 attenuation between the secondary and stereo stages. It applies to live and future opted-in channels.
- Rust-only `HostServices::media_reader_with_pcm16_gain` attaches the fader to a reader-backed media track and retains it across PCM channel recreation. Existing reader methods, media options and serialization keep their existing signatures. The reference fingerprint is unchanged.

## `a5986f521965c214` (2026-11-19) — quantized PCM16 voice gain

- Rust-only `AudioParameters::pcm16_gain` opts a channel into fixed-point attenuation at publication. The default is `None`. Complete Rust struct literals now supply the field or use `..Default::default()`. Play request variants and existing project data are unchanged.
- `HostServices::update_audio_pcm16_gain` updates the raw secondary word or stereo coefficients on a client channel. It preserves the gain and transport clocks and is a no-op on an empty channel.
- The generated reference now includes the new Rust field and the two PCM16 gain types. The fingerprint changes from `8693c021aef05c1f` to `a5986f521965c214`. Script bindings and TypeScript declarations are unchanged.

## `8693c021aef05c1f` (2026-11-18) — buffered stereo source levels

- Rust-only `HostServices::begin_metered_audio` and `audio_source_level` make a fixed-window PCM16 meter available on a client channel. An opt-in `AudioSourceMeterSpec` defines its time base, window and bounded history. Admission, preparation and lease publication follow the existing channel playback boundary. Existing playback requests and reflective API signatures are unchanged.

## `8693c021aef05c1f` (2026-11-18) — scoped audio replacement

- Rust-only `HostServices::replace_audio_channels` installs detached playback recipes into a bounded set of client-owned channels. It clears selected channels without a recipe and preserves all channels outside that set. The original-save full-client replacement has the same contract as before. The reflected API fingerprint is unchanged.

## `8693c021aef05c1f` (2026-11-18) — scoped playback gain updates

- Rust-only `HostServices::set_playback_gains` validates and commits live audio and retained media gains in one client scope. It preserves transport fades, clocks and source positions. The reflected API fingerprint is unchanged.
- CMVS configuration accepts `movie_volume` as the raw 0–256 category setting. `movie_gain` remains accepted for existing projects. The raw setting takes precedence when both are present.

## `8693c021aef05c1f` (2026-11-18) — native window icons and CMVS LZSS layouts

- Rust-only `HostServices::request_window_icon` takes ownership of straight RGBA pixels for an icon up to 256×256 through the presenting host.
- `LzssParams` accepts exact-length 11/5 and 10/6 bytewise layouts for CMVS scripts and system saves. The existing 12/4 bytewise layout retains its padded-output behavior.
- Rust-only `crysirror_video::Container::Cmv` identifies indexed CMV6, CMV8 and CMV9 streams. Exhaustive Rust matches add the variant. The existing Media API opens these streams, and metadata probes identify the CMV container and embedded audio.
- Rust-only `crysirror_formats_arc::images_a::pb3_bit_depth` reads the validated source depth from the shared PB3 header parser.

## `8693c021aef05c1f` (2026-11-17): canvas camera channels, packed-alpha media and stereo DSP

- `CanvasLens.model_rotation_offset` adds default-zero Euler channels in radians before sprite model rotation. Existing JSON preserves its geometry; Rust struct literals must supply the new field. This permits native camera-angle subtraction without replacing the sprite's animation or rotating a completed view matrix.
- Rust-only `MediaRequest::PlayAlpha` and `MediaOptions.packed_alpha` select top-half colour/bottom-half green-channel alpha. Shared decoder workers unpack before publication, and displayed height is halved. Ordinary `Play` and omitted JSON options retain opaque playback. Exhaustive Rust matches add the variant; `MediaOptions` literals supply the field or use `Default`.
- Rust-only `AudioRequest::SetStereoEffect` and `StereoEffectSpec` add bounded streaming chorus and late reverb to an owned live channel. Existing Echo, gain, rate and transport controls retain their contracts. The portable DSP does not promise sample-identical DirectSound output. Exhaustive Rust matches add the control; `None` removes it. `StereoEchoSpec` is also included in the generated parameter reference.
- N2System slot sound records add an optional, default-empty stereo-effect description. Old records remain readable; decoder handles and DSP history are recreated, not serialized.

## `97f00ec0e0c50f30` (2026-11-17): polar twist and tile rotation

- Rust-only `FieldWarpSpec::PolarTwist` adds a scoped radial geometry field, centre, radius, strength and clear pixel. `crysirror_image::radial_phase::twist` samples the inverse polar rotation. Exhaustive Rust matches must handle the new variant; existing sampling profiles are unchanged. N2System uses this as an explicit Spiral compatibility extension because the native constructor installs an incompatible callback.
- `TileDraw.rotation_degrees` adds centred sprite rotation to `ImageEffect.tile_compose`. Omitted JSON values default to zero and retain the existing axis-scaled path; Rust struct literals must supply the new field. `TileCompose` and `TileDraw` now have generated parameter tables. N2System LookupRain uses this to draw its recovered internal white stripe.

## `78307311e0154f29` (2026-11-17): N-System tile and streak image operations

- `ImageEffect` adds `tile_compose` (`spec`: `canvas`, `tiles` of `src`/`dst`/`scale` in 1/256 with sign mirroring/`opacity`, at most 65536), specified by `crysirror_image::tile_compose`: scaled sprite/tile composition onto a new transparent canvas whose size replaces the image size (N-System Split, TurnOver, Drop, FloatBubble).
- `ImageEffect` adds `streak_field` (`spec`: `streaks`, `ramp`, `color`), specified by `crysirror_image::streak_field`: vertical alpha-ramp streaks stored into the image (N-System Rain 438E90).
- Rust-only, not in the script reference: `crysirror_image::tile_compose`, `crysirror_image::streak_field`.

## `2bbf87c0234528eb` (2026-11-17): N-System Mosaic and Wave image operations

- `ImageEffect` adds `full_block_average` (`cell` 1..4096), specified by `ImageSurface::full_block_average`: truncated straight RGBA means over complete square cells only; the right/bottom remainder keeps its source pixels, and a cell as large as the image is a no-op (N-System Mosaic 433630). It does not replace `block_average`, which still averages partial edge cells.
- `ImageEffect` adds `row_shift` (`offsets`, one signed pixel shift per row, at most 16384), specified by `crysirror_image::row_shift`: each row moves horizontally independently with transparent fill (N-System Wave 433E80).
- Rust-only, not in the script reference: `crysirror_image::row_shift::apply`.

## `e11262cc5a343e2b` (2026-11-17): N2System spherical lens warp

- `FieldWarpSpec` adds `Lens { table, origin, full_cover }`, sampled by `crysirror_image::lens::warp`: a spherical-lens displacement of an arbitrary-extent source into a square output through an immutable `LensTable` (per-pixel offsets, four /255 bilinear weights, outside-disc flag). `LensTable` is a new Rust-only data type; tables are built by `crysirror_image::lens::prepare`. No existing variant changed; consumers matching `FieldWarpSpec` exhaustively add one arm.
- Rust-only, not in the script reference: `crysirror_image::lens` (`prepare`, `warp`).

## `b27e595a1c729491` (2026-11-17): RScript image-load conversions, WCG writer, canvas draw_subtree

- `ImageEffect` adds `cel_convert` (`spec`: `mirror` bits, `degrees`, `percent`, `tone` 0..15), specified by `crysirror_image::cel_convert`: the codeX RScript load-time mirror, rotation/zoom and tone conversions.
- The script command list adds `canvas.draw_subtree` (`CanvasCommand::DrawSubtree { source, destination }`, through `Canvases.apply`).
- Opaque trait handles (`CivilTimeZoneQuery`, `HttpRequestJob`) now resolve their source declaration in the reference.
- The regenerated artifacts also carry the uncommitted declarations present in the working tree at generation time (`feedback_box_blur`, audio `SetEcho`, layer `stencil`/`coverage` and the progress-curve shapes).
- Rust-only, not in the script reference:
  - `crysirror_image::wcg_encode` (`encode_wcg`, `percent_shrink`, `WCG_VERSION`).
  - `crysirror_image::cel_convert::emboss` (the CMath emboss table).

## `730548bc6fc7fb95` (2026-11-16): N-System transitions and MGD regions

- `TransitionEffect::Mask` adds `vague` (1..=4096): a continuous soft edge in rule levels, weight (r + t·(vague+255) − 255)/vague, replacing the discrete `fade` step when set.
- `TransitionEffect` adds `spin_zoom` (`reverse`), `bounce` (`count`, `vector`, `damped`, `stretch`) and `door_flip` (`axis`, `focal`), wipe codes 910–912 in the shared wipe shader (old→new family). PSV `shared_wipe_fs_main_k*` GXPs are recompiled, with new k910–k912 variants; Switch programs were not recompiled.
- The committed artifacts were still at `53f4100163bef8b4`: the `9a5bbadd4d94916a` entry below was logged without regenerating them, so this fingerprint also covers its declarations.
- Rust-only, not in the script reference:
  - `crysirror_formats_arc::images_c::mgd::regions` returns an MGD sprite region table. The MGD compression field is now read as a u16 (N-System `DecodeMain` 0x43C2F0); files whose following u16 is non-zero now decode instead of failing.
  - Legacy archive indexes no longer apply `max_entry_size` to Raw (stored) members.
  - New crate `crysirror-nsystem` (`NSystemVm`, `Config`, `register`, `script`, `message`), registered as engine `nsystem`.

## `9a5bbadd4d94916a` (2026-11-16): layer utility effects, glitch transitions, multi-track voices, WaveFilter transforms, carrier generators

- `ImageEffect` adds `blue_to_alpha`, `fill_alpha`, `shrink_fast` (one image) and `shrink_copy`, `overwrap_rect`, `stitch_wrapped_copy`, `glitch_copy` (two images), specified by `crysirror_image::layer_ex`.
- `SubtreeTransitionEffect` adds `glitch` (`params`, `mode` `exchange`/`fade`, `seed`). An old-image `fade` may name its destination as `begin_crossfade_effect` source (a single-subtree transition).
- Rust-only, not in the script reference:
  - Multi-track voices: `AudioResourceTransform::MultiTrack(Arc<[MultiTrackStem]>)` (stems: the resource's own or another asset, an optional stereo pick of up to 32 decoded channels, a start gain) and `AudioRequest::SetTrackGains { channel, positions, changes: Vec<AudioTrackGain> }` (linear per-track gain ramps in source time, optionally waiting for a source frame). New `crysirror_audio::multitrack` (`TrackSet`, `GainLine`, `TrackGains`), `AudioSource::{track_count, take_tracks}`, `Mixer::{set_track_gains, track_count}`, `decoder::open_asset_picked` and `Stream::open_picked`. The mixer reads each stem in lockstep after its own worker buffer.
  - `crysirror_image::layer_ex` adds `GlitchParams`, `GlitchField`, `GlitchTransition`, `GlitchFrame` (CPU reference `render`), `glitch_pick`, `glitch_tint` and `apply_glitch_tint`; `crysirror_scene::layer` re-exports the first four and `SubtreeTransitionEffect::{allows_self_source, glitch_frame}`.
  - The shared composite shader adds mode 4 (glitch). The PSV `shared_composite_fs_main_f` GXP is recompiled; Switch programs were not recompiled.
  - `crysirror_core::service::AudioResourceTransform` adds `WaveFilters(Arc<[WaveFilter]>)` and no longer derives `Eq` (`WaveFilter` holds floats). `WaveFilter` is `Delay`, `FreeVerb`, `GraphicEq`, `Iir`, `GainLimit`, `PhaseVocoder`, `Abs`, `Add`, `Mul`, `RingMod` or `Vocoder`; `Oscillator` describes mono carriers. `IirKind`, `IirFamily`, `IirStructure`, `IirParamInfo`, `iir_params`, `iir_default_params` and `iir_max_order` describe the DSPFilters designs.
  - New: `crysirror_audio::iir_design`, `crysirror_audio::wave_filters::apply` and `crysirror_audio::carriers` (`generate`, `Vocoder`) and `crysirror_audio::phase_vocoder::process`.
  - `crysirror_core::region_map` adds `RegionMap::labels`, `RegionMap::paint_plane`, `LabelPlane` and `alpha_to_labels`.
  - `crysirror_image::psd` (PSD/PSB documents, layer pixels and descriptors) and `crysirror_tjs2::runtime::Runtime::put_object_member` (assignment that runs property setters).
- Migration: Rust code that requires `AudioResourceTransform: Eq` uses `PartialEq`.

## `53f4100163bef8b4` (2026-11-16): showcase recorder, live material surfaces, HTTP verbs/headers

- PBR material texture paths (`base_color_texture`, `emissive_texture`, …) may name a live engine surface (`__surface/<slot>`) or a published image (`__image_resource/…`). The renderer rebuilds the material when the surface revision changes. Static override textures are decoded once and shared across materials that differ only in factors.
- Regeneration also picks up declaration changes since `daf54ba690642f95` that had not been regenerated (`Ui.beginWindowDrag`, `Ui.resetWindowShape`, `Ui.setImeCursorArea`, multiline committed text).
- Rust-only, not in the script reference:
  - `crysirror_core::service::HttpMethod` adds `Put`, `Delete` and `Head`, and `HttpResponse` adds `headers` (names lower-cased, at most 128 entries).
  - New: `crysirror_resource::lz4_frame` (`decode_complete_frame`, `has_frame_magic`), `crysirror_launch::detect::kirikiri_script_search` (the `krkr_grep` example) and `crysirror_wgpu::Renderer::set_movie_capture` (consecutive captures keep temporal history instead of re-accumulating warm-up passes).
  - GPU timestamp queries are limited to four renderers per process.
  - The KRKR adapter configuration adds `locale`.
- Desktop CLI: `crysirror showcase <manifest> --output <mp4>` records a scripted scene whose screens show headless native games, with spatially mixed game PCM written only to the file (see `examples/showcase/README.md`). `play --headless` honours `CRYSIRROR_HEADLESS_CAPTURE_EVERY=N`.

## `daf54ba690642f95` (2026-11-13): linear alpha lookup, xorshift128, D.C.III screen frames

- `ImageEffect` adds `linear_alpha_lookup` (`LinearAlphaLookup`: alpha from a byte table indexed by `offset + x*step[0] + y*step[1]`, optional complement, out-of-table value `outside`).
- Rust-only, not in the script reference: `crysirror_vfx::xorshift::Xorshift128` (Marsaglia xorshift128 with serializable state), and the Circus `EffectRequest::OriginalScreen` / `OriginalScreen` frame requests used by D.C.III (0x8a71) 0x0E.
- Regeneration from `7cb560d0edb00d9a` also includes other uncommitted declaration changes in the working tree that had not been regenerated.
- Rust-only, not in the script reference (fingerprint unchanged, 2026-11-14): `crysirror_vfx::streaks` (stateless falling-streak field, `StreakField`/`Streak`) used by WillPlus 0x56 weather.
- Rust-only, not in the script reference (fingerprint unchanged, 2026-11-15): `crysirror_core::image_processing::RgbaImage` (zero-copy shared RGBA `ImageResource`), `IMAGE_GRAPH_MAX_NODES`/`IMAGE_GRAPH_MAX_DEPTH` with `ImageInput::within_graph_bound` (the worker's graph admission bound, now shared), `crysirror_wgpu_deko3d::driver::Driver::{renderable, blendable, storage_format}` (per-format capabilities reported through adapter format features; `storage_format` replaces the earlier whole-device `compute`), `crysirror_console_player::set_log_sink`, `crysirror_console_player::selftest` (renderer self-test without game data: box filter, lite rays), `crysirror_wgpu_gxm::gxm::trace` and `crysirror_wgpu::Renderer::use_lite_rays` (the lite fragment ray profile, chosen automatically for `ComputeRaytrace` on GPUs without compute storage).
- Rust-only, not in the script reference (fingerprint unchanged, 2026-11-15): `crysirror_resource::Archive::entry_metadata` and `ArchiveMount::entry_metadata` return an entry's complete JSON facts. Formats with large indexes keep per-entry facts typed and build them on request, so `ArchiveEntry::metadata` may hold only part of them (XP3 entries now store `Null`). `Resources.list` publishes the complete view as before. Migration: Rust callers that read XP3 keys (`adler32`, `flags`, `xp3_info_name`, `cxdec`, `hxv4`, `hash_fold_xor`, `adler_xor`, `visible_name`, `xp3_shadowed_native_ids`) from `entries()[i].metadata` call `entry_metadata(id)` instead.
- Rust-only, not in the script reference (fingerprint unchanged): the new `crysirror-lua` crate (Lua 5.3 binary chunk loader and interpreter, `Lua`/`Host`/`Value`) and `crysirror_image::pna` (WillPlus PNAP layered images).

## `7cb560d0edb00d9a` (2026-11-13): drifting-atlas streak cells

- `VfxDriftingAtlasSpec` adds optional `streak_cell` `[width, height step, row base]` for the streak motion (default `[2, 24, 1]`); DC3WY rain uses `[1, 56, 0]`.
- Rust-only, not in the script reference: `crysirror_circus::Program.original` and `dc3_original` (D.C.III to DC3WY opcode translation), and the Circus `VisualRequest::Zoom` request.
- API artifacts were regenerated from the current declarations.

## `62e8043e2e182acc` (2026-11-12): regenerated reference; exact resampling and keyed stamping

- API artifacts were regenerated from the current declarations, including declaration changes made since `bdc9bef9fbf234c4` that had not been regenerated.
- `crysirror-image` adds `overlap_resample::{overlap_resample, stepped_nearest}` (exact-overlap area resampling with an optional colour key, and stepped nearest stretching) and `keyed_over::keyed_over` (exact integer source-over between keyed and straight-alpha RGBA). These are Rust functions and do not appear in the script reference.

## `bdc9bef9fbf234c4` (2026-11-11): native window requests, editor fields and bitmap text

- `Ui.beginWindowDrag`, `Ui.resetWindowShape` and `Ui.setImeCursorArea` submit client-owned requests to the presenting host.
- Text fields support explicit multiline, read-only, no-wrap and line-scroll modes. Positioned glyph styles add underline and strikeout using the selected font's design metrics.
- `ImageEffect` adds alpha-border nine-patch resizing, checked external glyph coverage and circular coverage blur. `CoverageBitmap` and `NinePatchLayout` describe their data; image workers reserve output and temporary memory before execution.
- `FontCatalogFace` describes authorized font names, pitch, outline format and declared code pages. Rust text measurement exposes simple-layout ink bounds.
- Shared floating-point polygon-fragment raster declarations are included in the regenerated reference.

## `3192c2516eb8d6cc` (2026-11-11): shared natural-effect resources and batches

- `PngHeader` observes bounded PNG metadata independently of row decoding; `LinearByteStorage` preserves retained allocation tails. DWQ exposes preserved indexed planes and original BMP sample depth.
- `RadialNoiseAtlas` and `FragmentGrid` prepare brush data and image partitions with transactional random state. `AlphaBrushes` updates retained coverage through ordered rationally scaled brushes.
- `PackedRepeatRows` retains explicit packed coordinate and byte-blend profiles. `RadialRgbBrushes` and `PackedPairBatch` execute sprite batches with one output allocation; runtime admission includes their temporary tables and metadata.
- API artifacts were regenerated from the current declarations.

## `8159f272408687b5` (2026-11-11): retained radial displacement fields

- `PackedRadialField` prepares bounded signed direction and radius-offset data for publication as a client-scoped immutable image resource, with explicit lane and squared-distance profiles.
- `ImageEffect::RadialWavePair` samples two independently positioned pictures through that retained field using signed-word multiply-high offsets and low-word RGB weights. Its output extent and temporary radial table are included in runtime admission budgets.
- Native HAMON uses retained field preparation for setters and original-save restoration. API artifacts were regenerated from the current declarations.

## `7e7fb3db3c803f31` (2026-11-11): packed RGB spans and sprite batches

- `StoredWordSpans` provides ordered wrapping affine RGB spans and signed additive ramps, preserving clipping phase and destination alpha.
- `LinearMaskOffset` composes RGB offsets through independently strided linear mask buffers. `ByteRaster::SpatialColorBand` adds quantized spatial highlights before stored-mask composition.
- `ByteRaster::RowBlitBatch` runs ordered sprite/scanline writes with one output allocation. `SignedScaledMaskedRgb256` exposes packed signed-word scaling and separately truncated coverage products.
- Native SAN, KOMOREBI and CHARAMETAL call these shared image operations; API artifacts were regenerated from the current declarations.

## `20f2722a04d4b13f` (2026-11-11): worker spectrum visualization

- `HostServices.begin_spectrum_graph` prepares a client-scoped image and retained spectrum history from an independent audio decoder, without seeking a playing voice.
- `SpectrumGraphSpec` selects fire or LCD bars. The shared audio and image modules implement the sine window/transform, logarithmic bands, decay, peak hold and bounded rasterization.
- `SpectrumGraphReceipt` commits the image and next history together; dropping a query cancels pending worker work.
- Subtree crossfades accept a destination containing its source back page. The old image excludes that source branch; the source is isolated in the destination position. A source containing the destination remains unsupported.

## `afcc01d161cccaa1` (2026-11-11): stored byte-gradient spans

- `ImageEffect::StoredGradientSpans` exposes ordered solid RGB spans and independently saturating byte-weight ramps with low-word summed products, clipping and retained destination alpha.
- Native SPEEDLINE maps its shared line-bank geometry onto the engine operation, including the lower-row solid/ramp overlap.
- Regenerated API artifacts from the current declarations.

## `fcd36b9afb3dd835` (2026-11-11): repeated fixed-point sampling and mounted media

- `ByteRaster::RepeatScaleSample` repeats scaled source windows in destination-sized work, retaining independent clipped-tile starting phases, quantized16.16 steps and stored RGB/coverage profiles. Native RAIN4 maps its depth planes onto this shared operation.
- `HostServices.mounted_media` exposes a client-scoped `MountedMediaCatalog` of host-published labels, legacy drive identities and asset roots; `SharedHost.publish_mounted_media` supplies those mappings explicitly.
- Regenerated API artifacts from the current declarations.

## `371babf82b229087` (2026-11-11): resident interaction label planes

- `ImageMetadata.begin_region_map` prepares client-scoped PNG/BMP label images on workers, retaining charged immutable results and typed errors. Palette indices remain separate from visible colours.
- `RegionMap` composes resident plane views and bounded fills; clipped copies, resize and reflection keep snapshot and zero-hole semantics without copying image pixels on the owner.
- `LabelImage` and `PngRows` expose bounded source-index decoding. The same generation includes `AxisBoxSample` for delayed one-axis raster sampling.
- Regenerated API artifacts from the current declarations.

Newest first. Each entry is keyed by the API fingerprint printed in
[REFERENCE.md](REFERENCE.md). `tests/api_docs.rs` fails until the current
fingerprint appears here. Mark breaking changes with **Breaking**.

## `7da35e89980fd41c` (2026-11-11): RGB-mask pair composition

- `ImageEffect::MaskFieldPair` composes two equally sized immutable images using per-channel low-word RGB-mask weights or bounded signed blue/green displacement.
- Colour coverage retains the packed-word lower clamp and the 255 upper endpoint; displacement retains independent /128 contribution floors. Both retain destination alpha and execute in the scoped cancellable image worker.
- Regenerated API artifacts from the current declarations.

## `09d7148b9cc5c678` (2026-11-11): radial twist image sampling

- `ByteRaster::RadialTwistSample` samples an immutable source through independently quantized radial scale and angular twist, with an explicit field origin, source centre and coverage percentage.
- `ByteRasterMath::AlphaPercentRgb256` scales source alpha by an integer percentage before summed RGB composition and preserves destination coverage.
- Regenerated API artifacts from the current declarations.

## `3fa1b8c90f54de5e` (2026-11-11): affine samples and linear channel buffers

- `ByteRaster::AffineSampleBlend` exposes staged double affine mapping, nearest or scale-distance neighbour sampling, signed RGB offsets and coverage-aware normal/add/subtract composition.
- `ByteRaster::LinearChannelTransfer` copies a bounded channel span between linear pixel streams independently of their image widths.
- `ScanlineMath::CoverageWordSeparate` exposes low-word scaled coverage, explicit complement and independently truncated RGB products.
- Regenerated API artifacts from the current declarations.

## `e3b1ea9eda1cda76` (2026-11-11): wrapping scanline sampling

- `ByteRaster::FixedScanlinesWrapping32` samples signed16.16 origins and steps with explicit32-bit wrapping; the existing checked64-bit scanline operation retains its own contract.
- `ScanlineMath::CoverageRawSummed` exposes low-word coverage/complement RGB composition without zero/opaque endpoint shortcuts.
- Regenerated API artifacts from the current declarations.

## `1cb18cdf5bd33b2f` (2026-11-11): circular colour offsets

- `ImageEffect::CircularOffset` applies bounded saturated channel offsets inside integer-radius circles and toruses, with explicit row endpoint policy. Circle spans use the squared-radius formula rather than normalized ellipse sampling.
- Regenerated API artifacts from the current declarations.

## `d7252ee23e23aac8` (2026-11-11): quantized row-edge image offsets

- `ImageEffect::RowEdgeLift` applies caller-provided row boundaries, grouped pixel widths and quantized saturated channel offsets through a bounded shared image operation.
- Regenerated API artifacts from the current declarations.

## `2c4c24949bb58563` (2026-11-10): palette-indexed image plans

- `IndexedPlane` validates a bounded linear index plane and exposes existing `Red8` packed-pixel preparation followed by integer channel replication and RGB palette lookup. Indices and palette remain ordinary owned data; processing uses scoped engine image jobs.
- Regenerated API artifacts from the current declarations.

## `78becb9bba7a93d2` (2026-11-10): elliptic paint and integer sprite samples

- `ImageEffect::EllipticPaint` paints bounded inclusive ellipse spans with explicit packed-byte RGB mixing or saturated offsets. `EllipticCutout` selects inner-span intersection and padding; optional row bounds retain caller-selected endpoint policy.
- `SpriteSamples` returns current/next cells and an exact integer fraction for forward or ping-pong sequences with repeat or hold behavior. Callers retain clock advancement and choose byte or floating-point quantisation. Native menu and film-button animation use this shared sampler.
- Regenerated API artifacts from the current declarations.

## `aa85a05cf400745e` (2026-11-10): stochastic block image and RNG receipts

- `PatternRequest::SampleBlocks` admits scoped immutable source/destination graphs, a bounded `MaskedBlockSample` specification and caller-owned RNG state. The cancellable worker returns the ready image and its actual post-sampling RNG together as `PatternOutput::ImageWithRng`.
- `BlockSamplePaint` selects per-pixel coverage or flat sampled coverage, including an optional explicit percentage. Search, graph, temporary memory and random-draw costs are bounded before publication.
- `ScanlineMath::CoverageByteSeparate` retains explicit byte-truncated covered weights and separately floored RGB contributions, distinct from the existing complemented-word profiles.
- Regenerated API artifacts from the current declarations.

## `5b67e8f325ba4b84` (2026-11-10): directed mask clips and packed pair composition

- `ByteRaster::MaskedClipBlit` combines directed scanline clipping with explicit incremented-mask, wrapping packed-word scale/add and covered RGB products while retaining destination alpha.
- `ImageEffect::PackedPairBlend` composes two immutable image sources with independent weights, optional coverage and an explicit signed destination residual. The unmasked profile omits destination RGB.
- `ScanlineMath::ConstantSummed` and `CoverageSummed` expose stored-byte summed-product rounding independently from the existing separately truncated profiles.
- Regenerated API artifacts from the current declarations.

## `b963b04efb70ebdb` (2026-11-10): directional glyph coverage stamps

- `GlyphRasterOptions.coverage_offsets` max-unions the morphed glyph mask at caller-specified signed working-pixel offsets before reduction. An omitted value retains the existing coverage path; an explicit set includes `[0, 0]` when the original mask is required.
- Offset count, per-axis displacement, expanded surface, worker cost and temporary memory are bounded before publication. This shared Rust image capability is independent of native VM state.
- **Breaking (Rust struct literals):** complete `GlyphRasterOptions` literals must provide the new field or use `Default`. Regenerated the API reference from the current declarations.

## `363e1b5338b4b802` (2026-11-10): row tint and masked backdrop displacement

- `ByteRaster::RowTintBlend` composes caller-supplied row colours and immutable source RGB using separately truncated weights, followed by stored-coverage blending. An explicit colour branch supports packed dispatch policies; destination alpha remains intact.
- `ByteRaster::MaskedBackdropShift` displaces an immutable backdrop through an independent thresholded channel mask. Coordinates, work and cancellation stay bounded.
- Registered both typed specifications and regenerated API artifacts from the current code.

## `363665633b82cc34` (2026-11-10): stored-channel extents and packed box sums

- `CoverageBounds` queries tight nonmatching stored-channel extents with cancellation; source-window intersection preserves source-to-destination phase independently of physical image clipping.
- `ImageEffect::NeighborRelief` provides immutable neighbour differences, independently truncated weights and optional row-end repair, retaining coverage alpha.
- `ByteRaster::NativeBoxBlur` provides clipped sliding box sums with distinct covered-RGB and premultiplied-mask normalisation. Explicit source bounds and bottom-update policy preserve caller-selected edge semantics.
- Registered typed specifications and regenerated API artifacts from the current code.

## `41bca722ef33c7e0` (2026-11-10): stored-mask and fixed scanline image operations

- Added `PackedColorMask` and `PackedDualMask` three-input operations with independent stored-channel/alpha coverage, low-word arithmetic, source-colour transformation and destination-alpha retention.
- `ByteRaster::FixedScanlines` samples explicit16.16 source phases through bounded spans. Pixel profiles include separately quantized coverage products; source RGB adjustment follows corner-key comparison.
- `ByteRaster::ClippedScanlineSample` combines fixed-grid sampling with directed half-plane or rectangle clips. Geometry and image leases remain independent of native VM state.
- `FloatingLines` provides single-precision half-biased DDA strokes with ordered signed stored-colour interpolation and bounded cancellation.
- Registered the corresponding typed specifications and regenerated API artifacts from the current code.

## `dafe34bba251d36c` (2026-11-10): row-specific stored-byte coverage

- `RowBlit.row_math` selects independent stored-byte arithmetic per source row. An omitted or empty array retains the existing uniform `math`; clipping preserves the original row index.
- `ByteRasterMath::AlphaWeightedSeparateRgb256` scales coverage by a bounded /256 weight and floors the source and destination RGB terms separately, retaining destination alpha.
- `ByteRasterMath::MaskPiecewiseWrapping256` exposes packed16-bit mask multiplication and RGB product/sum wrapping, with an explicit full-mask branch and unchanged destination alpha.
- Regenerated API artifacts from the current code.

## `ef6a03ea3de51950` (2026-11-10): source-pass completion and clipped scanline composition

- `AudioPlaybackInfo.first_pass_complete` exposes a mixer-owned first-endpoint latch. Decoder read-ahead, pause and wall time do not set it; native channel queries can distinguish an initial pass from continuing loop playback.
- `Audio.playbackInfo` and `Audio.nativeChannelInfo` return the same observation.
- Shared `RowBlit` composes independent horizontal scanline offsets with bounded clipping and cancellation. Byte raster math adds weighted source-mask /256 blending with an opaque shortcut and saturated RGB add/subtract.
- Regenerated API artifacts from the current code.

## `14da84badb19e7e1` (2026-11-10): source-device input and shared image/path operations

- Added bounded source-device observations: keyboard/pointer samples remain separate from normalized controller buttons and axes. The host publishes the next input generation; native adapters observe it without polling or consuming edges.
- Added shared quadratic/cubic Bézier paths and per-row image displacement. Box reduction supports sliding window origins and explicit half-word arithmetic.
- Documented reusable byte-pattern matching, clipped alpha-plane multiplication, index-plane blending, BGR inversion and low-word RGB lookup construction as Rust service entry points.
- Rust byte-string services also expose fixed Windows CP932 CRT comparison, with reached-unit ordering and canonical encoded aliases.
- Majiro configuration exposes an explicit native-optical or verified-installed-resources media policy.
- Regenerated the API artifacts from the current schema without running tests.

## `685454f030910411` (2026-11-10): shared wipe geometry and stored-byte colour operations

- Added explicit scanline, star/heart, zoom, polygon-fragment and random-sample wipe specifications. Random inputs and clocks remain caller-owned; image workers own bounded rasterisation and cancellation.
- Added luminance-indexed tonal offsets, masked scaled addition/subtraction and per-row packed-word colour offsets. RGB lookups can retain pixels beyond a checked prefix.
- Rectangle sampling supports independent fixed-point phases on each axis; box reduction supports explicit weighted2x2 windows.
- Regenerated public API artifacts from the current schema.

## `a209e8a1ef60f853` (2026-11-10): shared integer strokes and image wipes

- Added `ImageEffect::IntegerLines` and `IntegerLine`: clipped RGBA strokes with explicit signed half-biased rounding, aggregate work limits and worker cancellation.
- `ByteRaster::IntegerWipe` exposes integer scanline, circular/clock, additive, bouncing and rolling-panel transitions over two immutable images. Callers own clocks and parameters; shared workers own pixels, clipping and cancellation.
- Regenerated the API reference; the recorded schema fingerprint changed from `92012dee4a633ce5` to the value above.

## `92012dee4a633ce5` (2026-11-10): glyph views and shared native drawing/audio

- `Paint.glyph_view.range` selects a half-open positioned-glyph range while
  retaining its source run's reveal phase and cluster steps. Separate rows can
  interleave with other Layers without starting independent text clocks.
- Stored-byte blending adds exact `MaskSeparate256` and
  `MaskPiecewise256 { weight }` profiles, retaining destination alpha. Shared
  CPU kernels and destination-reading shader paths use the same integer rules.
- Shared image workers expose byte raster, axis projection and triangle raster
  operations. Native surface ownership and clipping policy stay in adapters.
- Rust audio services add asynchronous source-format/sample queries and reusable
  PCM packing, spectrum analysis and source-layout inspection. Result leases
  retain their byte-budget charge until the final reference is released.
- `GlyphReveal::change_rate` preserves the visible prefix and remaining cluster
  boundaries when changing an existing reveal clock.


## `ad858c719a96dcc0` (2026-11-09): segmented audio and nearest image resize

- Rust `AudioRequest::PlayPlaylist` accepts immutable encoded clips and a bounded
  `AudioPlaylistNode` program with nested finite or infinite repeats. The shared
  audio worker handles decoding, gapless segment boundaries and seeking; repeats
  do not expand source bytes or PCM. Replacement preparation supports the same
  source type for atomic native-save restoration.
- `ImageEffect.resize_nearest` uses top-left proportional integer sampling and
  preserves stored RGBA bytes, including hidden RGB. It is available through the
  existing shared asynchronous image processing interface.
- Rust `HostServices::begin_detached_layers_image` composes a bounded candidate
  of shared Layers without installing it in the live scene. It uses the existing
  capture presenter, scoped resource generations and asynchronous image jobs;
  this method adds no reflective script signature.

## `e021971b72099695` (2026-11-09): tinted fixed luminance

- `FixedLumaSpec.tint` (optional `[r, g, b]`, default absent) scales the
  grouped fixed-point luminance per channel, `(luma*tint)>>8`, before the
  /256 mix. Absent keeps the grey target, so existing specs are unchanged.
  System-NNN LAYERSEPIA128 uses it (MUGEN2 MMX kernel 0x4D7EF0).
- Regenerated with the release API example; crysirror-image fixed_luma unit
  tests passed. The API docs test suite was not run.

## `25d0b1766c932638` (2026-11-09): integrated RFVP and optional remote resources

- The default RFVP config now describes its integrated runtime and an injected
  remote-resource capability marker. Original extraction crates remain available.
- Siglus and BGI configs expose a host-bound `remote_root`; the reference now
  reads the active BGI config and includes the integrated SoftPAL config.
- Network backend features are default-off on the four integrated adapters.
  Regenerated with the release API example; no API test suite was executed.

## `4c6395d28206ec40` (2026-11-07): stored channel copies and font fallback

- `ImageEffect::CopyChannel` copies one stored byte channel into selected RGBA
  channels, with optional inversion, through the shared asynchronous image worker.
  Unselected channels and hidden RGB are preserved.
- Majiro `Config.font_fallback` selects an explicit prepared host font alias when
  the requested native family is unavailable. Its default requires the exact family.
- Regenerated the API reference with the release example; no tests were run.

## `079f7138a813b097` (2026-11-07): coverage keys and cyclic cell sampling

- `ImageEffect::CoverageReferenceKey` derives binary coverage from a checked
  reference texel and selected RGBA channel while preserving stored RGB.
- `ImageEffect::CyclicCellSample` repeats bounded per-cell sample offsets over
  full cells in a region, preserving partial cells and outside pixels.
- Rust `ImageCompositeMath::MaskIncrement256` and `PixelMath::MaskIncrement256`
  expose mask0 skip, mask255 copy and exact partial `(a+1)/256` RGB mixing.
  Destination alpha is retained and extra layer opacity is rejected.
- Regenerated the shared API reference. No test or game execution was performed.

## `9983e526641d9b01` (2026-11-07): random runs, row blur, disc copies

- `ImageEffect::RandomRuns` (`random_runs::RandomRuns`: `span`, `color`
  `middle|mean`, `state`): rows cut into `rand() % span + 1` runs from an
  MSVC-style stream, each filled from its source run;
  `RandomRuns::state_after` reports the stream state without pixels.
- `ImageEffect::TruncatedRowBlur` (`radius`, `row_overrun`): horizontal mean
  over a row-truncated window, optionally one sample past the row end (the
  next row's first pixel, zero after the last row).
- `ImageEffect::DiscCopies` (raster input: destination + source): midpoint
  circle discs copied from the source with their own centres and radii.
- Rust-only: the shared prefix image capture groups several top-level
  layers under one identity root before materialization.
- Regenerated using the release example.

## `11159237bd0ab027` (2026-11-07): picture glyphs, cell widths, onset scan

- `PositionedGlyph.image` (`GlyphImage`: `asset`, `rect`, `uv`, `tint`)
  draws an image cell instead of the scalar's outline; the scalar still
  counts as one reveal step. The wgpu glyph expansion emits it as an
  ordinary image draw.
- `CellLayoutOptions.cell_widths` gives listed scalars an explicit cell
  width in the shared fixed-cell layout (defaults to empty).
- Rust-only additions without reference entries: `WidgetHitMask::hit` is
  public, and `crysirror_audio::onset::{OnsetProbe, first_audible}` scans a
  decoded source forward for the first probe above a mean-magnitude level.
  `crysirror_text::legacy_width::{to_halfwidth, columns}` convert to
  half-width forms and count single/double columns (added after the
  regeneration; the fingerprint did not change).
  `crysirror_scene::layer::local_to_parent` maps a layer-local point into
  its parent's pixels through the extraction affine.
- Regenerated using the release example.

## `488cc60b60c63756` (2026-11-06): reference regenerated over pending work

- Regenerated after the workspace built again. The new members in the
  reference come from in-progress work already in the tree: audio gain
  envelopes, `Layers.setClockPaused`, `Ui.setHitMaskGeometry` and image
  effect specifications (`dual_texture`, `rect_map`, `tile_reveal`,
  `sequential_box_blur` and others).
- Rust-only additions without reference entries: `buttons::ALT` (Windows
  VK 18), `DesktopRequest::ClipboardRead`, `HostInformation::locale`,
  `crysirror_core::key_hold`, `checksum::sha256` and
  `checksum::crc16_ccitt_serial`.
- Regenerated using the release example; no tests ran for this entry.

## `bb71826030da5aa3` (2026-11-06): asymmetric blur and nearest grid samples

- Added `BoxBlurSpan`, retaining the phase of even or asymmetric averaging
  windows, and `NearestCellSample`, sampling each grid cell's upper-left pixel.
- Rectangle maps admit2048 regions with their existing pixel/write limits;
  serialized image effect specifications remain bounded at256KiB.
- Regenerated the reference using the release example; no tests or games ran.

## `9e0fd03ab0f15bf7` (2026-11-06): image inputs through scoped storage

- Added `ImageInput::ScopedAsset` for decoding images through an explicit
  client storage mount, including images written to Save.
- Image metadata queries accept scoped paths while retaining original encoded
  properties and source identity through asynchronous preparation.
- Regenerated the reference using the release example; no tests or games ran.

## `7c7c6e8d8779b120` (2026-11-06): tiled affine redraw, channel range stretch, PCM filter chains

- Added the `tiled_transform` image effect (source stretched to an output
  size, tiled around the centre, redrawn under offset/scale/rotation with
  bilinear or nearest sampling) and `channel_range_stretch` (one channel's
  observed range mapped onto 0..=255).
- `AudioResourceTransform` gained `PcmFilters`, an ordered stereo PCM16
  filter chain (echo, reverb, chorus, volume, distortion, noise, 3-band EQ,
  mono, pitch shift, AGC) of at most 16 stages.
- Rust-only additions without script signatures: `pointer_swipe` gesture
  queries and `box_downscale` integer reduction.
- Legacy byte strings gained explicit two-band lead-byte classification,
  unsigned32 auto-base conversion, decimal formatting and front splitting;
  scoped byte tables preserve raw cell data independently of native VMs.
- Regenerated the reference using the release example; the fingerprint also
  covers concurrent workspace changes made since `c39454c495b3e405`.

## `c39454c495b3e405` (2026-11-06): box reduction and explicit filter sampling

- Added box-window reduction with grid or proportional anchors, explicit
  integer arithmetic and alpha output policy.
- Added linear byte-channel gradients and sequential box filtering for
  overlapping source/destination regions.
- Shared format services now include bounded QNT encoding with explicit
  header versions and independent colour/alpha planes.
- Regenerated the reference using the release example; no tests or games ran.

## `a1e5b96ca19f37a1` (2026-11-06): grouped reveal and explicit byte composition

- Added optional glyph reveal prefix steps for multiple glyphs per reveal tick.
- Added distinct `Scalar256` and `OpaqueScalar256` stored-byte blend profiles
  and the byte256 rectangle-resampling image effect.
- Rust host services now expose scoped committed-text/IME sessions; shared
  resources expose bounded record sequences and capability tree operations.
- Regenerated the reference using the release example; no tests or games ran.

## `1aba56991a2984b2` (2026-11-06): output-clock gain envelopes and native UI resources

- Added channel and pooled-voice Audio gain-envelope setters and queries, with
  stepped dB100 levels, output-clock phase, cancellation and completion state.
- Added shared rectangle-mapping and alpha-lookup image effects and cyclic
  atlas motion declarations; SugarPot configuration accepts its resource EXE.
- Regenerated the reference using the release example; no tests or games ran.

## `fdebedc6bde8a668` (2026-11-06): canvas perspective and explicit admission order

- Added `CanvasLens` for canvas-centred perspective, declared near-plane
  admission and screen-affine UV interpolation.
- Added retained layer `tie_order` for explicit equal-priority ordering, and
  legacy mesh post-transforms and face-culling policies.
- Regenerated the reference with the release example; no tests or games ran.

## `d14f5d69b7cf0edf` (2026-11-06): ordered composition and retained input geometry

- Added shared projected/model mesh paints, ordered render-state and coverage
  operations, glyph views sharing their source reveal clock, stored-byte blend
  modes and the explicit seven-bit image sampling profile.
- Added projective hit-mask region unions and `Ui.setHitMaskGeometry`, which
  updates retained geometry without retransmitting immutable coverage pixels.
- Shared host contracts expose scoped scene checkpoints and asynchronous audio
  reconstruction. These source APIs do not establish complete native saves.
- Regenerated the reference with the release example; no tests or games ran.

## `20f7dc585b1b21b5` (2026-11-06): default integrated Siglus configuration

- **Breaking:** the default `siglus` adapter now documents the integrated
  runtime configuration: host-injected resource/save roots, native Gameexe/Scene
  names, optional material override and host font selection.
- Extracted `message_window`/resource mapping settings belong to the explicit
  reference VM profile and are not accepted by the default integrated factory.

## `beda65124bb4e01a` (2026-11-06): retained subtree clocks and glyph profiles

- Added `Layers.setClockPaused` and `set_clock_paused` for inherited automatic
  subtree motion, glyph reveal and crossfade clock suspension.
- Added per-glyph coverage, outline and shadow profiles, including bounded
  2/4/8 supersampling and monochrome coverage without flattening text runs.
- Shared services expose cached authorized font aliases, host information and
  source PCM peak observations. Two-texture byte operations use image workers.

## `ff03c58bed4e9be5` (2026-11-05): shared stored-byte image operations

- Added image-effect declarations for affine blending, byte resizing, wave
  displacement, masked blur, spatial gradients, shape masks, rectangle
  resampling, lens distortion and integer colour/lookup operations.
- Refreshed the retained motion reference for repeats and filter RGB channels.
  Reference generation did not execute game or test cases.

## `7ae03e5812971428` (2026-11-05): whole-surface Canvas presentation

- Added `CanvasCommand::PresentTarget` (`present_target` via `Canvases.apply`)
  to compose a paged surface into one bounded isolated target before applying
  global masks or distortion. Existing presentations retain their paging policy.
- Whole-target dimensions are limited to 8192 per axis; renderer memory and
  device budgets remain applicable. Source edits rebuild the retained paint.
- Shared Rust `ScalarOscillator` signals and constant-colour screen lookup
  arithmetic add no reflective script signatures.
- Regenerated the reference using the release example, separately from game
  execution and rendering verification.

## `aad72562918cd94f` (2026-11-05): retained parenting and scoped clocks

- Added `Layers.setParent` / `LayerCommand::SetParent`, preserving paint,
  retained motion and child hierarchy while validating parent ownership/cycles.
- Added `Layers.advanceTime` / `LayerCommand::AdvanceTime`, advancing only the
  calling client's automatic motion, glyph reveal and transitions. External
  producers, other clients and global/input clocks retain their own time.
- Added shared Rust text helpers for full-width conversion and fixed-cell
  forbidden line-end rules. They add no reflective script signature.
- Regenerated the reference with the release example; execution verification
  of native games remains separate from schema generation.

## `db5dbce037a2d408` (2026-11-05): affine masks and quartic interpolation

- Added `LayerMask.scale`: the selected encoded mask byte is multiplied by
  this signed value before applying its offset, truncation and saturation.
  Its default of one preserves existing mask operations; negative values
  support inverted rule-image fades.
- Added quartic in, out, in-out and out-in easing to shared layer motion and
  scalar interpolation. Regenerated the reference using the release example.
- Rust hosts can submit scoped `AssetReader` media with `media_reader` and
  `MediaSourceOptions`, using the existing decoder workers, clocks and surfaces.
  This Rust host method adds no reflective script signature.
- `MediaFrame::as_image(client)` retains a completed picture and its storage
  lease as an immutable engine image without encoding or copying pixels.
  Layer/Canvas snapshots also accept additive, subtractive, multiply and screen
  blending through the isolated renderer.

## `bddc750232fac763` (2026-11-05): selective straight-RGBA lookup profiles

- Added shared `RgbLookupProfile::StraightNonzeroAlpha` and
  `StraightColorKey { color }` to preserve transparent or keyed source pixels
  while applying the same byte-channel lookup tables to visible pixels.
  Existing straight/premultiplied profiles retain their behavior.
- Regenerated the current workspace reference with the release example. The
  previous recorded fingerprint was `acee7de9dee324bf`; generation is separate
  from installed-game behavior verification.

## `acee7de9dee324bf` (2026-11-05): retained UI geometry and layer paint

- Added `Ui.setRect` / `UiCommand::SetRect` to update a client-owned widget's
  geometry while preserving input ownership and existing motion state.
- Added `Layers.setPaint` / `LayerCommand::SetPaint` to replace paint without
  recreating the layer or cancelling its queues. Changing away from image paint
  clears image-only sampling and UV state.
- Both commands are available through the shared Rust and reflective APIs.
  The release reference generator completed; the prior fingerprint was
  `9f6118935489bf0d`. Compilation and API generation do not establish full-game
  behavior. Actual offscreen runs are recorded separately.

## `9f6118935489bf0d` (2026-11-05): shared native control capabilities

- Added scoped UI keyboard ownership, pointer observations, stepped pointer
  motion and positioned-glyph crop updates.
- Layer queues expose signed/IEEE velocity policy, finite draw gates,
  texel-center image projection, centered alignment maps and retained binding
  updates. Qualified `LayerSequence*` data types avoid world-queue name collisions.
- Added live audio playback-rate control, owner-authorized HTTP jobs, historical
  civil-time profiles, and the device-independent `FrameAdmissionClock` helper.
- Regenerated all three reference artifacts with the release example; the prior
  fingerprint was `54bfb53217aaaad3`. Generation succeeded; it does not establish
  native game behavior or hardware validation. No test suite was executed.

## `54bfb53217aaaad3` (2026-11-05): scoped UI key receipts

- Added `WidgetSpec.key_events` with press, release and delayed held edges.
  The focused client and modal subtree own delivery; release requires prior
  ownership of the held key.
- `Ui.drainSubtreeEvents(root)` consumes retained events for one client-owned
  subtree, independently of other dialogs and native input consumers.
- Regenerated the public reference using the release example. The fingerprint
  changed from `203f0e16362f21cd`; generation does not establish game behavior.

## `203f0e16362f21cd` (2026-11-04): retained queues, alpha resources and atlas motion

- Shared layer queues expose frame cursors, record-owned frame progress,
  retained control restoration and image-UV bindings. Explicit reset preserves
  queued records and their recorded frame progress.
- Layer masks support encoded byte channels, signed offsets, source-space
  coordinates and per-texel preservation of original alpha. Shared image
  workers can generate an alpha plane from current, baseline and rule resources.
- `Layers.rebindImage` updates one client's live image paints atomically while
  frozen snapshots retain their previous resource generation. Rust hosts can
  borrow a client-owned published image lease without decoding generated PNGs.
- Shared VFX atlas drift supports streak, buoyant, falling-glow and rising-glow
  motion; paused emitters retain their pool and RNG state while hidden.

## `2ea382a5cb5bf198` (2026-11-04): destination-in Canvas masking

- Added `Blend::DestinationIn` (`destination_in`) to shared Layer/Canvas drawing.
  It multiplies the premultiplied destination and its alpha by source alpha,
  ignoring source RGB; isolated Canvas snapshots support this mode.
- The operation is available to Rust and script clients through existing Layer
  specs and Canvas draws; it adds no VM-specific API.

## `c1a04f58f36209a0` (2026-11-04): browser host boundary and workspace regeneration

- Added the Rust host entry points `Runtime::with_storage`,
  `Assets::from_storage` and `RuntimeStorage`. `SharedHost.fs` now uses the
  capability router; native constructors and native DiskVfs delegation remain.
  Physics accepts the existing `FileSystem` contract. The separate browser
  bindings are generated by wasm-bindgen with the browser artifact; these
  constructors add no reflective engine class.
- Registered the existing `CanvasCommand::Retire` under `Canvases.apply` and
  restored source discovery of `PathCommand` after its move into core's vector
  scene module. This repairs reference metadata without changing these drawing
  operations.
- Regeneration also captures concurrent shared image work: row deformation,
  paired alpha planes, stored-mask blending, particle planes, pyramid levels,
  field warps and immutable process subgraphs. These are engine changes already
  present in the workspace, not browser-only algorithms.
- The reference now records the current serialized `TextureAddressMode`
  spellings (`clamp_to_edge`, `repeat`, `mirror_repeat`). Use these serde names
  for project data. Explicitly unmapped UI/layer/input commands remain visible
  in the reference; generation is not a compatibility claim for them.

## `c47c9352e8db80c9` (2026-11-03): native presentation boundary and workspace regeneration

- The live host can query `Runtime::presentation_pending`, backed by the
  default-false `VmAdapter::presentation_pending` contract, to keep a completed
  frame visible during resource-suspended drawing. Modal UI remains presentable.
  These Rust methods add no reflective JS/TS member.
- This workspace regeneration also records the shared physical window request
  commands `set_window_size`, `set_window_position`, `set_window_visible` and
  `set_window_minimized`, and the explicit stored-byte bitmap arithmetic of
  `ImageCompositeMath::IntegerStored`, added by concurrent engine work.

## `336786cebccc73ce` (2026-11-03): current workspace regeneration and copy-history pruning

- Regenerated the current shared workspace while validating the RiddleJoker
  performance repair. The reference includes accumulated audio channel seek
  and exchange APIs, voice gain/loop controls, `Canvas.applyBatch`, input
  observations and layer source-crop/viewport-clip declarations. These are
  existing implementation changes, not all additions from this repair.
- The new Rust `ImageInput::with_composite_layer` helper discards overwritten
  destination history only for a complete, full-opacity four-channel copy.
  Partial and channel-only operations retain their prior inputs. Callers may
  replace manual Composite appends with this helper; its source mapping and
  pixel arithmetic are unchanged. It adds no reflective JS/TS method.

## `de52270e1e820320` (2026-11-02): frame capture and image export services

- Regeneration after the 2026-11-02 native-adapter work, which added the Rust
  `HostServices::begin_frame_capture`/`Runtime::prepare_frame_captures` frame
  capture path, canvas image export and the `crysirror_image::bmp_encode` module.
- `crysirror_resource::xml::XmlDocument::parse` now also accepts documents that
  declare a legacy encoding such as Shift_JIS. Callers need no migration.
- The field-level delta from `0154abd7b61f34df` was not diffed separately, because
  the earlier generated files were not retained. Check `docs/api/REFERENCE.md`
  for the current signatures.

## `0154abd7b61f34df` (2026-11-01): source crops, explicit blend arithmetic and session seed material

- Registered the actual CropRect shared geometry result, LayerSpec.source_crop,
  PositionedGlyph.crop and scoped Layers.setSourceCrop/sourceCrop. Local source
  geometry crops preserve UV/perspective mapping without inheriting or reflowing.
- Registered the full Rust ImageCompositeMath enum and six normalized/fixed
  blend types, including independent /255 product rounding and source gates.
  Bitmap.beginFixedBlend uses existing immutable source snapshots and destination
  revision/epoch/cancellation rules; no reflected Rust capability class is added.
- Registered bounded SessionSeedSource/Material/State. The optional Rust query
  samples explicit fixed words or authorized OS worker material; it never falls
  back to a simulation clock. Cat Config.random_seeds is the real optional u32 pair.
- Actual combined regeneration changed verified `531a48a7ce30ee00` to this value.
  Tuple payloads retain their declared Rust paths, including the qualified
  ImageCompositeMath::Normalized(crate::image_blend::RgbaBlendState) declaration.

## `531a48a7ce30ee00` (2026-11-01): viewport clipping and retained voice transport

- LayerSpec retains optional viewport_clip separately from local clip.
  SetViewportClip and Layers.setViewportClip/viewportClip share scoped state;
  empty extents are empty, snapshots preserve viewport coordinates, and
  unsupported capture inverse transforms fail explicitly.
- VoiceSpec/AudioParameters expose retain_on_finish and transport_active with
  compatible defaults; AudioPlaybackInfo reports both. SetTransportActive and
  Mixer controls retain source/fader state independently of ordinary Pause.
  Existing Audio.setVoice/playSfx maps expose actual options without a new class.
- Retained EOF/gain-stop voices remain non-stealable; explicit stop/retirement
  and prepared per-client replacement release them while preserving peer DSP.
- Actual combined regeneration changed verified `1bea1d048c6dd276` to this hash.
  The first coverage attempt correctly found the missing command-to-facade map;
  the real map was added before generation, without weakening coverage.

## `1bea1d048c6dd276` (2026-11-01): atomic prepared-layer batches and glyph images

- Canvases.applyBatch accepts the real six-section ServiceBatch. Its new
  default-empty prepare_layers stage runs before Canvas operations inside the
  same engine transaction; old layers/apply order remains compatible. New
  parents and external motion producers no longer require intermediate publish
  and private compensating rollback.
- Registered the actual ServiceBatch Rust/JSON data contract and GlyphImageSpec
  scoped worker contract. The facade keeps aggregate command/serialized-byte
  budgets and host resource/font preparation; the glyph path freezes an
  authorized face and shares existing cancellable rasterization/ImageJob state.
- The observed Glyph-only `081fcf5a46b10d77` generation was interrupted by this
  concurrent public batch before no-bless verification; it was not reported as
  a passing complete reference. This final generation records the combined
  frozen shape; `a7e4bc50985744b6` remains the prior verified reference.

## `081fcf5a46b10d77` (2026-11-01): scoped glyph rasterization into immutable images

- Registered the real Rust GlyphImageSpec data contract for the optional
  ImageMetadata.begin_glyph_image capability. Runtime freezes an authorized
  face and uses the existing bounded cancellable TextSystem coverage kernel
  and ImageJob resource/publication path; no new reflective class is invented.
- Resident results compose through the existing image graph, including
  CopyAlpha, while retaining font/resource accounting through completion.
  This does not mark any native HLL or STRING lifecycle complete.
- Actual regeneration changed `a7e4bc50985744b6` to the fingerprint above.

## `a7e4bc50985744b6` (2026-11-01): prepared audio restore, unit flow and retained glyph bounds

- OriginalRestoreCommit accepts an opaque prepared audio replacement. The
  client-scoped Core preparation status preserves typed failures; Audio's
  worker job/prepared value/exclusive install token expose no decoder payload.
  Final owner validation installs target audio and image/scene state together;
  sequential requests and unsupported host producers remain explicit failures.
- Text.layoutUnitFlow documents bounded logical cell layout with four shared
  Rust data types. Text.beginGlyphBounds/glyphBoundsStatus/releaseGlyphBounds
  document four typed metadata contracts and retained font/budget ownership.
  These do not imply completed native STRING rendering or native save imports.
- Documented static-font dependency visitors, target-only input reset and
  opt-in checked resident pixel reads. Dynamic is_loaded checks do not become
  an atomic font lease. HG3 native pixel-profile metadata remains a Rust
  resource/decoder contract without a new script class or decoder ID.
- Actual single regeneration changed `f52fcaec3a9ab322` to the fingerprint above;
  Rust traits and opaque resources are not fabricated as reflective classes.

## `f52fcaec3a9ab322` (2026-11-01): scoped font metrics and retained channel seeks

- Text.resolveFontAlias/fontMetrics expose the authorized drawing alias and
  immutable one-scalar font metrics. The existing Rust FontMetrics DTO is now
  registered; static family aliases do not manufacture dynamic font handles.
- Audio.beginSeekChannel/channelSeekStatus/cancelChannelSeek expose real source
  cursor completion independently of later EOF and frame-worker publication.
  Rust SourceSeekReceipt stays opaque and SourceSeekState preserves its exact
  Ready frame / Failed String payloads; these are not serialized script classes.
- FontLibrary's new methods retain Unsupported defaults. Static family metadata
  lookup and bounded seek/receipt ownership remain common engine capabilities.

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

## 2026-11-01 — bounded raw-byte grouped token buffers

Fingerprint: `c4f0841c769d7a18` (previous `66c74a7ee0a69bf1`).

- Added shared Rust `crysirror_text::byte_groups::split_grouped_bytes`,
  `ByteGroupSplitLimits` and `ByteGroupSplitOutput` to the public reference.
- The raw-byte profile preserves nested-parenthesis grouping, quote NUL/copy
  tails, logical length and visible output spans. It is distinct from the
  delimiter-only / explicit Shift-JIS-boundary `byte_split` profile.
- Full output extent/overflow/depth checks precede allocation; both passes
  support cancellation without publishing a partial output buffer. No engine
  reflective Class method or Unicode/escape behavior is implied.

## 2026-11-01 — immutable three-input mask blending, CG catalogues and paged images

Fingerprint: `1171d6b86d446b00` (previous `c4f0841c769d7a18`).

- Added `MaskedPairBlend`/`ImageEffect::MaskedPairBlend`, root-only
  `ImageInput::Ternary` and `Bitmap.beginMaskedPair(slot, first, second, options)`.
  Inputs are frozen; independently quantized14bit mask contributions and RGB
  products preserve destination alpha. Weights sum to at most256. Existing
  owner/epoch/revision, cancellation, graph/output/shared-memory bounds apply.
- Added shared `CgCatalog`/`CgCatalogEntry`, bounded CGTABLE/CGTABLE2 decoding and
  Resources format `siglus/cgm`. Metadata lookups do not own native progress.
- Added Rust-only `PagedImageLayout`, `PagedImageRegion`, `PagedImage` and
  `PagedClearPlan`, with4096tile/512MiB layout constants, cancellable intersection,
  existing image-request clear planning and validated immutable completion.
  Undefined partial pages, stale/foreign completions and incomplete batches fail.
  No reflective class or private image renderer is introduced.
- The intermediate masked-pair/catalogue-only reference was `dc94c5e89e5e6072`;
  paged-image source was included before final verification of this batch.

## 2026-11-01 — scoped in-place solid paint colour changes

Fingerprint: `c8c180ba4ff65706` (previous `1171d6b86d446b00`).

- Added `LayerCommand::SetSolidColor { id, color }` / wire `set_solid_color`
  and the public `Layers.setSolidColor(id, rgba)` facade.
- The target must already be an owned Solid layer. Only its four stored colour
  bytes change; transform, motion, opacity, order, clipping and all other layer
  fields remain intact. Foreign, absent and non-Solid targets fail, preserving
  the normal command-batch rollback contract. This avoids replacement Put calls
  that accidentally reset retained state and is usable without CatSystem2.

2026-11-01 Rust-only integer-fields follow-up: documented and registered the
source of `crysirror_core::integer_fields::{logical_size,get,set,fill_clipped}`.
There is no new reflective method, command or DTO. The generated signature
fingerprint remains `c8c180ba4ff65706`; bounded slice ownership, signed32-bit
whole words, unsigned narrow fields and clipped inclusive fills are described
in README. Source registration/regen passed before no-bless verification.

2026-11-01 intermediate Input/Platform reference: generation observed signature
`112d043e04e6dba1` (previous `c8c180ba4ff65706`) for Input.observation,
Platform.integration, read-only Rust input/platform records and shared C/L/Z/
middle-button mapping. README records exact numeric strings, unknown pointer
metadata, partial VK mapping and unavailable SDK-backend semantics. This is an
intermediate batch signature; the final combined source catalogue also includes
ReadingWait and shared ALD types and will be verified after the owner-scheduled
combined release run.

## 2026-11-01 — retained input observations and prepared platform metadata

Fingerprint: `bef9ef5b2845175f` (previous verified `c8c180ba4ff65706`).

- Added `Input.observation()` with client/revision/time, held/pressed masks and
  optional pointer/inside/focus metadata. Exact integer fields are decimal
  strings at the script boundary; unknown observations remain absent, including
  before owner admission and after restore. C/L/Z/middle-button constants and
  the partial shared Windows virtual-key mapper are documented without claiming
  arbitrary key or physical-device support.
- Added read-only `Platform.integration("steam")` over a prepared optional host
  provider. An absent backend reports unavailable; metadata does not initialize
  an SDK or claim entitlement, authentication or achievement support.
- Registered the Rust-only shared `ReadingWait` retained deadline, with pure
  saturating elapsed-time/character-delay evaluation and no clock/input access.
- Registered `AldVolume` and `AldCatalog` as Rust capability-reader/archive
  contracts, preserving sparse original ALD IDs, holes and cross-volume links.
  They have no invented serialized reader representation or reflective class.
- The earlier `112d043e04e6dba1` was an intermediate Input/Platform-only generation.
  This final batch was generated from the actual combined source catalogue.

## 2026-11-01 — Native System4 Rust contracts and shared ALK (`cbe5a4c32a15c8b9`)

- Register the actual optional `system4` native adapter configuration, matching
  the desktop factory/detector and original INI/ALD bootstrap policy.
- Document native bootstrap values, entries, mixers, assets, input and sprite
  state with System4-prefixed documentation aliases; original Rust names and
  serde field names are unchanged. These are native Rust contracts, not new
  reflective engine classes or claims of complete SACT/VM/game support.
- Register shared GPL `AlkCatalog` and its original numbered sparse entry
  capability. Legacy filename extraction and the native ID view delegate one
  shared parser; no duplicate format registration or private decoder is added.
- Previous generated fingerprint: `bef9ef5b2845175f`. This fingerprint was
  obtained from actual source registration by local release generation.

## 2026-11-01 — Encoded image queries, visual motion and legacy CPU profiles (`1cae8f9410b1b5b9`)

- Registered the actual Core ImageEncodingInfo/receipt/state Rust contracts.
  ImageMetadata's asynchronous scoped query reports original header/plane
  properties and a byte fingerprint; Ready does not promise pixel checksum
  validation. Failures retain their engine error class, including NotFound.
- Layer motion now covers all13 existing transform/opacity properties through
  shared scalar tracks. Rotation is in radians; opacity and status use the same
  truncated byte value. UI motion still supports only X/Y.
- Registered the optional GPL text-legacy byte-string/numeric profiles and
  Resource's caller-owned Mt19937_1998 state as Rust APIs. No reflective script
  classes, VM heap or rendering ownership was introduced for these kernels.
- CivilTime::weekday/service::civil_weekday provide a pure validated Gregorian
  Sunday0 query from retained local date fields, without another clock sample.
- Previous verified fingerprint: `cbe5a4c32a15c8b9`. Regeneration printed the
  fingerprint above; independent no-bless verification is recorded separately.

## 2026-11-01 — Shared split-word MT stream and explicit native text decoding (`9dac24fecb3e8275`)

- Registered Resource Mt19937Split as an opaque Rust stream with new,
  next_u32 and fast first_u32. The split-word69069+1 seed profile remains
  distinct from Mt19937_1998 while sharing tested twist/temper machinery.
- Documented bounded text_stream::decode_shift_jis for explicit native CP932
  input, including malformed-input errors and bytes also valid as UTF8.
  Encoding selection is explicit; no reflective class or VM renderer was added.
- Previous verified fingerprint: `1cae8f9410b1b5b9`. The new value above was
  printed by actual regeneration; independent no-bless verification follows.

## 2026-11-01 — External motion producers, affine layer matrices and alpha-only copy (`42a8c3604407b796`)

- Layers now offers beginExternalMotion, commitExternalMotion,
  requestExternalMotionSkip and removeExternalMotion with scoped driver
  generations/revisions, plus setRect. MotionStatus includes actual driver
  completion; a skip request alone never completes a producer.
- Registered ExternalMotionPose/Stamp/Status. Stamp words are exact decimal
  strings. Optional rect/transform/opacity pose updates and completion commit
  atomically. Active producers must retire before scene snapshots.
- SpriteTransform.local_matrix adds an optional column-major affine transform;
  existing scalar transform defaults remain valid. Unsupported projection,
  singular matrices and 3D parent layouts fail rather than flattening poses.
- ImageCompositeMath/PixelMath CopyAlpha exposes alpha-only image composition
  while preserving RGB, with the existing rounded opacity and clipping rules.
- Previous verified fingerprint: `9dac24fecb3e8275`. Actual regeneration printed
  the new fingerprint above; independent no-bless validation follows.

## 2026-11-01 — Shared byte-span replacement and safe C32 formatting (`ceb843236c48cb69`)

- Registered Rust-only Printf32Argument/Printf32Output from shared Text source,
  preserving the actual Word(u32) and borrowed Bytes tuple payloads in the
  generated reference. No reflective class or pointer-varargs interface added.
- Documented byte_replace::replace_spans_bytes as a bounded cancellable,
  encoding-independent ordered-range operation. Replacement bytes are not
  rescanned and repeated empty-range insertions keep their declared order.
- Documented printf32::format_printf32_bytes and its explicit C32 word/byte
  model, limits and unsupported-format failures. Existing format_one_bytes
  retains its first-compatible-placeholder contract without migration.
- Previous verified fingerprint: `42a8c3604407b796`. The new fingerprint above
  comes from actual regeneration; independent no-bless verification follows.

## 2026-11-01 — Shared audio dB units (fingerprint remains `ceb843236c48cb69`)

- Audio.setVoice's existing options Map now accepts gain_db100 in hundredths
  of a dB (-16000..1204), mutually exclusive with linear gain. Ordinary -10000
  is finite attenuation; gain:0 requests silence. Fades remain linear amplitude.
- Documented the four Rust audio::levels conversion functions, including the
  explicit legacy -10000 silence sentinel and bounded reverse conversion.
  The existing release/capture paths reuse that common implementation.
- Actual regeneration kept `ceb843236c48cb69`: reflected parameter/return types
  still use the same Map contract and these new Rust free functions introduce
  no DTO/class signature. Prose/options documentation changed; no signature
  fingerprint change is claimed. Previous typed-shape history remains intact.

## 2026-11-01 — Live isolated Canvas presentation (`7139e6a8e9650eac`)

- CanvasCommand adds present_isolated(id,layer,order,parent,position), reachable
  through the existing Canvases.apply API. Original present semantics remain
  available; the new command composes the source as one immutable snapshot draw
  and refreshes it automatically after edits. Parent transform/opacity applies
  to the result of composition.
- Isolated presentation state is retained in scene state; absent map fields
  default to ordinary presentation. Snapshot inlining now requires the complete
  identity SpriteTransform so local_matrix, depth and other properties cannot
  disappear during optimization. Existing renderer/resource limits still apply.
- Actual regeneration changed `ceb843236c48cb69` to the fingerprint above for
  the typed CanvasCommand variant and its command mapping. CPU/shared-public
  behavior evidence is separate from GPU visual acceptance.

## 2026-11-01 — Independent stereo voice gains and typed audio requests (`5ba5be1c88e5f542`)

- Registered the complete existing Rust AudioRequest enum as a capability
  contract, including new SetStereoGain {channel,gains:[f32;2],fade_ms}. Resource
  payloads remain Rust capabilities, not JSON commands or a reflected class.
- Audio.setVoice's existing options Map accepts stereo_gain:[left,right] with
  each finite multiplier in0..4, independent of equal-power pan/spatial DSP.
  The whole options set validates before mutation; output gains precede Master
  and reverb sends. Rendered voice-frame ramps hold while paused or waiting.
- Documented Rust Mixer.set_stereo_gain/stereo_gain and the shared level
  validator. Worker-owned controls reject mutation; immutable observations and
  completed PCM/control state publish together.
- Actual regeneration changed `7139e6a8e9650eac` to the fingerprint above for
  the newly documented typed request enum. The reflected setVoice Map signature
  itself is unchanged. Native/audio-device acceptance is separate evidence.

## 2026-11-01 — Staged native restore, audio transport and activity/media controls (`feb2ec9be29523ba`)

- Registered Rust original-save ticket/status/progress/batch/owner-only commit
  contracts, with retained typed failure payloads. Preparation is scoped and
  read-only; worker-validated immutable resources and scene install together.
  Audio/World/external-producer replacement remains an explicit unsupported
  profile. The boxed VM plan has no Serialize/project/JSON command interface.
- Media adds setGain/gain without decoder/clock restart. Input.pointerActivity
  exposes an optional exact backend motion token; shared ActivityTimer and the
  full-domain integer-line sampler support owner-clock policies independently
  of frame/input revisions.
- Audio adds counted decoded-frame loop schedules, channel exchange and own
  gain-target/finish/stop-after controls and public playback-info fields. These
  remain distinct from transport pause/stop clocks and stereo output gains.
- Registered five Rust Alice audio metadata version/catalogue/entry types;
  they do not create decoder IDs or native HLL/gameplay completion claims.
- Actual regeneration changed `5ba5be1c88e5f542` to the fingerprint above.
  Source/API behavior and independent no-bless verification are recorded in
  STATUS; no device/game/GPU result is implied by generated declarations.
