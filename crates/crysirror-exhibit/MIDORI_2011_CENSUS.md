# Midori no Umi: 2011 ExHibit v3 static census

Separate from the v1 and both Stray Sheep v3 profiles. This uses only the
already-admitted 2011 configuration/bitmap and resident initializer and
never executes the original executable, launches a game, or reads saves.

```sh
CRYSIRROR_EXHIBIT_MIDORI_ROOT=/path/to/installed/midori cargo test --release \
  -p crysirror-exhibit static_midori_2011_corpus -- --ignored --nocapture
```

## Accounting and adapter admission

All 379 scripts parse structurally, consuming exactly 51,797 records:
104,604 prefix bytes, 207,188 opcode-header bytes, 1,962,236 numeric-operand
bytes and 2,191,362 string bytes. Leftovers: zero. The strings are not emitted
by the audit. The top-level Rust parser admits 8,579 operand records and rejects
43,218. Across 108 top-level `(opcode, flag, arity)` shapes, 26 are
fully admitted, fourteen mixed and 68 rejected. At opcode granularity 21
IDs admit all observed records, fourteen are mixed and 62 admit none.
Acceptance by this parser does not execute the opcode or validate its
runtime effects. Five admitted 0x4a operand records fail the subsequent
source geometry guard, leaving 8,574 code-and-resource candidates. The
mode-9 and scene-pan subsets require previously published scoped background state;
resource preflight alone does not establish reachable runtime coverage.

### Actor declaration prelude (0x30 / 0x31)

The stock `defChara.rld` consists of 34 character registrations followed
by 270 body definitions. The native 0x30 handler (0xb2750) retains
the fourteen-field CP932 actor record but assigns the v3 register
descriptor `[200+4*actor, 4]` (0x36ea0), not v1's eight-word layout.
IDs 3–36 and capacities up to 36 are admitted in this profile. This
descriptor assignment does not clear live numeric registers.

The native 0x31 handler (0xb2990) passes the 359-field mode-1 body to
`setBody` (0x3b1a0) and `setBodyCorrectPos` (0x37ac0), including
primary signed corrections and both authored -12345 secondary
sentinels. Native body mode is 0–4. The 42 possible face tuples all have
the canonical default format in this corpus; the populated prefixes
total 2,352 faces. Native `setFace` (0x38d70) uses eye 0 and the default
mouth without advancing the rotating eye index. Hair/accessory paths,
custom face sequences, positioned actors and v3 voice remain separate.

The static test installs all 304 records into an isolated catalog in
source order, verifying each body has a registered actor and allocated
variant, and no duplicate occupies a slot. No scene is executed.
`def.rld+0x20f3` selects this prelude via ordinary `defChara,*,0`, but
native `cmdChangeScenario` (0x124e20) captures through `blockEntry`
before and after loading. The declaration support does not skip that
startup dependency or implement the transfer.

### Immediate portrait draw slots (0xfb/0xfc/0xfd)

The three `def.rld` registrations bind the ordinary actor-draw slots
0, 1 and 2 to renderer-native layer pairs 137/138, 139/140 and 141/142.
The command at 0xe5700 maps logical IDs 800/802/804 through
`cmdLid2lid` (0xdbec0); 0x38940 assigns the two lanes and clears
their active owner. Actual portraits are a separate path from regular
positioned character enter/leave (0x20–0x27).

116/360 `0xfc` records are admitted: the original 52 absolute
literal-coordinate (-12346) immediate draws, and 64 authored packed
positions (60 immediate visible effect 255, four retained hidden effect
253). Duration is zero. Native 0xcbae0 selects the inactive member via
0x36a40. At 0x3a6b0 effect 255 shows the new bitmap at level 16 and
removes the old lane; effect 253 retains the newly allocated lane at
level zero (0x185360) and also disposes of the old one. When allocating,
layer priority defaults to native layer ID (0x183520). The old portrait
remains installed during asynchronous preparation, and the two layer
changes commit atomically.

The six admitted packed words all take the centered-x / valid-bottom-y
branch at 0x38340, with a signed low 16-bit x offset and no camera
projection. X is half the nominal view width minus half the body width,
plus that offset and the actor/body x correction. Y is the valid
bottom minus the full body height, plus the y correction. Corrections
are the primary signed values retained by `setBodyCorrectPos`
(0x37ac0); rounding truncates the two half widths separately. The
stock EXE WinMain reads the 1024×576 configured view and its ordinary
geometry initializer 0x414c40 leaves the optional aspect rectangle empty
when not overridden. The resident constructor 0x6e610 then initializes
the full view as its valid rectangle. This bounded route uses that
stock geometry only and does not restore a dynamic viewport.

