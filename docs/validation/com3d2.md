# Custom Order Maid 3D 2 standalone

## 2026-11-23 · Unity 5.6.4p2 retail startup

The local installation identifies itself in `COM3D2x64_Data/app.info` as
KISS / CUSTOM ORDER MAID 3D 2. Its `globalgamemanagers` uses serialized file
format 17. Detection produced 20 KISS WARC mounts from GameData and
GameData_20. GameUty, SceneLogo, SceneWarning, TitleMgr and TitleCtrl in
the installed managed assembly supply the archive order, input edges, fades,
camera and title transition. Evidence and private captures live under the
ignored `target/com3d2-validation` directory.

The standard release desktop player ran headless on Apple M4 Max / Metal at
1920×1080, with audio output disabled. The input sent Return down/up at
frames 100/102 and 285/287.

| Frame | Observable result |
| --- | --- |
| 90 | SceneLogo displays the installed KISS logo. |
| 245 | SceneWarning displays the retail caution image. |
| 750 | SceneTitle has 13 image widgets, 26 club mesh renderers and BGM004.ogg selected. The cursor marks Start. |
| 795 | A subsequent Down press moves the cursor to Continue. |

The 1920×1080 Metal readback shows the courtyard, fountain, night sky and
title overlay. Pointer movement onto Config changed the focused button.
Both a same-button mouse press/release on Start and Return resumed AllMain.ks at `*title_end` and
reported the first unsupported scene at line 43, `SceneNetorareCheck`.
Playing a complete new game, the other title actions, original saves, audio
device output and equivalent lighting to the Windows player remain unverified.


The installed-data Unity test `com3d2_boot_scene_and_streamed_logo` verified
the logo scene path, external stream, dimensions and transparent/colored
pixels. An independent CM3D2 1.00 desktop regression reached SceneLogo by
frame 120. The COM3D2 texture header census found 10,922 v1010 ARGB32
containers in parts.arc and 119 more in parts2.arc. These carry encoded
images. The older v1000 layout also occurs in 48 parts.arc textures.

The release owner-corpus test `com3d2_parts_use_shared_kiss_formats` parsed
all 550 meshes in the two parts archives. The main archive contains 56
v1000, 341 v2000 and 145 v2001 meshes; the second has eight v2001 meshes.
The shared reader consumes the optional per-bone local scale of v2001 and
passes it to the exported node. The same test resolved a representative
item menu's icon reference, a material's shader and properties, a keyed
animation track, and a v1010 texture decoded to 512×1024 pixels. These are
resource checks. Character assembly and motion playback in a COM3D2 game
scene still need validation.

## 2026-11-24 · Initial settings and standalone TJS

The original `SceneNetorareCheck` methods and `PlayerStatus.Status`
accessors establish the flag polarity. `EndNextScene` takes the On value of
the first and second visible rows, and inverts the On value of the third
(user-draft) row. It fades BGM for one second. On fade completion it reads
the saved SceneCall label and resumes KAG. The standalone TJS host reads
the same player flag dictionary as `GetManFlag`. `IsComPlayer()` inverts
`__isComPlayer`. The installed opening script uses this result to choose
between `OP_0001a` and `OP_0001`.

| Installed release-player input | Observed output |
| --- | --- |
| Return through both splash screens, then Start, frame 1100 | Initial settings displays 30 widgets and 22 static background meshes. The three visible groups start Off, Off and On. |
| Keep the defaults and click OK | All three stored flags are false. KAG reaches the same player-editor request. |
| Click On at (1420, 325), On at (1420, 560), Off at (1578, 825), then OK at (1820, 985) | All three stored flags are true, BGM stops, and KAG requests the player editor at `AllMain.ks:48 *user_edit`. |

The shared KAG/TJS parser runs the shipped scripts. The Rust standalone
bindings cover setup flags, player money, event items, club gauge and grade,
system flags, and shop lineup among other queries. The release unit regression
executes TJS expressions against mutable host state and checks the older
CM3D2 result for `SetManFlag`. The standalone version returns void.
System flag tests cover the active session. Native save serialization and
system-settings reload require separate evidence.

