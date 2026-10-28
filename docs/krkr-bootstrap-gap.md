# KiriKiri protected bootstrap: evidence and next boundary

**Update 2026-10-28:** `Plugins.linkZ` is now implemented for the validated HXV4
bootstrap profile (see STATUS.md, same date); game-kr proceeds into its KAG system
scripts. The research record below is kept for provenance; the endianness/slash
spelling of pathHash it inherited from xp3-brute was wrong and is corrected in
`crysirror_resource::compound`.

The existing shared Hxv4 reader, JSON facade and bounded solid vector drawing
facade are separate capabilities. Their implementation and recorded checks are
not evidence that the protected bootstrap or the whole game runs. In the current
`game-kr` probe the original EXE resource startup reaches `Plugins.linkZ` after
`Storages.setDefaultPath`; no rendered game frame was established. Probe evidence
is local/ignored: `target/verification/game-kr/embedded-container-probe.log`.

## Source provenance and limits

Primary research repository:
[hktkqj/cxdec-hxv4-static-analysis](https://github.com/hktkqj/cxdec-hxv4-static-analysis/tree/3c6dee001df011fa7ed557c7463635c6f67698f6),
pinned at `3c6dee001df011fa7ed557c7463635c6f67698f6`. The local checkout is ignored
under `target/research/cxdec-hxv4-static-analysis`.

This repository contains analysis, annotated/decompiled sample startup, static
parsers and tools that call native DLLs. It does **not** supply the complete native
`V2Link`/`RegisterSystemStorageHooks` implementation. Native tools were inspected
as evidence and were not executed. Sample startup is evidence of call sites, not
an authoritative implementation of all hook behavior, and is not copied here.
No public key, derived key, game DLL or game script bytes belong in this record.

Files relevant to this analysis, with SHA-256 of complete files at that pin:

| File | SHA-256 |
| --- | --- |
| `docs/core/Reverse.md` | `4fa918537828f97f0a45220faf21bab4c63315b76f779e55e710e25450b4b7dc` |
| `docs/core/FileSignatureCheck.md` | `6dfade8a64ec0eda3c3f8fb074fedf776f3225cb43c24a397316b2bdb4dcc704` |
| `docs/core/hxv4/02-bootstrap-kdf.md` | `f9f501c1032e8cf8c934afce0430eace82bd1dafe63013750aa9b5460441d2e0` |
| `docs/core/hxv4/03-resource-hash.md` | `ce9adebc63a75e22dabedfb7c84c16675a520933c285493231f8c8fa7d16890e` |
| `docs/core/hxv4/07-filter-manager-runtime.md` | `a418d6ae8f92328f9ac0cbbdcdf819ff7af85f427c0f22cd397c89fd8591cf1b` |
| `docs/core/hxv4/08-runtime-read-path.md` | `bd3f746223e7360b057b7f5b1bea84a345f282c7e4861b797f6fa0e388b352af` |
| `docs/diff/DllDiff.md` | `11d95873506ea9701a69238a2c13b95ae0026eba9c419f96768e230c17a631c9` |
| `docs/static/DeriveFilterManager_Static.md` | `a0b73a50d6d2e33a046a8b03c173239f77a51acdb82cf8d2a61269d82e61323a` |
| `tools/FilterManagerDerive/Program.cs` | `231211137f5a84753416dc964dd1b3cb87a7ec99e9f8617a816b027f7ce5ff8b` |
| `sample/startup.tjs` | `ca73849cbd79e5179d25254a9e4fc5cca046d9247fe7398a1f5801230dbe8105` |
| `src/static_extract/bres_bootstrap.py` | `7b8083625dd4cb37970458c294ade66e61f3bf5823aa9ea223b23caa69b0c390` |
| `src/common/resource_hash.py` | `577134b9ce6ff686569177ccff3b74f7fea803fbefbc75f82903f00cbe6aa195` |
| `src/common/xp3_inspect.py` | `ed8cb240648e1f2ca11522b5fd483888cc665a7e1e6aa29c684c8ecf502a2fdf` |
| `src/dynamic_capture/capture_bootstrap_args.py` | `8275a6bcb7a3d4679290e64e92792f689083a93d723667f9c2cdd2ded869f3d1` |

## Observable protocol and unresolved details

| Operation | Evidence at the analysis pin | Boundary |
| --- | --- | --- |
| `Plugins.linkZ(storage)` | `Reverse.md:77–87`: read a compressed PE, decompress, load it and call its `V2Link(exporter)` | This is a registration/state transition. Exact native argument-count and return-value contract is not established by this source. CrySirror must identify the supported bootstrap and install real semantic operations without loading native code. |
| `System.bootStrap(prefix, autoPathCallback)` | `02-bootstrap-kdf.md:10–29`; registration at `FileSignatureCheck.md:64–86` | The native name is `bootStrap`; `_bootStrap` is a script wrapper. Prefix is combined with embedded `WARNING`, with `PARAMS` participating in derivation. A fourth octet argument of at least eight bytes selects a custom archive seed (`02:441`). Third-argument semantics and callback lifetime/order remain unresolved. |
| Bootstrap return | `02:447–456`; `Program.cs:299–316` | Returns a 32-byte Octet. It is distinct from the archive encryption key and cannot be replaced by an arbitrary Octet or Void. |
| `System.checkSignature(file[, signatureStorage])` | `FileSignatureCheck.md:88–153,155–194,207–343` | Bootstrap initializes a verifier from its embedded key. Absent/empty second argument selects `file + ".sig"`; result is Boolean. Missing verifier is a native failure, not success. Verification is independent of archive AEAD. |
| `Storages.archiveUniqueKey` | `DllDiff.md:24,37`; `sample/startup.tjs:150–161` | Binary and string hooks affect filter-manager state. Sample code reads this property, later sets it and deletes it. Exact get/set/delete behavior and return values are not fully established; a plain ignored string property is insufficient. |
| `Storages.CompoundStorageMedia("arc", mediaName, hashKeyOctet)` | `03-resource-hash.md:113–183`; `sample/startup.tjs:119–145` | Owns media/hash/domain state. Sample media name is derived from the bootstrap prefix or defaults to `xp3hnp`; each target must establish its actual name/profile. |
| `pathHash(path)` / `fileHash(name)` | `03:157–183,185–282,284–324` | Input is UTF-16LE input followed by UTF-16LE media name. The analyzed build has zero effective hash-key length despite copying the supplied Octet. Do not generalize that observation to every build. |
| `parseArchiveIndex(path)` | `sample/startup.tjs:23–109` | Returns an Array according to the call site; exact native element layout is missing. |
| `mount(path,index)`, `entryDomain(id,index)`, `assignDomain(id,target)`, `assignFiles(id,pathHash,target,targetHash)`, `clearDomain(id)` | `sample/startup.tjs:23–109` | Calls establish and mutate archive/domain routing. `mount` result is used as an integer domain index. Complete native return, error, remapping and unmount semantics are unresolved. |
| `Storages.mountBRes(path, flag)` | `sample/startup.tjs:167–196`; `FileSignatureCheck.md:364–379` | Flag-one call yields a domain used in `bres://<domain>/<name>/bin`. The corresponding flag-zero transition must be established, including lifetimes; it is not an always-success resource alias. |

`RegisterSystemStorageHooks` is identified at RVA `0xFAE0`, `System_bootStrap`
callback at RVA `0xEEB0`, and `checkSignature` callback at RVA `0xF3E0` for the
analyzed DLL. These are sample-relative research identifiers, not portable API
addresses or proof of identity for `game-kr`.

### Candidate bootstrap-return reconstruction

The C# tool's `DeriveHashKey` checks manager readiness bits and calls the native
derivation function with output length 32, input `manager + 0x3040`, input length
64 and seed `-1`. Comparing its manager field layout identifies the candidate
input as `key32 || nonce1_full32`, before the optional archive-specific derive
step. The existing shared Rust bootstrap computes that full nonce locally but
retains only its first 24 bytes in the returned recovery state.

This gives a concrete reconstruction to implement and verify, not a verified
`game-kr` return value. There is no native return-value oracle for the local game
in this investigation. Truncating then padding the nonce, returning the archive
key, or relying on the analyzed build's zero effective lookup-key length would
not preserve this API's semantics.

### Signature primitive

The documented signature wrapper contains Base64-encoded RSA-PSS signatures,
using SHA-256, MGF1 with SHA-256 and a 32-byte salt. The embedded key uses PKCS#1
RSA public-key DER despite a `PUBLIC KEY` PEM label in the analyzed sample.
Modulus/signature size must be taken from the actual validated key; the sample's
size is not a universal constant. Real target bytes and the selected signature
storage must be read through the client's scoped resources. A signature mismatch
returns false; unavailable/malformed inputs must follow the established failure
contract. Successfully authenticating Hxv4 does not verify these file signatures.

## Differences from the validated XP3 recovery family

Current recovery uses
[xmoezzz/xp3-brute](https://github.com/xmoezzz/xp3-brute/tree/c82cb14302138125640478556723705aa720007b)
at `c82cb14302138125640478556723705aa720007b`. Relevant complete-file hashes:

| File | SHA-256 |
| --- | --- |
| `src/krkr_exe.rs` | `aa29da8749a1a2e1a2deb7f8bdedac83a7291c02e3bada37a0fba7131fbe307e` |
| `src/hxv4_native.rs` | `bd81a4c9436dbe0c2a5846a38c2aec38fc69ac37596c619849db47163c3dc9f8` |
| `src/hxv4.rs` | `20f2bd5d7850de13d0fb041a25551e8365db71aa1b79f1e17a984c091e28f167` |

The analysis pin is not a drop-in replacement for that recovery source:

- `02-bootstrap-kdf.md:157–364` describes the core derive as a SHA3-512 multistream
  construction. The pinned Rust recovery identifies the bundled function as
  Argon2i with a Keccak-derived salt. Do not replace the authenticated local-game
  recovery using the older interpretation.
- `Reverse.md:317` prescribes descriptor flags for `open_flag`. The pinned Rust
  recovery explicitly uses each entry's local flag in `hxv4_native.rs:192–204`.
- `Reverse.md:318` suggests a startup-based `entry_base = 1`. Current recovery
  uses validated explicit joins and canonical entry identifiers. Do not introduce
  heuristic index shifts into the shared reader.
- Both resources describe related bootstrap/Hxv4 families. Successful Special
  authentication and resource Adler verification establish the reader's local
  behavior; they do not establish every native callback ABI or storage mutation.

## Concrete next implementation profile

The next phase can add these independently reusable engine capabilities, then
adapt the proven native signatures:

1. **Validated bootstrap context.** A client-scoped, immutable context binds the
   source PE/BRes identities, approved bootstrap family, parameter-table identity,
   media name, archive mount generation and verified recovery state. Parse and
   derive on bounded resource workers; retain no native code/device pointer.
   Unknown family or mismatched context fails before native methods are installed.
2. **Shared RSA-PSS/SHA-256 verifier.** Parse the established key/wrapper profile,
   stream scoped resource bytes on workers and return actual verification results.
   Test valid, modified, malformed, missing and wrong-key cases independently of
   TJS. Bind verifier initialization/lifetime to that bootstrap context.
3. **Shared compound storage.** Own media registration, stable logical domain
   handles, mount generations, archive/index ownership and explicit domain/file
   routing mutations in the engine. An initial profile can target one verified
   root archive and empty initial domain, provided its actual native layouts,
   callback order and mutation/return contracts are first established. Reads and
   existence queries route through shared authenticated archive/VFS services.
   Unsupported secondary archives, custom keyed-hash layouts or unknown mappings
   fail visibly rather than returning fabricated domain indexes/empty Arrays.
4. **Shared scoped PE resource media.** Provide read-only BRes domains with real
   mount/unmount ownership and stable handles. Native wrappers convert storage
   syntax and values; they do not own PE parsing or decoding algorithms.
5. **Thin protected-bootstrap facade.** Only after those semantics are proven,
   `linkZ` may transactionally install the recognized profile. `bootStrap` must
   validate its arguments/context, return the genuine Octet and retain/call the
   script callback with the original VM continuation semantics. Duplicate calls
   require established idempotency rules or an explicit unsupported-state error.

Open evidence required before calling that facade complete: exact `linkZ`
parameter/return and unregister behavior; `bootStrap`'s third argument and callback
protocol; local-game hashKey oracle; `archiveUniqueKey` hook get/set/delete rules;
archive-index Array layout; CompoundStorage domain mutation/error/return behavior;
and BRes flag-zero transition. Collect authoritative native source or independently
checked disassembly/call traces for the actual supported family. The annotated
sample and a matching resource decryption alone do not close these gaps.

## Validation record

- Research: primary repository pinned, relevant complete-file hashes recorded;
  analysis and native-tool source inspected without invoking DLLs.
- Implementation: this pass adds this document only. No new bootstrap facade,
  signature verifier or compound-storage engine operation is claimed.
- Compilation: no cargo invocation for this research-only pass.
- Behavior: no successful protected-bootstrap or full-game run is claimed. Existing
  reader/JSON/vector validation remains recorded in its own test results.
