# ExHibit native dispatch

## Evidence and counting

The unit in this inventory is a distinct 16-bit RLD opcode. A record can also carry numeric operands, NUL-terminated strings, a format nibble and an operation-specific subtype. Counting a handler once does not establish its operand coverage. The retail modules below were read as data in IDA, with their exported symbols and the original x86 dispatch. The interpreter rejects unmatched operations and reports the scenario and record position.

| Local profile | Original dispatch RVA in resident.dll | Static script evidence |
| --- | --- | --- |
| RLD v1, Famisupi | `liteExec` 0x70c10, `cmdExHIBIT` 0x68b20 | 357 files, 57,255 records, 91 unique top-level IDs |
| RLD v3, Midori no Umi | `liteExec` 0x145e10, `createMethodTable` 0x13acf0, `cmdAction` 0x104930 | 379 files, 51,797 records, 97 unique top-level IDs |
| RLD v3, Stray Sheep volume 1 | `liteExec` 0x1d0ba0, `createMethodTable` 0x19ecf0 | 65 file headers. Separate 1920×1080 launch metadata and DLT 0x0103 resource evidence; ordinary payload initialization remains open. |
| RLD v3, Stray Sheep volume 2 | `liteExec` 0x1d1120, `createMethodTable` 0x19f290, distinct resident build | 76 file headers. Separate 1920×1080 launch metadata and DLT 0x0103 resource evidence; nested dispatch parity and ordinary payload initialization remain open. |

The three v3 residents use a direct switch and a method table. Their upper opcode indices are 0x10a for 2011 and 0x11e for each later resident. The 2011 constructor has 212 explicit method assignments after its default fill, including a write of the default handler at 0xda. Its 211 non-default methods and 13 direct labels account for 224 distinct dispatch IDs. The two later method constructors have identical recovered slot assignments: 234 after the default fill, including an explicit write of the default handler at 0xda. Their direct dispatchers have identical recovered structure apart from relocated function/data references, with 13 direct labels disjoint from the 233 non-default method slots (246 distinct dispatch IDs per volume). This establishes the top-level dispatch surface, not nested action parity or identical handler bodies. Midori `0x18/0x8c` also resets R6 and a branch field. The volume 1 handler additionally calls `rdhExitScene`.

## RLD v1

See the [retail static census](V1_RETAIL_CENSUS.md) for byte accounting, exact parser admission and nested call coverage.

The original top-level switch has 118 case labels. `0x2f` is an additional label record indexed by `blockId2Index` (RVA 0x62090) and falls through the original execution switch. These are the original case IDs in hex.

```text
05 06 07 08 0a 0b 0c 0d 11 12 13 14 15 16 17 18 1c 1d 1e 20
21 22 23 24 25 26 27 28 29 2a 2b 2c 2d 2e 30 31 32 33 34 35
36 37 38 39 3a 41 42 44 45 46 47 48 49 4a 4c 4e 4f 50 51 52
53 54 55 56 57 58 5a 5b 5c 5d 5e 5f 61 62 63 64 65 66 67 69
6a 6e 70 71 72 73 74 75 76 77 78 79 7a 7b 7d 7e 7f 80 81 82
83 88 8b 8c 8d 8e 8f 90 91 93 94 95 96 97 98 99 9a 9b
```

The adapter has handlers for these IDs and specific operand forms.

