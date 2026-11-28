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

## 2026-11-25 recruitment profile, portraits and character lighting

The original `SceneEdit.OnEndDlgOk` updates the stock profile, waits
90 updates, captures a 90 × 90 icon and a 630 × 890 character card, fades
out, and starts `KasizukiThumShot.ks`. That script supplies a separate pose
and camera. The last image is captured after the two 30-update waits. All
three PNGs use the subject GUID under the client’s `Save/Thumb` directory.
The installed Metal fixture checks the return label, names, UTF-16 input
limits, comment tabs, image sizes, transparent background and visible subject
pixels. The World image test separately checks ownership and frozen geometry
and interleaves a screen capture.

The installed standalone `LightMain.Start` and `OnLevelWasLoaded` zero both
ambient color and ambient intensity. `LightMain.Reset` uses Euler angles
(40, 180, 18), white directional light at 0.95 and shadow strength 0.098.
`SceneEdit`, `TitleMgr`, `SceneUserEditMain`, `StartDailyAPI` and
`SceneNetorareCheck` call that reset. Only the last of those sets 0.65
afterward. `BaseKagManager.TagLightMain` preserves the previous rotation or
intensity when the tag omits it. Evidence was read from the installed
`Assembly-CSharp.dll` and the serialized directional lights. Native Unity
shadow bias does not have a matching per-sun field in the shared raster
renderer.

An ordinary muted headless replay selected Muku from the recruitment menu,
confirmed her name and wrote all three PNGs. The initial follow-up replay
reached `OP_0002` and exposed the missing temporary broom at frame 10203.
The next replay processed the broom and reached the character-offset tag in
the same motion script.

### Standalone face and hair rendering

The installed Unity 5.6 `TBody.ChangeTex` / `InfinityColorTextureCache`
key each part colour by material **and texture property**. Muku's hair menu
assigns HAIR to the surface, shadow, highlight and diffuse ramp, then
HAIR_OUTLINE to the outline texture and its separate ramp. A single palette
per material let the last assignment replace all preceding ones. The
`resources.assets` fragment programs 9784 and 9750 respectively sample the
palette at `(source.r * 0.995, 0.5)` with source alpha, and extract red for
the R8 optimization. The importer now reproduces these per-texture operations
before layer compositing. The installed first-generation path retains its
material-wide palette.

Standalone shaders 9723 and 9765 use the original hair/ordinary toon forward
passes and pass 9732 for the textured outline. These names previously selected
the fallback material. The hull now samples its own texture and light ramp;
the surface uses its diffuse ramp. An image reused for base and outline gets
separate sRGB and raw GPU views. The embedded preview in `pre_muku.preset`
is a reference for the authored colour family, with a different camera and
lighting.

The 1920 × 1080 muted offscreen Metal replay (Apple M4 Max) captured
Muku at frame 9900 before and after with the same input track and camera.
The former purple fringe, green eyes and red cheeks changed to chestnut,
amber and a light complexion, respectively. This is a visual check against
the installed preset preview, not a pixel-perfect comparison with a running
Windows player. Local captures are in
`target/com3d2-validation/{before-photo,face-final,face-before-after}.png`.

The full opening input track passed the broom scene in `OP_0002` and stopped
at frame 12314 at `DaytimeMain.ks:10 *昼メニュー`. The standalone `SceneDaily`
daytime management controller was missing in that build. All seven standalone
installed-data UI tests passed, including the post-portrait return, the
temporary hand prop's right-hand bone, the temporary body morph and their
restoration.

The final desktop release reproduced the frame-9900 image pixel for pixel.
The release adapter library suite passed all 45 tests, the model importer
passed all five CM3D2 tests, and the first-generation title/settings/load
cancel fixture passed with the installed CM3D2 data.

### Standalone daytime and night management menus

The preceding opening replay reached the menu call at frame 12314. This pass
inspected the installed 1.0.4 managed assembly, Unity 5.6.4p2 scene and
KAG scripts directly. The tests advance no gameplay frames.

| Original evidence | Coverage |
| --- | --- |
| Level 3, DailyCtrl.menuButtons, DailyMgr.OnClickMenuButton | Resolve the 20 serialized UIButton references. Reflow active buttons and use the corresponding colliders. Day and night have different visibility rules. Night disables the casino until a casino facility exists. |
| DaytimeMain.ks, NightMain.ks, SceneMgr.GoToLabel, DailyAPI.EndNextScene | Resolve each visible scripted destination against the actual caller. The native controller uses 16 named destination attributes across all modes and a separate default label for Next. Schedule, Save and Load call local panel managers. |
| uGUITutorialPanel, tutorial_list.nei, tutorial prefab, system.arc and system2.arc | Open the seven day and three night pages, decode each page image, hit the page toggles and OK, isolate the underlying menu and track the first-open flag separately for each period. |
| StatusViewer, FacilityManager.SetUpFixedFacilityArray, Status.CheckTrophyMainMenu | Bind club name, grade, gauge, evaluation, cash, roster count, day and main business. The installed initial theater selects アイドル系. Renaming updates session state. The two facility-count thresholds award separate trophies. |
| ScheduleAPI.SlotEmptyCheck, BackupParamAccessor.BackupParam, Status.BackupScheduleSlot, Facility.UpdateAllocationMaidList | Prefer the stock leader on an empty schedule. Daytime Next records 32 morning fields for each occupied slot and backs up GUID, work IDs and both communication flags across 40 slots. Both shifts refresh facility assignments. Night preserves the morning backup. |

The focused suite contains three fixture tests and two tests using installed
assets with temporary writable storage. The installed UI test checks each
visible scripted target in its own KAG file, every tutorial page, the rename
dialog's hit isolation and a partially filled club gauge.

```sh
COM3D2_ROOT="$HOME/Downloads/game-com3d2" cargo test --offline --locked --release \
  -p crysirror-cm3d2 --features runtime --lib daily -- --include-ignored
```

This earlier menu pass stopped before the separate schedule, result and
save/load controllers. The following pass exercises the basic-training route.
Window appearance, physical input, IME and audio timing await runtime verification.

All five focused release tests passed on the installed corpus. The separate
release regression for standalone stock creation and live presets also
passed after the class experience change. Logs are in
`target/com3d2-validation/daily-verified.log` and
`target/com3d2-validation/daily-stock-regression.log`.

### Standalone basic training through the second day

2026-11-25. The local standalone COM3D2 1.0.4 installation uses Unity
5.6.4p2. The authored daytime menu opens a local schedule panel. Both shifts
are editable there. The night menu has its own Next action. Eight enabled
basic jobs in the installed table (2001–2008) award training progress and
have zero base pay.

