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
