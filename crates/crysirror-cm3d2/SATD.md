# CM3D2 self-admitted technical debt

## SATD-cm3d2-game-logic

**Scope and current location.** The port of `Assembly-CSharp` game logic in
[`src/scenes.rs`](src/scenes.rs), [`src/kag.rs`](src/kag.rs) and
[`src/tjs.rs`](src/tjs.rs). Ported: scene start/update of `SceneFirst`,
`SceneLogo`, `SceneWarning`, `SceneADV`, `TitleMgr` (Start button);
ADV tags `scenecall`, `wait`, `motionblendenabled`, `eval`, `s`; TJS
functions `SetManFlag`, `GetManFlag`, `SetTmpFlag`, `GetTmpFlag`,
`SetNewTrophy`.

**Remaining work.** Scene-specific branches beyond the [installed-data runs](../../docs/validation/cm3d2.md) in `SceneUserEdit`, `SceneEdit`,
`SceneCharacterSelect`, `SceneDaily`, `SceneStartDaily`, `SceneYotogi`,
`SceneMaidManagement`, `SceneShop`, `SceneTrophy`, `SceneTryInfo`,
`SceneToTitle`, `SceneSingleEffect`, `SceneStaffRoll`, `SceneDance_DDFL`,
and `SceneCompetitiveShow`;
the remaining ~90 KAG tags of `BaseKagManager`/`ADVKagManager`/
`YotogiKagManager`/`MotionKagManager`, the 45 remaining TJS functions of
`ScriptManager.Initialize`, unverified message-window voice behavior, unported settings controls and save variants,
`CharacterMgr`/`Maid`/`TBody` (`.menu` assembly, parts, morphs, bone morphs,
physics hair, IK, eye/head look), `MaidParam`/`PlayerParam`/`Schedule`/
`Yotogi` state from the `.nei` tables, `SoundMgr` (BGM/SE/voice from
`sound.arc`/`voice*.arc`), and original `SaveData/*.save` compatibility.

**Closure evidence.** Installed-game headless runs through user edit, maid
creation, schedule days, yotogi, saves/loads and an ending, recorded in
[docs/validation/cm3d2.md](../../docs/validation/cm3d2.md).

