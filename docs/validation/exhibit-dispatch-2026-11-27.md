# ExHibit dispatch and retail playback, 2026-11-27

This inventory counts a native top-level dispatch slot once even when its target function has aliases. The v1 dispatcher has 118 cases and a separate block label (0x2f). The 2011 resident has 13 direct cases plus 211 non-default method slots. Each later resident has 13 direct cases plus 233 non-default methods. The direct cases and method assignments are disjoint in all three v3 builds. The constructors also explicitly rewrite slot 0xda to the default method. The v3 label 0x2f has independent significance to the branch index and is outside the non-default dispatch totals.

These counts describe script-callable VM behavior. Platform calls in the host, Windows imports, and the number of distinct function addresses are separate measures. Top-level IDs below are hexadecimal. Selectors under 0x0b and 0xbf are decimal. The resident table RVAs and complete selector ranges are in the [ABI reference](../../crates/crysirror-exhibit/NATIVE_ABI.md).

## 2011 action routes

| Direct script call (0xbf) | Selectors | Native behavior and checks |
| --- | --- | --- |
| Settings and register operations | 167–168, 200–218, 220–224, 226, 298–304, 306–313, 324–330, 335–336, 339 | Skip settings, parameter getters, normal/system/relative/text banks, individual and global voice settings, mute bits, packed gains, temporary flag save/restore and integer range conversion. The result goes to the specified local register. Readback tests distinguish the result overlay from the normal bank and check adjacent bits, volume limits, invalid character IDs, and signed arithmetic. |
| Local/system flags with native conditions | 152, 154, 171–178, 198–199 | Conditional set/clear or toggle on the original flag bank. Paired getters and subsequent flag operations see the change. |
| Earlier routes | 186–191, 262, 338, 340–342, 346, 348, 360 | Thirteen register/flag operations and the stock form of the UI sound definition (338). The latter has restricted operands. |
| Native return in direct script context | 50, 52–55, 57–67, 70–72 | Nineteen assigned selectors. The 0xbf caller at RVA 0x104930 supplies a null selection view and ignores method returns other than 4098, 4099 and 4100. Each of these methods returns without a state change in this context. Arguments are still evaluated and the result cell is copied back. These entries do not establish working save screens, widgets or selection callbacks. |

Of 232 non-default action selectors, 78 have an operation implemented for at least one form and 19 have a verified direct-script return. The remaining 135 assigned selectors lack an admitted direct-script operation. Separately, all 168 unassigned/default slots in the 400-slot table point to actionDummy (RVA 0xb7d70). Its return and caller writeback also have a test. The action envelope's expression and callback coverage places limits on all these counts.

The script evaluates four numeric parameters before invoking a method, with the final string occupying the remainder of the record. The 2011 expression evaluator at RVA 0xfe330 uses a stack for arithmetic. Scalar sources include R/S/L/F/C, BR register-bit reads, and recursive MG/MS/MM/MV experience queries. Eight of the eighteen expression functions at RVA 0xdc560 have scoped evaluators (modulo, distance squared, 8-way direction, radius, clip, empty/nonempty text and cursor flag). Unsupported Q-state and other recognized built-ins remain explicit errors. State tests use the normal script parser, with live local registers, CP932 text, results, division by zero and an unsupported argument that precedes a write. Native functions for audio status, layer size/position, live screen state and random selection need their owners.

The role-specific volume limit comes from the character count plus three in RetouchParam::create (RVA 0xa9270), passed into the character manager at RVA 0x1438d5. The independent control slots are S[43+2*i] and S[44+2*i]. Global mode uses S30 bit 19 and the upper half of S34. The same behavior is read by actions 211/215 and changed by 327. Stereo control volume (336) follows the clamp at RVA 0x5fef0.

## Checkpoint boundaries

The 2011 direct entry reaches a taken 0x05 in start.rld at +0x114. The original launcher also performs a checkpoint at def.rld+0x155 before the handoff to start. Native blockEntry (RVA 0x108d20) serializes live state. R7–R9 read a viewport and zoom, T3 reads active layer clippers, and T5 serializes eight camera slots with their range lists. The T205 rows for never-drawn actors do not have established initial x/y values. A positioned portrait, the six entered-character slots and the printer socket are independent owners. Substituting zeroes in these fields changes subsequent state and restore behavior.

Additional evidence is in the [2011 census](../../crates/crysirror-exhibit/MIDORI_2011_CENSUS.md) and [early census](../../crates/crysirror-exhibit/V1_RETAIL_CENSUS.md).

## Installed operand coverage

The scanner visits each source record independently. A record counts as admitted when the profile parser produces a typed action. A record may still need resources, a matching live owner or a supported checkpoint when the VM executes it. The rows below list IDs with all observed encodings admitted, IDs with mixed results, and IDs with no observed encoding admitted. Absence from a game script cannot establish that an original dispatcher slot was empty.