| Original evidence | Implemented and exercised |
| --- | --- |
| Level 3 `ScheduleCtrl`, `MaidStatusListCtrl`, `ScheduleTaskViewer`, schedule category 51 and the character/task prefabs | Fill an empty row from stock and exclude staff already scheduled. Assign each shift, hover a task for its name, switch or clear the per-shift communication selection, and confirm the panel. Right-click removal uses the original OK/Cancel dialog. The only remaining leader cannot be removed. Hit tests use the serialized colliders, grid sizes and panel clip offsets. |
| `Schedule.WorkResultScene.Calc`, `ScheduleCalcAPI`, `schedule_define.nei` and the installed facility, recipe, class and correction tables | Settle occupied slots in order and guard each shift against duplicate settlement. A job advances to level 3 on its twentieth execution. The tests assert the communication multiplier, fatigue outcome, trait correction, class experience, facility level and a failed later slot with no earlier award committed. The unmodified training room earns 15 experience per employee and shift from its base recipes. |
| `ResultTrainingTask`, `ResultWorkCtrl`, `ScheduleMaidStatusUnit`, `ResultIncomeMgr/Ctrl` | Show the task and outcome icons and positive stat changes using the prefab’s two parameter grids. Display six income and evaluation fields from the first occupied slot's Start backup and live club status. A separate receipt fixture checks signed values and grouping, including the original double minus for a negative evaluation change. Result office prefabs, BGM and SE references resolve in the installation. |
| `GameMain.OnEndDay/OnStartDay`, `Status.ApplyScheduleSlotBackup`, `ScheduleAPI.DayStartManage` | Resolve fatigue and restore current HP and mind, restore morning GUIDs, work IDs and communication flags, choose the leader when needed, advance the day and reset the settlement flags. The KAG setters complete before subsequent script conditions run. |

Four short installed-data fixtures provide different checks.

- `installed_schedule_and_result_controls` drives the authored colliders with pointer press/release events. Two stock characters receive both shifts. The fixture checks selection, the removal dialog, task icons and result labels.
- `installed_eight_basic_jobs_on_both_shifts` assigns each of the eight jobs separately to each shift and checks its specific growth, work count and zero pay across 16 cases.
- `installed_training_settlement_and_second_day` checks work counts, experience, cumulative snapshots, facility growth, rollback and carryover into day two.
- `installed_management_script_callbacks` creates Muku through the retail TJS function and uses temporary writable storage. It starts at the return from the first daytime menu in `AllMain.ks:129`. The original script calls `ScheduleProcess` at lines 133 and 156 for gardening and piano. It passes daytime results, the night menu, night results, income, `EndDay`, the next date screen and `StartDay`, then reaches `DaytimeMain.ks` again with day 2. Each job counts once. The 1.0.4 filename substitution resolves personality placeholders from the installed personality table.

The script fixture returns from scene calls through their supplied labels and
consumes ordinary scenario waits without rendering. The widget fixture reads
prefabs and builds host resources independently. Both avoid the game process
and user saves.

```sh
COM3D2_ROOT="$HOME/Downloads/game-com3d2" cargo test --offline --locked --release \
  -p crysirror-cm3d2 --features runtime --lib -- --include-ignored
```

The release library run passed all 54 tests, with zero failures and zero ignored tests. The test binary linked against the runtime feature. This pass did not relink the desktop executable.

### Standalone default night work and paid facilities

2026-11-25, COM3D2 1.0.4. The original-script fixture `installed_default_night_work_to_second_day` leaves Muku's two initial assignments untouched, resolves the actual `YotogiMain.ks` candidate list, dismisses and then accepts the character picker's cancellation dialog, and follows the original callbacks through the two work receipts, income, `EndDay`, and the day-two daytime menu. Work 10000 remains assigned. Its played-work flag is set, the activity counter and revenue stay unchanged, and the night receipt records the cancellation. This exercises the management return path.

`installed_theater_selection_and_shift_receipts` uses the serialized work and room controls. Only the matching theater is selectable in the twelve-cell grid. One employee falls below the installed minimum of two and receives no wages or work count. Two employees in the same room earn sales and rating, record visitor counts, and advance facility experience. The per-guest results feed the club and individual receipts. `installed_roster_mouse_drag` checks swapping two occupied slots and moving into an empty slot using press, motion and release. These are pointer fixtures without a window.

The release library run including the installed-data tests passed 57/57 at this checkpoint. The local log is `/tmp/cry-com104-business-regression.log`.

### Standalone facility upgrades and native slots

2026-11-25, installed standalone COM3D2 1.0.4. Tests use scoped temporary
save storage. The script fixtures start at a management callback and use
the installed KAG scripts, tables and serialized scene controls.

| Behavior | Evidence |
| --- | --- |
| Manual upgrade work 3000 | `installed_upgrade_script_to_second_day` selects the employee and training room through the original `FacilityMain.ks` path, equips an owned material, confirms, checks the resulting room recipe and receipt, and reaches day two. `installed_manual_facility_upgrade` separately checks ownership, requirements and the cached result. The fixture now buys material 10 from the installed shop catalog before scheduling the upgrade. Shop UI purchase and cancellation are tested separately below. |
| Separate result panel and camera fades | `result_panel_fades_independently_of_camera` checks the work and income panels during their 250 ms transition. The surrounding camera transition takes 1 s. The installed assembly populates result rows and stamps synchronously. The managed work/income `StartAnimation` methods have no call sites in this profile. Window presentation and audible timing are unverified. |
| Paid facilities | `installed_theater_selection_and_shift_receipts` checks the selected employee's room, visible status with one and two workers, ratings and thumbnail. The grid uses the room detail component referenced by the installed level 3 `ScheduleFacillityPanelCtrl`. `installed_cafe_sales_follow_room_and_shift` checks sales work 3002 with two existing café rooms in temporary state. With the same two grade-one customers, the worker earns 160,000 when their colleague is elsewhere and 167,000 when both work together. Two understaffed separate rooms pay zero on actual shift settlement. Construction is outside this fixture. |
| Standalone save headers and comments | `installed_native_slot_catalog_and_comment` checks slot numbering on the title panel, date and status labels, page changes, the 25 UTF-16-unit comment limit, stale edits, and preservation of both relative bookmarks and the following bytes. |
| Daytime and night slot panel | Both menu actions now enter the local panel and return to the calling menu. `installed_daily_slot_modes_and_confirmed_deletion` drives the installed daily panel through save, load, slot selection and cancel. It checks that empty loads are inert, an unsupported overwrite preserves the old file, and right-click deletion removes only the confirmed temporary slot. These tests exercise the panel through pointer events without rendering a game window. |
| Player and facility data | `installed_managed_player_writer_matches_both_schedule_banks` and `installed_managed_writers_match_room_and_manager_bytes` compare Rust output with bytes produced by the installed `PlayerStatus.Status.Serialize`, `Facility.Serialize` and `FacilityManager.Serialize` methods, invoked on detached test objects. The reference records are 269, 102 and 177 bytes respectively. |
| Character appearance in slots | `native_stock_appearance_stops_at_the_first_employee_status` checks stock male properties, active slot, icon bytes, timestamp, RGBA and voice pitch, then the first employee's properties and all nine palette slots. Its property record contains two sub-property slots and a bone-length adjustment written by the installed `MaidProp.Serialize`. Truncation at every byte of the stock prefix fails inside the first bookmark. Empty rosters reach the script section. |

The slot reader works on detached data. Loading a character roster, reattaching
its models and restoring KAG, desk, scenario and other managers as a single
operation remain open. The game has not been launched during these checks.

The final release library run passed **66/66**, including every installed-data
test. The local log is `/tmp/cry-com104-management-final.log`.

