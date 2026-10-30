# ADR 0097: KiriKiri archive protection without JIT, brute force or bulk extraction

Date: 2026-10-30
Status: Proposed (design; nothing below the "today" section is implemented)

## Today

- CXDEC (ADR 0096) and HXV4 recover their parameters statically from the
  user's own EXE/TPM: pattern recognizers over PE code/data, never executing
  game code. Decryption is per entry and on demand through the shared
  `kirikiri/xp3` ArchiveFormat; nothing is unpacked to disk.
- Where recognizers find several parameter sets, a bounded set (at most
  4096 statically found combinations) is checked against the archive's own
  Adler-32 checksums and exactly one must survive. This is disambiguation of
  statically found facts, not a key-space search.
- Not adopted from xp3-brute: Unicorn emulation (`x86_filter.rs`,
  `win32_host.rs`), symbolic execution (`tjs_symexec.rs`) and the brute-force
  modes (`brute.rs`, `solver.rs`).

## Problem

1. Ordinary (non-CXDEC) protections register an arbitrary x86 callback with
   `TVPSetXP3ArchiveExtractionFilter`. xp3-brute runs that callback under
   Unicorn. Unicorn translates guest code to host code at run time (a JIT);
   iOS forbids writable+executable memory, and consoles generally do too.
2. Static recognizers are brittle against new protector builds.
3. Any fallback that searches key space is unacceptable (owner decision,
   2026-10-30): it is slow, unbounded and not deterministic.

## Decision (proposed)

### Tier 1: deterministic interpretation, no host code generation

A pure-Rust 32-bit x86 *interpreter* (decoder: iced-x86, MIT; execution loop
written for CrySirror) executes only code whose provenance is proven
statically: the callback whose address is passed to the
`TVPSetXP3ArchiveExtractionFilter` import, and the module initializers
(DllMain/V2Link) that set up its tables.

- Sandbox: PE image mapped read-only from the user's file, private stack and
  heap, no host pointers. Imports resolve to a fixed shim table (heap, TLS,
  `GetLastError`, `GetModuleHandle`, and similar), modelled on retrowin32's
  shim/loader structure (Apache-2.0; attribution as in xp3-brute's NOTICE).
  Any other import fails visibly.
- Budgets: instruction count per call, memory, and wall time. Exceeding a
  budget fails the archive open; it never retries with guesses.
- Acceptance: the same rule as today. Every sampled protected entry must
  reproduce its stored Adler-32 before a reader is published.
- The interpreter is the same code on every platform, so iOS, Android and
  consoles need no JIT entitlement.

### Tier 2: lifting

Interpreting x86 for every read is slow. After Tier 1 accepts a callback,
its code is lifted once:

- Decoded instructions become a small CrySirror filter IR: 32-bit integer
  ops, rotates, multiply, table loads from snapshotted read-only memory,
  and loops over (offset, buffer, file hash).
- The lift is accepted only if the IR matches the interpreter on all
  validated entries plus randomized blocks (differential test). On any
  divergence the interpreter stays in use and the gap is logged.
- The IR runs in a portable interpreter. Desktops may later compile the IR
  to native code ahead of time.
- Machine code from the game is never executed natively and is never
  written out as host code.
- CXDEC is already handled this way: its runtime-generated per-archive code
  is modelled as data (`cxdec/engine.rs`) and interpreted.

### Derived data and caching

The interpreter's state snapshot (tables after initialization) and the
lifted IR are cached per module SHA-256 in the user's state directory. This
is derived data, never shipped and never placed in the repository. Archives
stay encrypted on disk; entries are decrypted on demand.

### Brute force and the bounded disambiguation

- No brute-force tier, at any priority.
- Once Tier 1 can derive the CXDEC/HXV4 parameters by interpreting the
  generator, the bounded static disambiguation will be removed. An ambiguous
  or unrecognized protection then fails visibly with the evidence found.

## Consequences

- New work: iced-x86 dependency, interpreter, Win32 shim subset, IR, lifter,
  differential tests, cache.
- The game's executable is still only read as data; it is never launched or
  loaded as a native module.
- Engine detection (`crysirror play`) selects the protection by opening
  `data.xp3` with each supported profile. It gains an "interpreted filter"
  profile once Tier 1 lands.

## Refinement: two different intermediate representations (2026-10-30)

Keep archive-filter lifting independent of the TJS language compiler. The
first-class TJS runtime already compiles source through HIR/MIR into verified
TJS bytecode, caches decoded instructions and branch indices, and interprets
that bytecode. This path generates no host instructions. Its native host calls
still need implementation on each target; portable interpretation alone is
not an iOS player or a completed plugin implementation.

For arbitrary extraction callbacks, prefer a small filter-specific IR over
translating x86 into TJS, JavaScript or the graphics shader language:

- Define fixed-width wrapping integer arithmetic, shift masks, guest flags,
  byte order and checked guest addresses explicitly. Guest memory remains
  data; no executable memory, host pointers or arbitrary imports cross this
  boundary. Unsupported x86/SIMD/floating-point operations reject lifting.
- Use a decoded x86 execution plan first, retaining original instruction
  addresses, branches, ABI arguments and instruction budgets. This eliminates
  repeated decoding without claiming a high-level lift. Only validated regions
  become filter IR. Never reorder writes across callback/shim boundaries,
  native exceptions or mutable state.
- Existing CXDEC expressions (`xp3/cxdec/engine.rs`) already model the native
  generator as data. Keep this functioning path until a generic replacement
  has independent parity evidence.
- Specify callback state and access granularity. Seeking/split reads require
  a stateless offset-addressed transform or an explicitly checkpointed stream.
  Test whole reads against unaligned, split and repeated reads; initialized
  tables and mutable per-call memory have distinct ownership.
- Validate control flow, references, buffer bounds and budgets before
  installation. Differential tests, archive checksums and deterministic
  generated blocks supplement validation; finite matching samples do not
  prove equivalence for arbitrary machine code. Reject unverifiable regions
  or retain the bounded reference interpreter. Never publish a reader after
  validation failure.
- Cache identity includes module digest, guest architecture, entry point/ABI,
  initialization inputs, shim version and IR/verifier version. A module digest
  alone cannot identify two differently initialized callbacks. Treat cached IR
  as untrusted data, revalidate it and retain scoped resource authority. Do not
  serialize usable host pointers or executable blobs.

This refinement is a design decision, not an implemented x86 interpreter,
lifter or performance result. Current boots still use static HXV4/CXDEC.
No generic no-JIT fallback or new iOS support is claimed.