| Installed corpus | All observed encodings | Some observed encodings | No observed encoding |
| --- | --- | --- | --- |
| Famisupi, 357 scripts, 11,639 of 57,255 records admitted | 36 IDs: 05–06 08 0d 13–14 16–17 29–2c 2e–3a 42 46 48–49 5d 65 69–6a 71–72 8c | 16 IDs: 0b 11–12 4a 4e 51–52 5b–5c 5e–5f 64 75–77 8b | 39 IDs: 0c 15 1c 1e 20–21 23–28 47 4c 4f–50 53–58 5a 61–62 70 73–74 79 88 93–9b |
| Midori, 379 scripts, 8,833 of 51,797 records admitted | 25 IDs: 06 0d 12 16–17 2c 2e–31 33–35 3a 3d–3e 46–49 6d bd f9–fb | 13 IDs: 05 08 0b 14 32 38–39 4a 50 a7 bf fc–fd | 59 IDs: 0c 11 13 15 1c 1e 20–21 23–27 36–37 3b–3c 40 42 4c 4e 51–52 57–58 5a–5f 61 64–66 69–6a 70–75 84 88 93–94 a0 a2 cb–cf ef f2–f3 f8 ff |

Famisupi has 16 of 23 native services with a scoped route. Of 18 selectors in the installed corpus, 12 accept every observed call, one (42) accepts 13 of 137, and five (2, 3, 24, 25 and 32) reject all 57 calls. In total, 245 of 426 service records are admitted. Service 35 prepares an effect on either lane without playing it. The 13 admitted calls to 42 install gallery movie definitions. The other 124 uses include other gallery categories.

Midori uses seven service selectors. Its records for 30, 57 and 60 are admitted (4, 1 and 2 calls). Its records for 25, 27, 28 and 51 are rejected (1, 1, 1 and 10 calls). Of the 13 action IDs in its scripts, records for 190, 262, 338, 346, 348 and 360 are admitted (373 calls). Calls to 151, 164, 166, 250, 265, 345 and 350 remain unsupported (15). Services such as 33, 39, 47 and 48 also have state tests independent of these script frequencies.

## Later definitions and ordinary scenarios

Each late resident exposes 246 non-default top-level IDs, 51 direct service selectors and 296 non-default actions. Both residents return immediately for 41 in-range default top-level slots. Profiles dispatch independently. Service 30 writes the scenario entry name. Action 342 updates the printer's ruby fade bit. Tests run both original entry-name records and the ruby record from each installed definition against their state owners.

| Original definition | Top-level IDs and record counts (hex ID × decimal count) | Nested selectors (decimal) |
| --- | --- | --- |
| Volume 1, 76 records | 05×15, 08×1, 0b×2, 11×1, 2c×1, 3d×2, 40×13, 42×8, 47×1, 49×4, bf×20, dd×1, fb×3, 100×4 | Service 30×2. Actions 117, 118, 250, 269, 271, 288, 342 and 349 once each, 338×5 and 286×7. |
| Volume 2, 68 records | 05×15, 08×1, 0b×2, 11×1, 2c×1, 2e×1, 3d×2, 40×13, 42×8, 47×1, 49×4, bf×11, dd×1, fb×3, 100×4 | Service 30×2. Actions 117, 118, 250, 269, 271, 288, 342 and 349 once each, 338×3. |

The installed sc00 export in each v3 profile starts the definition program before it initializes controls and hands off to the ordinary entry. The launch detector selects this bootstrap by the native exports. Explicit manifest entries retain their own starting script.

The late initializer resolves the main executable module, then tries bitmap 152 and RCG 152 on that module. Both resource entries are absent in both installed executables. The native fallback constructs a red 32×32 picture. Its blue channel produces a zero initialization word, so the ordinary-script transformation at this stage is disabled. Each stock `start.rld` declares 25 records. On the path established so far, the first ordinary opcode at +0x114 is 0x7132 in Volume 1 and 0x9dc5 in Volume 2. Both exceed the native table's 0x11e upper bound. Of the 65 and 76 RLD files respectively, only the empty exit script passes the ordinary untransformed record walk to EOF. The separately initialized definition programs are covered above. The source of valid ordinary script records remains unresolved, so neither volume has a whole-game operand-coverage denominator.

## Numeric and input behavior in the later residents

Seven of the 246 non-default IDs in each later build have a scoped dispatch route: 06, 07, 08, 0b, bf, f9 and fa. The two nested families contribute one selector each (30 and 342). 07 performs register arithmetic. The paired f9/fa operations handle a new input edge and skip waits in an active skip block. The 41 native default methods remain separate from this count.

