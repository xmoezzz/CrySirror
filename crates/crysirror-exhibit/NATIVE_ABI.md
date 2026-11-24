# ExHibit native dispatch

## Evidence and counting

The unit in this inventory is a distinct 16-bit RLD opcode. A record can also carry numeric operands, NUL-terminated strings, a format nibble and an operation-specific subtype. Counting a handler once does not establish its operand coverage. The retail modules below were read as data in IDA, with their exported symbols and the original x86 dispatch. The interpreter rejects unmatched operations and reports the scenario and record position.

| Local profile | Original dispatch RVA in resident.dll | Static script evidence |
| --- | --- | --- |
| RLD v1, Famisupi | `liteExec` 0x70c10, `cmdExHIBIT` 0x68b20 | 357 files, 57,255 records, 91 unique top-level IDs |
| RLD v3, Midori no Umi | `liteExec` 0x145e10, `createMethodTable` 0x13acf0, `cmdAction` 0x104930 | 379 files, 51,797 records, 97 unique top-level IDs |
| RLD v3, Stray Sheep volume 1 | `liteExec` 0x1d0ba0, `createMethodTable` 0x19ecf0 | 65 file headers. Separate 1920×1080 launch metadata and DLT 0x0103 resource evidence; ordinary payload initialization remains open. |
| RLD v3, Stray Sheep volume 2 | `liteExec` 0x1d1120, `createMethodTable` 0x19f290, distinct resident build | 76 file headers. Separate 1920×1080 launch metadata and DLT 0x0103 resource evidence; nested dispatch parity and ordinary payload initialization remain open. |