| IDs (hex) | Behavior in the adapter | Native evidence and boundaries |
| --- | --- | --- |
| 05 | Block numbering, marker entry, conditional skip, and ordinary v1 block checkpoint | Flushes numeric overrides, draw and sound registers, read history, and title. Captures R4 low bits from printer and local-control visibility before flushing (v1 blockEntry 0x54b70). Slot writing is a separate `0x0c` operation. |
| 0a | Conditional text register assignment | CP932 byte strings, with tested replacement and numeric formatting. |
| 06, 08 | Conditional numeric assignment to a local or system register | `liteSetReg` 0x608b0. Six-field CSV, local override lookup, indirect system destination and numeric operations. Persisted system writes and other layouts remain open. |
| 07 | Binary register arithmetic | `liteCalcReg` 0x614f0. Immediate/local/system sources and conditional writeback by destination kind. |
| 0b | Nested calls 0/1, 26–30, 33, 35–41 and gallery 42:3 (decimal) | `cmdExHIBIT`. Title/exit script reloads resume the existing instruction index and call stack. Call 26 creates a timed logo. Calls 27/28 clear skip/auto. Call 29 resolves the parent and disables the selected control, redrawing its third atlas cell when crossing −1. Call 35 prepares an effect on either native lane without playing it. Call 37 reads slot occupation. Gallery subtype 3 runs the numbered movie/effect sequences and marks CG experience. Other gallery subtypes, full character/layer binding and persistent experience remain open. |
| 0c | Write the v1 autosave ring or consume the resumed-block guard. | `liteExec` 0x70c10, `autoSave` 0x3ed00, `saveEngine` 0x3e580. Slot body, DOS timestamp and memo index, and `CLASS.system.dat`. Global R3 is cleared after completion and S7 advances after a successful write. |
| 0d | Timed waits | `liteExec`. Host deadlines cover elapsed time. Native input/skip wakeups remain open. |
| 11 | Scenario transfer for the two-string form | The second string is a condition. The transfer commits local numeric overrides, resets R2 and clears T1. Scene capture, persisted register flush, read-history and other transfer forms remain open. |
| 12 | Open the selected message backdrop or force-close the current frame. | `enterMessage` 0x75870 uses S15+2 and S21 on fade entry. Delta-backed entry remains open. `leaveMessage` 0x75a20 clears the attached printers, optionally fades slot 51, hides it and restores its level. Only the exactly-two-word close reads a duration. |
| 13, 2c | Authored v1 local control definitions and nested toolbar scope. Eight indexed parts at slots 61–68 are prepared with the parent palette and alpha; default 300 ms fade, two-bank toggle art and captured pointer routing. | `cmdDefCtrl` 0x67b00; `UxLocalCtrl::create/enter/leave` 0x50f40/0x51310/0x513e0; toggle atlases 0xb0a00. Three definition layouts occur. Auto (215) and skip preference (216) act during the admitted select wait. Other typed actions and modal UI/click-sound behavior are still unsupported. |
| 14, 16, 17, 2c, 2f | Conditional block jump, call, return and label | `liteCheckCondition` 0x61750/0x61f80, `blockId2Index` 0x62090. First matching explicit or implicit ID, resume after call, five binary comparisons or CP932 comparison strings where accepted. Missing terms, block entry side effects and native return overlays remain open. |
| 18, 8c | End scenario execution | `liteExec`. Session return to the surrounding native engine remains open. |
| 1c | Print plain inline dialogue or dialogue with separate body and speaker panes, retain read/history and optionally enter hit-wait with unpositioned voice. | `RetouchAdvCharacter::say` 0x288a0, `messageEngine` 0x54da0, printer loop 0xaba50. The framed `0x8000/0x8301` font pair uses left-aligned body text and a centered, width-limited speaker name on slots 52 and 54. Narration clears the previous name. Other printer profiles, rich text and skip/auto remain open. |
| 1e | Clear the bound text page immediately or after a blocking, quantized fade. | `clear` 0x2d700, `enterClear` 0x2d7e0 and `leaveClear` 0x2d840. Immediate clear optionally includes separate name slot 54. Timed clear uses channel 5 for both attached printers, then clears both and restores level 16. |
| 29, 2a, 2b, 2e | Independently retain the native save, load, log and choice-view definitions. | See the field, font, alias and page contracts below. Opening these screens requires separate native commands. |
| 2d | Read a scoped asset into the file cache, optionally by a prefix in MiB. | `cmdPreCache` 0x67960. Uses at most 8 MiB per read, with native full-block rounding and short-read termination. |
| 42 | Register an ordered native dialog description with its background, selection image and up to three button definitions. | `defDialog` 0x6a860, image setters 0x767e0–0x768a0. `0x41` presents a dialog and is separate. |
| 30 | Define an ordinary unplaced v1 character in `defChara`. | `createCharacter` 0x63410 and helpers 0x26410–0x265b0. Retains body-variant capacity, CP932 expression/name/address fields, explicit voice bit and the eight-register descriptor beginning at `100+8*id`. Ordinary character voice uses S30 selection and S43/S44 pairs. Redefinition and positioned voice remain open. |
| 31 | Define a body variant already allocated by an ordinary v1 character. | `defCharacter` 0x63650, setBody/face 0x26600/0x26690. The 213-field observed form retains body mode words, signed correction, face origin and the native default eye/mouth selection for each active face. Body-scope duplication, hair/accessory or authored sequence override and onstage resource/animation behavior are not admitted. |
| 32–3a, 46 | Music, environment lanes, loop/one-shot effects, voice selection, and environment definitions | Shared mixer and resident PCM preparation. Alternate render-synchronous fades, mode 4 environment lanes, and full definition bootstrap are open. |
| 48, 49 | Retain the authored v1 printer definitions and bind the selected body, optional name and backdrop. | `liteExec` 0x72127 and `messageFrame` 0x755c0 preserve the upper S14 bits. The installed one-word form closes and reopens an active frame when switching areas. The selected ID goes to the local R77 overlay, and reselecting the active area preserves R77. Fourteen definitions project to seven per-ID objects. Ordinary inline printing takes its font size and row pitch from the selected area. |
| 4a | Ordinary mode-9 CG crossfade or signed solid fill, with mode-255 immediate draws for zero-rectangle records. | `cmdDraw` 0x6da70 and `draw` 0x37780. The signed word's low 24 bits specify RGB, and 0x30860 excludes solid fills from the CG experience bank. Mode 9 defaults to 16 blend steps. The 0x8c580 RGB upsample branch uses 8.8 coordinates and 255-sum weights. The engine prepares it on a scoped image worker and publishes the authored register state and experience at the end of the fade. The opening MMX path intersects oversized source and destination independently. Other scaling, filter and mask forms remain open. See [CG validation](../../docs/validation/exhibit-opening-cg.md). |
| 4e | Black scene fade, CG replacement and stepped music stop for the title and the ordinary printer-closing fade. | Native `sceneFadeOut` 0x3e1d0, cover at layer 83 and Q8 stereo coefficient steps at 0xbc750/0xbc51c. `[1200,0x30001,400,0,83]` follows title teardown. `[800,0x801,400,0,83]` uses the 0x800 entry hook (0x560f0) to close the active printer and leave the visible local controls before removing the cover. Other flag combinations remain outside this dispatch. |
| 51 | Timed pan/zoom of the current CG without blur or orientation flags. Rectangles use four independently interpolated source-pixel edges. | `enterClip` 0x397c0, `TmAsyncMagnify::exec` 0x11f00 and `FCAsyncEffect::calcInt` 0x253e0. The VM continues during the animation. Other flags, blur and composited sources remain open. |
| 52 | Retire magnification after freezing the current crop or waiting for the original target. | Six/ten-word forms. With settle clear, the first word selects immediate freeze (1) or waiting for natural completion (0). The optional destination and timing are ignored. `leaveClip` 0x39cf0 and `tmExitMagnify` 0x48640 preserve the final live rect and wait one tick before resuming. The both-flags-set, no-rectangle route also retires a completed magnification. Resampling on interruption, replacement motion and explicit alternative destinations remain open. |
| 5b | Prepare an image select item in a native group and parent slot. The three-cell horizontal atlas supplies the idle frame on attachment. | `cmdPrepareSelectItem` 0x64d00, image constructor 0xb17f0, binder 0xb1190 and redraw 0xb1ab0. Admitted form is the extended CSV kind 1, unkeyed copy, 256-entry indexed GYU palettes, no extra sound, tooltip or mask. Native indices are expanded with the parent palette, replace its colour plane and preserve its alpha. Input execution is separate (`0x5c`). |
| 5c | Wait for a primary pointer activation in an attached, enabled group and store the selected item ID in a normal register. | `cmdExecSelectItem` 0x66570, group wait 0x856b0, hit-map lookup 0x98d30. This v1 path admits the title's infinite wait, no auxiliary result registers and mask 23. Only ordinary unmasked image controls are supported. Native hover/entry/exit, special layer returns, other event types, timeouts and masked hit maps remain open. |
| 5d | Destroy the selected group's controls, clearing their rectangles to the parent palette index without changing the alpha plane. | `cmdExitSelectItem` 0x659f0, group reset 0x792c0, group list destruction 0x168c0. |
| 5e | Create an authored-path billboard or numbered CG on a literal image slot. Numbered CGs use their bitmap dimensions and count as CG experience after presentation completes. | `cmdBillboard` 0x65a20, numbered/path overloads 0x36640/0x36720. Numbered effect 9 starts transparent and waits for a quantized channel-8 fade, 519 remains at zero, 255 shows directly and the ordinary AsyncShow path sets visibility. Indexed alpha overrides the palette key at 0x958d0/0x94c00. Indirect coordinates, text attachments, additional properties and palette keys without separate indexed alpha still require handling. |
| 5f | Fade and retire an ordinary image slot. The unsigned −1 sentinel retires immediately. | `liteExec` 0x72683, `removeBillboard` 0x36900 and slot release 0x80cd0. A timed removal replaces fade channel 8 and waits for the first native sample at level zero. An attached control or an unknown inactive-slot level can still block execution. |
| 64, 65 | Start and leave six-lane literal billboard movement, absolute or relative. | `cmdEnterMove` 0x63fe0, update 0x24260. The x86 mode-2 case falls through into the logarithmic branch. Negative-duration paths and overlapping lanes remain open. |
| 69, 6a | Start and leave eight-lane literal billboard fades. Levels 0–16 update shared layer opacity during scenario waits. | `cmdEnterFade` 0x64830, prepare/delay/exec/exit 0x46530–0x46610. The last positive delay on a prepared lane sets the channel delay. An unbound zero target is admitted only in a synchronous channel with a bound level-16-to-zero lane of exactly the same duration. Unbound descriptors cannot accept alpha writes (0x81090), and any legal initial level then completes no later than the bound lane (0xd7a0). Other inactive-slot forms, appends to retained channels and non-billboard targets remain open. |
| 8b | Increment or release input-block counters on a single numbered layer or its complement. | `cmdInputLayerControl` 0x67000. Counters cover 127 slots before surface creation, independently of per-control enable counts. Input hit detection consumes a zero count at 0x98d76. Other range forms and original event callbacks remain open. |
| 71, 72 | Path/ID movie start and leave, optional effect, one-second startup interval | `cmdEnterMovie` 0x68960, `enterMovie` 0x3c2f0/0x3c3c0, `leaveMovie` 0x3c450. Default layer and flags only. Playback and presentation require a player run. |
| 75, 76, 77 | Prepare, play and retire the scoped ordinary opening DLT on channel 0 and layer 4. | Exact pinned opening operands only. Native prepare 0x3c650, start 0x3c7a0 and leave 0x3c900. Uses the file period, independent frames and 800 ms entry/exit fades. Static and unit validation only; see [DLT validation](../../docs/validation/exhibit-dlt.md). |

