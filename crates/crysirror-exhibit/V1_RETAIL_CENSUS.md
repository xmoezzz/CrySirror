# Famisupi v1 retail compatibility census

This is a static inventory for one pinned retail resident and its loose RLDs,
not whole-engine coverage. The resident was read as data. No game or vendor
binary was run for this census. No scenario text or assets are exported.

- Resident SHA-256: `dc1add582bf76754e5f38d6ab47e69f2921e4a900c244062c72149e6a519d808`.
- Ordered RLD manifest SHA-256: `09fb1784ea40b8220dcd5edcc37f2fd7cfcfbf2b37eea48da711e94d7a2901ba`.
  For each file in bytewise filename order, hash the UTF-8 basename, NUL and
  binary SHA-256 of the complete file.
- 357 RLDs, all code origins at 0x114, 57,255 records. Total 4,179,871 bytes:
  98,532 bytes before the code offsets and 4,081,339 code bytes (229,020
  instruction-header bytes + 1,459,716 numeric operand bytes + 2,392,603
  string bytes, including NUL terminators). Every file ended exactly at
  the declared record count: 0 leftovers. Prefix accounting does not
  establish the meaning of every prefix byte. This is the v1 offset rule;
  the 2011 v3 +4 adjustment and v3 action IDs are separate.

## VM opcodes and modes

This is the baseline recorded after the scoped scene-fade work and before
the printer-frame follow-up. Later parser additions can change admission
counts without changing the resident/corpus denominator.

The native `liteExec` switch has 118 labels plus the 0x2f block marker
from `blockId2Index`. The corpus exercises 91 of those 119 IDs and 102
distinct `(opcode, flags, numeric arity, string arity)` shapes. Known
operation-specific numeric discriminators split these into 263 signatures:
68 parse all occurrences, 7 mix parser acceptance/rejection, and 188
admit none. A question mark in the detailed inventory intentionally
means the data-dependent selector is unresolved or not included in the
current analyzer, not a proof that the wire shape is a complete mode.

| Status by observed opcode | IDs | Interpretation |
| --- | ---: | --- |
| All observed records reach a dispatch arm at parse time | 25 | Runtime parity not certified |
| Partial parser acceptance | 12 | More forms or preconditions remain |
| No records reach a dispatch arm | 54 | Missing or unmatched; not implemented |
| Unseen in the corpus | 28 | Resident switch evidence only; no execution claim |

In total, 10,151 records parse and 47,104 reject. A state or resource guard
can still reject an admitted record. No handler inferred from its symbol,
placeholder, parser arm or silent skip counts as complete. The
[ABI reference](NATIVE_ABI.md) records the implemented narrow forms
and the independently observed title milestones.

| Semantic family | Observed IDs | Parser admits | Parser rejects | Entirely missing IDs |
| --- | ---: | ---: | ---: | ---: |
| VM control, registers, checkpoints | 11 | 4,491 | 242 | 1 |
| Printer and interaction | 15 | 0 | 31,640 | 15 |
| Character, slots and select input | 14 | 236 | 5,403 | 10 |
| Voice and audio | 10 | 2,599 | 0 | 0 |
| Scene drawing, clips and camera | 22 | 2,659 | 8,511 | 12 |
| Movies, animation, raster, parallax | 18 | 100 | 948 | 16 |
| Nested services and debug | 1 | 66 | 360 | 0 |

The first consolidation dependency is the authored definition bootstrap and
printer/control state; the opening relies on both before the first dialogue.
Then close scenario checkpoint/transfer and opening animation, followed by
characters/selection, render channels, and the remaining audio/movie
behaviors. A native case name is not enough to infer an implementation.

## VM-to-engine calls (0x0b)

The primary `cmdExHIBIT` switch has 23 call IDs. The corpus uses 18 of those
(426 records, 21 `(primary, declared arity, nested discriminator)` tuples);
7 observed IDs admit all their observed calls at parse time, and 11 observed
IDs admit none. Status is not an engine-API or OS-syscall inventory. Primary
31 branches recursively but is unseen. Primary 42 is observed in nested modes
0, 1, 3, 4. Its native modes 2 and 5 are not exercised in this corpus. Primary
39's observed selector 2 targets scene experience. Calls must be validated
for side effects and waits as well as header matches: 26 creates a timed
layer, 27/28 change skip/auto state, 29 updates selection controls via an
image worker, 37 obtains a slot result asynchronously, and 38/39 change
native register or experience state. Supported declarations are listed in
the ABI reference; slot results, native saves, original modal workflows,
work-surface lifecycles and renderer/audio receipts are distinct obligations.