```sh
COM3D2_ROOT="$HOME/Downloads/game-com3d2" cargo test --release --offline --locked \
  -p crysirror-cm3d2 --features runtime --lib -- --include-ignored
```

### Standalone shop and management scene coverage

2026-11-25, standalone 1.0.4. Both `DaytimeMain.ks` and `NightMain.ks`
call `SceneShop` from `*ショップ`. Day supplies background ID 0, and night
uses the same default. BuildSettings resolves the scene to level8. The
serialized panels contain three UIGrids, with cell heights 64, 64 and 110.
The scene uses `GameInShop`, `GameInShopMain`, `ShopItem` and `ItemInfoWnd`.
The installed base tables contain 64 individual articles, 8 bundles and
34 event items enabled by the base ID list. Entry checks club grade,
building flags, group membership and background before exposing the lineup.

| Release fixture | Observed behavior |
| --- | --- |
| `installed_shop_day_and_night_callers` | Starts at each installed shop label with a detached Muku roster. The ordinary scene loads the dance room, stock actor, camera, idle motion and shop widgets. Press/release on the category and item controls previews a skirt and restores its original property. Cancelling the material purchase leaves 30,000 CR and no material. Confirming debits 30,000 once, grants material 10, and shows the receipt. Leaving reaches a new `SceneDaily` call of the correct day/night type. |
| `installed_shop_controls_and_ownership` | Checks installed collider bounds, hover description, working-funds label and a disabled purchase at 29,999 CR. After purchase the material has its own ownership flag and the event item's lineup entry retains its original status. A second purchase is rejected. Buying an individual garment reduces the bundle price and completing the set unlocks the editor card. |
| `installed_upgrade_script_to_second_day` | Buys shop item 10000 through the purchase logic, schedules work 3000, applies a room recipe using material 10 and reaches day two through the installed scripts. The shop visit and the upgrade route have separate short fixtures. |

A static scan of explicit SceneCall names found 14 in `DaytimeMain.ks` and
15 in `NightMain.ks`, with 16 unique names across the two scripts. The
standalone dispatcher has entries for `SceneCharacterSelect`, `SceneDaily`,
`SceneEdit`, `SceneShop` and `SceneUserEdit`. Individual modes within those
controllers require separate coverage. The 11 destinations still missing
from this dispatcher are `SceneCasino`, `SceneCompetitiveShow`,
`SceneCreativeRoom`, `SceneDanceSelect`, `SceneDeskCustomize`,
`SceneFacilityManagement`, `SceneMaidManagement`, `ScenePhotoMode`,
`SceneScenarioSelect`, `SceneStaffRoll` and `SceneTrophy`. This count covers
direct destinations only. For example, `FacilityMain.ks` separately calls
the implemented `SceneFacilityPowerUp`.

The original construction controller checks pending costs before accepting
an order. It queues a replacement (or an empty slot for demolition) with
`FacilityManager.SetNextDayFacility`. `UpdateNextDayFacility` charges the
cost and replaces the facility if the funds are still available; otherwise
it discards the order. This is separate from work 3000. Construction,
whole native slots, shop scroll dragging, per-hair fitting adjustments and
windowed visual/timing parity remain open.

The final release library run passed 68 tests, with zero failures or ignored
tests. The standard desktop release executable linked and its detect-only
entry recognized this installation as COM3D2. No full game replay or
windowed input was run for this pass.

To repeat the installed release fixtures, set `COM3D2_ROOT` to the install
directory. For an installation wrapped in bzip2, set
`COM3D2_MOTION_ARCHIVE` to the absolute path of the seekable motion archive
already prepared by the player. The fixtures write saves only under a
temporary directory.

```sh
cargo test --release --locked --offline -p crysirror-cm3d2 --features runtime --lib -- --include-ignored
```

### Standalone management component inventory and facility construction

2026-11-25. Scope: the union of direct scene destinations in the installed
1.0.4 `DaytimeMain.ks` and `NightMain.ks`, plus the facility upgrade called by
`FacilityMain.ks`. The component names below come from the MonoScript references
on the serialized levels, the managed assembly and, for `FacilityDetailsUI`,
the component the original controller adds at runtime. A shared component
on another level does not establish that a scene's controller works.

| Original component and scene | In the standalone adapter before this construction pass | Observable behavior now |
| --- | --- | --- |
| `DailyMgr`, `DailyCtrl` (level 3) | Day/night menu, tutorial and original label selection exercised | Facility Management can dispatch to level 47. Both menus have their own script return label. |
| `ScheduleMgr`, `ScheduleCtrl`, `ScheduleFacillityPanelCtrl` (level 3) | Both shifts, ordinary tasks and assignment to an existing paid room exercised | A newly constructed room keeps its physical slot when offered for allocation, including gaps. |
| `ResultWorkCtrl/Mgr`, `ResultIncomeCtrl/Mgr` (level 3) | Default night, basic training and paid work have installed day-two fixtures | The previous settlement fixtures remain regression checks. |
| `GameInShop`, `GameInShopMain`, `ShopItem`, `ItemInfoWnd` (level 8) | Purchase/cancel, equipment preview/restore, price and ownership exercised | The previous installed shop fixtures cover both callers. Continuous scroll dragging and the individual hair adjustments remain open. |
| `SceneFacilityManagement` (level 47) | The standalone KAG dispatcher rejected the entire destination | Select among twelve physical slots, queue creation, replacement or demolition, cancel or reset. Fixed training facilities reject replacement. The installed room and available type lists have separate native toggle callbacks and use different data. |
| `FacilityDataTable`, `FacilityManager` | Fixed room initialization, assignment and manual upgrades existed. No construction order was executed | A reservation leaves the current room and money alone. At the next day start, recheck funds per order, debit in order and install the room in its physical slot. An unaffordable order drops without charging. Demolition costs zero. |
| `FacilityInfoUI`, nested `FacilityDetailsUI`, `uGUIListViewer` (level 47) | The schedule panel had a separate display. Level 47 had no controls | The physical room card shows a thumbnail, its name and reservation/deletion badge. Pencil edits target the displayed live or proposed instance. Empty edits keep the old name. The current-room inspector shows operating status and staff. The catalog shows minimum/maximum staff, construction cost, rank, and the type's recipes, categories, conditions and parameter bonuses. |
| `FacilityPowerUpManager`, `SceneFacilityPowerUp` | Work 3000, material selection, confirmation, receipt and day-two fixture | Existing manual upgrade/receipt fixtures also exercise the list layout after loading the new canvas. |
| `SaveAndLoadCtrl/Mgr` (level 3) | Slot listing, metadata, comment, delete and restore validation exist | Writing and restoring the complete standalone management state, including construction orders, remains open. |
| `SceneUserEditMain/Manager` (level 12), `SceneEdit` (level 5), `CharacterSelectMain/Manager` (level 1) | Player editing, ordinary stock editing and supported picker modes have entries | This pass does not extend their mode coverage. |

The following **eight destinations have no standalone dispatch entry**. Their
deserialized components narrow down which implementation is missing. This is
a count of destinations in the two scripts, not a count of all Unity
components or original methods.