The later 06/08 implementations use the separate Vol. 1/Vol. 2 `liteSetReg` methods at RVAs 0x1d4520/0x1d4aa0. Their CSV form supports integer and register sources, a numeric comparison, arithmetic and bit operations. On 06, field six bit zero mirrors a local result into the global register. On 08, field three bit eight triggers native system-register persistence independently of that mirror. That form fails before a write because the required persistent state owner is absent. Legacy binary immediate forms have a separate decoder. Compound expressions and legacy register RHS operands remain open. The behavior fixtures exercise both profiles and check register values after true/false conditions, a local/global mirror, a relative system write, an invalid divisor and a rejected flush.

## Release playback through the installed-directory entry point

Built the desktop `crysirror` binary with `cargo build --locked --offline --release -p crysirror-desktop --bin crysirror`. The crate-wide release test ran 110 tests, all passing, including installed-data tests for all four games. The original executables and DLLs were inspected as data. All four game sessions below used the same CrySirror binary, Metal on an Apple M4 Max, separate QA data directories, no window and muted audio.

| Installed game | Actual playback result |
| --- | --- |
| Famisupi | A 1,500-frame run without input reached the visible interactive title and waited for selection. The START track pressed primary at (420,475) on frame 1601 and released it on 1605. It selected item 10, entered F00A1101_2.rld, opened message area 5 and its local controls, and stopped on the native autosave (0x0c) at +0x1b4 before the first dialogue. |
| Midori no Umi | The original definition entry began. The first taken checkpoint at def.rld+0x155 (0x05, block −1) requires the outstanding view, layer and printer state. No title frame was presented. |
| Stray Sheep Volume 1 | Stopped at def.rld+0x114, 0xbf/271, before a title frame or ordinary script. |
| Stray Sheep Volume 2 | Independently stopped at def.rld+0x114, 0xbf/271, before a title frame or ordinary script. |

The late action controls a signed nesting count for display updates. While display updates are suspended, layer state continues to change. On resume the renderer must publish the accumulated dirty rectangle. The current engine layer API immediately modifies the composed scene. Both residents use the signed viewport field at +0x8bf74. Modes 1 and 0 decrement and increment it. On the −1 to 0 transition, Vol. 1 consumes and submits the accumulated rectangle via RVAs 0x251890, 0x28c850 and 0x250f00. The original field's constructor value has not been established. The adapter's background, actor and movie owners publish layers independently, and the existing layer capture API only reads pixels. A whole-frame snapshot or a queued script command cannot reproduce these transitions.

Playback command: `CRYSIRROR_OFFSCREEN_PACING_MS=3 ./target/release/crysirror play "$game" --headless --no-audio --data-dir "$fresh_qa_dir" --frames 900 --capture "$fresh_qa_dir/frame.png"`. The Famisupi START variant uses 3,000 frames and the input track described above. Failed sessions produce a diagnostic JSON and a nonzero exit code. Only the successful title-only run wrote a screenshot. All files containing game visuals remain local.

## Autosave re-entry and late integer tables

Famisupi `0x0c` compares the current block to global R3. Re-entering the
same block after load clears global R3 to −1 without opening storage.
The targeted VM test verifies this behavior even when the LocalHolder
override contains a different number. On the write branch the instruction
stays pending, preserving its offset and both registers.

The Famisupi slot writer enters `saveEngine` at resident RVA 0x3e580,
copies character state and emits the engine payload at 0xa5930. Virtual
calls at FCScenario offsets +0x18 and +0x24 lead to the installed
`ExHIBIT.exe` bank writers (RVA 0x2e190 for `FCReg`, 0x2ddd0 for
`FCStrReg`). Each writes its name as a length-prefixed NUL-terminated
string, then a DWORD count. The normal and system banks then write
that many little-endian DWORDs. The string bank emits the same length
format for each entry. The slot follows the two banks with FCScenario
strings at +0x40d4 and +0x40c8 in that order, and the audio trailer at
resident RVA 0xb9260 contains two DWORD flags and a playback path. The
paired reader at 0xa5990 restages more state and stops at the audio loader
0xb92c0. The stock installation contains a save directory note and no native
slot fixture. Ownership of the two auxiliary strings, the audio trailer
and complete restoration still need verification before emitting slots.

Vol. 1 and Vol. 2 use independent INTDB handlers at 0x162df0/0x162ec0
(define), 0x17dca0/0x17dd70 (read) and 0x18c1e0/0x18c330 (write).
Both store eight tables with signed keys and eight columns per row.
Definitions replace their table. Reads leave destinations unchanged on a
missing row. Writes stop at the first negative column and do not create
missing rows. Both installed definitions provide two 43-row inverse
tables; the installed test verifies 38 → 36 and 36 → 38. The synthetic
VM test covers replacement, duplicate keys, invalid columns, signed
arithmetic values and live register operands. Lookups and writes accept
scalar late-resident expressions; complex expressions remain guarded.