Of 91 sample opcode IDs, 54 reach an adapter dispatch arm for at least one observed form, and 37 admit no observed records. The unhandled sample IDs are:

```text
15 1e 20 21 23 24 25 26 27 28
47 4c 4f 50
53 54 55 56 57 58 5a 61 62
70 73 74 79 88 93 94 95 96 97 98 99 9a 9b
```

The v1 `0x1c` dispatch admits the observed seven-word plain say encoding.
The execution path uses message area 5 with ordinary unpositioned voice,
seen bits, rolling history and a fresh input edge to release hit-wait.
Other area profiles, rich text, positioned voice and auto/skip still require
native paths. See [opening printer validation](../../docs/validation/exhibit-opening-printer.md).

Native `0x0b` has 23 primary subcommands (decimal 0–3 and 24–42). Famisupi calls 18 of them in 426 records. Primary handlers 31 and 42 themselves dispatch further. The sample calls are:

```text
2 3 24 25 26 27 28 29 30 32 35 36 37 38 39 40 41 42
```

## RLD v3

The verified 2011 initializer takes the ordinary scenario seed from the installed configuration and the initialization bitmap, and obtains the definition seed from the installed resident module. The transform starts after the 16-byte header, processes at most 0x3ff0 words and leaves the trailing bytes intact. The record origin is the header offset plus four. All 379 decoded Midori files consumed exactly the declared records and ended at EOF in an independent static reader.

