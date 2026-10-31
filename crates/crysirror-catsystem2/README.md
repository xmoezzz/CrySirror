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
key. The original task ID and virtual-canvas queries preserve native configuration;
byte-string copy preflights every destination and result slot before mutation.
XML resource loading reserves a shared worker before scoped I/O and CPU parsing.
It publishes immutable `crysirror-resource::xml::XmlDocument` resources before
returning logical native handles. XML integer/text queries retain the original
`document` wrapper and default-value policy. Handles are bounded, client-owned
integers, never native pointers. Unsupported parser encodings/entities and
internal faults remain visible; only an actual `NotFound` returns native zero.
Other native calls fail with their original PC and pending arguments.

Config is `{script, font, heap_bytes, executable?, canvas}`. `parse_startup` preserves the original
`SCRIPT/start`, virtual screen size and configured KCS heap. Installed resource
roots and mounts are selected by the host. A `.kcs` suffix alone is insufficient:
the worker validates the 52-byte native header, original zlib extent, static
string offsets and owned-memory bounds before admitting the program.

The implemented VM subset covers typed byte/word/dword operands, signed/float
arithmetic and comparisons, narrow assignments, prefix/postfix increment, compound
assignment, stack operations, jumps, original call/return frames, address/copy
operations, conversion and pending gcalls. Unmapped debug instructions fail with
their native PC. CatScene line decoding is present as a separate bounded
format parser; CST/FES command execution is not implemented.

Reference pins, source hashes, original-module static-analysis hash and exact
IDA functions are in [SOURCE.json](SOURCE.json). No original executable, DLL,
font, script or game content is part of this crate. FelineSystem2 was inspected
as a reference only; its unlicensed code and renderer are not adopted.

Local validation:

```
cargo check --release -p crysirror-catsystem2 --features runtime
cargo test --release -p crysirror-catsystem2
CRYSIRROR_CATSYSTEM2_GAME=/path/to/authorized/data cargo test --release -p crysirror-catsystem2 --features runtime --test owner_game -- --ignored --nocapture
```

The opt-in owner test opens original archives, parses original KCS/CST records
and reports the original configured main script's first pending native call.
It does not establish a rendered title, gameplay, original-save compatibility
or a completed CatSystem2 integration. Local evidence belongs under
`target/verification/catsystem2/`; ignored tests are not counted as passes.


Continuation2026-10-31: native400 imports original LoopPoint metadata through
shared client-scoped AudioLoopCatalog. Its native parser/name policy runs on a
bounded worker; owner publication completes before the original void call is
acknowledged. Source coordinates explicitly retain44.1k reference frames and
open end(-1). The common catalog/checked conversion API is reusable without
CatSystem2 and never changes PlayAsset defaults. Native sound playback remains
unmapped; importing metadata is not audible game support.

Native359 now projects exact5628/72/32/12-byte configuration records from the
reviewed native constructor and startup overlays, then the original gamedef and
client Save/setup.xml sequence. Native resource mode3 prefers configured loose
files before archives. Native789 preserves GAME/flag/max. Shared XML/Storage
own parsing and I/O; only original record layout remains in the adapter.
Diagnostics explicitly state that Windows settings were not imported. Unknown
or malformed XML fails before native acknowledgement; the original permissive
parse-failure fallback and complete settings mutation/persistence are unfinished.
The original interpreter now has executable bounded integer expressions, live
and backup flag banks, parser/command-slot policy and source dirty state.
Native string comparisons, NUL-buffer ABI, memory fill, debug log, numeric
string variables and CSS save-container loading retain original byte/value
policy. Generic byte delimiter splitting lives in `crysirror-text::byte_split`;
CSS compression uses shared Resource transforms and scoped Save I/O.

Reviewed type2 planes own shared transparent Layer groups and Solid payloads.
Native global flat Z is mapped to shared Scene stable floating depth state;
task parents do not become render parents. Original47/48 visibility events
update shared resources with their imperative source semantics. Type6 click
controllers begin genuinely unbound, with no original input registration or
geometry; their later hit registration must use shared UI services.
Sound config938 retains an actual shared XML AST and found predicate; this
representation is not audio playback. Native104/105/848/849 commit real receiver
state into shared InputRouting, which owns priority, coverage, capture, hover,
journal and leased delivery. Native71 preserves the original event-wait yield
and next-owner-tick false result; original keyslot/message callback/result mapping
remains unsupported. Shared explicit Drag is tested, but automatic hardware drag
admission is unfinished.

Type15 image lists begin with the original empty bitmap wrapper and no manager.
Native256 reserves shared workers, parses actual startup page dimensions and
uses shared ImageJobs for the transparent atlas and hidden base-plane resources.
Native260 retains upload XY and262 retains source publication flags; neither
performs rendering. No original compositor or private bitmap storage is hosted.
The latest completed owner test executes20,227 instructions, then preserves
native447 atPC0x1926 with arguments[8,406], textflow. Type34 FES construction
and449 atlas binding retain original state; the player/load remains unsupported. Its true3072x4096 atlas
and1280x720 base image are engine-owned resources. Four click/Ctrl edge probes
verify retained failure receipts, not successful gameplay input. FES execution,
image content/composition, Cat title/GPU/text/audio, original-save restoration
and completed game integration remain unfinished. Source evidence and separate
validation are in the linked audit files in SOURCE.json and STATUS.md.

The runtime feature also runs the authored startup-fence test and the opt-in
canonical Runtime owner regression. The latter mounts all31original INTs into
one Runtime, resolves published images through the common asset store, verifies
client isolation and passes click/Ctrl down/up through Runtime.tick and shared
InputRouting. These probes retain the unsupported original callback and pending
PC; they do not advertise working gameplay controls. Startup source publication
keeps instruction/frame/clock at0 until the first ordinary tick.