| Scope | Count |
| --- | ---: |
| Distinct names registered by retail `ScriptManager.Initialize` | 128 |
| Registered names with a Rust binding in the standalone profile | 69 |
| Registered names missing at the initial-settings milestone | 59 |
| Registered names appearing in source-call scans across 5,824 base archive scenarios and the TJS files | 78 |
| Of those source-call names, bound / missing at that milestone | 45 / 33 |

The scan ignores whole-line comments and counts textual call expressions.
It does not establish which routes actually execute. Of the 69 names, 49
also occur in CM3D2's table. Matching names alone do not establish matching
signatures or behavior. `SetManFlag`, `GetPhase`, and `GetSalonLoan` use
standalone overrides. The bootstrap, settings screen and controller return
were tested offscreen with audio output disabled. The subsequent editor,
opening playback, original saves, and retail lighting fidelity remain open.

The final offline, locked release build of `crysirror-desktop` linked the
standard `crysirror` binary. All 26 crate library tests passed with the
runtime feature. The unchanged CM3D2 1.00 installation still reached
`SceneLogo / ClickWait` at frame 120. Local evidence is in
`target/com3d2-validation/setup-final.png`, `setup-default-final.log` and
`setup-changed-final.log` under that same directory.

## 2026-11-24 · Standalone retail TJS dispatch

Compared the installed Unity 5.6.4p2 `Assembly-CSharp.dll` registration
calls with the three Rust dispatch tables. The executable registration
regression resolves each name in a fresh standalone TJS context.

| Scope | Initial settings milestone | Current |
| --- | ---: | ---: |
| Distinct registered names bound | 69 / 128 | 128 / 128 |
| Registered names in the textual source-call census bound | 45 / 78 | 78 / 78 |
| Registered names without a binding | 59 | 0 |

The source-call census covers 5,824 base installation KAG scenarios and their
associated TJS sources. It measures names appearing in call expressions.
The retail overrides for `GetMaidStatus`, `GetMaidFlag`, `SetMaidFlag` and
`SetContractType` have a regression for active slots, flags in compatibility
mode, relationship text, contract changes and the legacy affinity cap.
33 bindings still use the older CM3D2 function body without a retail
override. In particular, `CreateNewMaid` still depends on the older
character factory. The two exceptional retail method bodies are intentional.
`PresetLoadNPC` raises the native unimplemented-method error and
`SetSexualParam` returns void without changing state.

| Behavior | Inputs and assertions | Scope |
| --- | --- | --- |
| ADV title bar | A KAG fixture calls the TJS method against the installed SceneADV background and embedded font. Published layers move horizontally, fade and end transparent. | Actual serialized UI, fixture script |
| Preset and body properties | Read the Unity secretary TextAsset and an archive preset. Check numeric and named MPN reads, active-slot selection, limits and failure on an absent preset. | Actual preset bytes, fixture character |
| Character settings | Compare distinct active GUIDs, legacy-mode assertions, acquisition vs level, feature lookup, relation changes and parameter thresholds. | Installed tables, fixture characters |
| Important events | Changing affinity makes installed scenario 10 newly available. Repeated polling is quiet. Completing it for that maid removes the candidate and clearing the flag permits a new notification. | Installed scenario definitions, fixture roster |
| Scheduling and costume | Use slots 0 and 39, pair a trainee with the trainer, apply installed default and edited work costumes, then reset temporary slots. | Installed work definitions, fixture assignments |
| Activity queries | Verify dance selection and reset, rank and type matching, competitive queue copy/dequeue, casino all-ID checks, base vs owned skills, scene absence and VR tag presence. | Controlled controller state, installed skill and casino tables |
| Kasizuki | Check distinct character and system flag namespaces, work-count and affinity thresholds, room names, guest selection, and both sides of the prerequisites for installed row 1010. | Installed tables, fixture character and guest state |
| KAG compatibility | A nested KAG test switches the file scope and restores it on return. Standalone playback consumes queued sound operations. | Scoped archive fixtures and original startup playback |