| Level and destination | Original components whose scene behavior is missing |
| --- | --- |
| 7 · `SceneMaidManagement` | `MaidManagementMain` and character selection/status via `CharacterSelectManager`, `CharaSelectStatusMgr`, `StatusCtrl` |
| 23 · `SceneDeskCustomize` | `DeskCustomizeMain`, `DeskCustomizeWindowManager` |
| 49 · `SceneScenarioSelect` | `SceneScenarioSelect` and its character selection flow |
| 27 · `ScenePhotoMode` | `PhotoWindowManager`, `ObjectManagerWindow`; rendering a save portrait exercises a different path |
| 59 · `SceneCreativeRoom` | `CreativeRoomManager`, `CreativeRoomUIObjectSettings`, `CreativeRoomUISaveLoad` |
| 2 · `SceneCompetitiveShow` | `CompetitiveShowMgr`, `MaidExaminationMgr` |
| 21 · `SceneDanceSelect` | `DanceSelectManager` |
| 54 · `SceneCasino` | `SceneCasino`; the daily menu already evaluates whether the button is available |

The subsequent wardrobe pass below covers the fourth tab and editor-return
callback. Continuous scrollbar dragging, animated feedback, window appearance
and physical IME were not verified. The construction tests use an isolated
save root and the installed game assets. They do not write to a user slot.

| Installed release fixture | Observation |
| --- | --- |
| `installed_construction_controls_and_second_day` | Click protected slot, select empty slot 9, cancel then confirm café construction. The cost label reads 500,000. The existing-room list excludes the pending room. Browse catalog materials without reserving anything else. At day 2 the café occupies slot 9, cash is zero, and intervening slots remain empty. |
| `installed_construction_replacement_demolition_and_lost_funds` | Overwrite an existing order, reduce available funds after reservation, process two orders, demolish the surviving bar for zero, and preserve peak facility count. |
| `installed_room_name_edits_follow_live_and_reserved_instances` | Click the level's name-edit buttons. Commit Japanese text through the host broker, check trimming, blank input and Escape, then verify the queued café keeps its name after opening and the training room's new name reaches the upgrade selector. |
| `installed_daily_facility_entry_and_return` | From the daytime and nighttime scripts' own facility menu labels, exercise the actual game update/dispatch, open level 47, check its native entrance camera, press/release OK, wait 499 + 1 ms and return to each original management menu. |

The normal return fixture also reproduced a host error when the menu removed
the world-fade layer before its first draw. Submission now tracks ownership
after a successful frame and removes that layer once. The full release
library run against the installed corpus passed 72 tests, with zero
failures and zero ignored. This includes the old work-3000 selection and
shop-to-day-two tests. No complete game replay or window was started.
The standard desktop release binary linked with the workspace lockfile.
Its `play <install> --detect-only` entry selected the `com3d2` adapter
and the `COM3D2x64_Data` directory for the tested installation.

### Standalone facility uniforms

The fourth level-47 toggle calls `OpenFacilityCostumeSettingWindow`. Its list
resolves each of the twelve physical slots to its proposed room when an order
exists, or its live room otherwise. Pending demolitions drop out. Each room
keeps its own Heroine, Default or Edit selection. Switching modes preserves
the edited outfit. The default preview uses the thumbnail named by
`facility_default_costume.nei`.

Both installed callers jump through `label_edit`, issue `SceneEdit
facility_costume`, then re-enter level 47 with `update_costume`. The adapter
now takes this branch, gives the stock leader temporary clothing properties
from the room, filters the editor by `facility_costume_mpn_enable_list.nei`,
and writes changed temporary filenames back to the selected room on return.
The ordinary recruitment/name-confirmation flow remains independent.
The native custom portrait path uses a 630 × 890 card, a 90-update wait,
`SE022.ogg` and the room GUID under the scoped Thumb directory. Real GPU
capture and visual comparison for this particular editor mode are unverified.
The fixture originally covered clothing selection. The subsequent level-5 pass below adds the per-part enable/disable panel (`着衣設定`). In the original, `CostumePartsEnabledCtrl.LoadMaidPropData` reads each room’s forced flag. `SetButtonActive` writes that flag back, and the all/clear buttons change the flags across the list.

The installed `installed_facility_wardrobe_controls_and_props` fixture drives
level-47 toggle, list, mode and editor-button input. It compares the stock and
temporary skirt properties, then updates the proposed room, advances the day
and checks that the opened facility owns the edited outfit.
The installed release library suite passed all 73 tests, including the facility uniform fixture and the existing recruitment, management, and shop regressions.

### Standalone level 5 force settings and level 19 credits

The uniform editor's 着衣設定 category uses the installed category definitions,
button mapping, colliders and four-column table. There are 25 enabled
properties and 24 individual buttons. The hair-extension control shows the
ribbon icon as well. In 1.0.4, every individual click also writes the ribbon
flag. Select All and Deselect All call the same setter. Editing retains each
menu filename independently of its forced flag. During work an unforced slot
resets to the character's own clothing.

Both daily scripts call level 19 with a period-specific return label. Its
credits use the BGM and duration in `staffroll_data.nei`, staff entries and
stop marker in `staffroll_stafflist.nei`, and the label and texture widgets
from the level. Releasing Return starts the one-second exit fade. At natural
completion the scroll stops at the logo and the controller holds for nine
seconds before exiting. The standalone scene inventory now has seven direct
destinations with dispatch entries out of sixteen.

The installed release suite passed 74 tests, with zero failures and zero
ignored. The day and night credits fixture submitted the original scene's
scrolling text and an illustration to the host. It returned to the daytime
menu on Return release and the nighttime menu at natural completion. The
uniform fixture used the level's real hit rectangles to select all, clear
all, and disable the skirt. It checked all 25 flags, retained the selected
skirt filename after construction, assigned the character to the café, and
checked the actual temporary property after both calls to
`SetMaidCostumeFacility(0)`. No physical window or full-story replay formed
part of this verification.

### Standalone level 10 trophy collection

On the 1.0.4 installation, both management scripts call `SceneTrophy` with
their own return label. The dispatcher now handles eight of their sixteen
direct destinations. Level 10 uses the serialized NGUI category and trophy
grids, the shop category-button prefab and all three ranks of the trophy-row
prefab. The installed base tables enable 15 heroine and 20 system trophies.
The collection counts only enabled trophies towards each category and sums
points only for owned trophies. A missing Extra category hides its counter.
Each row uses its native title-mask flag and the current player's ownership
to choose its text, medal opacity, card thumbnail and clickable area.

The two-page tutorial uses the installed assets and player tutorial flag.
Card inspection uses the small and full card textures, the serialized
400 ms sine-eased fade, movement from the clicked thumbnail, and smooth
scaling from 12 percent. The list supports clipped dragging, thumb dragging
and row arrows. The card and tutorial isolate pointer input from the list.

`installed_awards_categories_cards_and_both_callers` exercises the installed
day and night scripts, first-visit tutorial, second help page, single owned
card, masked title data, counts and points, zoom geometry at half time,
dismissal, category switching, scrolling and each caller's return. It uses
an isolated management fixture and headless host. Desktop-window appearance,
inertial scrolling and physical input have not been compared against Unity.

The release library suite passed all 75 tests against the installation,
with zero failures and zero ignored. Both return labels reached their
original management menu in the fixture.

### Standalone management destinations after level 10

