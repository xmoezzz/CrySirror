# Resource formats and archive mounts

`crysirror-resource` owns shared CPU contracts. `crysirror-formats-arc` supplies
optional Rust archive parsers migrated from the pinned arc_unpacker source.
The format plugins have no CLI, renderer, window, audio device or native VM.
Archive entries become normal client asset VFS paths. This work does not replace
or re-register the existing BGI VM adapter (ADR 0039).

The optional plugin implements all **289 named upstream decoder IDs**: 152
archives, 96 images, 16 audio decoders and 25 encoded-file converters. The
reviewed inventory records **258 ported and 31 partial** implementations;
registration is not a claim that every version, encrypted title or game works.
[arc-resource-support.json](arc-resource-support.json) records archive/audio/file
variants and evidence; [arc-image-support.json](arc-image-support.json) retains
image-specific boundaries. Unsupported variants and missing native profiles
fail explicitly. There is no pending ID hidden behind a registered stub.

Select the plugins explicitly:

```sh
cargo run -p crysirror-desktop --features arc-formats -- --help
cargo test -p crysirror-engine --features arc-formats --test resource_api
```

`arc-formats` is an opt-in feature because the translated source retains its
GPL-3.0-or-later license. The resource contract crate retains MPL-2.0.
The complete pinned decoder inventory records implemented, partial and pending
formats; pending formats are never registered as working decoders.
This ledger describes this migration only. Existing CrySirror image, audio and
native BGI decoding capabilities remain available through their existing APIs.

Rust, JS/TS and inspection tools use the same `Resources` class:

```ts
resources.formats();
resources.probe("data.xp3");
resources.list("data.xp3", { format: "kirikiri/xp3", offset: 0, limit: 128 });
resources.mount("data.xp3", "story", { format: "kirikiri/xp3" });
bitmap.load(1, "story/image/background.png");
```

## Native pictures and textures

`Runtime.assets.decoders` owns a content-aware CPU registry. Registered formats
include QNT/PMS/AJP/DCF, TLG5/6, G00/PDT, CBG, RCT/RC8, PGD, NVSG, PB3,
ERI, GIM/GTF/XTX and the remaining native IDs recorded in the ledger. Standard
PNG/JPEG/BMP/DDS/TGA/WebP codecs reuse shared implementations where appropriate.
Normal Bitmap, text-icon, retained-layer and renderer uploads use this registry;
the PBR loader accepts `load_pbr_with_decoder` so glTF textures use it too.

```ts
resources.imageFormats();
resources.probeImage("story/background.tlg");
bitmap.load(2, "story/background.tlg");
resources.decodeImage(3, "story/portrait.rct", { format: "majiro/rct", profile: "story" });
layers.put({ id: 7, rect: [0, 0, 640, 480], paint: { kind: "image", asset: "__surface/3" } });
```

Native bases, palettes, masks and reference images resolve relative to the
current logical image path through the same client capability. Only a typed
NotFound permits an optional resource to be absent; denied/corrupt reads remain
errors. Parent traversal and host paths are rejected. Recursive dependency
decodes stop after 16 levels; per-image and cumulative byte limits apply. These
limits account for encoded/decoded payloads, not every codec's transient memory.
Specific native wrappers outrank generic codecs; headerless formats use path
hints, and ties require `decodeImage(...,{format:...})`.

Password-dependent FC01/MCG and Majiro/RCT algorithms request keys by native
format ID from the client's host key provider. They contain no title key tables.
Use `Resources.decodeImage` with a host-configured context for these resources;
raw key material is never a JS/profile/save argument. The low-level
`ImageDecodeContext` can also accept a Rust key/transform context directly.

Behavior evidence contains 177 external upstream fixture comparisons: 174 have
identical dimensions and RGBA bytes, while AJP/JPEG/WebP account for three
documented lossy-codec rounding differences. No upstream fixture binaries are
distributed. Synthetic integration tests verify archive-to-native-image decode,
exact Bitmap pixels, client keys and references, failure atomicity, and native
glTF material texture mips. Per-format variants and limitations remain binding;
these surfaces do not claim a native VM's frame/pivot/cut/save semantics.

An actual Metal renderer smoke test uses entirely self-authored TEYL, PGX and
NVSG images plus a PNG reference, read from a generated SAR archive. Reproduce:

```sh
python3 tools/make_native_image_smoke.py local/work/native-image-archive-smoke
cargo run -p crysirror-desktop --features arc-formats -- render \
  local/work/native-image-archive-smoke/project.json \
  --script local/work/native-image-archive-smoke/mount.ts \
  --output local/work/native-image-archive-smoke/render.png \
  --data-dir local/work/native-image-archive-smoke/state
```

The `resources` global is generated from the live class registry. `probe` reads
bounded prefixes/index data; ambiguity needs an explicit `format`. `list` pages
metadata only (default 128 entries, maximum 1024 per response). Unknown formats,
unsupported layouts/codecs and missing keys return errors.

Mounted providers open `AssetReader` streams. Subranges and concatenated segments
support random reads without copying the entire archive. Compressed entries
are currently materialized within explicit entry and retained decoded-byte
budgets. Zlib and raw deflate detect corrupt/truncated streams and output beyond
declared limits. Long compressed-entry streaming/seek indexes remain future work.
The byte budget counts retained decoded payloads and reservations, not total
process memory or the transient allocations inside external codecs.

Mounts follow the existing VFS lifetime and limit (64 providers per client).
The capability source is captured before mounting; native paths are normalized
and validated, with traversal and duplicate canonical paths rejected. New mounts
invalidate that client's ordinary asset byte cache; generated resources keep
their immutable namespace. Mount before loading GPU assets: this interface does
not promise live replacement of already uploaded textures or meshes. Mounts,
decode profiles and keys are host configuration, not a portable save snapshot.

