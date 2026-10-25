# ADR 0039: BGI as a native VM calling CrySirror services

Status: implemented first supported profile; evidence and limits below.

## Requirement

Register BGI in the monorepo and execute it through CrySirror. Preserve the
original interpreter and native policies while expanding the shared engine's
image, glyph, storage and audio capabilities. A standalone upstream launch or
its precomposed framebuffer is not this integration's acceptance criterion.
Do not modify the parallel renderer/scene-view/asset/GoW work.

## Source and ownership

The complete local ethornell-rs checkout was inspected at HEAD
4c41dc3a566338d2ea8400635e23b062613e3782, with existing owner changes whose
tracked patch SHA256 is c81879bd9acf6fa62fc75576e3b15744d24a485ca13a88057142e3186dd501fa.
This is explicitly a dirty source snapshot, not a clean upstream release.
crates/bgi-upstream contains attributed source, the original manifests/lock,
patch, source manifest and a portable CPU host. No game assets, fonts, compiled
programs, private platform stores or proprietary SDK files are included. The
original workspace declares MPL-2.0; the standard license text is supplied from
CrySirror because the original checkout had no root LICENSE file.

The independent original VM, values/opcodes, native layout, graph/buffer state,
resource formats, cooperative scheduler and original-save codecs stay upstream.
crysirror-bgi is a safe adapter with dormant factory construction. Desktop owns
registration, windows, event loop, GPU and output device. Its inspect command
uses the same registry as execution. JSON data-program instructions cannot be
supplied to the BGI factory.

## Shared services

- ClientStorage.open_asset supplies a read-only AssetReader with length and
  offset reads. The filesystem host streams capability-opened files instead of
  materializing large archives. Native ArchiveSource adapts it to Read/Seek.
  Provider overlays retain priority; OS paths are not exposed to the VM.
- Resource policy is per session. Original UserData aliases resolve to private
  save storage, and original resource saves are not silently imported. Atomic
  persistence failures leave the old in-memory and persisted save visible.
  A host storage lease prevents concurrent writers to that save namespace and
  releases on drop/process exit. Safe standard file locks require Rust 1.89;
  actual compilation here uses the installed toolchain, not an MSRV test.
- The native host emits an owned draw plan. Shared crysirror-image executes
  floating image/quad/clipping/sampling/blending and recovered packed bitmap
  kernels; shared crysirror-text rasterizes float-positioned glyph cells.
  Native layout/ruby/reveal/shadow policies remain in the frontend. The original
  whole-frame compositor is not called by the adapter. One resulting shared
  surface feeds the existing Layer and renderer, with XRGB display alpha made
  opaque only after preserving/hash-checking native-format bytes.
- Audio commands become client-scoped Core AudioRequests. Native BURIKO header
  sample limits and loop points remain adapter policy. Shared streaming decode,
  intro/loop sequencing, gain/pan ramps, pause/fade/rate and mixing live in
  crysirror-audio::channels and Mixer. Runtime produces 48kHz stereo frame
  output. Desktop combines canonical/script/native PCM into one output sink.
  Public Audio bus/ducking controls also reach the native channel mixer; native
  automatic ducking is not invented unless explicitly configured.
- PCM is transient per-frame output and is not replayed if a headless caller
  leaves it unread. Foreign clients, invalid samples/rates and queue overflow
  are rejected before publication. Native VM/device objects are never saves.

The vendored CPU profile excludes GPU/window/Ruffle/device frontends. Existing
Siglus Kira CPAL is now opt-in via native-audio, preserving its decoder formats
without a default device dependency. Borrowed MPEG packet-range helpers are
added to the shared decoder rather than linking a duplicate MPEG implementation.

## Semantic boundaries

The original native frame quantum/watchdog and memory budget remain intact;
one adapter step executes one bounded native tick. The core budget is not
misrepresented as an exact per-opcode quota. Pending host dialogs/installer
requests require an explicit host response and currently fail in this adapter;
they are never auto-confirmed. Native unknown visual modes/effects fail visibly.

Display selectors 1 and 0x20 are explicitly known aliases in the recovered
dispatch. Their display path matches the original Rust snapshot's float
coverage behavior. This is not a claim of bitwise equivalence to the commercial
Windows renderer; recovered packed bitmap kernels are a separate exact API.
Calendar remains actual host local time. Embedded filesystem fallback is
disabled, and platform memory metadata uses equivalent kernel APIs without
starting sysctl/vm_stat processes. Original interpreter unsafe code remains in
the trusted attributed upstream crate; core and adapter code forbid unsafe.

## Acceptance evidence

Implementation, compilation and behavior are recorded separately in STATUS.md,
bgi-integration-results.json and associated bgi-*.json/log files.

- Production storage policy tests include multi-session scope, large sparse
  streams and save persistence failures; independent Core/runtime tests cover
  leases, provider priority and PCM rollback/fresh-frame behavior.
- Native draw/glyph/pixel functions are compared against unchanged original
  source using tools/compare_bgi_cpu.py. No game/font data is shipped.
- Actual local trial runs must use the registered BGI factory, original VM,
  shared composition/glyph services and ordinary Runtime input/time. Title,
  dialogue, clicks and non-silent PCM are reported by observed checkpoint.
- GPU evidence comes from the real desktop host on its actual adapter, not a
  generated data-VM project or a screenshot injected as a replacement game.

## Remaining limits

This is not complete BGI compatibility. Runtime snapshots/import staging remain
unsupported; original in-game save slots are the supported persistence route.
Atomic native rename and persistent empty directories are not implemented.
Host modal/installer/IME capabilities need explicit services. Some video/audio
containers and native display modes remain unsupported. CPU composition and
native buffer algorithms are a correctness path, not an AAA throughput claim.
No Windows/Linux, mobile/Web, console, hardware RT or audible-output validation
is implied by local CPU/GPU/PCM checks.