The native path builder at 0x37010 uses P000 and F in the actor/body
group folder. 0x386f0 and 0x151620 split the face atlas into horizontal
cells, and 0x150fe0 uses an unkeyed indexed copy into the authored
face destination. The scoped route covers 25 stock body profiles.
Multi-face bodies have matching indexed palettes, explicit 8-bit body
alpha, a color-only face atlas and an in-bounds destination. Native
one-face profiles load only P000, with no F atlas. The shared worker therefore applies exact
RGB replacement while retaining the body alpha, including transparent
and partially transparent pixels. Divergent palettes, face alpha,
clipped cells, palette-filter changes and accessory variants are not
assumed. The corpus reads all 116 placements and checks the actual
packed coordinates on pinned visible and hidden records, then verifies
one CPU-only result per distinct body profile (full independent source
pixel comparison for pairs). Synthetic tests exercise two independently
owned slots, failed replacement, immediate packed retained-hidden to
visible replacement, and retirement via the real owner layer API.

65/79 `0xfd` records are immediate 255 clears with no auxiliary flag,
move or timing operand. Native 0x5cc90 / 0x3b430 remove the current
lane only, then clear its descriptor. Other clear modes, other packed
bit layouts, and timed crossfades remain unsupported. In particular
the nine -12345 position reuse records all use effect 9 at 200 ms.
0xcbae0 retrieves the active descriptor's original packed word (and
literal coordinates when applicable); the renderer then needs its
effect-9 overlap alpha and fade protocol, not an immediate replacement.
Regular character animation, active portrait reconfiguration and
viewport/status serialization remain unsupported.

### Printer area definitions (0x49)

All six observed records use flag 0x91 and mode 0x00800001. The
handler at RVA 0x10ea00 reads 42 top-level fields; the last field
contains two nine-field CP932 font records, and character shadows have
their own semicolon separators. The six authored records are two
branches over three IDs. Font choice is not identical between branches;
the adapter installs only the branch that actually executes, replacing
the same ID only after image preparation succeeds.

The path-backed `setFrame` overload at 0x62e00 measures decoded art
instead of using the encoded nominal 768×140 frame extent. The three
installed GYU headers all declare 1024×147, and each image also decoded to
1024×147 through the shared registry in an isolated resource-only check.
The resulting backdrops are [0,429,1024,147]. The text and name rectangles
remain independent frame-local
rectangles, [180,40,680,120] and [160,14,300,28]. Direction, colors,
shadow offsets and both fonts are retained. No system register or live
layer is changed by area declaration. Synthetic shared-worker tests
exercise decoded rather than authored dimensions, branch replacement
and preservation of the previous catalog entry on a failed asset load.

The area declaration itself does not change viewport depth or draw text.

### Printer frame selection and visibility (0x48 / 0x12)

All 346 observed records evaluate `L9,0,-1` (flag 0x31). The adapter
resolves L9 through S15, uses the native area-0 fallback and treats -1 as
a forced rebind. If the resolved request is already S14's style, the
native closed fast path writes R77 without an allocation, while the
active fast path leaves it alone. A successful new selection preserves
upper S14 bits and writes R77 through LocalHolder.

`messageFrame` (0x10fae0) and `createLayer` (0x6b3a0) allocate native
slots 131/132/134: a hidden image backdrop, a body plane and a separate
name plane. Positions for the last two are frame-relative. The initial
DIB (0x15bfe0) fills index zero and starts with an identity grey palette
(0x152390); selection writes turquoise transparent key, separate font
colours in indices 1/2, then white and black in 3/4. Since the native
planes are initially all key pixels, CrySirror's visible transparent layers
are lossless for this state, and keep the separate font/palette owners.
Optional native index 6/7 override values are not pinned or guessed; a
later text writer must resolve these if its pixels depend on them.

The three layer bindings commit as one scoped batch. Failure preserves
the previously selected frame and register state. The active, ordinary
empty-printer replacement performs the observed instantaneous close
followed by rebinding and reopening using S21, without dropping its
active scope. Active negative aliases other than -1 are not admitted.

The 913 observed `0x12` records admit open/close with immediate, default
and literal-400 ms arguments. `enterMessage` (0xe9c20) and
`leaveMessage` (0xe9ff0) use S21/S22 for the default durations
and the low S4 byte as opacity only when its override bit is set.
S15+2 selects immediate vs timed entry for the admitted image-only
non-socket, non-group frames. Duplicate enter leaves the positive
depth unchanged. Leave forces depth zero and hides the image; a
timed leave restores the hidden frame to its authored level. Integer
fade interpolation (0xa720) can reach the descending endpoint early
by quantization. Startup key skip is owned/focused Ctrl; additional
mouse/keyboard inputs are not checked until 500 ms (0x71850/0x6ef40).

Body/name ownership remains separate across these transitions. No
glyph output, extended nesting, native group/animated window modes or
0x13 toolbar ownership is claimed. The startup checkpoint still needs
the other native camera/layer/control state before gameplay.