Ordinary Start and confirmation still request `SceneUserEdit` at
`AllMain.ks:48 *user_edit`. The activity tests directly supply controller
state that later scenes would ordinarily create. Their results do not
establish that those scenes can be played, that an actual purchase or dance
updates the queried value, that a worn preset appears on screen, or that
the state survives a retail save round trip.

The standard offline, locked desktop release linked successfully. With audio
disabled and ordinary input replayed from the title, the release player
reached the three initial-settings radio groups at frame 1,100. Clicking OK
then returned to the original script. Frame 1,206 failed with an explicit
missing `SceneUserEdit` controller. The CM3D2 1.00 installation reached
`SceneLogo / ClickWait` at frame 120 in the same binary. These are headless
replays. Local logs are under `target/com3d2-validation/` in
`retail24-final-setup.log`, `retail24-final-game.log` and `retail24-final-old.log`.

With `COM3D2_ROOT` set to the installed game, all 34 release library tests
passed with the runtime feature. Both explicitly enabled standalone corpus
tests also passed: the animated ADV title bar and the shared KISS resource
readers for the installed parts archives. The logs are `retail24-release-lib.log`
and `retail24-owner-final.log` in the same directory.

## Standalone player editor · 2026-11-24

`AllMain.ks` calls `SceneUserEdit` at line 47 and waits at line 48. The
installed BuildSettings selects level12. Its two managed controllers read
the first-edit flag and caller label from KAG and drive the original NGUI
controls, player-name field, and stock man. Installed data contains a
single-page first-visit guide, `tutorial_daily02.tex`, an 800 × 800 DXT1
surface. An ordinary input regression opens the editor from the title.
The intended return target is `*user_edit_end`, followed by `OP_main_0001`.

The standard desktop release now accepts the installed `motion.arc.bz2` as
input to the existing KISS WARC reader. The source was 1,227,073,793 bytes
and the atomically published seekable cache was 2,432,450,301 bytes. On
the M4 Max the first run took about 74 seconds from starting decompression
to completing the launch mounts while a separate release test was building.
Subsequent replays reused the same cache. The asset ticket for
`man_porse01.anm` was ready, and the rendered body entity had the original
clip playing in a loop at weight 1 after replacing its head and body.

In a muted 1920 × 1080 Metal offscreen replay from the title, frame 1,460
showed the edited stock man with native sliced atlas edges on the name field,
category frames and sliders. The next capture at frame 1,475 shows the color
after scrolling and adjusting the three RGB channels (218, 164, 139 on the UI).
The scripted text presenter entered `新主人` through the focused native field.
At frame 1,950 the editor had returned and the
mansion, speaker and first line were visible (`OP_0001a:29 *L0`). Pressing
Return advanced to the narration at `OP_0001a:33 *L1` by frame 2,060.
The message and name widgets come from the persistent `_cm3d21` scene.
The installed-data regression also enters a name using the host text-commit
service and asserts that both it and the original opening line appear in
visible, opaque layers. It checks tutorial click isolation, selection of a
new head and body, opacity and abdomen changes. A separate replay of CM3D2
1.00 reached `SceneLogo` at frame 120.

Release tests passed for all 35 adapter library cases (the installed TJS
test was run separately), three installed COM3D2 corpus cases, three KISS
texture cases and the compressed-archive cache regression. The latter covers
random reads after reuse, truncated input and cancellation.

Local evidence in `target/com3d2-validation/` includes `resume-editor-layout.png`,
`resume-editor.png`, `resume-opening.png` and `final-*-verified.log`.
The release replay used ordinary input. Physical keyboard/IME composition,
re-entering the editor through the later daily menu and original save/reload
of the edited player were not exercised.

## 2026-11-24 · Opening cast and title actions

The Unity 5.6.4p2 standalone profile was checked against the installed
`Assembly-CSharp.dll`. `BaseKagManager.TagCharaActivate`,
`CharacterMgr.ActivateNpc` and `MotionKagManager.GetMaid/GetMan` determine
pool allocation and the interpretation of slot zero inside a motion script.
The retail `MaidParts` enum adds separate hair and skin outline colors.
The CM3D2 1.00 color block still contains seven entries.