2026-11-25, standalone COM3D2 1.0.4. Of the eight destinations previously
missing from the day/night management dispatcher, four now have original-script
input fixtures. This brings dispatch to 12 of 16 direct destinations. The
following results concern ordinary management actions in the installed data.

| Level and original controller | Installed-data release fixture and observed result | Outstanding coverage |
| --- | --- | --- |
| 49, `SceneScenarioSelect` | `installed_event_list_activation_and_return` takes the night cancel path back to its own menu. The day caller presents event 10 for an eligible Muku. Clicking the plate and confirming publishes `A1_KAN_0001` and `*top` to KAG. The adapter reads the event, condition and character lists and the serialized office/music settings. | Other event combinations, simultaneous fixed casts, physical appearance and full event playback. |
| 7, `MaidManagementMain` | `installed_personnel_buttons_and_both_callers` checks the night hiring control is absent, edit is available, and cancel returns to the night menu. In the day caller, confirm on the installed 3,000,000 CR offer takes that exact amount, creates a second stock character in active slot zero and resumes the caller. | Status and class panels, thumbnail display, complete scrolling, dialogs and the editor round trip. Dismissal and reassignment of the leader have state handlers but no installed click fixture. |
| 21, `DanceSelectManager` and `DanceSelect` | `installed_free_dance_song_cast_and_cancel` uses the night KAG free-dance entry, chooses viewing mode, clicks a song and a stock employee, and checks the original scene name in `tf`, the active actor and the cast count. Three back clicks unwind cast, song, and mode to the night menu. | Choreography playback, challenge and versus flows, dance settings, and visual transitions. |
| 23, `DeskCustomizeMain`, `DeskItemWindow` and `DeskManager` | `installed_desk_seasonal_prop_and_return` selects item 100, instantiates its installed mesh as a world entity, changes seasonal visibility, focuses the camera and returns to the office. Re-entering and clearing visibility removes the world entity. The installed catalog enables 13 prefab items in one category. | Numeric transform editing, manipulation handles, complete window controls, and original slot persistence. The fixture checks world submissions and session state. Pixel comparison remains open. |
| 27, `PhotoWindowManager` | No standalone dispatch. | Photo windows, object/character placement, save/load and screenshot actions. |
| 59, `MyRoomCustom.CreativeRoomManager` | No standalone dispatch. | Room composition, placement UI, native room files and return. |
| 2, `CompetitiveShowMgr` and `MaidExaminationMgr` | No standalone dispatch. | Character selection for the examination, eligibility, result presentation and continuation. |
| 54, `SceneCasino` | No standalone dispatch. | Facility and dealer selection, exchange UI, coin thresholds and menu script return. |

The fixtures drive an isolated management state with the original level data,
hotspots and scripts. They do not open a desktop window or write a player slot.

The final release library suite passed 79 tests with zero failed and zero
ignored using the installed assets. The crate also passed its default-feature
release check. The release desktop executable linked and `play --detect-only`
identified the standalone installation as `COM3D2`.

### Standalone title Config and static UI coverage

For the installed standalone 1.0.4, the six ordinary title choices in
`TitleMgr.OnClick` and the management script destinations are separate
inventories. Before this fix, Start, Continue, End and Shop had dispatch.
Config and Guest both reached the generic unsupported-title callback.
`TitleCtrl.OnConfig` opens the persistent `ConfigMgr` system panel over
the title. Guest calls `SaveAndLoadMgr.OpenLoadPanelKasiduki`, a distinct
load mode which still lacks standalone title dispatch.

The original `ConfigCtrl.Init` registers 32 callbacks, counting the two
directions of each switch separately. All 32 have bindings in the standalone
panel. The panel sends physical resolution and fullscreen requests through
shared UI commands, sets the shared presentation capture scale to 1×, 2× or
4×, and displays the host's measured presentation rate through shared text.
The settings panel is available from Title, the system gear and the ADV
message window.

| Installed callback family | CrySirror consumer |
| --- | --- |
| Resolution, fullscreen, capture magnification, FPS | Shared UI state and desktop presentation. Resolution choices use the native preset sizes, reported display size and current client size. The desktop uses borderless fullscreen. |
| Anti-aliasing, shadow quality, texture quality, bloom | Shared PBR environment and raster quality. Sample counts fall back to the highest supported count at or below the selection. Shadow tiers select the cascade map size or disable sun shadows. Texture tiers control sampler anisotropy, and bloom scales the CrySirror postprocess. |
| Player model and message transparency | Shared character materials and message window layers. |
| System button visibility, text speed, auto speed, sequential voice | The shortcut follows the visibility preference. ADV reveals text at the native grapheme interval, completes the current line on the first click, waits at the native auto interval, and queries or stops the shared voice channel according to the sequential-voice switch. |
| Category volumes, dance volume, four 3D toggles, special SE switch | Shared channel gains, the independent dance mix group and spatial processing on playing and newly prepared sources. The native special SE predicate runs before scheduling `SE016.ogg`. |