Toolbar 0x13 is not the printer's depth flag: the 2011 `UxLocalCtrl`
instance walks sixteen entries and uses an inverse hide count.
`enter` at 0xa2520 transitions from hidden count 1 to 0 using S19;
`leave` at 0xa3bc0 transitions from 0 to 1 using S20, updates hit
ownership, waits for its own fade channel, then hides each stored bound
layer. The supplied definitions include one 0x2c type-3 record and seven
separate 0x40/0x51 controls with their own action/atlas inputs. Do not
treat the printer open/close or v1 toolbar as an equivalent replacement.

### Ordinary looped music (0x32/0x33)

The 774 literal `0x32` starts cover 17 installed, single logical-stream
44.1 kHz stereo Vorbis tracks. `cmdPlayMusic` (0xd7330) passes the three
CSV indices as decoded PCM16 stereo bytes, multiplying by four. In the
supported records initial position is zero, the repeat starts at the
authored frame (including zero), and the -1 end is clamped to zero by
the native command, meaning the natural EOF. Sound transport at
0x19b540 retains the byte-counted repeat origin; at 0x199eb0 it rewinds
to that origin through the 176,400-byte/s seek path at 0x199d40.
`createMusicFilename` (0x50dc0) uses `res/s/m/mNN.ogg`. The music gallery
record with dynamic ID, duration and two register-derived loop bounds
is not admitted; the native additional codec hook is also not enabled.

`playMusic` (0x641d0) keeps requested music separate from the audible
instance, suppresses duplicate active IDs and uses S30 bit 16 for mute,
with packed left/right gains from S31. A successful owner publication
increments its saturating sound experience once (0x4d590). The two
music banks are separate from effects and ambience. A first fade-in
need not wait for the infinite new loop; a timed replacement waits
on old-lane retirement. All 130 authored `0x33` stop records are the
direct stop form; 127 immediate and three explicitly timed. The
second-operand natural-wait variant remains held. Shared transport
and gain lifetimes are implemented, not native sample-exact fade
interpolation.

The corpus checks the audio headers, logical stream framing and all
authored repeat positions against terminal Ogg granules. A separate
offline mixer test seeks a genuine stock source near its terminal
frame and checks its authored repeat region, signal presence and
scoped retirement. No original game, device or save is involved.

### Ordinary ambience (0x34/0x35/0x46)

The stock `def.rld` defines 14 ordinary single-lane WAV loops (all mode
2 and zero delay; the three other lanes are absent). All 62 play calls
resolve to nine of those definitions; all 27 stop calls are scoped to the
current ambience. The native definition and local hooks at 0xdd1f0,
0xf7ac0 and 0xf7b00 lead through 0xc8d20/0xc8d80 and 0xb2550/0xb25a0.
`playEnv` (0x69450) retains the requested ID and authored override levels
independently from its currently audible ambience. Native S30 bit 18 is
ambient mute. The two words of S32 supply master gain (left high, right
low); effect gain S33 is not used. Each explicit nonnegative per-side
level replaces that side's definition default. A duplicate active ID
does not reopen the source. The shared audio owner prepares the ordinary
loop on an alternating bank, publishes it, then retires the old bank.
For a nonzero replacement or stop the VM waits on old-lane retirement,
not the infinite new loop. Ordinary skip cancels the fade, not playback.
The tests verify request lifetimes, gains and the retirement gate against
a controlled host, not sample-exact native fade interpolation. Additional
native lanes, delay/nonnumeric modes and fallback ambience remain outside
this scope.

All fourteen stock source files were checked statically as 22,050 Hz
mono/stereo PCM16. No game, device playback, current-save serialization
or native crossfade comparison was run.

### Ordinary WAV effect lane (0x38–0x3a)

The 2011 wave effect uses `cmdPlaySEEngine` (0x1002b0), `playEffect`
(0x60720), `leaveEffect` (0x58510) and `waitEffect` (0x58450), not
the v1 register/gain convention. Flag 0x10 with low effect type 0
uses the padded numeric `res/s/e/*.wav` filename. This covers 370/374
asynchronous starts, 211/211 synchronous starts and 369/373 leaves.
The 66 distinct chosen asset IDs all resolve to a RIFF/WAVE header in
the stock tree; no in-game or physical audio was played.

Each slot keeps its independent shared audio channel. Native mute is
S30 bit 17; packed S33 supplies left high-word and right low-word gain,
then the shared 257-entry coefficient table (2011 table at RVA 0x2317f0
matches all entries). A repeated play stops the same lane and requests
the scoped asset from the shared worker. Leave `-1` stops immediately;
leave `0` waits then stops. The native polling loop at 0x171110 probes
every 100 ms; held Ctrl can cancel immediately, while held Enter, Space
and the primary mouse button are not checked until the sixth sleep.
Input belongs to the session, and editor/UI-owned input cannot cancel
the compatibility wait. Voice, alternate effect extension/expression,
positive-duration leave and omitted native fast-mode sources are not
claimed. Shared worker and protocol unit checks are not gameplay.