| Path | Observable result |
| --- | --- |
| Start, settings, player editor, opening | An input-track replay loads the NPC in slot 4, composes the body and clothing, runs the authored bow and returns to dialogue. Frame 2860 shows the actor and the player's chosen name. |
| World-only fade | The installed regression advances through the first voiced NPC exchange, verifies animation on the body entity and checks a fully opaque world fade under the next narration at `*L15`. |
| Title End | The installed title selects End, opens the original Japanese confirmation dialog, isolates underlying controls and rejects a drag between choices. Cancel returns to the title after its fade, OK yields `Halted`. |
| Title Shop | A recording desktop provider received one request for the original standalone URL (`ctv=1&cmd=0`). It completed asynchronously. No external browser was launched by this test. |
| Award and body queries | Installed tables supply shop items, set cards and scenario awards. The test checks ownership before and after the retail acquisition method, case normalization, the compatibility guard and the unknown-item error. Slot queries exercise both enum names and numeric IDs. |

The standalone VM registers 128 TJS names. At that revision, 27 dispatched to
the older profile's implementations. Matching method names or managed wrapper
bodies do not verify their underlying data and controllers. The six retail
overrides added in this pass are `IsAvailableRanking`, `IsMVPEvent`,
`UpdateMaidRanking`, `GetManFlag`, `SetNewItem` and `IsItemSlot`.
`PlayerStatus.Status.isAvailableRanking` is false in this assembly, including
when the legacy unlock flag is set.

The 14,000-frame replay request stopped at frame 5882 in `OP_0001a:580 *L103`.
The game had activated NPCs of both sexes and reached the secretary's first
camera chase. `CameraChase.StartChase/Update` set the camera target from
the **current animated world position** of `Bip01 Neck` plus the authored
offset every frame for 500 ms. That release reported that operation
as unsupported. Later scene controllers, stock character creation, a linked
older installation and standalone slot round-trips were unverified at that point. The
direct replay used headless presentation and muted device output.

For native saves, the installed `GameMain.Serialize` writes magic
`COM3D2_SAVE`, version 1040 and a five-field preview header (timestamp,
day, player name, stock character count and comment). The original filename
is `SaveData/SaveDataNNN.save`. Two bookmark lengths delimit sections.
The original writer includes character and script managers, desk state,
scenario selection, facilities, Kasizuki and casino state. The older
`CM3D2_SAVE` writer has a different layout. No original slot file was
present in this standalone installation for an interoperability round-trip.

Local captures and structured replay logs are in `target/com3d2-validation/`.
The input track is `audit-opening-advance.json`. It replays the same title,
initial settings and editor inputs with dialogue advances thereafter.

Final release verification passed 39 library tests, five installed COM3D2
cases and the CM3D2 1.00 title/settings/load-cancel regression. The library
run includes a byte-level check of the legacy seven-color record and a
pooled-actor replacement check. The final desktop binary exited successfully
at frame 5800 (`OP_0001a:554 *L101`). The longer run with the same binary
reproduced the frame-5882 camera error above. Local 1920×1080 captures are
`npc-exchange-final.png`, `retail-through-5800.png` and `quit-dialog.png`.

## 2026-11-24 standalone opening choices and first day

The standard release desktop player used the original Unity 5.6.4p2
installation, 1920 × 1080 headless Metal and muted device output. The input
track starts at the logo and uses title, editor and scenario controls.
Each frame number below belongs to a specific input track.

| Stage | Observed result |
| --- | --- |
| Secretary at `OP_0001a *L103` | The installed-data test checks successive camera targets from the animated neck and successive local head rotations on the submitted body through `*L106`. |
| First choices, frame 8376 | Two original labels render inside the serialized scroll panel above the message. The default selection goes through the first branch. Down and Enter reach the distinct second-branch line at `OP_0001a:1122 *L186:2`. |
| Date screen, frame 9020 with input held back | The room is a loaded original background. Title, strip, numeral and day unit draw from the native layout and atlas. The game displays 1 while player status still holds 0. Enter dismisses the panel and returns to the original label. |
| First day before hiring | After `@StartDay`, player status holds day 1, the facility record is 2, and 29 night-work records qualify from the installed enabled tables. The three authored recruitment rows render in `OP_main_0001 *top2`. |
| 14,000 requested frames, default input | Playback stops at frame **9218**, `OP_main_0001:64 *choices_1`. Choosing the first candidate calls `CreateNewMaid("Muku")`, which reached the earlier game's factory in this build. |

