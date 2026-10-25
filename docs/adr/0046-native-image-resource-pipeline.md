# 0046 — Native image resources through the shared engine

Date: 2026-10-25. Status: implementations and integration verified; individual
variant limits remain recorded in the support ledger.

## Implementation

The complete pinned arc_unpacker image inventory contains 96 decoder IDs.
All 96 now have registered Rust implementations/adapters in the optional
`crysirror-formats-arc` crate: A21, B37, C32, D5 and the reused CBG CPU decoder.
The image ledger records 80 ported and 16 partial implementations. Combining
these with the five archive decoders gives 101 registered implementations;
188 other archive/audio/file decoders are still pending. The inventory tool
retains all 289 original registrations and the 870-file source hash manifest.

`crysirror-image` owns the safe CPU `ImageFormat`/`ImageDecodeContext` contract,
registration, content recognition and path hints. More specific native wrappers
outrank a generic embedded codec; weak extension hints do not override a strong
PNG/JPEG signature. Identical raw layouts use known extensions or an explicit
format ID. Equal-priority ambiguous matches fail visibly.

Runtime owns the registered decoders. Bitmap, text icons, retained-layer upload,
transition masks and the renderer all call `Assets.decode_image`. The CPU PBR
loader accepts an injected image decoder, preserving native glTF texture bytes,
material color-space decisions and engine mip generation. No native VM,
original whole-frame compositor or tool executable is introduced here.

Optional bases, masks, palettes and references resolve relative to the image
through one client's capability VFS. Only typed NotFound becomes an absent
optional dependency; denied/corrupt reads remain errors. Traversal, recursion,
encoded/decoded size limits and surface validation are enforced at the common
boundary. Keys come from the shared client-scoped `KeyProvider`. Native
FC01/MCG and Majiro/RCT password algorithms use caller bytes; no per-title key
tables are embedded. Transient decoded pixels and key material are not new save
fields, native executable pointers or GPU handles.

The reflected Resources class adds `imageFormats`, `probeImage` and
`decodeImage(slot, asset, {format?,profile?})`. Existing `Bitmap.load` calls use
the same pipeline. TypeScript declarations and machine-readable API docs are
generated from this live registry.

## Compilation and behavior evidence

Group implementation, source/hash lists, compiler outcomes, exact external
commands and variant-specific behavior are recorded separately in:

- [A21](../arc-images-a.json): AliceSoft, Kaguya, RealLive, SPB, TLG, PRT/SOTES,
  PB3 and ERI.
- [B37](../arc-images-b.json): standard wrappers, Crowd, FC01, ActiveSoft,
  TwilightFrontier and other compressed formats.
- [C32](../arc-images-c.json): Majiro, PGD, FVP, Leaf, LIAR, ELG, WBM and others.
- [D5](../arc-images-d.json): GLib PGX/JPEG-PGX and GIM/GTF/XTX texture layouts.

The groups report 88 ordinary image tests. Seven explicitly run external tests
compare 177 image outputs against the upstream expected PNGs: 174 RGBA arrays
match exactly; three AJP/JPEG/WebP comparisons have quantified shared lossy-codec
rounding differences. External fixture binaries remain outside the deliverable.
The support ledger links every decoder to its real implementation and evidence;
registration strings alone cannot change a pending entry to supported.

Three engine integration tests generate their own resources: SAR-to-QNT/TEYL
through Bitmap and Resources, scoped MCG keys, RCT base/mask permission and
corruption behavior, failed-write atomicity, and complete glTF geometry with
native material images and exact sRGB/linear mip pixels. Common registry tests
cover specificity, ambiguity, cyclic dependencies, path escapes and errors.

An actual Apple M5 Metal run loads self-authored TEYL, PGX and NVSG data from a
generated SAR through `Resources.mount`, then the ordinary retained-layer
renderer. All three 128×128 native regions match the PNG-reference region
pixel-for-pixel (49,152 native pixels); 16 interior samples also equal their
declared source RGBA. Reproduction is documented in [RESOURCES.md](../RESOURCES.md)
and the capture is [native-image-registry.png](../../images/native-image-registry.png).
This is one local GPU run, separate from ignored GPU suites or commercial-game
compatibility.

## Remaining boundaries

The per-format [support ledger](../arc-image-support.json) is authoritative.
It includes ASCII-only filename-derived BJR/Cronus keys, scoped dependency
discovery limits, explicit unsupported headers/flags, and the reused CBG
decoder's evidence boundary. Original unsafe partial reads and uninitialized
tables are replaced by visible errors. TLG/PB3/ERI support is bounded by named
layouts and decoded-memory limits, not a claim that every existing game image
is accepted.

These APIs return static RGBA surfaces matching the source decoder's image
contract. Native VM cuts, pivots, message pages, animations, waits and original
saves remain adapter policy. This work does not register RealLive/AVG32 VMs,
claim all 289 formats are complete, or establish another operating system's
GPU behavior.
