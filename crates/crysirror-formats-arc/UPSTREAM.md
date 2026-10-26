# arc_unpacker resource plugins

This optional crate is **GPL-3.0-or-later**, translated from
[`vn-tools/arc_unpacker`](https://github.com/vn-tools/arc_unpacker). Rust translation
does not remove that licence. `crysirror-resource` and `crysirror-image` have their
own shared API implementations; selecting this particular plugin crate is an
explicit GPL opt-in. The `arc-formats` feature in the engine/runtime is not a
blanket relicensing statement for a combined binary. `LICENSE.md` reproduces the
upstream GPLv3 text. Translated files retain the upstream rr- attribution and GPL
SPDX identifier; the MT implementation also credits Matsumoto and Nishimura.

The complete read-only reference checkout is kept in ignored
`local/reference/arc_unpacker`, not vendored verbatim. The audited clean commit is
`456834ecf2e5686813802c37efd829310485c57d`. No user game data, font, proprietary SDK,
upstream binary fixtures, title key catalogue or generated executable decryptor
is distributed with these plugins.

## Reviewed implementation scope

[`upstream-formats.json`](upstream-formats.json) inventories **289 concrete
upstream decoder IDs** from 870 pinned C++ source/header files. All 289 now have
concrete registered Rust implementations. Registration and availability are
separate from supported variants and behavior evidence:

| Kind | Registered | Ported native variants | Partial, with explicit boundaries |
|---|---:|---:|---:|
| Archive, including image/audio archives | 152 | 138 | 14 |
| Image | 96 | 80 | 16 |
| Audio | 16 | 15 | 1 |
| File transform | 25 | 25 | 0 |
| Total | 289 | 258 | 31 |

These are scoped implementation states, not a claim that every title, layout,
codec or original VM is supported. In particular, a valid archive mount does
not establish that the game inside it can execute.

The support authority is explicit reviewed metadata:

- [`docs/arc-resource-support.json`](../../docs/arc-resource-support.json) lists
  all 193 archive/audio/file IDs, parser entrypoints, implementation files,
  native variants, limits and ordinary/external evidence references.
- [`docs/arc-image-support.json`](../../docs/arc-image-support.json) retains the
  reviewed 96 image IDs and **16 partial image boundaries**. Its linked image
  evidence records distinguish exact RGBA comparisons from codec tolerances.

`tools/inventory_arc_unpacker.py` reads those manifests. It verifies the actual
source commit, clean tracked source bytes, SHA-256 manifest, unique upstream IDs
and inherited decoder kinds. It then checks that the native archive `FORMATS`
records agree with the reviewed parser entrypoints, every group reaches the
aggregate iterator and registration wrapper, direct archive constructors are
registered, and the file/audio ID lists reach their native decoder dispatch.
Image registrations retain the equivalent checks. Neither a filename, a helper
template, an ID string nor adding an unreviewed registration can grant a
successful support state. These consistency checks do not execute the decoders
or substitute for behavioral review.

## Native semantics and remaining boundaries

Archive plugins parse native indexes and return shared `Archive` resources.
Raw ranges remain random-access subreaders; native compression, segmented
members, image archive frames and audio archive members use bounded shared
resource decoding. PNG and WAVE construction, generic AES/Blowfish/RSA/RC4/MD5
primitives, image composition and scoped companion resolution are reusable
engine capabilities. No extraction subprocess, original engine renderer, native
VM or executable resource code is hosted by this crate.

`DecodeContext` owns client-scoped key aliases, format parameters, transform
pipelines, source names, companion resolvers and decoded-resource budgets.
Encrypted native profiles must be configured explicitly. Title material comes
from `KeyProvider`; parameters reference aliases rather than copying title
catalogues. Companion reads use the caller's capability resolver and canonical
logical paths. The original tools' ambient directory searches are represented
by explicit caller-supplied companions/keys. Missing keys, required profiles,
unknown layouts and unsupported native operations fail visibly.

Examples of required native configuration include Leaf LEAFPACK keys, PAK1
compression version, NPA filename/data/permutation keys, NPK2 AES keys, CPZ
plugin constants plus secret aliases, Kaguya image-span key policies, GLib
permutation mappings, PBGZ/THA1 decryptor maps, and TFPK RSA modulus/name lists.
THBGM accepts a scoped `thbgm.fmt` sibling or explicit definitions path. QLiE
supports explicit basic/external-key ciphers and bounded internal
`pack_keyfile` updates without searching unrelated executables.

Material partial boundaries remain recorded per ID:

- NSA compression1/SPB archive conversion is not wired into this archive path;
  raw and compression2 LZSS work. Keyed whole-archive NSA requires an explicitly
  transformed source. SAR/NSA probes are bounded; larger indexes use explicit
  format selection.
- XP3 supports v1/v2, raw/zlib indexes, native segment ordering and filename
  maps. Chained indexes and unknown flags/chunks fail. Encrypted entries need a
  caller pipeline; no title decryptor catalogue or cxdec code generation is
  included. `adlr` is metadata/key input, not a validated integrity checksum.
- Encrypted Malie LIBU/LIBP header/index/member cipher stages use explicit
  caller pipelines. MBL's `mg_data` layout is not a built-in native decoder.
  DX preserves compressed member payloads as the pinned archive reader does;
  it adds no second DX decompressor.
- Unity Assets covers v14/v15 little-endian object metadata with nonembedded
  type trees. PSB covers image-resource PSB2/3 with PNG/TLG chunks and layer
  placement; encrypted PSB needs caller preprocessing. These are resource
  formats, without Unity or E-mote runtime hosting.
- Leaf A encryption1/2, GXT palettes/nonlinear textures, HG3 `img_jpg` and
  Cyberworks type `a2` retain the pinned reader's explicit unsupported branches.
  RPA accepts a bounded data-only pickle subset and rejects executable or
  unknown opcodes/structures.
- MIO implements PCM16 lossless Huffman and LOT/LOT-MSS synthesis; PCM8, DCT1
  and Nemesis remain unsupported as in the pinned source. HCA uses the shared
  decoder for cipher0/1/56; cipher56 needs a nonzero caller key. External PCM
  parity currently covers cipher0, not a cipher56 title.
- WARC1.70 remains partial: native profile fields, key aliases, region-image
  companions and title-extra transform stages are explicit; YLZ is unimplemented
  in the pinned reader. S25 indexed rows and incremental frame prediction also
  have explicit upstream corpus validation; this is resource capability only.

Entry names retain native resource semantics; mount path canonicalization,
traversal and duplicates are checked by the shared mount API. Extraction-tool
extension guessing is not universally reproduced. Format-specific filenames,
original size declarations, flags, timestamps and sample IDs remain metadata
where relevant. WAVE loop values retain their original native units instead of
being silently converted to a different loop convention.

## Implementation, compilation and behavior evidence

**Implementation.** All 193 resource IDs and 96 image IDs have real parser/codec
paths, checked against native registration and aggregate wiring. The reviewed
partial list above remains part of that claim. Tests use self-authored native
fixtures or read external source fixtures only at runtime. No game/font binary
fixtures or title secrets are added to Git.

**Compilation.** Local checks have successfully compiled the plugin crate and
the engine/runtime `arc-formats` dependency path. The plugin crate forbids unsafe
Rust. There is no GitHub CI or remote build automation added for this work.

**Behavior.** Evidence is kept distinct from inventory membership:

| Validation group | Completed local evidence |
|---|---|
| Existing archive paths | Ordinary malformed/index/segment/key/budget tests and an explicitly invoked upstream corpus test |
| Archive batch D | 7 self-authored tests covering all 11 IDs; explicit corpus test passed over 11 external archives |
| Archive batch E | 20 ordinary tests; explicitly invoked corpus test passed over 13 archives; Unity/PSB are synthetic fixtures |
| Archive batches F/H | 23 ordinary tests plus 2 explicitly invoked corpus tests passed; F includes PAK1 v1/v2, CZ10/PX and H includes ANM/THBGM/PAK1 audio |
| Archive batch G | 8 ordinary tests; explicit corpus test passed over 14 archives; PNAP/ODN/TAC are synthetic fixtures |
| File transforms | 10 ordinary tests; external test passed 21 fixtures across 17 IDs |
| Audio | 12 ordinary tests; external test matched 17 fixtures across 13 IDs, including HCA/MIO PCM samples |
| S25/WARC | 11 ordinary tests passed, including budget/tail regressions and Rust checks of 3 original-C++ table-cipher vectors; explicit S25 corpus passed 5 inputs/6 PNGs |
| Inventory tooling | Python3.11 inventory tests verify manifests, pin/source hashes and real registration linkage |

HG3 comparisons follow the original test's rule: RGB hidden under alpha0 is
ignored; visible pixels match. The image manifests separately retain JPEG/WebP
codec tolerances and unsupported image variants. An ignored external test is
not counted as a pass until explicitly run with the required pinned source.
Fixture parity means comparison to original test-declared bytes/pixels/PCM; it
does not by itself mean the original C++ executable was run. Separately,
`tools/verify_arc_warc_cipher.py` compiled unchanged functions extracted from the
pinned original WARC C++ source and reproduced 3 table-sized essential-cipher
vectors `(0,0)`, `(31,3)` and `(17,7)`. That is a limited native-function
differential, not execution of the complete upstream tool or an entry/title-key
branch. Negative floating-to-unsigned conversion in upstream C++ is not portable;
the Rust path defines truncation through a signed integer followed by wrapping.
No commercial game, console hardware result or full-engine integration acceptance
is inferred.

To reproduce validation locally:

```sh
cargo test -p crysirror-formats-arc
ARC_UNPACKER_SOURCE=/absolute/path/to/complete/pinned/arc_unpacker \
ARC_UNPACKER_ROOT=/absolute/path/to/complete/pinned/arc_unpacker \
ARC_UNPACKER_REFERENCE=/absolute/path/to/complete/pinned/arc_unpacker \
  cargo test -p crysirror-formats-arc -- --ignored --skip external_fc01_keys
python3.11 -m unittest discover -s tests -p test_arc_inventory.py -v
python3.11 tools/inventory_arc_unpacker.py \
  --source-root /absolute/path/to/complete/pinned/arc_unpacker \
  --output crates/crysirror-formats-arc/upstream-formats.json
```

The excluded FC01 external-key test additionally requires explicit host-provided
`ARC_FC01_KEY_101` and `ARC_FC01_KEY_200`; no key table is distributed. The other
15 external test groups were explicitly executed. Reproduce the WARC native
function vectors with `python3.11 tools/verify_arc_warc_cipher.py /absolute/path/to/complete/pinned/arc_unpacker`.

Local logs under ignored `target/verification/` include
`arc-audio-files-g-e-tests.log`, `arc-audio-files-g-e-corpus.log`,
`arc-workspace-check.log`, `arc-feature-check.log`, `arc-i-review-lib-tests.log`,
`arc-i-review-corpus.log` and the individually named image/API validation logs.
The manifests identify evidence references; the inventory tool does not run
Cargo, claim GPU passes, or manufacture missing validation results.