Midori's binary arithmetic export `cmdCalcReg` (RVA 0x94060) forwards to `liteCalcReg` (0xb2030), with the same source types and operations as v1. At `0xbf`, the 2011 script action dispatcher splits six numeric fields and a final byte string. Implemented literal-operand actions are 190 (set local flag 7), 262 (set/clear T206), 346 (clear a register range), 348 (replace bits 8–10 of S23), and 360 (update system flag 5 and T208/T209). The respective RVAs are 0xb8d50, 0xb9780, 0xba790, 0xba980 and 0xbab30. The action result cell starts at zero and a nonnegative destination writes it back to the local holder. Additional local/system-flag and ruby-line actions and their tested scalar operands are listed below.

| Nested dispatch | Midori sample IDs (decimal) | Raw records |
| --- | --- | ---: |
| `0x0b` | 25, 27, 28, 30, 51, 57, 60 | 20 |
| `0xbf` | 151, 164, 166, 190, 250, 262, 265, 338, 345, 346, 348, 350, 360 | 388 |

The 97 unique top-level IDs used in Midori are:

```text
05 06 08 0b 0c 0d 11 12 13 14 15 16 17 1c 1e 20 21 23 24 25
26 27 2c 2e 2f 30 31 32 33 34 35 36 37 38 39 3a 3b 3c 3d 3e
40 42 46 47 48 49 4a 4c 4e 50 51 52 57 58 5a 5b 5c 5d 5e 5f
61 64 65 66 69 6a 6d 70 71 72 73 74 75 84 88 93 94 a0 a2 a7
bd bf cb cc cd ce cf ef f2 f3 f8 f9 fa fb fc fd ff
```