These routes bring each later resident to ten of 246 non-default opcode
IDs with at least one established form. Service and action coverage is
unchanged. The two late definition tables count as verified independent
effects even though the normal startup is held earlier by action 271.

The final desktop release build and all 112 crate tests passed with the
four installed-data probes enabled. The same binary was replayed with
fresh QA save directories. Famisupi accepted START and opened the first
chapter's message backdrop before the autosave write error at +0x1b4
(first failure at frame 1842). Midori stopped at `def.rld+0x155`
with the live-state checkpoint error. Vol. 1 and Vol. 2 independently
stopped at `def.rld+0x114` with action 271. The three v3 failures
occurred before the first frame was presented.

## Famisupi autosave write path

The v1 dispatcher handles both comparison outcomes at 0x0c. A write holds
the script record until the original FCReg and FCStrReg banks, memo index
and class-named system bank are written to the client's save mount.
Success clears global R3 and advances S7. A short or malformed index
fails before writing, and a failed file replacement rolls back files
already replaced during that attempt. The files use per-file atomic
replacement with the index published last.

The Famisupi launcher passes `[setting] CLASS` to FCScenario in the
virtual call at resident RVA 0x505f9. The original launcher appends
`.system.dat` at EXE RVA 0x2eb3b. The two auxiliary FCScenario strings
start empty in the original EXE constructor and remain empty along
the supported fresh-start path. The legacy audio trailer is separate
from ordinary Ogg and WAV scene cues, which are stored in normal
registers. The only resident caller of the legacy MCI play routine
0xb9120 is its loader. Pack the DOS date and time from UTC FILETIME
fields, as the original path passes FILETIME through SYSTEMTIME
without the local-file-time conversion.

The writer adds all 41 observed 0x0c records to the admitted Famisupi
parser inventory. The updated static totals are 11,680 of 57,255 records,
with 37 of 91 sampled IDs admitting all observed forms, 16 admitting
some and 38 admitting none. The native ordinary slot-to-scene restore
and presentation of the load panel are separate work.

The desktop release replay used the installed Famisupi directory and a
fresh, isolated QA data directory. A primary click on START entered
`F00A1101_2.rld`, committed the autosave at +0x1b4 and stopped at first
dialogue (+0x1eb, 0x1c) on frame 1846. The saved slot contains 2,048
normal registers and 256 CP932 string cells. Independent parsing checked
block 1, the native scenario path, 25 index entries and 4,096 system
registers. Reopening the same QA directory and choosing START again
created `user/a1`, preserved `user/a0`, and advanced S7 to 2.

The same build stopped at `def.rld+0x155` on Midori's 0x05 checkpoint
and at `def.rld+0x114` on action 271 in both Stray Sheep volumes.

## Famisupi ordinary dialogue and remaining dispatch coverage

The area-5 path now links CP932 body/name layout, character voice eligibility
and preparation, read-history bits, the rolling text history, and hit-wait.
Its fixtures check retained glyphs and marker cells, no advance on a held
key, marker removal on a new key edge, and the voice channel's stop request.
The ordinary path is described in the [printer validation](exhibit-opening-printer.md).

| Profile | Registered opcode IDs | Installed records admitted by parser | Installed opcode IDs, all / some / no sampled forms admitted |
| --- | ---: | ---: | ---: |
| Famisupi | 118 + 1 label | 42,142 / 57,255 | 38 / 16 / 37 |
| Midori no Umi | 224 + 1 separate label | 8,833 / 51,797 | 25 / 13 / 59 |
| Stray Sheep Vol. 1 | 246 | Definition file only | 10 registered IDs have at least one implemented form |
| Stray Sheep Vol. 2 | 246 | Definition file only | 10 registered IDs have at least one implemented form |

All 30,462 installed Famisupi `0x1c` records admit the same seven-word plain
encoding. This is a parsing result. Live execution still requires the
matching ordinary area-5 printer and advance state. The counts for nested
calls are unchanged: Famisupi has 16/23 service IDs with a routed form;
Midori has 10/49 services, with 78/232 actions plus 19 verified direct-script
returns; each late resident has 1/51 services and 1/296 actions with a
routed form. Each late resident also has 41 verified default opcode slots
outside its 246 assigned IDs. Later ordinary scripts still lack a confirmed
corpus denominator.

The desktop release ran all four original installations using normal
`crysirror play` detection, `--headless --no-audio`, and separate QA save
directories. The release linked and all 116 ExHibit tests passed, including
the installed-data probes. Both 3,500-frame Famisupi runs accepted START and wrote `a0`.
Without further input the first line stayed waiting at `F00A1101_2.rld+0x23f`.
Two Enter presses on separate frames progressed to line 3 at `+0x2fe`, with
three history entries and the wait still active. The two 800 × 600 captures
differ at 9,013 pixels in the first two text rows. Device audio was disabled.
The three v3 replays still stopped before presenting their first frame:
Midori at `def.rld+0x155` (0x05), and both later profiles at `def.rld+0x114`.

