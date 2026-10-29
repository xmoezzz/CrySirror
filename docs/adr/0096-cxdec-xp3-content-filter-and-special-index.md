# ADR 0096: CXDEC XP3 content filter and indirect Special filename index

Date: 2026-10-30
Status: Accepted for the bounded static resource profile below

## Source and ownership

The owner's MIT-licensed xp3-brute revision
c82cb14302138125640478556723705aa720007b supplies the reviewed algorithms:
the parameterized CXDEC content core, static PE parameter recognizers, the
bit-permutation self-decoder, embedded `internal module` extraction, the
complemented ChaCha8 Special cipher, `setupArchiveData` SHAKE256/BLAKE2s
derivation and Special name records/tokens. They are adapted into the shared
`kirikiri/xp3` ArchiveFormat (`crates/crysirror-formats-arc/src/xp3/cxdec/`,
NOTICE-MIT there; file hashes in `docs/cxdec-source-audit.json`). Not imported:
Unicorn/x86 initialization snapshots, SteamStub unwrapping, tjs2dec symbolic
execution, CLI, GPU code, published per-title parameter tables or control-block
binaries. No game module code runs; no title value is embedded.

## Resource profile

```json
{"parameters": {"kirikiri/xp3": {"cxdec_modules": ["game.exe", "plugin/game.tpm"]}}}
```

Names are capability-relative companions read through `DecodeContext`
(`open_sibling`), at most 16 modules of 64 MiB each. Static facts per module
(including embedded and statically self-decoded images) are memoized by
SHA-256 in-process. An indirect Special descriptor (16-byte `offset, original,
stored` root record) without this profile, or a profile whose modules show no
generator evidence, fails the archive open.

## Recovery and validation

1. Special index (when present): the protection module that carries the
   `setupArchiveData` generator also names the native startup redirect
   (`archive.xp3>member`). The member must be stored in plaintext (raw bytes
   match `adlr`). Its compiled TJS2 string pool (or source literals) supplies
   candidate texts; a candidate is accepted only if the derived seeds decrypt
   the Special blob to a zlib stream of the declared size whose records parse
   exactly and whose MD5 tokens (prefixed by the descriptor's own tag) map
   every record to a visible entry. Unmapped token-shaped entries fail the
   open; unmapped visible names colliding with a recovered path are dropped
   as decoys. The redirect member is published as `startup.tjs`.
2. Content: candidates come from the module that is coherently tied to the
   generator (dispatch orders, Cabbage seed, Prefix8 words) plus the
   script-derived mask/offset/control table, otherwise from a bounded
   (4096) cross-module combination. Up to 32 (then 128) smallest protected
   entries whose raw bytes do not already match `adlr` must all be restored;
   exactly one behaviourally distinct profile may remain, else the open fails.
3. Every protected entry is checksum-verified again (64 KiB passes, original
   and decoded Adler) before a reader is published; ranged reads apply the
   same transform at logical offsets. Offsets above 32 bits fail.

CrySirror delta to upstream: the Prefix8 wrapper's two XOR words are profile
parameters recovered from immediates or from `.rdata` values a static
`mov r,[src]; mov [global],r` copies into the globals the key expansion reads.
Upstream's fixed `0x55555555/0xaaaaaaaa` pair fails on such modules.

## Validation (kept separate)

- Implementation: synthetic unit tests in `xp3/cxdec` (engine vectors,
  partial reads, Prefix8 constant recovery, ChaCha/derivation/records/tokens,
  TJS2 pool, an end-to-end self-authored PE + XP3 for the generic path and the
  Special naming path, fail-visible cases). No game bytes are fixtures.
- Opt-in real data: `tests/cxdec.rs` (ignored; `CRYSIRROR_CXDEC_GAME`,
  `CRYSIRROR_CXDEC_MODULES`). Results are recorded in STATUS.md only from
  actual runs.
- Boot behaviour is a separate KiriKiri-adapter question; decrypting archives
  does not establish game compatibility.

## Unsupported

Modules needing runtime initialization (Unicorn path upstream), packed
executables, Special layouts other than the 16-byte descriptor, multiple
Special descriptors, and generations whose parameters are not statically
materialized fail visibly.