The November 26 census found at least one dispatch form in 36 of the 97 sampled IDs: 05, 06, 08, 0b, 0d, 12, 14, 16, 17, 2c, 2e, 2f, 30, 31, 32, 33, 34, 35, 38, 39, 3a, 3d, 3e, 46, 47, 48, 49, 4a, 50, 6d, a7, bd, bf, fb, fc and fd. The 2011 0x0b selector 30 installs a literal name in one of eight scenario-entry slots. Selector 57 installs the ordinary control-part resource and anchor; paired action 338 updates only the authored plain UI sound slots. `0x2c` mode 3 shares the part owner for eight triples, retaining slots with native -1 sentinels. `0x6d` retains the authored shortcut keyboard/right-button bindings, separate from physical triggers or action execution. Selector 60 retains the scoped, decoded hit-wait atlas and actual authored marker position/T202; wait presentation and other selectors stay held. `0x47` clears the ordinary packed debug mode and retains its eight mappings without enabling a debug overlay. `0x38/0x39/0x3a` route ordinary WAV effects to independent audio lanes and distinguish timed input eligibility, synchronized waits and immediate stops. `0x3d/0x3e` create/query the scoped, script-owned integer tables; `0xbd` literal scene-done marks both the counter and the message bit. Literal `0x32/0x33` music keeps decoded-frame loop points, packed gain and bank-local retirement. The scoped `0x46/0x34/0x35` family defines, plays and stops ordinary single-lane ambience on separate loop banks using the 2011 mute and packed gain. `0xfb` installs the three actor draw-slot pairs; absolute and centered/bottom packed `0xfc` and immediate `0xfd` compose and retire the supported visible or retained-hidden portraits without enabling general positioned character rendering. `0x49` (0x10ea00) retains the actual path-backed backdrop dimensions and authored printer definition. `0x48` binds an ordinary frame and its separately retained empty body/name planes, including active rebind. `0x12` enters/leaves the installed backdrop via the native duration and quantization rules. `0x30/0x31` prepare all sampled defChara declarations with independent v3 four-register actor descriptors and 359-field bodies; the actual prelude transfer still needs native capture. The 0x13 toolbar, positioned actors, glyph output and animated/group modes are still unsupported. Conditions, calls, assignments and passive block markers have execution paths. The live BGM and ambience owners produce the native R61/T204 capture image, and live status provides independent masked printer-open and authored block-flag bits, T207 entry names and exact S16 experience span. Settled portrait owners supply detached T205 rows without guessing raw coordinates of never-drawn slots. The live `0xa7` mode-zero character palette owner supplies T4 and affects subsequent indexed portraits without recoloring already-live layers. The immediate `0x4a/255` full-size opaque draw and solid fill publish a retained background and detached R10–R28; the no-crop opaque `0x4a/9` form shares that owner and reproduces the 16-step byte-domain background blend without modifying foreground layers. The exact 1:1 retained crop variant preserves its backing, and `0x50` pans that backing in the bounded unscaled, no-HQ shape without claiming viewport clip ownership. Other source geometry and effects stay guarded. The live printer produces R78/T6 after admitted frame changes and settled leave; the scoped local-position work produces T7 from proven native reset/global-set states. The startup block still requires the native camera, viewport/layer, actor and remaining printer state. The November 26 direct-start replay stopped at start.rld+0x114. The accepted 2011 loader now publishes the actual successful native logical RLD path to T2; this is independent of T201 (the saved cursor string). The November 27 bootstrap work and replay of the installed definition entry are documented in the [updated coverage report](../../docs/validation/exhibit-dispatch-2026-11-27.md). At that baseline, the other 61 sampled IDs had no admitted form. The counts exclude nested methods and additional operand variants. See the [2011 static opcode/call census and exact startup dependency](MIDORI_2011_CENSUS.md).