The integer table pair (`0x3d`, `0x3e`) admits its one definition and
one lookup. The definition is a literal multiline, session-owned map:
`createDB` at 0x11e4e0 clears the selected table, splits numeric values
on comma/space/tab, zero-fills up to eight columns and replaces duplicate
keys. It is not the separately named save-data database. `getDB` at
0x108a10 and the lookup handler at 0x10ca20 preserve existing registers
on a missing row. On a match, the eight source/destination pairs apply
in order, invalid columns supply zero and negative destinations are
ignored (LocalHolder at 0x36100). The observed register-key form is
implemented without enabling other native expressions.

`0xbd` admits 21/21 observed records, literal `2,1,id` for IDs 1–21.
The plain-item branch in `cmdSetDoneFlag` at 0xb4880 both increments
the bounded scene counter (0x4d560) and falls through to the independent
message bit mark (0x4d510, bitmap helper 0x16e930). No range, clear or
other-bank form is admitted. Unit coverage verifies saturation, negative
and out-of-range IDs, and independent scene/message capacities.

The shared DLT decoder does not implement the Midori VM animation family.
`0x73` (one view-and-exit record) and `0x74` (112 start records) pass
flag 0x41; all 112 starts use 16 comma-separated fields and declare zero
linked tracks. The 112 records exercise channels 0 and 1, multiple native
target layer IDs, levels, fade durations and flag 0x100 (103 records).
`0x75` (98 leaves) uses a distinct three-number representation, with
both choices of the last flag. Native enter and exit are at 0xde600 and
0xb2eb0. Implement asset/proxy selection, live target mapping, shared clock
and retirement semantics before admitting these records; do not reuse the
fixed Famisupi image path. No DLT op is admitted in this tranche.

The matching resident method constructor at RVA 0x13acf0 has 212 explicit
table assignments after its 267-slot default fill. The observed 97 IDs
have 85 explicit method routes, 11 direct switch routes and one default
slot (`0x2f`, an explicit label indexed by `blockId2Index` at 0xb2330).
`0x06` and `0x08` alias `cmdSetReg`; a simple disassembly pass over
one literal-store form missed those shared-register assignments. This is
native routing, not an implementation count.

| Nested entry | Distinct selectors | Admitted records | Rejected records |
| --- | ---: | ---: | ---: |
| `0x0b` command | 7 | 4 | 16 |
| `0xbf` action | 13 | 372 | 16 |

The seven 0x0b selectors all have a second numeric field of -1. Their
third numeric fields (declared counts) are 0 for 25/27/28, 2 for 30, 8
for 51, 4 for 57 and 6 for 60. These are distinctions in the observed
call records. Only the four selector 30 records are now admitted.

The admitted 0xbf selectors are 190 (1), 262 (366), 346 (2), 348 (2),
and 360 (1). The missing selectors and per-selector counts are in the
audit output; nested selector counts are not added to the top-level total.

## Exact startup dependency

The ordinary start at `start.rld+0x114` is a taken extended block
(`0x05`, flags byte `0x21`, auto ID, false target 11). The parser already
accepts its shape. Native `liteExec` (0x145e10) sets the block/title and
temporary load-skip state, then calls `blockEntry` (0x108d20) to **capture
the current scene**, not to load a save. It follows with `saveViewClip`
(0x5d0d0). That function writes R7/R8/R9: fallback for an explicitly empty
view clip is zero, packed viewport dimensions and 256, respectively.
It is not sufficient to implement these three registers alone.

| Observable state | Required native capture |
| --- | --- |
| R4 and R6, T1 | Printer/modal status and the conditional block/temporary skip state |
| T3–T8 | Scoped clipper, character palette, camera, printer socket, position correction, actor capture |
| R10–R28, R61/T204, R100–R147 | Current draw descriptor, sound-scene lanes, six character slots and their active state |
| T2 | Native logical path of the last successfully loaded RLD |
| R78/T6 and T201/T202/T203/T205/T207 | Current printer socket, cursor, printer/actor/draw and entry data |
| S16-indexed experience bank | Copied to system registers after flushing the local holder |

The old v1 checkpoint uses different character register bases, audio
serialization and view/metadata rules. The fresh v3 constructors prove
individual defaults but not the exact complete state entering
`start.rld` after the native viewport/definition prelude. In particular
the live camera/clip serialization, current background and positioned
actor descriptors and live layer state do not yet have complete owners in
this adapter. The bounded audio register image below is prepared separately. The v3 path deliberately does not
apply the v1 checkpoint or mark the first startup block executed.
A synthetic test verifies refusal does not modify the touched block and
register state.