These bindings follow the installed `ConfigCtrl`, `CMSystem`, `SoundMgr`,
`AudioSourceMgr`, `MessageClass`, `ADVKagManager` and `SystemShortcut`.
The dance slider updates the shared mix group. See the later
[DanceMain playback record](#free-mode-and-dancemain-2026-11-26) for
the dance scene's use of that group.
Settings live for the current session. Native settings persistence remains open.

For a wider static count, enumerate the 5,824 indexed `.ks` filenames in
the installed base `GameData/script.arc`. Inspect line-command and bracket
forms of `SceneCall` outside comment/label lines, accepting literal
`name=Scene…` arguments. Of 29 unique literal targets, 17 match the
standalone KAG scene dispatcher. The other 12 are:

| Family | Literal targets without standalone dispatch |
| --- | --- |
| Management and editors | `SceneCompetitiveShow`, `SceneCreativeRoom`, `ScenePhotoMode` |
| Casino | `SceneCasino`, `SceneCasinoShop`, `SceneCasinoSlot`, `SceneBlackJack` |
| Other destinations | `SceneFreeModeSelect`, `SceneKasizukiMainMenu`, `SceneToTitle`, `SceneYotogi`, `SceneYotogiOld` |

Eleven indexed scripts contain a nonliteral scene name. The count excludes
`script_old_2.arc`, indirect calls from managed components and controls
inside a dispatched scene. It does not establish reachability in any one
playthrough. The four unresolved direct day/night management destinations
are included in the 12. In addition, level 7 explicitly rejects its status
and class panels. No exhaustive count of all nested controls is available.

The release library suite passed all 80 tests, including
`installed_title_config_modal_sound_and_return`. That fixture clicks the
title Config button, checks the changed mixer value, attempts a click on
the underlying Start button, and closes and reopens the panel to check
the thumb position. In two muted headless desktop runs from the normal
`play` entry, frame 810 showed SceneTitle with Config open (157 widgets).
After clicking the master slider and native OK, frame 852 showed SceneTitle
with Config closed (13 widgets) and focus on Config. Both runs used isolated
writable data and the installed 1.0.4 game files.

The 2026-11-26 release regression passed all 92 tests, including the
installed `installed_title_config_remaining_controls_publish_engine_state`
and `installed_config_text_speed_drives_reveal_and_auto` fixtures. The first
clicks all 24 newly bound controls and checks shared window requests, scene
quality, the FPS layer and the dance gain group. The second uses the
installed opening KAG script to check reveal completion, click consumption and
the auto deadline. The shared PCM test
`channel_spatial_control_updates_pcm_without_reopening_transport` checks
the active voice's stereo output and retained playback position.

### Standalone management menu coverage, 16 direct destinations

The standalone adapter now dispatches the four remaining destinations in
`DaytimeMain.ks` and `NightMain.ks`. Rechecking the installed script archive
against both dispatch tables gives **16 of 16** distinct literal scene names
in those two scripts and **21 of 29** across the archive. Each fixture
starts at the installed day or night menu label and clicks the serialized
management button with a temporary player state.

| Installed scene | Interaction covered by the fixture |
| --- | --- |
| 27, photography | Load the first available photo backdrop as geometry, hide and restore the toolbar, switch to the night rotary backdrop, cancel the exit dialog and then confirm it. |
| 59, creative room | Build the default room from ten boundary quads. Use the installed size sliders, save a rectangular room into a temporary native slot, edit another axis and reload the saved geometry. |
| 2, competitive show | Follow the single eligible entrant through character selection, timed judging, the installed personality motion and the acknowledgment that resumes the caller. |
| 54, casino | Build the casino through facility management, select a dealer, check the insufficient-chip dialog, cancel one exchange, then buy 310 chips for 310,000 CR and leave the casino. |

Scope below these entries includes photo posing and composition files, placed
objects and custom panel materials in room saves, additional competitive-show
award combinations, and the separate blackjack, slot and casino shop scenes.
The room loader rejects unsupported object or material data before applying
it. Physical input and desktop pixels were outside this fixture run.

All 84 release library tests passed with installed data, including all four
new fixtures, with zero failures or ignored tests. Each fixture returned to
its calling management menu. The competitive-show fixture advances the
installed result dialogue by clicking at its KAG input waits. The standard
release desktop player linked and `play --detect-only` identified the
installation as `COM3D2`.

The eight literal targets still rejected by the standalone dispatcher are
`SceneBlackJack`, `SceneCasinoSlot` and `SceneCasinoShop` from the casino;
`SceneFreeModeSelect`, `SceneKasizukiMainMenu` and `SceneToTitle`; and
`SceneYotogi` and `SceneYotogiOld`. The older CM3D2 profile's
`SceneToTitle` and `SceneYotogi` handlers are separate dispatch paths.

### Standalone opening story, entrance and office

The installed `BaseKagManager.Initialize` registers 131 literal callback
names and `ADVKagManager.Initialize` adds eleven, for 142 distinct names.
At least 50 have no corresponding name in the standalone ADV dispatch or
the shared KAG parser, including 26 `vr*` names. For this lower bound,
even a string literal elsewhere in the dispatch code was counted as a match.
Scanning line and bracket commands in the 5,824 base `script.arc` scenarios
finds 14 of the 50 used directly. Examples include `faceblend2` (748 files),
`motionnextabs` (90) and `calldialog` (34). Neither `OP_0001a.ks` nor
`OP_0002.ks` directly invokes one of those 50. Dynamic calls, transitive
motion scripts, UI callbacks and native branches within a recognized command
are outside this count.

`OP_0001a.ks` loads `EmpireClub_Entrance` at line 57 and sets the main light at
line 63. It loads `ShinShitsumu` at line 180 and changes the light's rotation at
line 190. Both commands use intensity 0.95. The office prefab has ten distinct
materials. Its monitor uses Unity's `Unlit/Texture` and its light overlay uses
`Unlit/Transparent`. Both shaders have an empty outer asset name in this
installation. Their serialized ShaderLab names and fragment programs identify
the texture-only paths. The importer now preserves unlit rendering and the
overlay's alpha blend. The entrance material `EmpireClub_Entrance_001` also
binds a separate `_LightMap` sampled with a second UV set. See the hall
lighting verification below for the later importer change.

The installed story body label has a 785 × 82 rectangle with a 25-pixel font,
ClampContent and a maximum of three lines. The speaker label has its own
252 × 36 rectangle. The standalone label renderer now applies the authored
clamp and vertical spacing and rebuilds cached layers when the widget moves,
resizes or changes tint. `installed_story_room_and_dialogue_layout` loads
the original office and message widgets, checks the monitor, overlay and an
opaque wall, presents four explicit lines, and checks that only three draw.
Reducing the body to one line also removes the previous lines and moves and
recolors the first. The release library suite passed all 85 tests with the
installed data and zero ignored.

The rebuilt desktop player ran the original opening in silent headless mode
on M4 Max Metal with separate writable directories. At frame 2860 the entrance
showed the speaker above one line of dialogue (`OP_0001a:111`). At frame 4507
the office showed the uncle speaking three lines (`OP_0001a:363`). The three
text-layer origins were 958.18, 983.18 and 1008.18 on the 1080-pixel canvas.
The final office frame and the pre-change frame with identical inputs were
pixel-identical. Neither capture reproduced overlapping glyphs. The long-label
fixture exercises the overflow case separately. Local captures are
`target/com3d2-validation/story-entrance-final.png` and
`target/com3d2-validation/story-office-final.png`.

Three separate installed release tests also passed:
`com3d2_user_editor_from_title`, `com3d2_stock_editor_controls` and
`com3d2_stock_editor_portraits`. They exercise the player editor return
to the opening dialogue, the recruited character's edit and cancel actions,
and confirmation including three scoped portrait images and the script return.

### Standalone hall wall lights

The installed hall has 124 `EE_Light_alpha*` meshes. Their material uses
`Particles/Additive (Soft)` and a texture with an alpha channel. The
serialized blend factors are OneMinusDstColor and One. The fragment program
premultiplies RGB by alpha. The shared renderer now uses that blend equation
and draws these meshes with depth testing and without depth writes.
`EmpireClub_Entrance_001` now binds the original RGB lightmap and its
secondary UV stream. The material multiplies base color by baked lighting
and uses the shared sun's shadow visibility for the native shadow tint.

The standard desktop release player reached the same original script position
(`OP_0001a:111 *L11`, frame 2860) with the same input track as the baseline.
The 1920 × 1080 offscreen Metal capture shows soft lamp halos and the warm
wall lighting. The earlier capture shows opaque rectangles around the
lamps. Both runs used isolated writable directories. The comparison uses
`target/com3d2-validation/story-entrance-final.png` and
`target/com3d2-validation/story-entrance-lighting.png`. Windows pixel parity,
the shader's depth-faded particle variant and the reflective water material
remain outside this verification.

All 86 release library tests passed with the installed data, including
`installed_story_entrance_lighting` and the shop's day and night entry
fixtures. The importer binds RGB maps only where the material supplies
the map and the mesh supplies UV1. The shop curtain without UV1 keeps its
previous material path.

### Standalone return to title and casino destinations, 2026-11-26

Three more literal destinations now have installed-script scenarios, bringing
the base archive's dispatcher coverage to 24 of 29 distinct names.

| Destination | Exercised path |
| --- | --- |
| `SceneToTitle` (level 17) | At the native `AllMain.ks` title call, capture the scoped session checkpoint. Modify player money and execute the original credits-return label. The new parser restores the amount and reaches the original title call and its Start label. The checkpoint stays in memory while the standalone native v1040 slot writer remains open. |
| `SceneCasinoShop` (level 58) | Build and enter a casino through the original night menu, click ItemExchange, select the facility materials category, cancel a confirmation, then purchase the first material. Verify the coin debit, acquired material ID 30 and sold-out listing for item 1000. OK resumes the casino menu. |
| `SceneBlackJack` (level 55) | At an established same-day table, click the original casino button, open the installed bet panel, place the minimum bet, settle a hand and confirm the return to the casino. The rule fixture also checks native natural and split/double settlement. |

The casino menu now strips its full path prefix before matching the native
button names in `casinomenu_data.nei`. The blackjack controls show the hands
in the score labels. Native card and chip animation and the dealer's
scripted voice and motion presentation remain unverified. The shop records
ownership during the session and uses the original costume menu data for
fitting. The current installed purchase fixture covers a facility material.

The 90 release library tests passed with the installed 1.0.4 data, including
all three new routes and the prior day/night, portrait-layout, and lighting
fixtures. The suite was built with a temporary copy of the workspace files
and an independent target directory. The originals and writable game slots
were separate.
The standard desktop release build linked in the original checkout. Its
muted headless startup reached `AllMain.ks:11 *title` at frame 795 with
Continue focused.

The five remaining literal destinations are `SceneCasinoSlot`,
`SceneFreeModeSelect`, `SceneKasizukiMainMenu`, `SceneYotogi` and
`SceneYotogiOld`. Level 56's `SlotManager` and `Reel` use the original
`casino_slot_haitou.nei` and `casino_slot_byou.nei` data and separate
stop arbitration, chance mode and bonus state. Level 24 directs the new
and legacy recollection inventories and temporarily saves player-selected
character state. Level 60 has its own guest and save flow. Levels 14 and 70
consume distinct skill and stage data.


### Five additional standalone dispatch entries, 2026-11-26

| Literal entry | Source implementation and outstanding behavior |
| --- | --- |
| `SceneCasinoSlot` | Level 56's original cabinet hierarchy, button sprites, tutorial/help, and exchange/return controls. Reel stops follow the authored strips, first-stop weighted draw, forced BAR, cherry slip and sequential reach rejection. The scene publishes per-cell reel highlights, lever motion, additive lamp fades, credit/payout digits, jackpot and chance-mode sequences, and total-coin trophy accounting. |
| `SceneFreeModeSelect` | Level 24's category and recollection controls, current and linked predecessor catalogues, VIP work details, replay labels and flag/relation restoration. |
| `SceneKasizukiMainMenu` | Level 60's main menu, man and room selection, room-specific roster and scenario filtering, and authored room/man/scenario replay lists, descriptions and KAG return labels. The guest slot format, character detail and man-model refresh remain open. |
| `SceneYotogi` | Level 14's stage previews and selection, supporting-character picker, active roster assignment, and the `@yotogicall` stage, supporting-character, free skill and Null routes. |
| `SceneYotogiOld` | Level 70's separate stage-unit and free-skill prefabs use `linked_cm3d2` to load the original legacy tables and previews through client storage. Stage-list suffixes and prefab overrides come from `YotogiOld.CreateData`. The supporting-character picker uses the legacy manager's transfer and rental rules. |

Native references for the current update are `SceneCasinoSlot`,
`SlotManager`, `Reel`, `CasinoDataMgr.AddTotalCoin`,
`KasizukiMainMenu.SetUpPlayInfo`, `RoomData.Data`, `PlayData.Data`,
`YotogiStageSelectManager` and `YotogiStageUnit` from the installed 1.0.4
assembly, plus the serialized controls in levels 56 and 60. Verification
was static. Compilation, tests and gameplay runs were skipped at the
owner's request.

**Slot-machine evidence.** In level 56, the serialized controller points to
the `Slot_Machine` cabinet mesh. The machine transform is its parent,
`SlotMachine`, which also owns the lever and reels. The original controller
references three masked reel meshes and 36 other lamps. The installed
`Particles/AdditiveMaskUV` pixel shader rejects a fragment when its
scrolling V coordinate lies outside all three authored intervals. The
adapter translates those intervals and the native tint to the shared
surface hook. The other lamps use material opacity updates or the visibility
of the original unlit renderer. The unit fixtures assert third-reel reach
rejection and the separate bank and display events during payout.

### Guest selection and status, 2026-11-26

Level 60's `KasizukiManSelectCtrl.OnClickManListButton` changes the
selection, refreshes the two profile labels and updates the model.
`KasizukiManager.SetManModel` uses the character preset key from
`kasizuki_man_list.nei` for an NPC and restores the player's appearance
for the protagonist. The adapter uses the existing character assembler
and animation service for the original `man_porse01.anm` pose. Closing
the picker preserves the selected man. Leaving the scene releases the
preview and restores the previous player identity and actor slot.

`MaidMiniStatusCtrl` populates the installed labels for the selected
character. The work-count and affinity locks use the original hit boxes
and govern the existing result counters. The native guest comment input
uses the shared text broker and its 70 UTF-16-unit limit from level 60.
Comments are held by character GUID for the session, pending native
guest-slot persistence.

The installed release fixture `installed_guest_man_profile_and_model`
clicked the original man picker, switched to an enabled NPC and back
to the protagonist, checked both original description labels and the
assembled actor, opened the character status, toggled the work-count
lock and entered Japanese text through the shared broker. The full
library suite passed 95 tests with the installed data. No game process
or graphics window was started.

### Blackjack dealer idle, 2026-11-26

Level 55's serialized `BjMotionControl` uses `*待機` from
`work_002.ks` for Normal.WaitStartLabel. The dealer now starts there
and publishes the assembled actor through the shared world. The
installed blackjack fixture checks a visible animated actor in that
world and drives the controls through betting, settlement and return.
The final installed release library run passed all 95 tests, and the
standard arm64 desktop release build linked.

### Standalone native slot codec, 2026-11-26

The v1040 reader and writer follow `GameMain.Serialize` in the installed
1.0.4 assembly. They preserve the two CSV path lists and the transferred
character flag, then read both stock lists through the last character.
Employee status includes current and legacy class and skill records,
experience, work counters, flags, selected classes and the end marker.
The record order follows `CharacterMgr.Serialize`, `Maid.SerializeMisc`,
`Maid.SerializeBody` and `MaidStatus.Status.Serialize`.

The remainder includes `ScriptManager` and its `BaseKagManager` envelope,
desk placement transforms, scenario state 1.3, facility layout 5, guest
slots version 5 and casino state 1.1. Both bookmark distances are written
relative to the end of their own 64-bit fields. Each bookmarked section
is bounded on read. Casino dealers have no on-disk presence flag. The
reader uses the following native tag to distinguish an omitted record
from the two character indices. The native game determines presence from
the installed casino facility types.

Checked behavior:

- `employee_status_matches_installed_serialize` matches the installed
  `MaidStatus.Status.Serialize` output byte for byte on synthetic fields.
  It checks independent and shared legacy entries, high unsigned
  counters, optional legacy status and invalid end markers.
- `native_stock_roster_round_trips_two_employees_to_the_script_boundary`
  uses a native MaidProp reference with two sub-properties and a bone
  adjustment. It checks an empty roster, multiple employees, the exact
  KAG boundary and truncation at every byte of the stock fixture.
- `native_slot_round_trips_all_sections_and_relocates_bookmarks` checks
  byte equality for a synthetic complete container, using guest bytes
  generated by the installed `SaveData.Serialize` and
  `FacilityFlag.DataArray.Serialize`. After length-changing edits it
  reads the new bookmarks and verifies the updated guest and casino
  state. It also rejects shortened management, guest and casino data.
- The installed daily slot fixture confirms the reader through the
  actual Load confirmation control in scoped temporary storage.

The release library suite passed 97 tests, including all installed-data
fixtures. The run is recorded in `/tmp/crysirror-com3d2-slot-1040-final.log`.

This codec pass preserved KagScript checkpoint bytes. Its fixtures did not
contain a player save from the original game.

### Standalone native save and load menu, 2026-11-26

The daily Save confirmation captures the running standalone 1.0.4 state
and calls the scoped atomic writer. Title Continue and daily Load stage the
slot against the mounted data, reconstruct the roster and KAG parser, then
run the original script from its saved label. Read-label history bytes from
an imported checkpoint are retained. The checkpoint loader decodes TJS
literal values directly and checks saved KAG macro bodies against the
registered definitions.

`installed_native_slot_saves_and_restores_the_daily_menu` uses the actual
Save confirmation and the title's Load panel with an isolated save mount.
It opens the installed daytime KAG entry, creates a slot, changes the live
funds and removes the staff, then loads the slot from the title. It asserts
the original day, player funds, casino coins and shop sale flags, work
roster, employee identity, rebuilt model, and stored portrait pixels. The
loaded script reaches the same daytime menu. A truncated copy is rejected
before the live session is replaced.

`native_management_state_round_trip_retains_both_schedule_banks` covers
both native schedule backup banks and an unchanged player when dimensions
are invalid. `checkpoint_accepts_native_literals_and_rejects_tjs_execution`
covers nested literals and rejects function calls or assignments in slot data.

The release library suite passed all 100 tests, including the installed
fixtures. The local log is `/tmp/crysirror-com3d2-live-test-final.log`.

These fixtures use installed resources and temporary files. A player save
produced by the original executable and loading a CrySirror-written slot
in that executable remain unverified. Linked predecessor saves with
transferred staff use a different profile. Guest child-slot data is carried
through the standalone slot; this fixture exercises the standard Continue
selection.

### Title Guest and club slots, 2026-11-26

`TitleCtrl.OnGuestMode` and `SaveAndLoadCtrl.LoadDataOnKasizuli` select a
campaign slot before the guest sub-slot. The new-customer row starts
with the protagonist and an empty guest state. The selected guest or
new customer sets the campaign's guest-mode flag before executing the
saved KAG label. The installed daytime and night scripts then jump to
the guest script as authored. Cancelling the title's guest sub-slot
panel returns to the campaign list.

The guest club uses its serialized `SaveAndLoadPanel`, data unit and
page sprites. Save writes the native v5 guest record with the selected
customer, total play count, per-character flags and notes. Load reapplies
that record and refreshes the selected customer model. Guest writes use
the two v1040 bookmarks to replace the guest section and its end offset.
The campaign data and casino suffix are copied verbatim.

The fixture `installed_title_guest_and_club_slots_round_trip` uses the
installed title button and the original guest script with disposable
campaign slots. It checks title paging and cancel, new guest creation,
in-club Save and Load, and a second title load using the saved guest
record. It checks that the guest comment and work-count lock return,
that a second campaign slot is untouched and that removing the test
guest record reconstructs the original campaign slot bytes. It does
not launch the player or use the owner's saves.

All 101 release library tests passed, including the installed-data
fixtures. The log is `/tmp/cry-guest-live-final.log`.

### Standalone free-skill selectors, 2026-11-26

`FreeSkillSelect` handles both the scene's initial screen and the
`@YotogiCall` callback. Level 14 embeds the panel. Level 70 instantiates
`SceneFreeModeSelect/Prefab/FreeSkillSelect` under the authored parent
and reads its skills and stages from the linked CM3D2 namespace.
The two manager variants filter their respective owned-skill catalogues
by the original cast and character conditions. The widgets use the
authored tab and stage templates and grid bounds. The selector records
one base skill, level and stage in manager state. Confirm and Cancel
resume their own ADV labels through the Null screen contract.

`installed_free_skill_callbacks_and_layouts` executes the actual
`freemodemain.ks` callback and clicks the authored controls. It
verifies the initially selected skill, level change, cancel and
confirm cursor positions, and the selection retained on re-entry.
The fixture also loads the original level 70 controls and legacy
panel grids. `free_skill_profiles_keep_level_and_reject_disabled_stage`
checks both profile variants with a bounded catalogue, including
retention, a disabled stage and the category restriction.
The linked predecessor catalogue and transferred-character route
require a separate installation to verify. Verification stops at the
ADV return label, before the downstream Play and Result controllers.
All 103 release library tests passed with the installed-data fixtures.
The desktop release binary was relinked at 20:49 JST. The test and
build logs are `/tmp/cry-yotogi-skill-final.log` and
`/tmp/cry-yotogi-skill-desktop.log`.

### Free mode and DanceMain, 2026-11-26

The everyday recollection catalogue uses the campaign's `2.0モード`
flag. Legacy rows read the linked NEI namespace, use its base and
suffix enable tables and apply the local legacy exclusion list.
The original `Old_FreeModeMain.ks` runs `SetCompatibilityMode(true)`
immediately before calling the selected file. The existing ADV
call-depth boundary restores the normal resource namespace on return.
The standalone installation used here has no complete predecessor
archive set to exercise that call in a transferred campaign.

The VIP panel uses the installed `ScheduleTaskViewer`, category button,
night-work card and condition-row prefabs. Menu selection uses the
work category and finished history. Pointer hover shows the work
description and conditions. Confirm passes the personality-resolved
filename to the original ADV label. The installed fixture supplies
temporary finished work entries, checks the description and cancels
through the serialized control.

`DanceMain` reads the chosen scene's master audio track, character
animation starts, camera action cache, camera properties and key
events from the Unity take. The shared mixer positions the camera,
explicitly sampled character ANMs, facial cues and 30 Hz mouth
patterns. The master directional light and camera fades also follow
this clock. The sound uses the configured dance mix group. The
take-end callback stops the sound and returns to the original KAG label.
Separate effect-object tracks, Unity Animator triggers and rhythm-game
scoring retain their own implementation requirements.

The fixture reads the three installed takes in levels 4, 20 and 44.
It follows the original `NightMain.ks` dynamic SceneCall into the
Entrance to You scene, resolves the stage and the dancer's ANM,
advances decoded audio through the shared mixer, and checks equal
audio and animation positions. A shortened fixture duration exercises
the return to `NightMain.ks:143` and retirement of the audio channel.
All 105 release library tests passed. The desktop release binary was
relinked at 21:28 JST. The test and build logs are
`/tmp/cry-dance-final-adv.log` and `/tmp/cry-dance-desktop.log`.