## Transform and key interfaces

Hosts can register a format through `ArchiveFormat`, a byte transform through
`ByteTransform`, and a key service through `KeyProvider`. `PipelineSelector`
chooses transforms by format and native entry metadata. The built-ins are
`identity`, `zlib`, `deflate`, `lzss`, `xor`, `aes-ctr128be`, `aes-ctr128le`,
`aes-cbc`, `aes256-cbc` and `rc4`. Shared Rust primitives additionally implement
Blowfish little-endian blocks, PKCS#1 type-1 RSA public decoding and caller-IV
MD5 for native layouts. CBC validates complete blocks and selected padding;
LZSS distinguishes bytewise and configurable bitwise layouts.
XOR and CTR streams support unaligned random reads. CTR and XOR are not
authenticated encryption; their output alone does not establish a correct key.
Format-specific integrity checks or authenticated transforms must enforce any
additional integrity contract.

Rust hosts set `EngineState.resources.keys` before defining profiles. A
`MemoryKeyProvider` indexes keys by `(ClientId, key_id)`; another client cannot
resolve that identifier. Key buffers are redacted in debug and zeroized after
the last reference is dropped. They do not implement serialization. Secret
material belongs in the host key service, never in project/profile/save JSON.

Profiles contain identifiers and algorithm parameters only:

```ts
resources.defineProfile("story", {
  rules: [{
    format: "kirikiri/xp3",
    path: "image/background.png",
    transforms: [{ algorithm: "xor", key_id: "provided-by-host", parameters: { offset: 0 } }]
  }]
});
resources.mount("data.xp3", "story", { format: "kirikiri/xp3", profile: "story" });
```

Rules match exact native paths, falling back to a rule for the whole format.
Profiles are immutable and client scoped (maximum 32); all profiles for the
same client share retained decoded-byte accounting. The default profile has
100,000 entries, 64 MiB per entry, 16 MiB index data, 256 MiB retained decoded
bytes and 1 MiB probing per format. Declared limits can be configured explicitly.
Game-specific XP3 crypt filters use the same transform interface and must be
implemented from an inspected format/profile; a generic cipher name is not a
claim of universal title support.

Resource tests include a published AES-CTR known-answer vector from
[NIST SP 800-38A, F.5](https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38a.pdf),
independent expected plaintext, cross-block seeks, malformed compressed data,
budget rollback and a synthetic 3 GiB random-access source. Engine integration
tests generate their own SAR and PNG data at runtime and exercise the existing
Bitmap, Fs and Assets APIs; no game data or fonts are included.

## Native file and audio conversion

`FileRegistry`/`FileFormat` return engine-owned `DecodedFile` readers with an
extension hint and native metadata. `Resources.fileFormats()` lists 25 file
converters and `audioFormats()` lists 16 audio converters. Converted images
remain ordinary image resources; sound conversions use the shared `WaveSpec`
serializer and feed the existing CrySirror audio decoder/mixer through VFS.
No external unpacker or legacy engine process is invoked. HCA uses the safe
Rust `cridecoder` library; MIO, NWA and the other native codecs are Rust ports.

Realtime callers schedule work on the existing bounded CPU task pool. Source
opening happens there too, including materializing a compressed mounted entry:

```ts
const ticket = resources.beginDecode("sound/line.p16", "file", {
  format: "leaf/p16"
});
// On a later update, after status.state becomes "ready":
resources.decodeStatus(ticket);
resources.mountDecode(ticket, "decoded/line.wav");
resources.cancelDecode(ticket); // releases the ticket; mounted readers survive
```

Use the registered format ID returned by `fileFormats()`/`audioFormats()`;
headerless or ambiguous formats require an explicit ID. `beginDecode` accepts
`file` (including audio) or `archive`. `readDecode` pages retained file bytes or
archive entry metadata. Polling, paging and publication never join a worker or
run a codec on the realtime thread. Pending/failed reads and invalid tickets
fail visibly. A client may retain 64 tickets, with 256 per host; completed and
failed tickets count until cancelled. Late outputs after cancellation are
released. Ready resources retain their original profile and byte reservation.
Offline `detectFile` probes formats; `readDecoded`/`mountDecoded` provide the same decode path synchronously;
realtime hosts reject these synchronous calls. Byte pages are capped at 64 KiB,
archive metadata pages at 1024 entries. Opening an entry of a mounted archive
may decode that entry; realtime consumers should use ordinary async Assets
loading. The initial archive mount does not predecode every contained file.

Profiles also accept `parameters`, keyed by native format ID, for binary layout
options and host key aliases. These are checked by the individual parser;
unknown options, unresolved keys and unsupported versions are errors. Title
key catalogues, logos and region images are supplied by the host when required.
Companion resolver calls detect cycles and stop after 16 nested resolutions.
Companion files resolve through a stable client storage capability, never an
ambient OS path. The resolver is weakly held by mounted providers to avoid a
VFS ownership cycle. Relative siblings, cross-client separation and symlink
escape rejection have engine API tests. Profiles, mounted resources and jobs
are runtime capabilities and are not serialized as portable saves.

The WARC implementation supports version 1.70 with explicit native profiles.
Its table-sized essential cipher has three differential vectors compiled from
the pinned original C++ functions (`tools/verify_arc_warc_cipher.py`); this does
not validate title-dependent entry keys or every encryption hook. Rust defines
negative floating-to-u32 conversion as truncation followed by modulo 2^32;
the original C++ expression has undefined out-of-range conversion behavior.
YLZ is rejected, as it is unimplemented in the pinned source. Other partial
families, including Unity serialized-file versions and palette variants, remain
listed individually in the ledgers.