| Date | Change | Remaining scope |
| --- | --- | --- |
| 2026-11-20 | First controllers, ADV KAG loop and TJS context; boot reaches the title with its 3D background. | Everything listed above. |
| 2026-11-20 | Title Start path, message window open/close/text, `talk`/`hitret`/`fade`/`fade3d`/`cameracontrol`/BGM/SE tags, `SoundMgr` volumes. The opening scenario reaches its first `hitret`. | Name entry (`SceneUserEdit`), typewriter/skip/auto/backlog/voice of `MessageWindowMgr`, delayed `playse`, and the rest listed above. |
| 2026-11-20 | Every registered tag of `BaseKagManager`, `ADVKagManager` and `MotionKagManager` (wait events, motion queue/layers/crossfade, items and temp props, masks/mask modes, face/blend/mouth/lip sync/kuchipaku, eye targets, IK targets, physics switches, prefabs on BG/characters, texture layers incl. `res:` textures, choices with `SelectButton` prefabs/`UIGrid`/`TweenAlpha`, voices, days), native `R_call`/`R_return`, `ReplacePersonal`; `MaidProp` (temp/normal files, `AllProcProp` re-dirty rules), `TBody.FixMaskFlag`, the `Maid.Update` face pipeline. The opening runs to `SceneCall SceneEdit`. | Applying head/eye look, IK, auto twist, hair/bust physics and character prefabs to the pose (needs the CPU skeleton); voice lip sync from audio level; `ScheduleAPI.DayStartManage`, `Maid.UpdateMaidRanking` and the full `MaidParam`/`PlayerParam`; Yotogi tags (`talkrepeat`, `voicewait`, `playbgmroom`); `SceneEdit` and later scenes; backlog/skip/auto/typewriter; saves. |
| 2026-11-20 | Typed `param.Status`/`param_player.Status` with their original `BinaryWriter` layouts ([`src/params.rs`](src/params.rs), [`src/binio.rs`](src/binio.rs)); `MaidParam`/`PlayerParam` operations ([`src/maid_param.rs`](src/maid_param.rs): `Initialize`, clamped setters with the original quirks, `feature_learn`/`propensity_learn`, class acquisition/bonus, seikeiken, skills, salon grade, items, trophies); every `.nei` decrypted at boot into [`src/gamedata.rs`](src/gamedata.rs) (class EXP, trophies, shop, salon grades, scenario items, yotogi skills, `MaidRandomName` from the installed Assembly-CSharp) and [`src/schedule_data.rs`](src/schedule_data.rs) (`ScheduleCSVData`); all 55 `ScriptManager` TJS functions with character-manager requests; `ProfileCtrl.UpdateProfileData` on the edit OK; SceneEdit OK → name call → confirmation. The opening, interview questions and first edit run to `SceneCall SceneStartDaily`. | `SceneStartDaily`, `SceneDaily` and the later scenes; `ScheduleAPI` (day start, work simulation, results); `UpdateProfileComment` text; whether `DLL_KAG_ClearCallStack` also clears the `R_call` stack (only the parser stack is cleared); `TrophyAchieveEffect`; Yotogi tags (`talkrepeat`, `voicewait`, `playbgmroom`); backlog/skip/auto/typewriter; saves (the parameter sections serialize; the save file frame and slots do not exist yet). |
| 2026-11-20 | `ScheduleAPI` ([`src/schedule.rs`](src/schedule.rs): slots, work availability and error checks, `DayStartManage`, random work, `DecideSuccess`, noon/night result simulation, `Correction`, `BackupParamAccessor`, `WorkResultScene` noon/night, result trophies); `SceneStartDaily` and `SceneDaily` (Daytime/Night menu with Next and the label buttons, `ResultWorkDaytime`/`ResultWorkNight` running the work results, `ResultIncome` with loan repayment and salon grade-up dialog) in [`src/daily.rs`](src/daily.rs); `StatusViewer`; TJS eval accepts a trailing `;`. A day runs from the date eyecatch through the noon result to the night menu. | `SceneCharacterSelect` (Yotogi/Vip/Communication/Dance) and the `WfScreen` framework, `SceneYotogi`, `SceneMaidManagement`, `SceneShop`, `SceneTrophy`; the schedule, save and load panels of the menu; the per-maid rows of the work results; maid thumbnails (`ThumShot`); the native KAG state blob of saves (`DLL_KAG_CreateSerializeData`); `MainCamera.Blur`/`Vignett`. |
| 2026-11-20 | `SceneYotogi` ([`src/yotogi.rs`](src/yotogi.rs)) over a generic `WfScreenManager`/`WfScreenChildren` ([`src/wf.rs`](src/wf.rs)): stage select (playable stages by best salon grade, thumbnails, BG/camera/BGM per stage), skill select (new-skill acquisition, category tabs, stage/condition/seikeiken/drunk filters, seven slots with HP cost and `only_one`, wait motion per personality/condition, costume edit label, swoon confirmation), play (`NextSkill`, valid commands per normal/insert state and skill level, `EvalExpression`, `ApplyExecCommandStatus` with corrections and study rate, propensity/erogenous-zone awakening, every normal/drunk/proclivity label rule, RC/RRC/RR classification, `yotogi_skill_call_main`/`command_call_main` dispatch, command factory groups, skill tower, parameter bars and scroll, repeat voice, control mask, undressing units, man transparency, swoon), result (skill EXP animation with level-up, parameter changes, acquired skills/propensities, `GIG` voices, yotogi trophies), `ApplyStatus` and `ResetWorld`; static tables in [`src/yotogi_data.rs`](src/yotogi_data.rs); `yotogi_kag` (`talk`/`hitret`/`talkrepeat`/`voicewait`/`playbgmroom`/`controlmask`, unregistered tags ignored as `KagScript.CallTag` does), `yotogicall`, `charaactivateleader`; `TagSceneCall` closes the message window; the file index descends dotted folders. A yotogi night runs from stage select through four skills and the result back to the daily scene. | `MaidManagement`, `Shop`, `Trophy` scenes; the schedule, save and load panels of the menu; the per-maid rows of the work results; maid thumbnails (`ThumShot`); saves (`GameMain.Serialize`: the `KagScript` blob is a read-history block, `KAGParser.store()` as a TJS struct string, a flag byte and the `KagStackCall` list — the KAG snapshot needs krkr line/position mapping); in yotogi, the head-bone camera target of `InitMaidGesture` (CPU skeleton pose), skill icon drag swapping and hover names, `UIWFTabButton` selected states, `HideScroll` window folding, `AttributeViewer` hover, `MulTexRemove` of only layers 10–20, `TrophyAchieveEffect`; `MainCamera.Blur`/`Vignett`. |
| 2026-11-21 | `SceneTrophy` (trophy list, totals), `SceneStaffRoll` (credits scroll and pictures of `Resources.LoadAll(SceneStaffRoll/Texture)`, Return skip, logo, label return), `SceneSingleEffect` with `startmvpeffect`; title Config/End; `MessageWindowMgr` skip/auto/voice/backlog/config buttons and the skip/auto advance; `ConfigMgr` panel (message, man transparency and volume sliders). | `SceneCompetitiveShow` (`CompetitiveShowMgr`/`MaidExaminationMgr`), `SceneDance_DDFL` (`DanceMain`, its Unity animation/audio timeline), `TrophyAchieveEffect`, `TutorialPanel`; config screen settings without a host counterpart (resolution, full screen, anti-aliasing, shadow/texture quality, bloom, capture size, 3D sound, sequential voice: recorded only), popup lists; skip ignores the read-history condition (`MsgAlreadySkip`) and the text speed has no typewriter to drive; maid thumbnails (`ThumShot`, needs an offscreen render of a second camera). |
| 2026-11-20 | Original saves ([`src/save.rs`](src/save.rs)): `GameMain.Serialize`/`Deserialize` with header, `CharacterMgr` (player, stock men, stock maids), `MaidProp` lists in `AddProp` order (`nFileNameRID` as Mono `String.GetHashCode`), `MULTI_COL`, `MaidParam`, `MAID_MISC`, and the `ScriptManager` KAG (`KagScript.Serialize` blob through the shared `krkr_store` CM3D2 profile and the plug-in's `Dictionary.toStructString`, `KagStackCall`); slot files `SaveData/SaveDataNNN.save`; `SaveAndLoadMgr`/`SaveAndLoadCtrl` panels from the daily menu and the title's Continue ([`src/saveload.rs`](src/saveload.rs)): pages, units, dialogs, save, load, cancel; `TitleMgr` writes slot −1 (`タイトルへ戻る`). Loading resumes at the saved label with the call stack and restored characters. | No sample save of the original game was available: interoperability with saves written by the game is unverified (field layouts follow the decompiled writers). Not written or kept: the read-history block (empty), maid thumbnails, comments edited in the panel (`OnChangeComment`), save inside `if` blocks or with active macro arguments (refused). `SceneToTitle` (loading slot −1), title Config/End, and the rest listed above. |
| 2026-11-23 | Verified the ordinary day-2 roster edit, work assignment, delete cancellation, save/load and daytime result panel from an installed-data slot. | Per-character result rows and original-game save interoperability remain open. |

## SATD-cm3d2-ngui

**Scope and current location.** [`src/ngui.rs`](src/ngui.rs): layout of
`UIRoot`/`UIPanel`/`UIWidget` and drawing of `UISprite` (simple, sliced),
`UI2DSprite`, `UITexture` and `UILabel` (alignment, pivot placement,
wrapping, `ShrinkContent`, line spacing).

**Remaining work.** `UILabel` effects (shadow/outline), justified text,
gradient, colour markup and bitmap `UIFont` labels, tiled/filled/advanced sprites, flips, panel clipping and soft
clip, anchors to targets declared after the widget and to cameras,
transform rotation, `UIGrid`/`UITable`/`UIScrollView` layout, tweens
beyond the verified title and roster clips, animated button transitions,
drag handling and the physical controller/IME paths.

**Closure evidence.** Captures of every scene's UI against the original
layout, and input tracks exercising its controls.

| Date | Change | Remaining scope |
| --- | --- | --- |
| 2026-11-20 | Widget layout, anchors to earlier widgets/screen, panel depth and alpha, sprites and textures as image layers. | As listed. |
| 2026-11-20 | Anchors resolve within their scene and within their `Instantiate` copy (`Scene::find_ref`); `UILabel` layout through the shared text measurement. | As listed, plus `TrophyAchieveEffect`. |
| 2026-11-21 | Stored-byte (gamma) blending of image layers, sliced-sprite padding in pixels, zero-size nine-patch centres, `UILabel` `spacingX` (glyph runs) and ClampContent line limits, `showScrollBars` fading, run-time `UITexture` Unity textures. | Label effects, popup lists, scroll dragging, the rest listed. |
| 2026-11-20 | Run-time `Transform.localScale`, `SetDimensions`, `UIButton.isEnabled` (collider off, `disabledColor`), `UI2DSprite.sprite2D` images; `UIPanel` clipping as layer viewport clips and clipped hit tests; `UIScrollView.ResetPosition`; `UITable` cells sized by descendant widgets; right clicks reach `onClick`; slider `onChange` delegates added at run time. | Soft clip edges (`mClipSoftness` alpha), scroll dragging and scroll bars, and the rest listed. |
| 2026-11-23 | SceneTitle now routes UIButton clicks to the captured top collider on release, including the settings overlay. Title hover selects the cursor, a drag cancels the click, and mouse-mode Escape clears text selection. Verified title → settings → OK and Continue → Cancel against the 1.00 installation. | Other scenes' press/drag/hover delivery, title keyboard navigation bindings and live window/IME interaction still need evidence. |
| 2026-11-23 | Title keyboard navigation shares the cursor and activation path. SceneDaily buttons now use release capture, preserve right-click removal, and respect the confirmation modal. UIButton target tint and sprite supply visible hover/press feedback. The five graphics labels render the fixed canvas / sample's profile defaults, and the shortcut toggle updates its visible state. | Native physical title-key dispatcher, physical controller/IME, animated hover transitions, interactive graphics popups, reading custom config.xml and applying graphics choices to the host are unverified or unimplemented. |
| 2026-11-23 | Restored the short roster card's 419 × 111 collider and its −140 local center offset, and sampled the installed LeftPlate root-scale animation forward/backward on hover. Tested a selection near the portrait edge and pixel output of both the daily button and roster hover. | General UIPlayAnimation, other card layouts, and the remaining NGUI controls above. |

## SATD-cm3d2-rendering

**Scope and current location.** Materials of [`src/bg.rs`](src/bg.rs) and of
`crysirror-asset/src/interchange/cm3d2.rs`; parts colours in
[`src/parts_color.rs`](src/parts_color.rs); body visibility and part
attachment in [`src/chara.rs`](src/chara.rs).

**Implemented.** `CM3D2/Toony_Lighted*` and `CM3D2/Lighted*` forward passes
as the engine toon model (ramp, shadow texture/rate, rim, hair highlight,
specular, `_ShadowColor` under cast shadows at the light's 0.1 strength),
the `CM3D2/Outline` hull, `_MultiColTex` tables of `MaidParts` (native
`WriteColorTableToTexture`/`…ShadowTable…`), `テクスチャ合成` layers
(`2dAlpha`/`2dMultiply`), `TBody.FixVisibleFlag` triangle removal, part
roots parented to the slot bone, the `LightMain` light and level
`RenderSettings` ambient, no tonemapping (Unity gamma space).

**Remaining work.** `CM3D2/Man` reads a GrabPass (`Color·ALPHAMAN +
grab·(1−ALPHAMAN)`, alpha `ALPHAMAN_CUT`), so its own overlaps do not stack;
the engine alpha blend lets them stack. `ボーンにアタッチ` hand items keep the
rest offset instead of the identity local transform, and
`アタッチ <slot> <point>` attach points are recorded but not applied. Unity
lightmaps (none in the 1.00 levels), `Bloom`/`Blur`/`Vignetting`/
`ScreenOverlay` camera effects (Assembly-UnityScript, not in the managed
schema), Unity `ParticleSystem` (`Prefab/Particle/*`: yotogi fluids,
`PKamihubuki` confetti; no reader for the native class yet) and `Animator`
clips of UI objects (the MVP sprite), the mosaic and `Lighted_EzRiflect`/`Mirror` shaders, Unity
`Particles/*` and `Toon/Lighted_CM3D*` materials (PBR approximation), and
point/spot lights of the forward-add pass.

**Closure evidence.** Side-by-side captures of maid, room and title scenes.

| Date | Change | Remaining scope |
| --- | --- | --- |
| 2026-11-20 | PBR approximation of game and Unity materials, base texture, colour and alpha mode. | Toon shading and everything below. |
| 2026-11-20 | Engine toon model and outlines; CM3D2 and Unity scene materials mapped to it; parts colour tables; texture layers; visibility by triangle removal; bone-attached parts; Unity lighting/ambient. | As listed. |

## SATD-com3d2-standalone

**Scope.** The Unity 5.6.4p2 retail standalone profile in
[`src/com3d2`](src/com3d2). Character preload at the warning screen,
system-shortcut menus, title actions beyond Start, the reserved title save
and the post-title scene controllers need their original behavior. The title
background still needs its baked lightmaps, animated fountain and water
reflection. Full archive coverage includes motion data; the installed sample
has `motion.arc.bz2` without `motion.arc`. The archive mount uses the
standalone installation and its supplied GameData_20 overlay. Older linked
installations and other retail revisions need separate evidence.

**History.** 2026-11-23 — added the standalone archive plan, Unity
serialized format 17 and startup through the retail title. Start resumes
the original scenario and reaches the request for `SceneNetorareCheck`.

2026-11-24 — Ported the initial settings controller through its KAG return,
including the three script-visible player flags. The standalone TJS profile
at that point bound 69 of the 128 registered names.
Same-name inherited methods also need semantic checks. In particular,
the retail `CreateNewMaid` takes a personality or sub-character identifier,
and `PresetLoad` must apply the selected character preset. The next scene
requested by ordinary Start/OK input is `SceneUserEdit`. Character assembly,
pose playback, system settings persistence and original slot round-trips
remain open.

2026-11-24 — Added handlers for the other 59 registered names. The retail
VM exposes all 128, including the original rejected NPC-preset call and
the original empty setter. Added the scoped resource loader, retail preset
properties, class and event queries, schedule and activity state, and ADV
title-bar output. Installed data and host-state tests exercise the changes.
The tested normal route still stops on `SceneUserEdit`.

At that revision gameplay work included providing the standalone character factory
and character body assembly to `CreateNewMaid`, consuming the preset and
temporary costume on a rendered character, the actual schedule/competition,
casino and skill-selection screens, dance results, and VR controller startup.
These controllers produce state read by TJS. Native save/load must restore
that state, including per-character flags, skill/class ownership, completed
scenarios, purchases, and facility assignments. A linked older installation
also needs its own archive and key profile. Audit the remaining 33 inherited
handlers against their standalone bodies and exercise them in authored scenes.

2026-11-24 — Connected the standalone player editor in `src/com3d2/editor.rs`,
the scene dispatcher and widget loader. It uses the installed stock-man
menus, shared character scene projection, player name, RGB/palette, opacity,
abdomen morph, single-page tutorial and caller-supplied return label. The
installed sample ships its posture animation in a bzip2-wrapped motion
archive. A regular mounted motion archive feeds the animation layer; the
wrapper is not mounted by the launcher. Desktop IME, second-entry UI, original
save persistence and later story controllers need further validation.

2026-11-24 — The standalone desktop launcher now prepares the installed
bzip2-wrapped WARC in the scoped game cache. The title-to-editor replay
shows the original idle clip on the edited body. After returning, the two
message labels from the persistent scene render the speaker and opening
line, and the next input advances to narration. Physical IME, return visits
and native save/reload remain outside this evidence.

2026-11-24 — Standalone ADV now allocates three pooled NPC maids, resolves
named presets from the mounted tables and Unity resources, and applies the
retail property layout and nine material colors to shared character models.
Animation scripts bind slot zero to their caller's actor. The installed
opening displays a clothed NPC bow, plays scenario voice requests, and fades
the world independently of message UI. Title End opens the serialized system
dialog and Shop issues the standalone host URL request. Six formerly
inherited TJS entries have retail handlers. Twenty-seven still use the
CM3D2 implementations. Stock character creation, post-creation preset changes
on live models, title Continue/Guest/Config, daily controllers and standalone
native save/load remain open. The normal-route failure at that revision in
`OP_0001a` was camera tracking of the secretary's animated neck at `*L103`.
Its managed `CameraChase` component reads the posed bone every update through
expiry. The subsequent `EyeToCamera` calls the native head and eye controller.

2026-11-24 — The opening now follows the animated secretary, updates head/eye
rotations and accepts both answers using the installed NGUI button prefab.
The date panel displays the next day, waits for input and returns to its
authored label. For standalone first entry with empty stock, the day change
records the installed default facilities and eligible night work, clears the
two result flags and advances the day counter. Occupied schedules, linked
installs and pending facility changes still require their native controllers.
Choice backlog UI, exact vignette output and physical input remain unverified.

All three recruitment labels in `OP_main_0001` request a new stock character.
At that revision the regular first-choice replay stopped in `CreateNewMaid("Muku")` at frame 9218.
The other two inputs reached the same factory for Majime and Rindere at frame
9330. The inherited factory requested legacy game tables. That revision required
`Maid.InitializeStatus`, retail class and skill acquisition, body-derived
profile values, and activation of the rendered stock body. Sub-characters
also apply the selected original/alternate status and preset with portrait
and thumbnail data. The opening NPC pool has distinct identity and lifetime.
See `src/com3d2/{actors,tracking,choices}.rs`, `src/tjs/retail/day.rs` and
the dated playback evidence.

2026-11-24 — Implemented retail stock creation, initial identity and class/skill
ownership, live appearance changes, named sub-character status/presets and
separate stock/NPC model lifetimes. The three recruitment arguments now pass
direct installed-data tests. Schedule casting uses the day/night job and
communication flag appropriate to each selector; empty random selection
deactivates slot zero. Contract and relation awards update the player trophy
set. Ordinary and transferred-state method tests cover temporary flags,
skill levels and mode-specific lookups.

The standalone stock editor now reads its main/original mode scene, background,
pose script, controls and owned item menus. Remaining editor scope includes
profile/category controllers, item icon groups and colors, eye/brow menu offsets,
physics-driven body properties and finalization. Stock constructor and slider
property logic moved into `src/tjs/retail/stock.rs` so both crate feature
configurations can use them. `src/bone_shape.rs` decodes the installed
`BoneMorph.Init` scale/position registrations. `chara::View` binds the
matching bone and vertex morph names on each loaded mesh. Original `SceneEdit`
captures a 90×90 icon and 630×890 card from `Bip01 HeadNub`, executes
`KasizukiThumShot.ks` for a separate portrait and returns to the saved KAG
label. The runtime still needs a separate 3D camera capture with its own
target, visibility and transparent clear. The current confirmation reports
this boundary. Linked/VR branches, populated day advancement, later scene
controllers and original v1040 slots also remain open.

2026-11-25 — Main-character recruitment now supports the profile panel's
name inputs and random buttons, body readouts, comment tabs and naming
dialog. Confirmation runs the installed profile conditions, captures the
icon and card from the posed head, executes the separate camera/pose script
and returns to the caller. See `src/com3d2/{stock_profile_ui,photo}.rs` and
the Metal portrait and UI tests. Main/original creation, facility costume
editing, VR and existing-character editing use different native branches.
The installed regression covers main creation. Additional profile selectors,
colors, editor categories, title and daily controllers, linked modes and
v1040 save/load still have open acceptance work.

2026-11-25 — Standalone editor lighting, per-texture palette bindings and the
textured toon forward/hull paths now use the installed 5.6 shader evidence.
Legacy material-wide tables remain active for Unity 4. The opening now passes
recruitment and the temporary broom in OP_0002. At frame 12314 the retail
SceneDaily main menu was the next missing scene (DaytimeMain.ks line 10). Pending native
save/load and remaining editor controllers still belong to this record.

2026-11-25 — The standalone Daytime and Night menu dispatcher now consumes
the 20 serialized DailyCtrl references. The scene loads the authored office,
camera, BGM, status widgets and tutorial pages, routes visible scenario
buttons and handles the 40-slot morning backup and facility reassignment.
The installed-data tests for both menus read resources and resolve KAG labels
without playing a game. See `src/com3d2/daily.rs`,
`src/tjs/retail/daily.rs` and the [validation note](../../docs/validation/com3d2.md#standalone-daytime-and-night-management-menus).
Schedule/Save/Load controllers, populated schedule work validation and
selection on entry, income/work result panels, status-only entry, subsequent
destination controllers, and gameplay/window acceptance remain open. Club
name edits update the session. Restoring them from v1040 slots is pending.

2026-11-25 — The standalone basic-training route now crosses the daytime,
night and income results and returns to day two's daytime menu. Schedule,
roster selection, communication toggles, result widgets and callbacks live in
`src/com3d2/business.rs`. Work counts, growth, facility experience and
day transitions live in `src/tjs/retail/operations.rs`. The ordinary installed
script fixture runs both `ScheduleProcess` calls and resumes each result
caller's original label. The eight basic jobs have no pay in this profile.
Paid facility work and room selection, upgrades and construction, additional
schedule categories, drag-and-drop reordering, detailed task descriptions,
result choreography and native v1040 persistence remain open. Windowed input
and presentation still need separate verification.
See the [day-two fixture](../../docs/validation/com3d2.md#standalone-basic-training-through-the-second-day).

2026-11-25 — The default work-10000 cancellation returns through the authored
single-employee selector and script to day two. Ordinary theater work now
selects a room through the Unity grid and settles both insufficient and
sufficient staffing. Row dragging updates roster order. Picker, room layout
and earnings are in `src/com3d2/business/picker.rs`,
`src/com3d2/widgets/rooms.rs` and `src/tjs/retail/operations/paid.rs`.
The management fixtures advance scripts and drive authored controls without
starting the game. See the [installed-data results](../../docs/validation/com3d2.md#standalone-default-night-work-and-paid-facilities).

2026-11-25 — Work 3000 now runs through the employee selector, the installed
facility upgrade canvas, the room recipe, shift receipt and its original KAG
return. Material ownership is checked. Buying materials in the standalone
shop, facility construction, and remaining destination controllers are open.
The result panel and camera fades use independent clocks. The room selector
shows the selected shift's operating status, staffing, ratings and thumbnail.
See `src/com3d2/power.rs`, `src/com3d2/widgets/rooms.rs` and
`src/tjs/retail/operations/{paid,upgrade}.rs`.

2026-11-25 — Title and daily slot discovery, paging, comment edits and
confirmed individual slot deletion now use
`src/com3d2/slots.rs`. Original-assembly writer fixtures cover v1040 player
status, both backup banks and the facility v5 records. CharacterMgr stock
decoding shares the standalone preset property reader, retains the male
metadata and reaches the first employee's status after the nine palettes.
Whole-slot save, full employee status, script and scene reconstruction, and
ordinary shop purchases remain open. See the
[management and slot evidence](../../docs/validation/com3d2.md#standalone-facility-upgrades-and-native-slots).

2026-11-25 — The standalone shop entry, catalog and purchase controllers
now live in `src/com3d2/shop.rs` and `src/tjs/retail/shop.rs`. The installed
short fixtures in `src/com3d2/shop/tests.rs` cover both original callers,
ordinary outfit preview and restoration, material purchase/cancellation,
item ownership and bundle prices. Work 3000's day-two fixture now purchases
its material. Scroll dragging, per-hair preview settings and window timing
remain open alongside construction, the other destination controllers and
whole-slot persistence. See the [scope and direct scene inventory](../../docs/validation/com3d2.md#standalone-shop-and-management-scene-coverage).

2026-11-25 — Standalone facility construction now dispatches to the
level 47 panel in `src/com3d2/construction.rs`. Its information and name
controls live in `src/com3d2/construction/`. The original twelve slots,
deferred orders, affordability recheck and next-day room installation are
in `src/tjs/retail/construction.rs`. Installed fixtures cover construction,
cancellation, protected rooms, replacement, lost funds, demolition, both
browse tabs, recipe details and live/reserved room renaming. The shared
facility-canvas reader remains under `src/com3d2/widgets/power.rs`. The
wardrobe tab/editor return, whole native slots, and ten other direct scene
destinations are still open. See the [component inventory](../../docs/validation/com3d2.md#standalone-management-component-inventory-and-facility-construction).

2026-11-25 — The fourth level-47 tab now controls live and planned facility
uniforms. `src/tjs/retail/wardrobe.rs` owns the mode, temporary clothing
properties and room-specific edit return. The stock editor and existing
portrait session branch on the original facility mode. Remaining evidence
includes the physical editor-to-portrait round trip and per-part enable
settings. See the [wardrobe fixture](../../docs/validation/com3d2.md#standalone-facility-uniforms).

2026-11-25 — Level 5 per-part facility force settings and the level 19
staff roll have player input and dispatch paths in `com3d2/stock_editor/costume.rs`
and `com3d2/credits.rs`. Window appearance, credits animator transitions,
native slot persistence and the nine remaining direct scene destinations
still require evidence. See the current [component inventory](../../docs/validation/com3d2.md#standalone-management-component-inventory-and-facility-construction).

2026-11-25 — Standalone level 10 now dispatches to `src/com3d2/awards.rs`.
The collection, help pages, ownership and card viewer use installed level,
table and prefab data. Eight direct destinations still lack dispatch. Native
slots, physical window comparison and the outstanding management components
remain in scope. See [the level 10 fixture](../../docs/validation/com3d2.md#standalone-level-10-trophy-collection).

2026-11-25 — Standalone level 49 event selection, level 7 personnel,
level 21 free dance selection and level 23 desk decoration now have player
dispatch and installed input fixtures in `src/com3d2/{events,personnel,dance,desk}.rs`.
At that point four direct destinations still lacked dispatch (photo, creative
room, examination and casino). Outstanding branches in the four newly reachable scenes include
personnel status/classes, dance execution and settings, desk transforms and
slot data, and broader scenario condition/cast validation. See the
[per-controller results](../../docs/validation/com3d2.md#standalone-management-destinations-after-level-10).

2026-11-25 — Standalone title Config now opens its system panel in
`src/com3d2/config_panel.rs`. Eight of 32 native panel callbacks have
handlers. The other 24 bindings, original settings persistence, and the
title Guest load mode remain open. See the [title and corpus inventory](../../docs/validation/com3d2.md#standalone-title-config-and-static-ui-coverage).

2026-11-26 — Connected the remaining 24 standalone settings callbacks in
`src/com3d2/config_panel.rs` to the window, renderer, shared mixer, text
and model paths. The 1.0.4 title fixture exercises all 24 controls and reads
the published engine state. Native settings persistence, the title Guest
load mode, and the `DanceMain` playback timeline remain open.
See the [settings inventory](../../docs/validation/com3d2.md#standalone-title-config-and-static-ui-coverage).

2026-11-25 — The remaining direct management destinations now dispatch in
`src/com3d2/leisure/{studio,room,examination,casino}.rs`. Per-controller
scope and installed-data fixtures are recorded in the
[16-destination inventory](../../docs/validation/com3d2.md#standalone-management-menu-coverage-16-direct-destinations).
Remaining work within these scenes includes photo character placement, posing
and composition files, room objects and custom materials, broader judging
and award validation, casino coin persistence and the separate casino games.

2026-11-25 — Opening-story indoor rendering in `src/com3d2/club.rs` now
preserves the installed office's unlit monitor and alpha-blended light overlay.
The entrance's second-UV `_LightMap` and soft-additive particle lighting still
need their native material paths. Standalone `widgets.rs` now honors the
dialogue label's line clamp and vertical spacing, with a regression for
cached geometry after moving or resizing a label. Visual equivalence across
all indoor camera positions and text layouts remains open.

2026-11-25 — The opening hall in `src/com3d2/club.rs` now imports its
soft-additive lamp meshes and RGB wall lightmap. This covers the installed
static glow geometry, whose vertices omit a color stream, and the exact
`Lighted_Lightmap` material family with a bound secondary texture. The
depth-faded particle variant and the separate reflective water shader
still need validation and implementation under their native contracts. The
shop curtain lacks secondary UVs and retains its previous material path
until Unity 5.6's missing-attribute behavior is verified.

2026-11-26 — `com3d2/mod.rs` connects `SceneToTitle` through a scoped
session checkpoint taken at the original title call. Restoring the native
on-disk -1 slot still requires the complete 1040 save path.
`com3d2/leisure/casino_shop.rs` connects the enabled reward tables, coin
exchange, purchase confirmation, ownership and fitting controls. Casino
shop inventory and coin balance still need native slot persistence.
`com3d2/leisure/blackjack.rs` connects betting, cards, turns and settlement
through the installed uGUI controls. It represents the hands in the score
labels. The original animated cards, chips, dealer voice and motion callbacks
still need their native paths. The five remaining KAG destinations are
`SceneCasinoSlot`, `SceneFreeModeSelect`, `SceneKasizukiMainMenu`,
`SceneYotogi` and `SceneYotogiOld`.

2026-11-26 — Registered the remaining five literal names in the standalone
scene dispatcher. Level 56 now binds the native casino slot UI and machine
geometry, original reel strips and payout tables, shared UV scrolling,
bet/stop/settlement, exchange and return. The original lamp, bonus,
reach arbitration and payout presentation timelines still need parity checks
and work. Level 24 binds the native recollection category/list controls and
script return variables, the native flag/relation checkpoint and
`@freecall`/ `@freerestorebackupdata`. Character presentation on re-entry,
VIP task-view parity and the linked legacy catalogue remain open. Level 60
binds the guest main menu, man and room selection UI, free-character and title
routes. Guest saves, room eligibility and dispatch, character detail and
model refresh still need their own controllers. Levels 14 and 70 bind the
stage selection and Null script-return paths. Skill lists, execution,
results, supporting cast and supplying the original linked compatibility
tables to the standalone player remain open. No build or player run was
performed for these changes at the owner's request.

2026-11-26 — Level 56 reads the serialized slot controller's node and
texture references. Shared world animation drives the reels, lever, credit
digits and lamp visibility, and shared composition drives the chance-mode
fade. The payout, jackpot and chance timelines now update the machine and
casino state, including native total-coin trophy accounting. Per-cell reel
shader masks, continuous lamp fades and the remaining reach/stop cases are
still outstanding. Level 60 uses the room, man and scenario tables to
filter the room roster and select the native scenario. The replay window
has the installed room/man/scenario lists, hover descriptions, and returns
through `label_free` and the native replay filename variable. Returning
from a scenario with `update_maid_status` applies the native counters.
Guest save/reload still needs the complete 1040 parent record and the
bookmark-delimited guest section. Guest character detail and man-model
refresh also remain open. Levels 14 and 70 record the selected stage's
native key separately from its display name and background, and stage
previews use the authored thumbnail naming and fallback. The linked
installation requirement and the non-stage Yotogi controllers remain.
Static inspection only for this update.

**Closure.** Exercise the remaining controllers, rendering and resource
variants through installed original scripts and ordinary input, including
native save/reload and character assembly. See [validation](../../docs/validation/com3d2.md).