The 2011 `0x05` checkpoint stages its register image before applying it.
The pre-flush writes are R4, the CSV R6 flag, R78 and the native text
packets; the LocalHolder flushes R2 with its current values before scene
draw, sound and entered-character normal registers are written. CSV blocks
then record R7–R9. The constructed viewport is unclipped, its camera
registry has eight empty slots, and no admitted command activates the
camera or entered-character manager. The disabled layer-save flag excludes
T-layer serialization. T205 canonicalizes the never-written raw X/Y to
zero on inactive rows and retains the real raw coordinates after a draw.
The two later residents use the same staging boundary in blockEntry
(0x159a30 and 0x159b00). Their bootstrap captures the constructor's nine
sound lanes, the additional R81 draw word and the T210 auxiliary scene
packet. Midori writes five sound lanes. The stock-start variants use a
default hit-wait configuration and have no active camera or entered
character. Active layer serialization is still checked before a commit.
These registers describe the checkpoint in memory. File writing and
restore are separate commands.

The v3 `0x42` handlers (0xe36d0, 0x168ec0 and 0x168f90) append an ordered
dialog definition from 26 positional fields. The count is in the low byte
and bits 16–23 encode the one-based default button. Background and
selection layer IDs use the resident's alias table. The selection resource
overload clears its two numeric settings. Empty, `*`, or numeric −1
selection names choose the numeric overload. Opening the dialog is `0x41`.

The late `0x100` handlers (0x16ab20 and 0x16abf0) split three integers and
a final resource string, then call UxLocalCtrl::defParts in mode 3. An
unsigned slot index below 32 replaces the resource and two position words.
Other indices return without a write. Notification display uses separate
local control entry points.

The three `0x40` cmdDefCtrl2 methods (0xdf3c0, 0x166c30 and 0x166d00)
replace one of sixteen retained control definitions. A definition owns its
native action and layer IDs, background and control images, optional
information image, button parameters, keys and property menu. The 2011
mode-5 record has 91 fields and seven property rows. Later mode 6 has
105 fields and nine rows. Later lookup changes the default layer bases and
binds its mode-254 self-reference to the current control slot. Native
immediate instantiation (control flags bit 3), composite atlases and live
character layer aliases still require their corresponding runtime paths.

The later `0x2c` selector 3 updates eight notification triples through
the same slot owner as `0x100`; a -1 resource preserves that slot.
`0xfb` (0xe5700, 0x1691f0 and 0x1692c0) changes the inactive portrait
pair for slots 0–7, with adjacent layers resolved through the current
resident's alias table. Out-of-range slots return. Redefining a slot with a
live portrait remains guarded.

The 2011 method-table fallback at 0x8f00 returns immediately for 42
unassigned execution slots. Its independent `0x2f` branch-index label
continues to be processed as a label.

The v3 `0x2d` cmdPreCache methods (0xb3150, 0x1853a0 and 0x1854d0)
read a native path and unsigned MiB limit. They reuse the v1 scoped
reader with the same 8 MiB block boundary, with zero requesting all
bytes. The adapter waits for its read worker before executing the
following instruction.

The 2011 expression operand `Q3` in liteStr2Value (0xd6810) uses the
UxOsVersion table (constructor 0x199570). Win32 platform 1 maps the
4.0/4.10/4.90 tuples to 1/2/3. Platform 2 maps 4.0 to 100,
5.0–5.2 to 101–103 and 6.0–6.1 to 104–105. The original default is
−1 for other tuples. Portable sessions use that default. A project can
provide `native_windows_version` as `[platform, major, minor]` when
matching a specific historical environment.

The 2011 `0x11` cmdChangeScenario (0x124e20) supports the old literal
record, the two-string conditional record and the CSV record. CSV mode 0
uses an authored path, mode 1 reads a text register, mode 2 evaluates the
second column of a string database in signed-key order and uses its first
matching destination, and mode 3 exits. The adapter keeps native CP932 in
T2 while opening the resolved path through scoped storage. The taken
extended form clears R3/R6 and T206, flushes outgoing LocalHolder and
checkpoint state, and starts the successor at R2=0 and empty T1. Before
publishing a successor it verifies the envelope and writes the FCReg
system bank atomically. A failed load or write leaves the old PC and live
registers intact. The original caller attempts the write before loading
the destination.