The next ordinary instructions already visible in the static walk are
`0xbf/190` at +0x133 and unsupported `0xbf/250` at +0x148. Those are not
runtime progress; the startup capture still blocks execution.

### Native definition prelude and successful scenario identity (T2)

The installed 2011 `sc00.dll` `start` callback (0x2440) calls
`liteExec("rld\\def.rld", 0, 1, 0)` before saving the camera baseline,
creating the local controls, selecting the message frame and handing off
to `ExHIBIT` (0x2880). That callback uses the configured `[exec] entry`
in the editor path and the literal `rld\start.rld` in the release path;
the stock configuration also names `start`. The first actual definition
record is `def.rld+0x114`, the packed `0x47` debug definition
(`cmdDefDebug`, 0xe2fa0). The stock value 0x20000000 clears the
low-28-bit debug mode (0xb54d0), so it does not enable its optional
overlay. The definition program still includes unresolved native
control definitions and transfers before the later `start.rld` block. Neither phase may be replaced by pretending the other completed.

Native `liteLoad` (0xc5d50) writes the **logical path argument itself**
to T2 after successful load/header and command-table preparation, not
the install-prefixed path, filename stem or attempted transfer target.
The adapter now carries the actual asset path with its accepted async
RLD result and publishes it to T2 for the admitted 2011/v3 loader before
the first record. Mixed case and native separators survive; an enqueued
transfer, missing file or rejected header cannot replace the prior T2.
This does not execute the definition prelude or alter the full-block
guard. It also does not apply to v1 or admit late-profile payload paths.

The native successful-load path additionally restores its previously
captured camera baseline only when the requested basename matches the
title entry (0xb1850 and 0x5d610). That camera path is not part of T2
publication and remains held.

T201 is a **different native owner**: the saved cursor resource string,
not T2 or the scenario name. In the installed resident's `INIT_ENTRY`
(0x95510), the optional `res\g\gn\default.cur` branch can seed the
cursor owner and T201; the branch is not proof of a populated cursor in
this installation. Its renderer/lifecycle and checkpoint are not
implemented here.

### Live draw-character capture pieces (T205)

Native `saveDrawCharacter` (0x5c9a0/0x366e0) serializes eight ordered
12-word rows: tag zero, the two native layer IDs, current A/B index,
actor, body, face, packed positioning word, raw authored X/Y, and actual
layer X/Y. The supported portrait owner retains its raw coordinates
only on successful layer publication. An absolute pose keeps its
authored X/Y; a packed pose keeps the authored zero pair separately
from its nonzero composed position. Both visible and retained-hidden
draws own a committed row. During asynchronous preparation the
candidate is withheld, and failed decode leaves the last committed
description unchanged.

A settled immediate clear resets index/actor/body/face to -1, packed
position to 0x01010000 and actual coordinates to zero, but retains the
last raw authored X/Y; defining an inactive layer pair does the same.
Native 0x366b0 and 0x38940 do not initialize those raw words for a
never-drawn row, so they remain unknown rather than being zero-filled.
The detached per-word image is not written into script T205, and a
whole eight-row native string, restore, and the startup block
remain guarded. No literal T205 reference was found in the 379
stock script streams; this does not rule out indirect/native consumers.

The independently owned non-marker block `0x40` flag maps to
R4 mask 0x80 in `blockEntry` (0x108d20), without changing the
printer-open bit or supplying the local-control-depth bit. The captured
mask reflects only accepted conditional-block traversal. Rejected
taken checkpoints remain unexecuted.

### Scoped prelude debug and hit-wait definitions (0x47 / 0x0b:60)

The packed `0x47` at `def.rld+0x114` clears the low-28-bit debug
mode and writes all eight ordered, signed debug-register mapping pairs
(consumed by native `liteRegInfo`), even though
no debug overlay is installed. The supported ordinary path commits
these data to its independent runtime owner. Other debug modes, an
active overlay, and retail debug-output routing are not inferred.

Both hit-wait definitions (`def.rld+0x158f` and `+0x1a37`) take
native selector 60 and target the dedicated printer marker set up at
layer 136 by the native initializer (0x143470). The observed command
replaces its atlas with the actual 240×30 indexed GYU, frame count
eight, and supplied animation packet; mode bit 1 retains (980,542),
while relative mode clears the two stored positions (0x13b550).
The adapter retains the decoded scoped image with that definition
after successful preparation. It does not make the marker visible
during definition. A failed replacement cannot publish new state;
the candidate is withheld while decoding. The detached T202 uses
the exact `saveMark` order (0x420e0) with the native resource
separators and unmodified animation packet, not literal script writes.
The read-only census decodes all 7,200 stock atlas pixels through the
shared image worker and verifies transparent and visible texels. This
does not implement timed marker presentation, pointer wait ownership,
unseen program formats, or the unexecuted outer definition callback.

### Authored control resource bindings (0xbf:338 / 0x0b:57)

