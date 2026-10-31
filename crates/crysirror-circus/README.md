# Native Circus MES adapter

First-class bounded parser for the offset2 MES ABI used by DC4 (0xaab6) and
DC4 Plus Harmony (0xabb6). Original operand widths, signed words, shifted
Shift-JIS strings, label metadata and 0x77 continuations remain native data.
Invalid headers, truncated operands and non-boundary PCs fail explicitly.

The 0xabb6 interpreter executes original native instructions through a thin
client-scoped HostServices adapter. `runtime` enables the optional factory
registration. The 0xaab6 profile is parsed but explicitly rejected by execution.
VM state includes native byte banks, system selectors, label/read/savepoint
metadata, string jump labels, branch scans, pending resource/UI declarations
and audio queues. No GPU/device pointer is part of that state.

Reusable CRXG and cursor decoding live in crysirror-image, PCK/DAT archive
indexing and seekable ranges in crysirror-resource, and XPCM PCM/Ogg decoding,
preparation and mixing in crysirror-audio. Retained layers and widgets belong
to CrySirror. Resource workers return immutable results; the VM owner alone
publishes resources/engine commands and advances completion-dependent PCs.

Ordinary native falling effects declare five ordered emitters in one shared
VFX system. The engine owns the existing particle pools, falling/atlas node,
screen mesh publication and worker completion receipts. The adapter retains
only native density/depth/resource declarations and explicit frame policy.
Its process-wide integer random stream is seeded once from virtual engine
milliseconds. Native random instructions borrow that same stream through an
acknowledged exchange before changing VM state. Storm birth/reset differences,
other effect families and concurrent/replaced effect pools still fail visibly.

This is partial native execution, not completed game compatibility. Unmapped
instructions, nonempty native UI font/markup, native group activation,
complex image animation, snapshots and original saves fail explicitly.
Parsing 536 installed scripts or registering a factory does not establish
title/story completion.

`native_save` preserves the complete original 356324-byte slot prefix, opaque
bytes and native read-run metadata. Its bounded patches implement the audited
PC, byte-bank, image-name, civil timestamp and checkpoint sprite fixups. It
deliberately cannot construct an empty slot body from logical VM JSON. The
live native body projection, original slot restoration and actual 128x72 PNG
thumbnail are still required before mandatory autosave can succeed; current
DC4PHDL execution stops explicitly at that operation.

`NativeLiveState` additionally tracks the audited original initialization and
live mutations: nonzero native sprite constructor fields, both byte-bank reset
arrays, original read-history metadata/counters, random history, texture
extents, sprite transforms, placement rows, voice timing and music fields.
This VM metadata is distinct from shared engine rendering resources. Successful
instructions synchronize it; unaudited instructions leave explicit admission
gaps visible in runtime inspection. The real sequence still reaches forced
autosave with these gaps and stops. Original thumbnail drawing selects native
layers and UI explicitly; a generic capture of all UI does not establish the
same result.

Run the actual engine without a window or audio device:

```sh
cargo run --release -p crysirror-circus --features runtime --example headless_boot -- GAME_ROOT PRIVATE_WRITABLE_ROOT
```

`native_trace` is a semantic diagnostic only; it does not render or replace
the engine host. Original game files and fonts are never embedded.

Pinned format reference and licence provenance are in SOURCE.json and LICENSE.
No original engine renderer, mixer, executable or game data is included.