The three v3 residents use a direct switch and a method table. Their upper opcode indices are 0x10a for 2011 and 0x11e for each later resident. The 2011 constructor shows 212 explicit method assignments after its default fill. The two later method constructors have identical recovered slot assignments: 234 after the default fill, including an explicit write of the default handler at 0xda. Their direct dispatchers have identical recovered structure apart from relocated function/data references, with 13 direct labels. This establishes the top-level dispatch surface, not nested action parity or identical handler bodies. Midori `0x18/0x8c` also resets R6 and a branch field. The volume 1 handler additionally calls `rdhExitScene`.

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
| 05 | Block numbering, marker entry, conditional skip, and ordinary v1 block checkpoint | Flushes numeric overrides, draw and sound registers, read history, and title. Captures R4 low bits from printer and local-control visibility before flushing (v1 blockEntry 0x54b70). Original file save and restore are not implemented. |
| 0a | Conditional text register assignment | CP932 byte strings, with tested replacement and numeric formatting. |
| 06, 08 | Conditional numeric assignment to a local or system register | `liteSetReg` 0x608b0. Six-field CSV, local override lookup, indirect system destination and numeric operations. Persisted system writes and other layouts remain open. |
| 07 | Binary register arithmetic | `liteCalcReg` 0x614f0. Immediate/local/system sources and conditional writeback by destination kind. |
| 0b | Nested calls 26, 27, 28, 29, 33, 37, 38 and 39 (decimal) | `cmdExHIBIT`. CG, scene and sound experience counters and the local flag. Call 26 creates the once-per-session timed logo layer. Calls 27/28 clear the startup skip/auto state. Call 29 resolves the parent and decrements the control enable count. The first transition to −1 redraws the third cell of an attached visible kind-1 image (0x87690, 0x994a0, 0xb1420, 0xb1ab0). Character creation and assignment to a layer remain open. Call 37 reads user/auto/quick slot indices. Persistent experience files are outstanding. |
| 0d | Timed waits | `liteExec`. Host deadlines cover elapsed time. Native input/skip wakeups remain open. |
| 11 | Scenario transfer for the two-string form | The second string is a condition. The transfer commits local numeric overrides, resets R2 and clears T1. Scene capture, persisted register flush, read-history and other transfer forms remain open. |
| 12 | Open a selected inactive message backdrop. | Native `enterMessage` 0x75870. Nonzero open ignores the second word; explicit closes, text and delta-backed entry remain open. Uses S15+2 and the S21 quantized frame fade where applicable. |
| 13, 2c | Authored v1 local control definitions and nested toolbar scope. Eight indexed parts at slots 61–68 are prepared with the parent palette and alpha; default 300 ms fade, two-bank toggle art and captured pointer routing. | `cmdDefCtrl` 0x67b00; `UxLocalCtrl::create/enter/leave` 0x50f40/0x51310/0x513e0; toggle atlases 0xb0a00. Three definition layouts occur. Auto (215) and skip preference (216) act during the admitted select wait. Other typed actions and modal UI/click-sound behavior are still unsupported. |
| 14, 16, 17, 2c, 2f | Conditional block jump, call, return and label | `liteCheckCondition` 0x61750/0x61f80, `blockId2Index` 0x62090. First matching explicit or implicit ID, resume after call, five binary comparisons or CP932 comparison strings where accepted. Missing terms, block entry side effects and native return overlays remain open. |
| 18, 8c | End scenario execution | `liteExec`. Session return to the surrounding native engine remains open. |
| 30 | Define an ordinary unplaced v1 character in `defChara`. | `createCharacter` 0x63410 and helpers 0x26410–0x265b0. Retains body-variant capacity, CP932 expression/name/address fields, explicit voice bit and the eight-register descriptor beginning at `100+8*id`. The separate staged character voice uses S30 selection and S43/S44 pairs, not effect gain. Redefinition, positioned voice and 0x1c remain open. |
| 31 | Define a body variant already allocated by an ordinary v1 character. | `defCharacter` 0x63650, setBody/face 0x26600/0x26690. The 213-field observed form retains body mode words, signed correction, face origin and the native default eye/mouth selection for each active face. Body-scope duplication, hair/accessory or authored sequence override and onstage resource/animation behavior are not admitted. |
| 32–3a, 46 | Music, environment lanes, loop/one-shot effects, voice selection, and environment definitions | Shared mixer and resident PCM preparation. Alternate render-synchronous fades, mode 4 environment lanes, and full definition bootstrap are open. |
| 48, 49 | Retain the authored v1 printer area schema and select its inactive backdrop; preserve upper S14 bits and write R77. | `messageFrame` 0x755c0, `defMessageArea` 0x62730, setFrame helpers 0x75ed0/0x760a0. All fourteen retail definitions project to seven byte-identical per-ID objects; other def.rld commands are not executed by the projection. Active changes, font rendering and dialogue remain open. |
| 4a | Mode-9 fullscreen CG, canvas-sized source crops, and the pinned v1 opening oversized rectangle. | `cmdDraw` 0x6da70. The opening x86/MMX path independently clips the 1600 × 1200 source request to the real 800 × 600 texture and destination, then copies 1:1. Other oversized rectangles, legacy non-MMX behavior, other draw modes and exact GPU blending remain open. See [opening CG validation](../../docs/validation/exhibit-opening-cg.md). |
| 4e | Timed solid scene fade, CG replacement and stepped music stop for the pinned ordinary START state. | Native `sceneFadeOut` 0x3e1d0, cover at layer 83 and Q8 stereo coefficient steps at 0xbc750/0xbc51c. Only `[1200,0x30001,400,0,83]` with the title teardown completed is admitted; shared worker/renderer parity and other flag combinations are not established. |
| 51 | Timed pan/zoom of the current CG without blur or orientation flags. Rectangles use four independently interpolated source-pixel edges. | `enterClip` 0x397c0, `TmAsyncMagnify::exec` 0x11f00 and `FCAsyncEffect::calcInt` 0x253e0. The VM continues during the animation. Other flags, blur and composited sources remain open. |
| 52 | Leave a completed magnification at its remembered destination and retire the native handle. | Six/ten-word forms with both flags set and zero target width. A valid object waits one tick. See `liteExec` case 82 and `leaveClip` 0x39cf0. Replacement motion and explicit alternative destinations remain open. |
| 5b | Prepare an image select item in a native group and parent slot. The three-cell horizontal atlas supplies the idle frame on attachment. | `cmdPrepareSelectItem` 0x64d00, image constructor 0xb17f0, binder 0xb1190 and redraw 0xb1ab0. Admitted form is the extended CSV kind 1, unkeyed copy, 256-entry indexed GYU palettes, no extra sound, tooltip or mask. Native indices are expanded with the parent palette, replace its colour plane and preserve its alpha. Input execution is separate (`0x5c`). |
| 5c | Wait for a primary pointer activation in an attached, enabled group and store the selected item ID in a normal register. | `cmdExecSelectItem` 0x66570, group wait 0x856b0, hit-map lookup 0x98d30. This v1 path admits the title's infinite wait, no auxiliary result registers and mask 23. Only ordinary unmasked image controls are supported. Native hover/entry/exit, special layer returns, other event types, timeouts and masked hit maps remain open. |
| 5d | Destroy the selected group's controls without retiring their parent bitmap. | `cmdExitSelectItem` 0x659f0, group reset 0x792c0, group list destruction 0x168c0. |
| 5e | Create an image billboard at a literal position on its numbered layer. Effect 255 uses the supplied level, 519 binds the surface at level zero. | `cmdBillboard` 0x65a20, path overload 0x36720, initializer 0x80ba0, placement 0x978e0. The bitmap supplies the dimensions. Cropping metadata for other source types has no effect in this overload. Dynamic/relative layer IDs, attached text, other effects and properties remain open. |
| 5f | Remove an image billboard and retire its composed image resource after the select controls detach. | `removeBillboard` 0x36900 and final slot release 0x80cd0. Admits literal slots and zero fade duration only. |
| 64, 65 | Start and leave six-lane literal billboard movement, absolute or relative. | `cmdEnterMove` 0x63fe0, update 0x24260. The x86 mode-2 case falls through into the logarithmic branch. Negative-duration paths and overlapping lanes remain open. |
| 69, 6a | Start and leave eight-lane literal billboard fades. Levels 0–16 update shared layer opacity during scenario waits. | `cmdEnterFade` 0x64830, prepare/delay/exec/exit 0x46530–0x46610. The last positive delay on a prepared lane sets the channel delay. An unbound zero target is admitted only in a synchronous channel with a bound level-16-to-zero lane of exactly the same duration. Unbound descriptors cannot accept alpha writes (0x81090), and any legal initial level then completes no later than the bound lane (0xd7a0). Other inactive-slot forms, appends to retained channels and non-billboard targets remain open. |
| 8b | Increment or release input-block counters on a single numbered layer or its complement. | `cmdInputLayerControl` 0x67000. Counters cover 127 slots before surface creation, independently of per-control enable counts. Input hit detection consumes a zero count at 0x98d76. Other range forms and original event callbacks remain open. |
| 71, 72 | Path/ID movie start and leave, optional effect, one-second startup interval | `cmdEnterMovie` 0x68960, `enterMovie` 0x3c2f0/0x3c3c0, `leaveMovie` 0x3c450. Default layer and flags only. Playback and presentation require a player run. |
| 75, 76, 77 | Prepare, play and retire the scoped ordinary opening DLT on channel 0 and layer 4. | Exact pinned opening operands only. Native prepare 0x3c650, start 0x3c7a0 and leave 0x3c900. Uses the file period, independent frames and 800 ms entry/exit fades. Static and unit validation only; see [DLT validation](../../docs/validation/exhibit-dlt.md). |