## Object/UI actions

Object-level callbacks are not additional top-level opcode IDs. The
observed incomplete systems include message-area definition/activation,
printer and advance/hide state, selection group/control lifetime, item
input and ID-to-branch results, character/layer binding, animation
transport and render resource ownership. An enabled START mouse branch
and its menu teardown were measured; that does not certify keyboard,
hover, skip/auto, native save/restore or first dialogue.

## Reproduce the static pass

```sh
CRYSIRROR_EXHIBIT_CENSUS_ROOT=/path/to/local/installation \
  cargo test --offline --locked --release -p crysirror-exhibit \
  static_v1_corpus_census -- --ignored --nocapture
```

The opt-in test independently walks declared record envelopes, checks EOF,
and calls only the script decoder on each record. Private detailed results
are `record-forms.tsv`, `resident-dispatch.tsv`, `parser-mode-tuples.tsv`
and `engine-calls.tsv` in the local evidence directory under
`target/exhibit-proof/v1-retail-census.*`. Simulator/engine tests are
separate and remain deferred until the implementation families are ready.


## Static follow-up: printer backdrop

The same corpus and byte totals now give 271 signatures after adding 0x12,
0x48 and 0x49 discriminators. Of these, 78 admit all records at parse time,
7 are mixed, and 186 reject all. Overall 10,704 records parse and 46,551
reject; 27 opcode IDs have all observed records admitted, 13 are mixed, and
51 have no admitted observed records. The 553 new admissions are 493 printer
opens, 46 frame selections and 14 area definitions. The pinned def.rld
projection returns areas 0 through 6 without copying strings or assets into
this repository. Eleven observed primary engine calls remain unsupported.

The next independent authored object family is v1 local controls: 0x2c appears
20 times in def.rld across a 9-field sound definition, a 25-field modal/menu
definition, and eight 33-field image part definitions, each duplicated. Native
0x13 operates the nested local-control scope and transitions eight native
slots (61–68), not just a skip/auto boolean. The opening then requires
checkpoint/auto-save and authored animation before its first printer text.
Neither a printed line nor overall startup parity has been measured in this
follow-up. V3 or later sample action inventories are separate.


## Static follow-up: v1 local controls

The installed corpus projects to ten branch-identical toolbar definitions
for twenty `0x2c` records: eight image parts, one sound array and one modal
background array. All 515 observed `0x13` scope records parse. The same
57,255 records now produce 274 signatures: 83 all-admitted, 7 mixed and
184 all rejected. At opcode granularity 29 IDs admit all records, 13 are
mixed and 49 admit none. Total is 11,239 admitted and 46,016 rejected.
The refreshed private table is `parser-mode-tuples-post-toolbar.tsv`.

The added runtime portion covers authored 3/6-cell image preparation,
nested scope/fade, the native half-open slot hit and auto/skip preference
actions in the admitted select wait. It does not silently forward the
six unsupported toolbar actions as ordinary script choices. Sound and modal
metadata are projected but their remaining callbacks are separate work.
First-line prerequisites still include checkpoint/auto-save, opening
animation, and actual printer text/advance. No title or gameplay was
launched during this follow-up.


The v1 in-memory block checkpoint now captures the two native R4 bits
before flushing locals: bit 0 is printer-open; bit 1 is local-control scope
depth at or below zero. That change does not add parser records. Later
0x0c remains rejected. Native auto-save writes the engine payload, then
notifies the file-time/memo slot database and advances/flushed S7 in the
ring. Complete scene/character payload restoration and the native format
are not yet available; the existing slot occupancy reader is not a writer.
No user save files were written to validate this.

### Autosave re-entry

At `liteExec` RVA 0x70c10, case 0x0c compares the active block to global
R3 using `UxAdvSystem::reg`. The LocalHolder overlay is ignored. If R3
matches, the resident skips the slot writer and sets global R3 to −1.
The VM now reproduces this branch without calling storage. If the two
values differ, it keeps the record pending and reports that native slot
writing is unsupported. Both branches share the same RLD operand shape,
so the 41 corpus records are excluded from the execution admission total.
The native ring normalizes invalid S7 to zero, supplies T1 for an empty
memo, writes the slot and index, advances S7 modulo the configured count
and flushes system data. The stock install's save-data note identifies
the system companion as `*.system.dat` one directory above `user`.