The 2011 system-bank writer follows the executable's FCReg vtable
(0xb5edc) and methods 0x5b600/0x5b6a0. It writes a length-prefixed,
NUL-terminated `FCReg`, a little-endian count and its signed DWORD
values. Original flushSysreg (0x51e60) clears S0 before this write.
The CP932 CLASS from the installed INI determines the filename. CrySirror
reads through the user overlay and writes to its Save mount.

The 2011 block capture checks system flag 5 before invoking `saveLayer`
(0xf4930). That routine visits logical IDs 51–54, which are native layers
131–134. With no allocated matching layers it clears a stale T208 and
preserves T209. The VM fixture checks this transition on a block command.
The adapter keeps the existing error when an installed printer or
active overlapping portrait needs a populated layer packet.

Both late residents handle `0x14`, `0x16` and `0x17` in liteExec
(0x1d0ba0 and 0x1d1120). The ordinary-frame route evaluates the
condition, resolves the first matching block or label, and only
then installs a jump or a call. A call saves its return record index
and block, and runs the admitted R/S/L/T assignments in order.
The normal return restores the saved index and block. Its clear
condition bit bypasses condition evaluation. The late label scanner
uses 0x159c90/0x159d60 rather than reusing the 2011 scan.

In both late residents `0xd0–0xd2` create, read and write the separate
eight string databases. The original create functions are at
0x162dc0/0x162e90, reads at 0x17da70/0x17db40 and writes at
0x18bef0/0x18c040. Rows preserve signed keys and original CP932 bytes.
The ordinary forms accept up to eight column pairs, expose the results
to native registers and clear old rows on replace. Function-type key
expressions and extended register destinations remain guarded.

## Retail v1 view-definition and gallery contracts

Only v1 uses the following resident RVAs. These are script-owned definitions;
opening a dialog, saving a slot and rendering a gallery are separate operations.

- `0x29`/ `0x2a` forward their string and format to `defSave` (0x6ad00) and
  `defLoad` (0x6b360). They use independent owners at +2032 and +2360. The
  legacy format splits 48 positional fields and assigns the native default
  font. Extended formats read a font in the final remainder. Each replacement
  installs five layer IDs, page/item counts, four image origins, item and memo
  geometry, four selection-image definitions, and freshly generated per-page
  coordinates. The ordinary definition allocates no window or game slot.
- `0x2b` defines the log view (0x6ba80), including the two fonts, special
  colors and the selector's placement. `0x2e` defines selection-window
  resource identities, normalized bounds, two placement parameters and font
  (0x639c0). Resource names remain native byte strings until a resource is used.
- Decimal native 30 assigns an FCString to one of five scenario-entry slots
  (0x68e08). The last argument keeps its comma remainder. Invalid slot indices
  preserve the previous state. `liteInit` (0x5ef40) initializes slot 1 to
  title and the other four to exit. This table differs from the eight 2011 slots.
- Decimal native 36 updates only the indexed load-view page origin.
  `defLoadPageOverride` (0x6b9c0) splits three words irrespective of outer
  arity. `setSIPagePos` (0x76f00) ignores absent pages and out-of-range indices.
- Decimal native 40 walks period-delimited experience IDs. A positive CG,
  sound or scene counter in the selected bank leaves the button unchanged.
  Otherwise a valid direct layer and item use the same disable path as native
  29. It does not resolve character aliases or write the result register
  (0x696d6–0x697fe, 0x87690, 0x994a0).

### RLD v3 selection definition

The 2011 constructor assigns 0x2e to resident RVA 0xc8de0. The later resident's
0x16ac20 independently has the same three layouts. The high header nibble
selects 11 fields (basic), 19 (eight extra item images) or 22 (three extra
numeric properties). The last field retains the complete font. Extra image
IDs 1–8 update independently; the exact byte string "-1" preserves that ID.
Setting an extra ID sets bit zero of the selection property and that state
survives a later basic definition (2011 helper 0x7b780). Every definition
replaces the default image, bounds, two steps, font and three mode properties.
Signed numeric fields in this profile add lowercase binary literals, unlike
v1. Only the admitted 2011 loader exposes this definition at runtime.

The 2011 method slots 0x29/2a/2b call defSave/defLoad/defLog at
0xe39b0/0xe3e80/0xe4350. The two slot constructors retain the 48/49-field
form; their five layer identifiers go through 0xdbec0. The log converts both
layer identifiers and passes the selector's Y to the native property at
offset +0x10, where v1 passes constant 20. Native 36 uses its own three
integers and 0xd79e0/0xcf230 to change only one load-page coordinate.
The corresponding later methods are 0x1a6490/0x1a5ba0/0x1a6150 and 0x1a6080.
The 2011 layer table and arithmetic ranges below 1000 are resolved when
the view is defined. Character and actor aliases 1000 and above require
their active native owners.