Of 91 sample opcode IDs, 47 reach an adapter dispatch arm for at least one observed form, and 44 admit no observed records. The unhandled sample IDs are:

```text
0c 15 1c 1e 20 21 23 24 25 26 27 28 29 2a 2b 2e
42 47 4c 4f 50
53 54 55 56 57 58 5a 61 62
70 73 74 79 88 93 94 95 96 97 98 99 9a 9b
```

The v1 `0x1c` character-say record remains unhandled. An exact first-opening
operand check and a separately tested area-5 glyph plan do not admit it:
native character voice, history/experience and hit-wait are one observable
operation. See [opening printer validation](../../docs/validation/exhibit-opening-printer.md).

Native `0x0b` has 23 primary subcommands (decimal 0–3 and 24–42). Famisupi calls 18 of them in 426 records. Primary handlers 31 and 42 themselves dispatch further. The sample calls are:

```text
2 3 24 25 26 27 28 29 30 32 35 36 37 38 39 40 41 42
```

## RLD v3

The verified 2011 initializer takes the ordinary scenario seed from the installed configuration and the initialization bitmap, and obtains the definition seed from the installed resident module. The transform starts after the 16-byte header, processes at most 0x3ff0 words and leaves the trailing bytes intact. The record origin is the header offset plus four. All 379 decoded Midori files consumed exactly the declared records and ended at EOF in an independent static reader.