The stock `def.rld+0x224` conditional plain system-SE definition
(`action338`, 0x11b5d0) writes 19 consecutive IDs from slot zero.
The dotted selector list goes through `UxLocalCtrl::setCtrlSound`
(0xa4930) to the bounded 100-slot table (0x7efb0). An explicitly
disabled (-1) slot differs from an unconfigured slot. The scoped literal
true/false path replaces only addressed slots and does not start or
mute any active sound. Unseen slash/chain forms, conditional per-lane
entries, active click routing and different result-register forms stay
held. The installed non-negative ID resolves to an ordinary 22,050 Hz
mono WAV with 9,378 PCM frames.

`def.rld+0x6f7` uses selector 57 (0x13b550) to call `defParts`
(0xa48c0), which stores an asset path and authored anchor in a 32-slot
registry separate from control actions or sound. The stock part occupies
slot 5 at (736,374). The sibling `0x2c` mode 3 definition at
`def.rld+0x689` (0xb33a0) applies eight resource/position triples to
the same registry: authored slots 0 and 1 install their own paths at
(736,374), while -1 entries retain the previous slot untouched. The
three stock sources are distinct 286×38 indexed GYU files with separate
alpha. A repeated definition replaces only the selected slot, and no
layer is mounted or made interactive by the declaration. The static
census reads the audio container and checks all 32,604 decoded stock
part pixels through the shared image worker.

The next 12 definitions at `def.rld+0x10ce..0x11dc` are `0x6d`
(`cmdDefKey1`, 0xdd580). They map native key IDs through the 0xb4260
table (1 and 112 both resolve to F1; ID 13 maps to the separate native
right-button binding), retain the low three situation bits and exact
signed argument words, and apply bit 0x80's regular table and four
legacy-slot reset before installing the new definition. The one
textual F1 binding converts the semicolon-delimited action packet
to comma-delimited text without executing it. Replacing a native key
drops its old auxiliary packet; an invalid form does not mutate an
earlier definition. These are authored bindings, not physical input
routing, executable action 104, or Ctrl style/dialog creation. No
devices or original scenario execution are used. Native mount,
intermediate block capture and DEF→START scheduling are still held.

### Native audio portion of block capture (R61 and T204)

The pinned `initScenario` (0x143470) chooses normal register 61 for
sound-scene capture. During `blockEntry`, after flushing the local
holder, `storeScene` (0x6a7b0) delegates to 0x68750. The writer emits
R61 = 0x800000 and T204 as five semicolon-separated lane records,
prefixed by the lane count. Each record is
`0,id,2.left.right,flags,extra_count` (0x68350 and 0x56560).
The untouched descriptor initialized at 0x69210 has id -1,
levels 214/214 and flags 0, not the result of stopping a lane.

The 2011 BGM and ambience owners now supply native lane 0 and lane 2
from their **requested** authored state. This preserves a muted music
request, the two raw authored levels rather than mixer gains, and
negative ambience override sentinels. The scoped play flags are 65
(BGM) and 512 (ambience). An explicit stop writes BGM id -1,
0/0, flags 64 or ambience id -1, -1/-1, flags 512. These are distinct
from the fresh descriptor, even if no source was actually playing.
The ordinary one-shot effect route does not update these descriptors.
The unadmitted loop-effect, environment voice, and auxiliary SoundInfo
lanes retain their verified virgin values. The supported writers never
populate the optional list. Native per-request music seek/loop positions
are not encoded by this text record.

A diagnostic can expose the prepared register image but it cannot change
R61 or T204. Pending decoder publication and asynchronous retirement
make the image unavailable until settled. Tests exercise exact native
text for virgin, muted, authored-override and explicit-stop states,
assert that script registers remain unchanged, and check the live BGM
candidate with a real stock asset in the offline CPU loop test.
The remaining checkpoint producers still block +0x114; this change
does not enable scenario transfer, restore or save handling.

### Detached live status pieces (R4 bit 0, T207 and S16 span)

The printer portion of `blockEntry` (0x108d20) tests the positive
message-open depth at +3607 DWORDs in the resident object. The candidate
takes this bit from the current printer owner, not from the selected
area, the visible opacity, or whether any glyphs were drawn. It owns
R4 mask 1 only; local-controls depth stays unresolved. A separate
accepted block flag owner supplies R4 bit 7. The candidate omits the printer bit while its layer setup or
fade is unsettled. Independently, T202 comes from the backed authored
hit-wait marker after its definition runs, not from the current area or
unverified virgin marker defaults.

`saveEntryName` (0xb17e0) independently captures T207 as the current
eight named slots, with decimal slot numbers, dotted name separators
and explicit empty slots. The resident 0x4d8f0 writer reads S16 after
local flush and scene capture and materializes a separate consecutive
system-register image: four counts (CG, scene, sound, history DWORDs),
then the three counter banks in that order, then the packed message
history. The bitmap allocator (0x16ea50) rounds a minimum of one bit
to whole DWORDs; its byte-local MSB-first ordering is already owned by
the message history bank. No native save is involved.