## Static follow-up: v1 opening DLT

The stock opening uses exact tuples for `0x76` prepare, `0x77`
play/fade-in and `0x75` fade-out/leave. These add parser admission for 2,
31 and 31 records respectively. The runtime also requires the pinned opening
script and channel/layer state; admission elsewhere does not establish
runtime support. The full 57,255-record snapshot has 281 signatures:
83 all-admitted, 10 mixed and 188 rejected. At opcode level 29 IDs admit
all records, 16 are mixed and 46 admit none. Total is 11,303 admitted and
45,952 rejected, with zero trailing bytes in the same 357 files.
No gameplay was run. See [DLT validation](../../docs/validation/exhibit-dlt.md)
for the independent ordinary asset and remaining profile limits.

### Numbered v1 DLT clips

Across the same 357 scripts there are 41 `0x76` preparations in 21 literal
tuples. All use channel zero with one of eight CG numbers, layer 4, 5,
20 or 40 and authored screen coordinates. All 41 `0x77` calls start
at the current position with level 16 over 800 ms (31), level 8 immediately
(7) or level 16 immediately (3). The adapter now decodes all of these
tuples and uses their resource, position, level and duration on a fresh layer.
A call targeting an occupied layer still fails visibly.

Of the 68 `0x75` records, 51 request a nonwaiting channel-zero exit with
0–800 ms fade. Seven address channel one and have native empty-channel
return semantics when that channel is unused. Ten channel-zero records
request a completion wait, including one retained-layer exit. These decode,
but a live completion wait remains unsupported. No v1 script contains
`0x78`; IDA shows it calls the same exit function. See
[the 1180 asset and chapter call sites](../../docs/validation/exhibit-dlt.md).

## Static follow-up: v1 opening printer preflight

A numeric-only scan of the same 357 scripts found 30,462 `0x1c` records, all
with seven words, flags 2 and two strings. Exactly one matches the first
opening tuple, at `F00A1101_2.rld+0x1eb`. Bytes and opcode admission
are unchanged. The scoped CP932 and area-5 glyph helper is tested with
generated Japanese text. All `0x1c` records remain rejected until
character voice, retained history/experience and native input wait are
wired together. No gameplay was run.

## Static follow-up: v1 ordinary character definitions

All 32 `0x30` records are in the ordinary `defChara` prefix, with IDs
3–34, a bounded body-variant count and a literal voice flag. They all
reach the typed adapter; the runtime rejects duplicates and execution
outside that definition script. Of the same 57,255 records, 11,335
are now admitted and 45,920 rejected. At ID granularity 30 admit all
observed records, 16 mixed and 45 none. 0x31 and 0x1c remain unsupported.
No gameplay was run.

## Static follow-up: v1 ordinary body definitions

All 105 `0x31` records occur in `defChara`. Each has 213 fields, the default
body mode without hair/accessory overlays, and a canonical 24-entry face
table. The active counts sum to 485 faces. The native `setFace` path for the
observed null sequence and mode zero uses eye-sequence table index 0 and
the default mouth sequence without moving the rotating eye index. The
parsed descriptor retains each active face, the original mode words,
signed body correction and face origin. Runtime installation checks that
the `0x30` actor exists and the definition fits an unused variant slot.

Static records now admit 11,440 of 57,255 (45,815 rejected); at ID level,
31 admit all, 16 mixed and 44 admit none. This is a definition, not an
onstage drawing, actor-entry, motion, lip-sync or dialogue implementation.
Tests use invented descriptors and in-memory host services; no stock
actor textures, game sessions or saves were used.


## 2026-11-26 view and control services

Current static pass: 11,602 of 57,255 records admitted, 45,653 rejected.
Of 281 signatures, 93 are fully admitted, ten mixed and 178 wholly
rejected. Of the 91 used IDs, 35 admit all sampled operands, 16 are
mixed and 40 admit none. The four view definitions (29/2a/2b/2e hex)
each occur twice. Native commands 30/36/40/41 (decimal) account for
4/2/144/4 records. V1 has 23 native command selectors, 12 with a
scoped implementation; 11 of the 18 sampled selectors admit records
(220 of 426). Counts describe parsing. The independent parent-pixel,
state and hit-target tests are documented in the
[validation note](../../docs/validation/exhibit-native-services-2026-11-26.md).

