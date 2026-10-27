# ADR 0088: KAG3 semantic adapter and shared engine prerequisites

Date: 2026-10-27
Status: Accepted for the bounded native profile below

## Purpose and source boundary

KAG3 is a TJS application using native KAGParser, Layer, Timer, AsyncTrigger,
WaveSoundBuffer, MenuItem and Window services. Running another engine underneath
it would bypass CrySirror's capability growth. The optional `krkr` adapter instead
executes the already pinned, unchanged original TJS2 VM and KAG parser. Newly
written native facades translate semantics to shared engine resources/operations.
No upstream engine, renderer, font rasterizer, mixer or media player is imported.

The stock KAG3 system scripts remain external runtime inputs. Source pins and
review scopes/hashes are in `docs/kag3-source-audit.json`. In particular, original
KAGParser callback and save field meanings were inspected before implementing
hooks; native label restore is not replaced with CrySirror's runtime save format.
The optional AGPL boundary remains confined to the existing three krkr crates.

## Reusable engine capabilities

- Core DeferredDispatch owns bounded client/target notifications, phase selection,
  cached replacement, cancellation and a captured turn fence. Callback-generated
  work cannot recursively run under the same fence. Core RegionMap owns clipped
  integer interaction-region fills and queries independently of visible pixels.
  Both are safe no_std + alloc kernels usable without any VM or device.
- TextMeasurement::position_simple and public Text.positionSimple return exact
  scalar baselines from the engine's existing simple font layout. Native bold
  text and unblurred offset shadows feed ordinary retained GlyphRun draws. The
  adapter contains no font layout/rasterization implementation. Complex shaping
  remains a separate engine API; this profile is simple-horizontal-v1.
- Ui.setWindowPolicy / setFullscreen store client-owned logical requests. Runtime
  snapshots retain them; the desktop winit owner applies decoration, resizing and
  borderless fullscreen. GPU code remains independent of winit. Native reads
  report requested state, not an asynchronous OS acknowledgement.
- ImageMetadata::opaque asks the shared decoder/cache whether all texels have
  alpha 255. This supports known-uniform mask decisions, without a private native
  pixel buffer. Drawing invalidates summaries conservatively. Nonuniform mask hit
  tests requiring CPU Canvas readback fail explicitly.
- ClientStorage::begin_write_atomic reserves bounded shared VFS worker capacity
  before accepting immutable bytes. A pollable completion retains Ready/Failed;
  capability directories and relative paths are the only worker inputs. The
  worker performs the existing fsync/rename commit. Asset writes and payloads
  over 4 MiB are rejected. Each VFS owns one worker/64 outstanding reservations;
  shutdown follows TaskPool's explicit join policy. Dropping an unfinished task
  requests cancellation, but does not roll back a commit already in progress.

Public API documentation and independent tests expose these capabilities without
loading the krkr VM. Existing retained Canvas copies, image decoders, menus, mix
hierarchies, prepared audio resources, voice playback and input journals are
reused rather than duplicated.

## Native execution and ownership

Plugins.link('KAGParser.dll') installs a real parser facade, not a registration
shell. The original parser owns text/tags, macros, conditions, expression
attributes, script blocks, calls/returns and labels. The facade publishes parser
state during callbacks and preserves nested label changes. The native macros
Dictionary has live identity and external body edits synchronize before parsing.
Macro parameter dictionaries are read snapshots; mutable macro parameter identity
is not an admitted compatibility claim.

A fresh native owner pass captures a deferred-event fence and runs exclusive,
input, normal/timer, continuous and idle phases. A bounded shared InputInbox
records ingress before dependency polling. Audio open/play and native saveStruct
park the original TJS stack until shared resources/voice publication or atomic
commit. A resumable phase cursor prevents restarting that phase or replaying its
input while waiting. A newly posted callback waits for a later fence. Native
keyboard/mouse input is framed, with the existing Core key domain; wheel, IME,
analog pads and sub-frame OS event history are not added.