For 2011, action-table stores at 0x14599b and 0x145d83 bind flag
families 186–191 and 340–342. 186/187 clear/set S23 bit 1. 188/189
set/clear local bit 3, 190/191 set/clear local bit 7. 340 uses its
two evaluated arguments to set an arbitrary local bit. For a valid bit
0–31 the native target is S[S15+4]; only a target index of 3 also updates
its cached boolean (0x52220). 341 toggles S23 bit n for value -1, or sets
it according to the supplied truth value (0x57dc0/0x57f10). 342 writes the
low nibble of S42 and forces bit 4 (0x58070). Native services 23 and 44
set S23 bits 0 and 1 using the whole argument tail and leave the command's
result register alone. The later resident changes action 186 and aliases
188/189 to action175/176; a shared numeric selector does not establish
an identical handler.


### Raw native nested-dispatch inventory (decimal)

| Resident | 0x0b direct selectors beyond the default branch | 0xbf action constructor RVA | Explicit action slots | Non-default action slots | Distinct target functions including default |
| --- | --- | --- | ---: | ---: | ---: |
| 2011 | 0–3, 8, 23–66 (49) | 0x1454c0 | 234 | 232 | 229 |
| Volume 1 | 0–3, 8, 10, 23–67 (51) | 0x19b440 | 300 | 296 | 293 |
| Volume 2 | 0–3, 8, 10, 23–67 (51) | 0x19b9e0 | 300 | 296 | 293 |

The 2011 nested switch is at 0x13b626. The two later dispatchers select
through their own jump tables at 0x177f58/0x178028. For 0xbf, these
constructor counts exclude the repeated default fill and include
explicit writes of the default at 0 and 80 (also 241 and 353 in both
volumes). One method may be named by multiple slots. The numbers count
resident action-table slots; the outer action dispatcher also recognizes
million-based user action categories, which are separate routes.

In both late residents, action 271 reads argument one and the signed screen
update counter. `1` decrements, `0` increments, `-1` selects one of those
operations from the counter's sign, and `-2` releases every outstanding lock.
Other modes leave the counter alone. The native refresh runs on the transition
from `-1` to zero. The action returns 4096 without modifying the outer result
cell, which the script dispatcher initialized to zero. The script route
evaluates all four arguments even though this action only uses the first.
The CrySirror mapping uses an immutable shared layer capture once at the outer
lock and retires it on the final release, while live layer state continues.
See the [late screen-update validation](../../docs/validation/exhibit-dispatch-2026-11-27.md).

Both late `0x47` handlers (0x168700/0x1687d0) use the existing 22-word
debug mapping in the packed form. With the stock `0x20000000` mode the
low 28 mode bits are cleared and the overlay branch is inactive. Eight
ordered signed mapping pairs still replace the prior mapping. The active
overlay and legacy binary forms are separate from this packed path.

2011 non-default action selectors:

```text
1–15 31–34 40–47 50–72 95 98–99 101–108 110–113 119–123 130–137
142–154 158–227 250–266 298–321 323–348 350–352 360
```

Each later volume's non-default action selectors:

```text
1–15 31–34 40–47 50–73 79 89–90 93–99 101–108 110–115 117–124
130–137 142–229 239–240 242–243 245–283 285–286 288–291 295–321
323–352 354–358 360–361 364–365 370 380
```

For the 2011 resident, the adapter has scoped routes for 10 service selectors
(23, 30, 33, 36, 39, 44, 47, 48, 57, 60). Of 232 non-default action slots,
78 perform an operation for at least one form and 19 have a verified native
return when invoked by 0xbf. The other 135 have no direct-script route. The
current [dispatch and playback matrix](../../docs/validation/exhibit-dispatch-2026-11-27.md)
separates the resident tables from corpus counts, operand variants, UI callbacks
and actual playback.

V1 nested 41 in cmdExHIBIT (0x68b20) selects an item range in one group and on one
direct layer. The group reset 0x5d also destroys controls. For the
admitted unmasked image items the destructor erases through 0xb1b40,
0x98940 and 0x964a0. It fills each rectangle with the parent's clear
palette index, preserves alpha and retires the control's hit target.