## 2026-11-27 dialog definitions, entry reloads, cache hints and gallery media

The 357 installed RLDs still yield 57,255 records. The decoder now accepts
11,639 and rejects 45,616. For the 91 sampled IDs, 36 accept every sampled
record, 16 accept some and 39 accept none. The 281 operand signatures split
96/10/175 on the same basis. Twelve newly admitted records are `0x42`
definitions from `def.rld`, starting at +0xa62. Native `defDialog` (RVA
0x6a860) appends the ID, flags, background and selection images, and up to
three ordered button descriptors. Repeated IDs remain in registration order.
Displaying an authored modal goes through the separate `0x41` command.

The resident also implements `0x2d` at 0x67960. It reads the asset named in
the first FCString, optionally limiting the prefix in MiB. Native reads
above 8 MiB use fixed 8 MiB chunks even when the requested prefix ends
inside a chunk. The adapter uses the client asset reader on a worker and
resumes the VM after completion or a cache miss. The sample has no `0x2d`.

Native services 0 and 1 (decimal) load entry-name slot 1 or 0 respectively.
`liteLoadTitle` (0x5f040) and `liteLoadExit` (0x5f0e0) resume the current
`liteExec` loop at its already advanced **record index**, retaining the
caller stack, current block and local values. A successful v1 `liteLoad`
sets T2 to the logical RLD filename at 0x5f8a3. Overridden entry names
preserve the original CP932 bytes in T2, including multibyte characters
with a backslash-valued trail byte. The ordinary script-transfer
opcode has different frame semantics. Services 27/28 ignore declared arity,
37 reads the destination directly from the remaining string, and 29/33/38/39
use the native argument count and integer conversion. The experience-list
operation skips empty tokens and ignores unknown banks.

Service 35 prepares a silent sound effect in lane 14 or 15. Original
0x694a0 calls the buffer initializer (0xa54b0 / 0xbb110) and omits
the transport-start call used by normal effect playback. The worker
prepares the new resource, replacing the previous source on just that
lane. The corpus uses it 12 times, e.g. `F21D1101.rld+0x229d`.

Service 42, subtype 3, plays gallery movies. Native `liteDrawMovie`
(0x60670) consumes dotted numeric groups of four, calling numbered
`enterMovie` (0x3c2f0) and blocking `leaveMovie` (0x3c450). It marks
CG experience, starts the optional independent WAV with the effect
volume settings, and advances after the one-second startup guard and
movie completion. The 13 calls run from `CGMode.rld+0xc982` to +0xcf3f.
The adapter's behavioral fixtures cover two successive movies, a negative
movie ID, an incomplete group, effect-resource replacement and its silent
preparation. Other gallery subtypes use different native handlers.

Sixteen of the 23 native service selectors have an implementation for at
least one form. Services 0, 1 and 33 are absent from the sampled scripts.
Of the 18 exercised selectors, 12 admit all sampled calls, 42 admits
one of its four sampled subtypes, and five admit none (2, 3, 24, 25,
32). Altogether 245 of 426 nested calls are admitted. Unobserved
selectors 31 and 34 also lack execution handlers. The callable service
total and static parser totals do not measure gameplay completion.

The stock START route enters `F00A1101_2.rld`. After the scene checkpoint
at +0x190 it requests `0x0c` at +0x1b4, before the DLT at +0x1c3 and first
line at +0x1eb. Native `liteExec` compares the current block with R3, calls
`autoSave` when they differ, then clears R3 to −1. The write at 0x3e580
collects character register state (0x294d0), writes `user/aN` through
0xa5930 and calls the slot database notifier (0x31ce0) with the resulting
file time and memo. The slot image includes two register arrays, two
FCStrings and the state writer at 0xb9260. The writer separately flushes
`system.dat`. The index stores the packed DOS date/time (0x31670) and a
length-prefixed CP932 memo; `autoSave` finally advances S7 and flushes
the system register bank again. This installation supplies no native slot
to compare. The adapter writes this save family on the fresh-start route.
Original slot-to-scene restore remains open. The following ordinary dialogue
path accepts the installed `0x1c` encoding and has been exercised through
three lines. See [the updated dispatch and replay counts](../../docs/validation/exhibit-dispatch-2026-11-27.md).