Nested Rust parser expressions/callbacks and input callbacks are not resumable;
audio/file operations there fail before admission. Ordinary conductor/timer/
deferred handlers may suspend. Async decoder/write failures after admission fail
the owner visibly; catchable TJS asynchronous I/O error re-entry is not claimed.
Script reads, discovery and image metadata still have synchronous boundaries.
This is an ADR 0050 stage, not a declaration that the entire engine is async.

WaveSoundBuffer owns only native semantic fields and scoped logical references.
Decoded sources, read-ahead, mix groups, looping, gain/pan/pause, fades, EOF and
status observations use shared HostServices. The mapping admits 64 buffers;
preparation/voice tickets are released on completion and explicit finalize releases
sources. No private decoder, voice mixer, DSP or output device is introduced.
These native channels currently use the shared Sfx bus; KAG-specific music/voice
bus inference, live seek and delayed native fades remain open.

Layer reparenting publishes ordinary engine retained parent changes and rejects
cycles, foreign windows, primary replacement and detached/null lifetime policy.
Known-uniform masks and Core province regions support input. Text, images, solid
fills, whole-destination atlas-cell copies and hierarchy properties are admitted.
Default/arrow/hidden cursors and empty hints are supported; other cursor resources,
nonempty tooltip UI, focusable layers and modal input require additional mappings.
Identity adjustGamma is an actual no-op. ADR 0090 subsequently adds shared
worker gamma, grayscale and flips for loaded/processed/uniform images, plus
a full-clip layerExRaster facade. Composed Canvas/text readback, unsupported
blend/clip/partial-replace operations still fail visibly.

## Native persistence and deliberately incomplete surfaces

Dictionary/Array.saveStruct still call the original VM serializers. The facade
only adapts write capabilities and commit completion. Default text remains UTF-16
LE with BOM; binary mode uses the original serializer. Writes are confined to the
virtual savedata/ root, mapped to each client's Save mount. Other modes fail.

KAGParser.store/restore use original dictionary field names. This bounded profile
admits only plain scenario-load/label boundaries, with no active native call or
macro stack. Restore reloads the storage and label, following original GoToLabel
semantics, rather than resuming the stored byte/line cursor as a new save format.
Unsupported stacks/cursors/layouts fail. Full KAG game-save/replay compatibility
and CrySirror runtime snapshots of the TJS VM remain unsupported. Full native
Window.add/remove registered-object shutdown is not mapped by this phase.

VideoOverlay can exist in the unloaded state for stock class construction.
Opening/playing/stopping media fails explicitly: shared prepared media plus
pause/seek/status semantics need a proper adapter. This declaration is not video
playback. Native XP3 auto-path mounting also remains unsupported; external assets
may already be supplied through the engine's resource/VFS layer.

## Verification boundary

Release tests distinguish shared capabilities without a VM, self-authored native
semantic fixtures and an ignored external-source/font KAG3 test that must be run
explicitly. The latter executes unmodified pinned stock system scripts with a
self-authored scenario and generated correctly sized wait-glyph sheets, testing
text, line/page waits and input. Actual compilation, behavior and GPU results are
recorded separately in STATUS.md; ignored tests alone are not passes. No commercial
game or complete KAG3/KiriKiri compatibility follows from this bounded profile.

The external stock-script acceptance can be reproduced without committing fonts
or upstream system files:

```sh
CRYSIRROR_KAG3_SOURCE=/absolute/path/to/pinned/kag3 \
CRYSIRROR_TEST_FONT=/absolute/path/to/system-font.ttc \
cargo test --release -p crysirror-krkr --test kag3 -- --ignored
```

A normal project declares adapter `krkr`, its asset-root-relative `config.root`,
and a default `startup.tjs`. Build the desktop with `--features krkr` explicitly;
add `arc-formats` when using shared optional TLG/native resource codecs. This
feature remains outside the default distribution. Unsupported native plugins do
not load OS DLLs; declared registration-only shells never export functional APIs.