The stock limits are 2,500 CG, 400 scene, 100 sound, and 48,000
message bits. They give 1,500 history DWORDs and a 4,504-register
span at the ordinary S16 base of 217 (exclusive end 4,721). The image
uses the *current* base, validates the whole range, and contains zeros
for absent values, not neighboring system registers. It reuses the
same proven V1 experience-image layout without changing its output.
Midori does not apply this image or the R4/T207 candidates until
the rest of the block capture is implemented.

## Entry names and scoped RNF state

`cmdExHIBIT` selector 30 (0x13b862) decodes two fields and copies a literal
name into one of eight indexed entry slots. All four ordinary records now
set that logical state; subsequent replacement is per slot. Native
`saveEntryName` (0xb17e0) serializes the table in stable order with dotted
slot/name pairs. The adapter can produce that representation but does not
pretend to have installed a complete startup checkpoint. The detached T207 image above is not a script register write.

The action250 follow-up isolates the statically verified literal `F`,
`R`, `S`, and `L` settings directives. Flags address the S15-relative
flag bank. `L` adds the S15 base. The native `R` case really falls through
to `S` (0xcce51–0xcce74), writing the same literal to both normal and
system registers without clearing local overrides. The scoped helper
accepts complete literal-only programs, checks every target before any
write, and rejects structural register changes or foreign directive
families.

This does not enable action250 in the VM. The two stock call sites need
`settings.init.rnf` in ordinary mode; it has 32 active directives: seven
`F`, nineteen `A`, and six `@`. The latter two include startup audio,
message timing and controls whose semantics cannot safely be skipped for
the flags. Unknown directives fail before any helper state change. All
0xbf admitted counts are unchanged. Action345 is also still unsupported:
its native temporary table captures layer x/y/opacity and restarts selected
move/fade channels, not a full scene snapshot.

### Character palette checkpoint (T4)

The Midori 2011 `0xa7` handler (0xca200) writes a palette target and
strength to character-manager fields 12 and 11. These are the same
UxAdvSystem fields used by `saveColorFilter` (0x52820 / 0x3e9d0).
Mode zero updates the stored filter without recoloring existing layers.
Future immediate type-8 indexed portrait draws take the filter from
that owner. Native `drawFaceForDraw` applies it once to the composed
palette (0x3a550 / 0x157260); the shared reader changes the already
matching body and face palettes before the RGB-only copy, preserving
independent alpha. All 16 set and four clear records are covered for
mode zero. The one mode-one record stays out: it also changes palette
on already-entered character layers, a different owner and transaction.

T4 capture emits `0,<unsigned target>,<positive strength>` from the live
owner, clears stale T4 when inactive, and performs no write when it is
already empty. Diagnostics expose a detached candidate; no script
register or actual save is written. This is not the independent
viewport/layer color-filter effect. R7–R9 still need the live viewport
clip owner (not the 0x50 magnify rectangle); T3 and T5 are not supplied with invented values. The later
printer-socket work below covers only observed transitions, not the prelude. The startup block at +0x114 remains held.


### Immediate background owner and detached R10–R28

Midori mode `0x4a/255` with no animation, crop, filter, extra options or
layer changes copies the source directly (0xcbe70 / 0x64f20 /
0x23ab0). The scoped form accepts ordinary opaque 1024×576 GYU sources
or the native high-bit 24-bit RGB solid, but not `-1` framebuffer reuse.
It prepares GYU through the shared CPU image worker and atomically
replaces the retained full-canvas background. Failed preparation keeps
the previously published layer and its live draw, without marking the
failed CG experienced. Solid fills do not mark the CG bank. Viewport
clip, general magnification and skip behavior are not projected onto
this mode.

Native `storeScene` reaches `saveGraphicInfo`'s layout through 0x57760
with base R10, widths in low bits / heights in high bits, and the
non-sentinel backing-surface bit set. The admitted 0x505f0 draw updates
the descriptor only after the copy. Constructor/reset 0x50670 initializes
the seven packed color words; no admitted Midori operation changes them.
R10–R28 are available as a detached candidate from a successfully
published background only, including during a replacement transaction.
An absent scene does not return a fabricated background. No script
normal registers, files or startup guard are modified by this projection.

Of 96 matching ordinary immediate operands, 4 are solid and 90 have
full-canvas opaque GYU backing (47 unique assets); 2 address a 1024×577
asset and fail the dimension guard. The ordinary no-crop mode 9 is
covered separately below.
R7–R9 still require the real viewport clip owner, T5 needs the camera
registry, and R78/T6 needs the independent printer-socket owner
(`savePrintSocket` 0x06ec0); it is not the 0xcb–0xcf RSV control window.
No commercial program, saves, or output devices were used.