## Famisupi printer clear and frame close

The V1 resident dispatches `0x1e` at RVA 0x7198a to
`RetouchPrintManager::clear` (0x2d700). The corpus has 28 calls. Nineteen
request `[1,400]`, eight request `[0,400]` and one requests `[1,0]`.
Representative sites are `F23B2A11_2.rld+0x4a65`,
`F25B1101.rld+0x550` and `F14B1101_2.rld+0x30e`, respectively.
Body slot 52 and the optional independent name slot 54 are set up at
0x755c0/0x76220. Instant clear erases the body and conditionally the name.
Timed clear replaces fade channel 5, fades both configured text surfaces,
waits, erases both and restores their levels to 16 (0x2d7e0/0x2d840).
Clearing an unattached printer returns immediately. A bound blank page
still consumes its authored fade. The frame, hit-wait marker and history
are independent of this command.

The force-close direction of `0x12` occurs 22 times, in addition to 493
opens. The resident at 0x7146b takes the duration from word 1 only when
exactly two words are present. Closing an open frame first clears both
text surfaces. An immediate close hides background slot 51. A timed close
replaces fade channel 6, fades the background to zero, hides it, and restores
its authored level ready for reopening (0x75a20). Twelve sample calls close
immediately and ten supply 300–800 ms. The release fixtures assert the
separate body/name/background updates, a 400 ms fade at 200, 375 and 376 ms,
the hidden frame's restored opacity and the record arity rule.

All 46 installed `0x48` records use the single-word frame form. At
`F00A1101_2.rld+0x8c5` the original changes an already open area 5 to
area 6. The dispatcher at 0x72127 closes the previous frame, binds the new
body/name rectangles, writes the selected ID into the LocalHolder overlay
at R77, and reopens the frame. Selecting the same open area returns before
the R77 write. The fixture checks hide/clear/bind/reopen order, the new
area ID, the untouched global R77 and no side effects on active re-selection.
Area 6 supplies a 75 px font and a separate 30 px line pitch. Native
`setProperty` (0x2c6a0) applies these fields independently. The text
loop (0xaba50) reserves four pixels for fitting, retreats at a prohibited
line-start character using the original CP932 table, and centers the
final piece. Intermediate wrapped pieces start at the left edge. The
ordinary inline printer tests assert the corresponding glyph positions
and marker cursor, including when the font is taller than the line pitch.

## F11A1101 chapter cards

After the opening transfers to `F11A1101.rld`, numeric CG billboards
1226 and 1227 appear at +0x158 and +0x1e4. Both installed GYUs are
800 × 130 with a separate alpha plane. The cards occupy slots 4 and 5
respectively, each at (0, 192). Each fades in over 400 ms, remains for the
script's explicit 2,000 or 5,000 ms wait, then fades out over 800 ms at
+0x1d8 or +0x264. The first checkpoint in the new file is at +0x334.

The numbered overload (RVA 0x36640) resolves a CG asset, passes its
bitmap to 0x36720, waits for mode 9 on channel 8, then marks CG experience.
The source bitmap supplies its own dimensions at 0x89730 regardless of
the width/height fields used by other source types. On retirement, 0x36900
waits for the fade and releases the descriptor (0x80cd0). Elapsed time to
zero is quantized by 0xd7a0. On the 800 ms fade from 16, 750 ms is level 1
and 751 ms is the first zero. The unkeyed blitter (0x94ce0) and the keyed
blitters (0x958d0, 0x94c00) use independent alpha whenever present.

All 158 sampled numeric `0x5e` records fit the source contract. The 127
that specify index 0 all name indexed GYUs with separate alpha. The other
31 use the unkeyed sentinel −2. All 909 literal-slot `0x5f` records now
decode, including 104 immediate −1 removals and 41 zero-duration removals.
A live ordinary billboard can be retired after its controls detach. Timed
removal still diagnoses a positive duration on an untracked empty slot
whose starting level is unknown. The release VM fixture supplies synthetic
GYUs and exercises both timed cards through the retained scene, including
the zero/full/intermediate opacity, real bitmap dimensions, independent
experience writes, both hold durations and removal. This test does not
advance the complete original chapter.