The two late action 271 handlers (RVAs 0x13a290 and 0x13a360) read a signed
presentation lock counter at screen-object offset 0x8bf74. Argument `-1`
toggles based on its sign, `1` decrements the counter, `0` increments it,
and `-2` repeatedly increments a negative counter until it reaches zero.
The `-1` to zero increment triggers a refresh in each resident
(0x251890/0x251e20). The ordinary update path also tests the sign after
recording its dirty rectangle. The stock `271,-1,1,0,0,0,*` at `def.rld+0x114`
is a stateful presentation operation. It still needs a mapping that keeps
the displayed frame stable while pending visual state changes.

## Later screen-update lock and packed debug definition

Action 271 now captures an immutable frame through the shared prefix-image
presenter on the first lock. The adapter waits for the scoped image before
committing the PC or native result register. Nested locks reuse the same
image while live layers continue changing. The final release reveals the
live state, then retires the image through the host. A paired-resident VM
fixture changes the live pixel under the frozen one and checks both colors,
asynchronous PC retention, single capture across nested locks, a forced
release, an ignored mode, a toggle and the outer command's zero writeback.

The next `0x47` record at `+0x129` uses the packed mode shared with the
2011 resident (0x168700/0x1687d0 in Vol. 1/2). Zero low mode bits prevent
the original overlay path from running; the original still copies all eight
ordered mapping pairs. Both installed definitions retain eight `[-1,0]`
pairs. This path covers the authored disabled overlay mode.

The desktop release linked and all 117 ExHibit tests passed with the four
installed-data probes enabled. The same release ran both installed later
definitions through `0xbf` selector 271 and `0x47`, with the presentation
counter at `-1`. Both now
stop at the taken `0x05` at `def.rld+0x16a`. Midori still stops at its own
checkpoint at `def.rld+0x155`. The paired late block writers (0x159a30 and
0x159b00) capture viewport, layer, camera, printer and actor owners before
flushing registers. They additionally write T210 from the late scene state
at object offset 0x1414 (serializer 0x269930/0x269ec0). Midori's earlier
block writer does not write T210. In the same release Famisupi again reached
the third dialogue line with two separate Enter presses. All four replays
used fresh isolated save directories, no window and disabled audio output.

Each later profile now has admitted execution forms for 11/246 top-level
IDs, 1/51 service selectors, and 2/296 actions. There are still 235 top-level
IDs with no route, 50 service selectors with no route and 294 action
selectors with no route in each volume. This inventory is independent of
the definition replay and the unresolved ordinary scripts.

## V3 checkpoints, dialog definitions and notification slots

The 2011 block writer (0x108d20) and later block writers (0x159a30 and
0x159b00) commit a detached native register image. The VM uses the live
printer, background, portrait, local-position and entry-name owners where
those operations are admitted. It also preserves register-write ordering
across the LocalHolder flush and fills the experience span described by
S16. The two late residents add R81 and T210 and write nine sound lanes,
while the 2011 layout has five. An active layer serializer request fails
before the script position or checkpoint is committed. Unused raw portrait
coordinates are zeroed on never-drawn rows because the native constructor
leaves them unwritten and the inactive-row reader does not place a sprite.
This path captures the in-memory checkpoint. It does not write or restore a
v3 save file.

The v3 `0x42` tests check packed button count and default button, both
selection overloads, authored item positions, and the changed late layer
aliases. The `0x100` tests check independent owned slots, replacement,
empty resource strings, a comma in the final string and unsigned
out-of-range indices. The two installed definition probes also dispatch
all four notification definitions. These definition methods do not open
dialogs or display notifications.

The desktop release ran from the normal installed-directory launcher with
headless output, muted audio and independent save directories. All 120
ExHibit release tests passed, including the four installed-data probes.
Local logs are under `target/exhibit-proof/block-sol/` in
`desktop-notifications.log`, `tests-final-verified.log` and `qa/`.

| Installed profile | Latest run |
| --- | --- |
| Famisupi | START and two Enter edges reached dialogue line 3 in `F00A1101_2.rld+0x2fe`. The autosave remained present and the 800 × 600 capture was written. |
| Midori no Umi | Committed block 7, installed seven dialog definitions, and stopped before a title frame at `def.rld+0x724`, local-control command `0x40`. |
| Stray Sheep Vol. 1 | Committed block 6, registered eight dialogs and notification slots 10–13 at (772, 1028), and stopped in the prelude at `def.rld+0x78a`, `0xbf` selector 288. |
| Stray Sheep Vol. 2 | Independently confirmed the same block, definition counts, positions and stopping point. |

