# AliceSoft System 4 native work

`crysirror-system4` reads original AIN v5 and executes a bounded native VM
subset. It is a first-class Rust workspace crate under GPL-2.0-or-later. No
original C engine, renderer, SDL loop, font renderer, mixer or composed screen
is embedded. See SOURCE.json and LICENSE for exact pinned provenance.

The VM retains original byte PCs and i32/f32 bits, heap slot identity, sequence
and reference counts, typed global/local/struct/array pages, two-word variable
references and original function argument/return convention. All global storage
exists before initial values, reference initializers, the original function
named `0`, child-before-parent global constructors and main. Constructor and
destructor calls are resumable continuations. Multidimensional arrays remain
page trees. Unknown types, instructions and native calls fail visibly.

HLL calls retain their PC, stack and heap until typed completion. Capture by
cloning preserves native state, including pending calls and destructor work;
this is not a serialized framework snapshot or original save implementation.
The worker adapter explicitly rejects both serialized snapshot methods.

`worker_adapter::NativeAdapter` implements the real core VM port, but **no
factory is registered**. Shared TaskPool workers own capability reads, envelope
and program parsing, layout validation, initial heap allocation and native
instruction batches. Only the owner completes shared host calls. The thin
bridge currently maps GetTime/Sleep to the shared engine clock, preserving the
Sleep operand and PC until its actual deadline. Config explicitly carries the
native logical canvas and Gothic/Mincho font aliases. SACT2.Init first verifies
published shared font faces, measurement and device-free audio availability,
then atomically creates the opaque-black hidden main Canvas and engine Layers.
Missing canvas/fonts fail without resource commit; same-client repeat Init is
idempotent and cross-client reuse is rejected. Backend readiness is not a claim
that native sprite/image/text/audio HLL functions are complete.

The reviewed 2004 OutputLog DLL obtains an optional *loaded Windows debug
interface*. With no such interface it returns `Create = -1`; that explicit
branch is protected by the original DLL SHA256. This is not a successful log
resource and does not copy xsystem4's private log system. A connected Windows debug interface,
unknown library revisions and unknown exports fail. The next actual Rance7
wall has moved beyond the original fallback `system.Output` (`CALLSYS 6`).
The bridge decodes its CP932 string and appends to the existing client-scoped
engine log, then acknowledges the exact pending call. It leaves the original
string on the native stack; subsequent native instructions release it. The
Engine script log and native Output share one canonical bounded ring.

Actual Rance7 owner validation advances through SACT2.Init and the shared byte
formatter to11,053 original instructions, five canonical log records and the
unchanged unsupported SR_ASSIGN at PC0x384a in menu_metrics_t@0. Main has not
been entered, no factory is registered, and no title/gameplay is claimed.

S_MOD maps native types/refcounts to crysirror_text::format::format_one_bytes.
That reusable device-free kernel replaces one compatible `%d`/`%s` field,
preserves encoded bytes, supports bounded width/zero padding and keeps the
original integer/string precision semantics. `%s` copies its declared byte
length including NUL values; templates containing NUL are explicitly outside
this profile. Unsupported conversions fail before native stack/heap/PC changes.
Native string arguments are released only when a matching `%s` consumed them,
while the original format slot is always released. No private printf kernel
or font/layout implementation is kept in the VM.

Actual release validation and owner bootstrap evidence are recorded separately
in STATUS.md and ignored `target/verification/system4-*` logs. Owner game files,
DLLs and fonts are never included. The native executor remains partial: later
opcodes, complete HLL lifecycle/settings, rendering, UI, text, audio, saves and
desktop bootstrap still need their original semantics and shared API mappings.