Two additional input tracks selected the other recruitment rows. At frame 9330
they reached the same inherited factory at `OP_main_0001:91 *choices_2`
(Majime) and `OP_main_0001:118 *choices_3` (Rindere). The three log files
retain the failing call and runtime state.

Release validation passed 41 library tests, five installed COM3D2 tests
and the earlier CM3D2 title/settings/load-cancel regression.
The non-interactive tests also exercise disabled rows, pointer release outside
the pressed button, choice dismissal timing and scrolling to a keyboard-focused
row. The date replay exercises one of nine authored camera/background pairs.
Other backgrounds, populated day advancement, the native vignette, choice
history UI, full stock creation, title Continue/Guest/Config and v1040 slot
interoperability remain outside this validation.

Local evidence lives in `target/com3d2-validation/`. The captures are
`retail-choice-centered.png`, `retail-day-one.png` and `retail-first-hire.png`.
`verified-day-long.log` records the first unresolved runtime call. The standard
release build and the standalone library, installed-data and older-title
regression logs have the prefix `verified-day-`.


## Standalone stock and script controllers, 2026-11-24

The following checks invoke the standalone adapter with the installed Unity
and KISS data. Their assertions are independent of the older 9218-frame
opening replay above. All three original recruitment labels were checked against
the installed `OP_main_0001.ks`. They call the factory, load the personality
preset, run the synchronous property update and enter `SceneEdit` in main
mode with distinct return labels. The current binding table has 118 retail
handlers and 10 shared handlers among 128 registered names. Those counts
measure dispatch, not scene or save compatibility.

| Test | Observable assertion |
| --- | --- |
| `standalone_stock_creation_and_live_presets` | Muku, Majime and Rindere acquire distinct stock identities, initial classes and skills, active bodies and their installed presets. Named sub-character creation loads the associated portrait references. An invalid name leaves the previous stock and active slot intact. |
| `standalone_scheduled_cast_and_activation` | Marked communication, random basic work, reception and evening selections use the assigned slot and shift. Empty selection deactivates the character. Ranking and stock iteration resolve the expected identity and visibility. |
| `standalone_trophies_follow_contract_and_relation` | Earned awards change the owned set and enqueue the presentation once. Sub-characters are excluded from thresholds and compatibility-mode calls leave the set untouched. |
| `com3d2_stock_editor_controls` | A short in-memory KAG entry recruits Muku, loads the installed preset and opens the original stock editor. Its rendered layers include the earned trophy title. Pointer press/release changes the hair menu and body measurement, selects the family-name form and cancels confirmation. The shared World contains a visible stock actor. Its body mesh receives the edited `hara` blend weight, and the authored height slider changes the X scale of `Bip01 L Thigh_SCL_`. |
| `standalone_script_status_without_legacy_game_tables` | Temporary flags enforce their argument counts, experience changes address each bit independently, and transferred sensitivity reads the old status at the 1000 threshold. |

The release library suite passed 45 tests. The installed stock-editor and
older CM3D2 title/settings/load-cancel fixtures each passed separately.
The stock-editor fixture took 0.72 s after linking. Its four recruitment
operations are injected into the entry script; subsequent editor widgets,
TJS dispatch, model assembly, sounds and animations use installed resources.
When the host has no Arial binding, the built-in Unity font reference falls
back to the installation's Noto Sans CJK JP Regular face.

Release logs are under `target/com3d2-validation/`. This pass uses method and
UI fixtures. A complete post-recruitment story route, pixel/timing parity of the trophy animation,
profile photography and native save interoperability have separate acceptance
criteria.
