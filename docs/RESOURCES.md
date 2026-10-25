# Resource formats and archive mounts

`crysirror-resource` owns shared CPU contracts. `crysirror-formats-arc` supplies
optional Rust archive parsers migrated from the pinned arc_unpacker source.
The format plugins have no CLI, renderer, window, audio device or native VM.
Archive entries become normal client asset VFS paths. This work does not replace
or re-register the existing BGI VM adapter (ADR 0039).

The same optional plugin crate now supplies **all 96 image decoder IDs** from
the pinned inventory: 80 ported and 16 with explicit partial/variant/evidence
boundaries. Together with the five archive IDs, 101 decoders have concrete
implementations; 188 other archive/audio/file decoders remain pending.
The authoritative per-format ledger is [arc-image-support.json](arc-image-support.json).
These counts describe named decoder implementations, not complete-game coverage.

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
`identity`, `zlib`, `deflate`, `xor`, `aes-ctr128be` and `aes-ctr128le`.
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