Midori's binary arithmetic export `cmdCalcReg` (RVA 0x94060) forwards to `liteCalcReg` (0xb2030), with the same source types and operations as v1. At `0xbf`, the 2011 script action dispatcher splits six numeric fields and a final byte string. Implemented literal-operand actions are 190 (set local flag 7), 262 (set/clear T206), 346 (clear a register range), 348 (replace bits 8–10 of S23), and 360 (update system flag 5 and T208/T209). The respective RVAs are 0xb8d50, 0xb9780, 0xba790, 0xba980 and 0xbab30. The action result cell starts at zero and a nonnegative destination writes it back to the local holder. Other actions, expression forms and opcodes fail explicitly.

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

Thirty-five of the 97 sampled IDs reach at least one dispatch form: 05, 06, 08, 0b, 0d, 12, 14, 16, 17, 2c, 2f, 30, 31, 32, 33, 34, 35, 38, 39, 3a, 3d, 3e, 46, 47, 48, 49, 4a, 50, 6d, a7, bd, bf, fb, fc and fd. The 2011 0x0b selector 30 installs a literal name in one of eight scenario-entry slots. Selector 57 installs the ordinary control-part resource and anchor; paired action 338 updates only the authored plain UI sound slots. `0x2c` mode 3 shares the part owner for eight triples, retaining slots with native -1 sentinels. `0x6d` retains the authored shortcut keyboard/right-button bindings, separate from physical triggers or action execution. Selector 60 retains the scoped, decoded hit-wait atlas and actual authored marker position/T202; wait presentation and other selectors stay held. `0x47` clears the ordinary packed debug mode and retains its eight mappings without enabling a debug overlay. `0x38/0x39/0x3a` route ordinary WAV effects to independent audio lanes and distinguish timed input eligibility, synchronized waits and immediate stops. `0x3d/0x3e` create/query the scoped, script-owned integer tables; `0xbd` literal scene-done marks both the counter and the message bit. Literal `0x32/0x33` music keeps decoded-frame loop points, packed gain and bank-local retirement. The scoped `0x46/0x34/0x35` family defines, plays and stops ordinary single-lane ambience on separate loop banks using the 2011 mute and packed gain. `0xfb` installs the three actor draw-slot pairs; absolute and centered/bottom packed `0xfc` and immediate `0xfd` compose and retire the supported visible or retained-hidden portraits without enabling general positioned character rendering. `0x49` (0x10ea00) retains the actual path-backed backdrop dimensions and authored printer definition. `0x48` binds an ordinary frame and its separately retained empty body/name planes, including active rebind. `0x12` enters/leaves the installed backdrop via the native duration and quantization rules. `0x30/0x31` prepare all sampled defChara declarations with independent v3 four-register actor descriptors and 359-field bodies; the actual prelude transfer still needs native capture. The 0x13 toolbar, positioned actors, glyph output and animated/group modes are still unsupported. Conditions, calls, assignments and passive block markers have execution paths. The live BGM and ambience owners produce the native R61/T204 capture image, and live status provides independent masked printer-open and authored block-flag bits, T207 entry names and exact S16 experience span. Settled portrait owners supply detached T205 rows without guessing raw coordinates of never-drawn slots. The live `0xa7` mode-zero character palette owner supplies T4 and affects subsequent indexed portraits without recoloring already-live layers. The immediate `0x4a/255` full-size opaque draw and solid fill publish a retained background and detached R10–R28; the no-crop opaque `0x4a/9` form shares that owner and reproduces the 16-step byte-domain background blend without modifying foreground layers. The exact 1:1 retained crop variant preserves its backing, and `0x50` pans that backing in the bounded unscaled, no-HQ shape without claiming viewport clip ownership. Other source geometry and effects stay guarded. The live printer produces R78/T6 after admitted frame changes and settled leave; the scoped local-position work produces T7 from proven native reset/global-set states. The startup block still requires the native camera, viewport/layer, actor and remaining printer state. The installed entry is still guarded at start.rld+0x114. The accepted 2011 loader now publishes the actual successful native logical RLD path to T2; this is independent of T201 (the saved cursor string). The stock external definition/camera/control prelude before start is verified statically, not replayed. The other 62 sampled IDs have no admitted form. The counts exclude nested methods and additional operand variants. See the [2011 static opcode/call census and exact startup dependency](MIDORI_2011_CENSUS.md).