The original action 288 handlers (0x13cb50 and 0x13cc20) initialize
favorite-voice settings and the saved voice catalog in mode 0. Other modes
use that catalog for playback, registration or list operations. The
remaining 2011 `0x40` records use the 91-field layout. The later
definitions use 105 fields and include a different local-control layout.
Both variants bind resources, native key codes and subordinate controls.

The latest installed-script census admits 8,840/51,797 Midori records
(26 all observed forms, 13 mixed, 58 with no admitted form across 97 used
IDs). The Famisupi count remains 42,142/57,255 records (38 all, 16 mixed,
37 none across 91 used IDs). Both later residents have execution paths for
14/246 top-level IDs, 1/51 service selectors and 2/296 action selectors.
The remaining 232 top-level IDs, 50 services and 294 actions in each later
resident have no execution path. Those counts exclude verified native
default table slots and do not imply parity of all operand variants
within admitted IDs.

## Local controls, draw slots and scoped precache

The `0x40` RLD local-control definition path now owns the authored
background and atlas positions, layer and action IDs, informational
image, button settings, keyboard codes and property items. The 2011
records have seven property rows and the later records have nine.
Replacing a definition overwrites that slot, including removed entries.
The original installed records use the deferred form. The VM fixtures
verify retained state after a replacement and exact-PC rejection for an
immediate instantiation. Composite atlas construction and live control
activation are outside this tested path.

Both late residents' installed-data probes dispatch all 13 `0x40`
definitions along with the `0x2c` notification bundle and three
`0xfb` portrait pairs. The bundle adds slot 1 alongside slots 10–13 and
uses the same owner as the direct notification opcode. The pairs are
(144, 145), (146, 147), and (148, 149). The paired-resident fixture
additionally checks slot 7 and that the original out-of-range index does
not overwrite it. The v3 `0x2d` parser routes the file path and MiB limit
to the existing read worker. Its shared reader's separate test checks
exact reads across the native 8 MiB boundary.

The 2011 dispatcher now preserves 42 original empty method slots,
excluding its branch-index label. The new test walks a reserved record
with operands, then executes the following record. Empty slots are
outside the non-default count.

Midori now admits 8,847/51,797 installed records. Among its 97 used IDs,
27 admit all observed forms, 13 have mixed forms, and 57 have no admitted
forms. Famisupi has the same 38 / 16 / 37 result as before. Both later
residents now have a path for 18/246 assigned top-level IDs. The other
228 top-level IDs and the previously reported nested coverage remain
open; this census does not establish support for all operand variants.

The desktop release linked and the ExHibit suite passed 123/123,
including four installed-data probes. The headless, muted installed
replay reaches Famisupi's third dialogue line at
`F00A1101_2.rld+0x2fe` and writes `a0`. Midori passes all seven
`0x40` definitions and commits
block 9. Its next error is the `Q3` operand of `0x14` at
`def.rld+0x120c`. Both Stray Sheep plays still stop on `0xbf/288` at
`+0x78a`, before their later `0x2c`, `0x40` and `0xfb` definitions,
which are exercised in the installed-data probes instead. Local logs
are `opcodes-final-desktop.log`, `opcodes-final-tests.log` and the
latest replay logs under `target/exhibit-proof/block-sol/qa/`.

The workspace Python suite reported ten errors in packaging, VFX schema
and NumPy-dependent tests. The log is `opcodes-python.log`.

## Printer, scenario transfer and late branch execution

The v1 printer now handles timed body/name clears (`0x1e`), immediate
and timed frame close (`0x12`), and changing an already open frame
(`0x48`). The transition keeps the native backdrop level for reopening,
uses the selected area's font size and row pitch independently, and
preserves the distinct local and global R77 values. The wrapped printer
uses the original line-start restriction and centers the last segment.
The looping DLT decoder has its own bounded worker. The old single
shared task slot failed when a decoded image and the next voice both
requested it at the fourth dialogue line.

The `0x4e` prologue fade also uses the real printer/toolbar owners.
The 800 ms black cover and music steps follow their separate native clocks.
At the native 0x800 entry hook, the adapter clears and closes the
printer and leaves the active local-control scope. A step fixture checks
the still-open frame at 799 ms, hiding it at 800 ms and replacing the
scene with a black CG. The existing title fade remains at 1,200 ms.