### Full-canvas byte-exact background fade (0x4a/9)

The no-crop, no-filter, explicit-duration mode-9 draw has an isolated
background effect, not a transition over the whole screen. Native
0x76bd0 selects 16 levels when its level argument is zero. At 0x23ab0
the level is floor(elapsed × 16 / duration); 0x18890 and 0x0bb90 retain
a copy of the old background for mode 9 before rendering. The opaque
0x166f20 → 0x1651d0 path computes native byte RGB
`(previous × (16 − level) + incoming × level) >> 4`, from that same
previous image on every step. The shared image worker uses
`MaskPiecewise256` (level × 16) on two independently retained opaque
inputs. Only the background layer changes, foreground paint is untouched,
and stale asynchronous steps cannot publish. The previous checkpoint
and experience state remain current until final publication; intermediate
resources retire when replaced. The schedule follows CrySirror's logical
clock and does not claim native refresh-cadence parity.

The mode admits 1,271 operand forms. There are 49 native solid-color records
and 1,219 exact-size GYU references, with 330 distinct ordinary sources;
three references to 2048×1152 art remain rejected at image preflight.
An old background must already be published, and skip/viewport/crop,
general flagged operations and source scaling stay unsupported. The static
corpus check therefore reports asset-ready candidates, not 1,268
successfully executed game commands. The startup block, real viewport
clip, camera and the unverified initial printer socket remain separate and held.


### Retained 1:1 CG cropping and scene-only pan (0x4a / 0x50)

The authored `0x4a/255` and `0x4a/9` variants with flag 65536 and an
in-bounds 1024×576 rectangle retain the decoded opaque RGB source when
it is larger than the visible canvas. Branch 0x64f20 copies the chosen
rectangle with equal source/output sizes (the equal-dimension path at
0x16a8e0); the adapter preserves the decoded resource and uses the
shared RGB image copy, not normalized UV edges or filtering. The
R10–R28 image still records the *authored* draw rectangle. The corpus
covers 35 immediate and 1,436 timed retained-crop operand records.
A timed replacement retains the old visible canvas until the final
image and new backing commit together.

The separate scene-magnify command 0x50 (0xcc2a0) can now use a live
retained source for a pure unscaled pan. This subset keeps source and
destination at 1024×576, uses curve 0 or 2 with positive duration, and
requires no event clipper, blur, or HQ/terminal resampling. At 0x19220
and 0x169c30 the equal extents reduce the sampled frame to an exact
copy; the integer edge interpolation comes from 0x341d0. Each frame
samples the same immutable backing. On completion a later pan starts
at the actual committed crop; the authored draw descriptor and CG
experience do not change. A transition may skip stale render frames
as the shared host clock advances. Device cadence is not asserted.

Eight authored 0x50 tuples match this bounded parser shape, of which
several reference a previous retained backing within the same script.
These are runtime candidates, not gameplay executions. Out-of-bounds
targets, missing backing, active HQ/skip, larger or smaller crop
windows, event clippers and 0x51/0x52 remain excluded. R7–R9 represent
the independent viewport clip and are not supplied by scene
magnification. T5 camera, the initial printer socket and the complete start block
also remain held. Stock pixels were compared in the shared CPU worker
against both the initial and panned source rectangles; no games,
devices or saves were used.


### Printer socket and local-position work (R78 / T6 / T7)

The R78/T6 writer at native 0x108d20 reads the live index and the retained
serialized socket packet, not the selected message-area ID. In 0x11af40,
closed frame selection sets the index to zero even when it selects the
same known area, but does not overwrite the packet. An active
same-area request takes neither branch and keeps both. An active rebind
runs the ordinary leave then reopen path. Actual immediate/timed leave
at 0xe9ff0 resets the index and writes exactly
`0;0x40000000,-1,-1,0,0` as the no-position packet. That string is
preserved, not normalized to decimal or inferred from the empty glyph
planes. The detached candidate is withheld during pending publication
and portions that have not been observed remain absent. This does not
claim the original prelude's initial socket, synthetic numbered
placement, or one-shot restore/printing of a saved socket.

The independent T7 serializer (0xdab80) uses the actual
`LocalPosCorrector`. Successful `liteExec` activation first empties the
list and explicitly clears both global offsets (0x145fe0). The VM
implements that activation and the native 0x6c reset and `-12345` global
XY-set modes (0x10fa60 / 0xbf9c0). With no per-actor correction rows,
its detached register bytes are `0,0,x,y`. The stock 2011 scripts do
not contain 0x6c, so this does not increase sample admission numbers;
the reset/change/serialization contract is tested with synthetic RLD
commands. Per-actor correction rows, removal, movement application,
scenario transfer, and the complete start block are still unsupported.
No registers are written by these checkpoint producers and no
original game, device or save is used.
