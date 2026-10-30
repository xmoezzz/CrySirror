# CatSystem2 native groundwork

This first-class workspace crate reads original KCS v2 bytecode. It is not a
host for CatSystem2 or another engine. The VM owns only bytecode, numeric
values/addresses, call frames, native pending calls and native configuration.
Archive decryption is the shared `crysirror-formats-arc` `cat-system/int`
plugin. Image decoding remains that plugin's existing HG3 support.

The engine adapter has a bounded, cancellable source job through
`crysirror-tasks`. Its immutable capability-scoped result is published by the
owner before execution. Factories remain dormant and do no I/O or create
workers. Mapped gcall 393 changes the original production menu item through shared UI
state and acknowledges its void return only after successful submission. The
debug-state query requires a reviewed module ABI and a confirmed absent debug
key. Other native calls fail with their original PC and pending arguments.

Config is `{script, font, heap_bytes, executable?, canvas}`. `parse_startup` preserves the original
`SCRIPT/start`, virtual screen size and configured KCS heap. Installed resource
roots and mounts are selected by the host. A `.kcs` suffix alone is insufficient:
the worker validates the 52-byte native header, original zlib extent, static
string offsets and owned-memory bounds before admitting the program.

The implemented VM subset covers typed byte/word/dword operands, signed/float
arithmetic and comparisons, assignments, stack operations, jumps, original
call/return frames, address/copy operations, conversion and pending gcalls.
Unmapped compound assignment, increment/decrement and debug instructions fail
with their native PC. CatScene line decoding is present as a separate bounded
format parser; CST/FES command execution is not implemented.

Reference pins, source hashes, original-module static-analysis hash and exact
IDA functions are in [SOURCE.json](SOURCE.json). No original executable, DLL,
font, script or game content is part of this crate. FelineSystem2 was inspected
as a reference only; its unlicensed code and renderer are not adopted.

Local validation:

```
cargo check --release -p crysirror-catsystem2 --features runtime
cargo test --release -p crysirror-catsystem2
CRYSIRROR_CATSYSTEM2_GAME=/path/to/authorized/data cargo test --release -p crysirror-catsystem2 --test owner_game -- --ignored --nocapture
```

The opt-in owner test opens original archives, parses original KCS/CST records
and reports the original configured main script's first pending native call.
It does not establish a rendered title, gameplay, original-save compatibility
or a completed CatSystem2 integration. Local evidence belongs under
`target/verification/catsystem2/`; ignored tests are not counted as passes.