The 2011 expression evaluator now exposes `Q3` using the original Win32
version table. A portable runtime with no configured Win32 tuple returns
the original unknown-version value of −1. The string database handlers
(`0xd0–0xd2`) implement the native signed-key ordering, field extraction
and register updates. The scenario transfer opcode (`0x11`) accepts
literal, conditional, text-slot and database-selected successors, and
can exit. It validates a successor and writes the native system-bank file
before installing the new script. Fixture cases verify all four modes,
register and bank contents, and preservation on a failed load. The
active layer capture still requires real layer descriptors. The native
empty-layer case clears T208 while preserving T209 and is covered by
the checkpoint fixture.

The late residents now execute `0x14`, `0x16` and `0x17` for ordinary
conditional jumps, calls and returns. The paired fixtures assert native
block resolution, sequential scalar and text writes, return condition
bypass, and restoration of the caller block and instruction index.
Complex condition functions and special callback frames still lack a
route. Original non-default favorite-voice action 288 uses the RNF
settings and catalog on initialization, menu activation, playback and
updates. Its two installed calls still stop the definition program.

The parser census now admits 42,230/57,255 Famisupi records (40 IDs with
all sampled forms, 15 mixed, 36 with none) and 9,257/51,797 Midori
records (29 all, 12 mixed, 56 with none). The `0xd0–0xd2` handlers have
behavior fixtures and no top-level occurrences in the installed Midori
scripts. Each late resident has an execution path for 21 of 246
non-default top-level IDs, one of 51 services and two of 296 actions.
The remaining counts are 225 top-level IDs, 50 services and 294 actions
per volume. The later ordinary-script transform is still unresolved,
so these counts come from the registered dispatcher.

The desktop release linked and the ExHibit suite passed 137/137,
including the installed-data probes. Playback used the same binary,
muted headless mode and separate QA save directories.

| Installation | Observed playback |
| --- | --- |
| Famisupi | START wrote the autosave. Playback presented 16 dialogue entries through ordinal 17, including the 75 px area-6 text, played through the opening movie path, completed the printer-closing fade and installed `F11A1101.rld`. It stopped at that file's first record at `+0x114`, a signed solid-color form of `0x4a`. |
| Midori no Umi | Passed the `Q3` branch and the empty-layer checkpoint. It stopped at settings action `0xbf/250` in `def.rld+0x20c8`. The native system-bank transfer was exercised in the fixture and lies beyond this stock stop. |
| Stray Sheep Vol. 1 | Stopped at favorite-voice action `0xbf/288` in `def.rld+0x78a`. |
| Stray Sheep Vol. 2 | Independently stopped at the same favorite-voice call. |

The local build and test logs are `batch-2-closure-desktop.log` and
`batch-2-closure-tests.log`. The replay logs are in
`target/exhibit-proof/block-sol/qa/` under `famisupi-yl388gjc`,
`midori-re19bp06`, `vol1-gxejh8ub` and `vol2-zr119xt2`.

## First chapter drawing, system sound definitions and later string tables

Famisupi `0x4a` now handles a zero-rectangle signed solid color through
the same scoped layer as an ordinary CG, using native RGB order and no CG
experience increment. It supports the 16-step mode-9 wait and the
zero-duration mode-255 replacement. The existing 1:1 crop uses the actual
GYU dimensions. The draw checkpoint keeps the authored source rectangle,
flags and scale sentinels. The VM fixture asserts the emitted color,
transition steps, retained registers and the resume time.

The first-chapter `0x5e/0x5f` digitized CG billboards use the existing
layer owner and track the native 16-level channel-8 opacity. The two
800 × 130 cards use the GYU alpha plane. A shared Runtime fixture
asserts placement, dwell times, both fade directions and the CG
experience update after entry completes. The ordinary framed printer
places the left-aligned body and centered speaker on distinct slots
using their own font and rectangle. In the `0x52` release path, script
mode 0 waits for natural pan completion while mode 1 freezes at the
current rectangle. The fixture asserts the retained UVs, the VM
instruction pointer during the wait and the separate one-tick resume.

The Midori `0xbe` and `0xea` handlers share the 100-slot sound table
with the existing control sound action. Their VM fixture exercises
list updates, explicit disabled slots, filename retention through reset,
default candidate labels and ordered de-duplication of candidate IDs.
These are registered methods with no direct use in the surveyed scripts.
The RNF settings action 250 still needs its nested modal, font and
live-audio behavior.

Both late residents now execute `0xd0–0xd2` for ordinary string-table
creation, reads and writes. The paired fixture verifies computed keys,
CP932 text, numeric conversion, readback after update, duplicate-key
replacement and clearing a table independently. This adds three
top-level execution routes per resident. Action 288 remains unsupported
and needs the favorite-voice catalog, selection UI and persistent updates.

The consolidated release suite passes 144/144, including the four
installed-data probes. The Famisupi scanner now admits 43,573 of 57,255
records, up by 1,343. Of the 91 observed IDs, 41 admit all sampled
encodings, 14 have mixed results and 36 still have no admitted form.
Midori remains at 9,257 of 51,797 records because neither of its two
new sound-table IDs occurs in the corpus. Each late resident now has
an execution path for 24 of its 246 registered top-level commands.
The other 222 still lack a route. Many admitted IDs retain operand
forms outside the implementation, and these counts do not measure
menu or full-game behavior.

