# ADR 0048: original RealLive VM calls shared engine APIs

Date: 2026-10-26. Status: implemented API bridge; native opening-profile
validation is recorded separately. Complete-game compatibility is not implied.

## Context

[ADR 0047](0047-engine-api-capability-extraction.md) rejects hosting another
engine and uploading its composed screen. Referenced engines exist to reveal
missing CrySirror capabilities. A new VM should need native semantic/syscall
translation, rather than carry its old generic drawing, text, UI, audio and
movie engine with it.

The original RealLive parser, expression/bank model and interpreter are already
available at the actual siglus_rs pin
`5f358ef1360b0439e92aed2b749e8f25d8721954`. They can be separated from its
original Engine/System/Graphics/Sound/Movie and painters.

## Decision

1. Retain original VM execution and bytecode/value/control-flow semantics in
   `crysirror-reallive/src/vm`. Emit pending engine-command/text events and
   acknowledge their PC only after successful shared API calls. Respect the
   caller's instruction budget and reject unsupported native operations.
2. Add engine-owned retained canvases and logical copy snapshots. Native DC
   operations map into typed Canvas commands, and presented nodes become
   ordinary retained Layers. A native composed-screen image is not an adapter
   interface. Missing isolated-composition capabilities fail visibly.
3. Represent explicit native text positions as shared GlyphRun paint. Shared
   text resources own glyph metrics/rasterization; shared retained state owns
   scalar reveal clocks. Native markup/window/grid/wait meanings stay in the
   adapter. Shared UI owns choice and editable widgets and event delivery.
4. Route audio through shared native resource decoders and the existing channel
   mixer. Route movies through engine-owned Media resources using scoped asset
   streams, bounded decoder queues, clock/state queries and separate media
   texture resources. The VM does not own a movie decoder or output device.
5. Expose these capabilities to ordinary engine clients too: Canvases and Media
   classes, Layer/UI state and Rust service APIs. Their dependency direction is
   adapter to engine; the shared services do not depend on the RealLive crate.
6. Preserve source attribution and a complete reference hash inventory without
   equating that inventory with opcode/engine-module coverage. The pure VM's
   SOURCE.json records selected source files and excluded engine modules.

## Rejected alternatives

- Restoring the removed hosted engine packages or using their renderer while
  labeling a final image upload an API bridge.
- Replacing original bytecode with data-VM instructions or treating unsupported
  native effects/layouts as successful no-ops.
- Keeping a private generic renderer/text/mixer implementation in the adapter
  because a fixture becomes easier to run that way.
- Calling compile success, matching pixels, factory registration, or an
  independent media test complete native integration acceptance.

## Evidence and limits

[REALLIVE.md](../REALLIVE.md) describes actual ownership and setup;
[reallive-source-audit.json](../reallive-source-audit.json) records provenance,
forbidden-path checks, shared API routes and independent media evidence.
Native integration fixtures must demonstrate inspectable shared nodes and
engine edits while the VM waits, as well as preserved VM flow and errors.
The owner-supplied opening profile remains a distinct integration result.

Native save files, whole-adapter snapshots, DLLs, every native effect/layout,
full-game coverage and additional desktop/device behavior are not claimed.
Movie audio remains bounded full-track PCM; active-media snapshots, seek/pause
and long-track streaming need their own shared implementations and validation.