The desktop release linked. In muted headless playback with an isolated
save directory, Famisupi displays the two chapter cards, reaches block 1
of `F11A1101.rld`, writes autosaves `a0` and `a1`, and reaches dialogue
ordinal 48 with the framed area 0 active. It executes the `0x52` stop at
`+0x591` and reaches the following `0x4a` at `+0x5bd`. The next CG
requests a 400 × 300 crop at (0,840) from a 1600 × 1200 source. The
current path rejects it because the source window needs scaling to the
800 × 600 viewport. The new Midori and late handlers have VM fixture
coverage. Their installed playback still reaches `def.rld+0x20c8`,
`0xbf/250`, and `def.rld+0x78a`, `0xbf/288`, respectively.

Build, test and Famisupi logs are under
`target/exhibit-proof/v1-drawing-family-1127/`, in `final-desktop.log`,
`final-test.log` and `chapter-cfuz_2y8/play.log`. The other playback
logs are under `target/exhibit-proof/block-sol/qa/` in
`midori-v62dkqez`, `vol1-ieviy28q` and `vol2-9wq1zquh`.

## Scaled CGs, transparent DLT and live audio selections

Famisupi's `0x4a` now runs the 0x8c580 crop-and-scale branch on the
shared image worker. The new effect uses the original 255-sum fixed-point
weights and in-source successor taps. The VM retains the previous frame
and its instruction position until publication, then waits for the
mode-9 transition before updating the checkpoint and CG experience.
Both the shared pixel test and the Runtime fixture passed. A scan of
the installed RLDs and GYU headers found 80 matching plain mode-9
crop records, including `F11A1101.rld+0x5bd`. These records were already
admitted by the parser. [Sampling evidence](exhibit-opening-cg.md#first-chapter-crop-and-scale).

The v1 `0x76/0x77` path now carries the authored CG ID, layer and
position to the DLT owner. Plain `0x75` exits use the original opacity
steps and slot retirement. The existing shared decoder now separates
the 0x0802/0x0803 color and LZSS mask deltas. For the stock 400 × 400
at (200,50) clip, the Rust RGBA output matches the independent decode
for all 25 frames. Eleven resource tests include five installed clips
across the four games. The adapter fixtures verify the retained layer
and resource retirement. The ten stock
`0x75` wait-for-end records paired with `0x74` remain outside the
end-to-end execution path.

Midori actions `315` and `316` now update the authored music and
ambience volume registers and the active PCM16 lanes when the second
argument is 1. Tests verify independent lanes, zero-gain muting,
negative-current-gain preservation and T204 capture. These are direct
script actions. The stock RNF settings loader remains responsible for
interpreting its separate `@` directives and reading their inputs.

The two later residents now route sound table opcodes `0xbe/0xea` to
their 100-slot native state and support ordinary `0x0a` text register
assignments, append/prepend, replacement and encoding-dependent clear.
The variant-specific tests verify both versions. Text modes 4–9 still
require the original NLS conversions.

The release ExHibit suite passed 148/148. The Famisupi parser admits
43,659 of 57,255 installed records. Of 91 observed IDs, 44 admit all
sampled encodings, 11 have mixed results and 36 have no admitted form.
Midori's installed count stays 9,257 of 51,797 because the two new
actions are not called directly by the installed RLDs. Each later
resident now has at least one execution route for 27 of 246 registered
top-level IDs, leaving 219 with no route.

The desktop release build passed. Muted, headless playback used the
installed directories and fresh QA data roots for all four games.

| Installation | Latest release playback |
| --- | --- |
| Famisupi | Passed the scaled CG and transparent DLT start and exit. Reached dialogue ordinal 52 and wrote autosaves `a0` and `a1`. Stopped at frame 11,155 on `F11A1101.rld+0x7ef`, where `0x50` requests a 500 ms viewport move. |
| Midori no Umi | Stopped on settings action `0xbf/250` at `def.rld+0x20c8`. |
| Stray Sheep Vol. 1 | Stopped on `0xbf/288` at `def.rld+0x78a`. |
| Stray Sheep Vol. 2 | Stopped independently at the same selector and offset. |

The release build, ExHibit tests and Famisupi replay logs are under
`target/exhibit-proof/scaled-draw-1127/`. The other three playback logs
are under `target/exhibit-proof/block-sol/qa/` in `midori-12mpeb8u`,
`vol1-fdcr4rpd` and `vol2-tbz9_hmq`. The headless command returned an
error at each stopping point and did not write a frame capture.
