# crysirror-cm3d2

Custom Maid 3D 2 (KISS, 2015; Unity 4.6.6p4 player, `cm3d2_x64.dll` native
plug-in) for CrySirror.

**Onboarding mode: `adopt`** (ADR 0102). The game logic of
`Assembly-CSharp.dll` is ported to Rust here by owner decision (2026-11-20);
no CIL interpreter, Unity runtime or native plug-in is hosted. Formats and
rendering are shared engine capabilities:

| Capability | Shared crate |
| --- | --- |
| `GameData/*.arc` (`kiss/warc`) | `crysirror-formats-arc` (`legacy/j.rs`) |
| `.tex` textures (`kiss/cm3d2-tex`) | `crysirror-image` (`kiss_tex.rs`) |
| `.model` skinned meshes, `.mate` materials, `.anm` clips → GLB | `crysirror-asset` (`interchange/cm3d2.rs`) |
| Clip retargeting by node name, followed skeletons, bone-attached parts, procedural node poses | `crysirror-scene` (`AnimationLayer.source`, `AnimationState.follow/attach/nodes`), `crysirror-asset::anim`, `crysirror-wgpu` |
| Toon shading and outlines (`CM3D2/Toony_Lighted*`, `CM3D2/Lighted*`, `CM3D2/Outline`), lookup bake, texture layers, skin-bone triangle hiding | `crysirror-scene` (`Material.toon`), `crysirror-shaders` (`pbr.wgsl`), `crysirror-wgpu`, `crysirror-asset` (`toon_parameters`/`toon_lookup`, `MaterialEdits.layers`, `Mesh::hide_skin_bones`) |
| Flat ambient, cast-shadow strength | `crysirror-scene` (`Environment.ambient`, `Sun.shadow_strength`) |
| Unity 4 serialized files, textures, meshes, managed fields, prefab → GLB | `crysirror-unity` |
| Adapter-built models | `HostServices::upload_model` |
| KAG parser (CM3D2 `file`/`label` dialect), TJS2 | `crysirror-kag`, `crysirror-tjs2` |

## Custom Order Maid 3D 2

The standalone KISS retail player has its own `com3d2` adopt profile for Unity
5.6.4p2, serialized file format 17. Run it from the installed game directory:

```sh
./target/release/crysirror play ~/Downloads/game-com3d2
```

Release Return or a mouse button after each of the two startup fades to
advance. The adapter reads the Unity BuildSettings, logo, warning and title
widgets from the installation. It executes the installed `AllMain.ks` and
`reset_scene.ks` through the shared KAG/TJS machinery. The title uses the
retail club mesh hierarchy, texture resources, six buttons and BGM004. Start
resumes the scenario at `*title_end`. The initial settings controller loads
the office prefab and the serialized NGUI radio groups. OK writes the three
player flags, fades out the BGM and returns to the script at `*user_edit`.
That label opens the standalone player editor, which reads its NGUI layout
from the installed scene and the stock man’s head and body from KISS item menus.
The desktop launcher also accepts bzip2-wrapped KISS archives when the
uncompressed archive is absent. For the installed motion pack, first launch
writes a 2.27 GiB seekable copy into the game’s disposable cache. Subsequent
launches reuse it after fingerprinting the compressed input. The editor plays
the installed idle clip. Finishing first-time setup on the default route
returns to the opening dialogue in `OP_0001a`.

The opening assembles named NPCs from retail presets and plays the installed
conversation animations on the assigned character slots. The verified route
tracks the secretary’s animated neck, presents the two opening choices,
shows the first calendar-day scene and reaches the three-person recruitment
menu. The standalone factory creates each candidate, activates the new stock
model and applies the installed preset. The stock editor loads its Theater
background, pose and serialized controls. Character rendering uses the standalone
per-texture palettes, textured toon outlines and scene-specific main light. Mouse input can change the owned
hair menu, body sliders and the name used in dialogue. The profile panel edits
names and free comments. On desktop the confirmation sequence photographs
the posed character, runs the installed photography script and resumes the
calling KAG label. Title End supports confirmation
and cancellation. Title Shop requests the unlinked installation's shop URL
through the desktop host.

The standalone daytime and night management menus read their button bindings
from the installed Unity scene. They resolve the calling scenario's navigation
labels, display the multi-page tutorials and club status, and record the
morning schedule on Next. In the standalone 1.0.4 profile the schedule editor
supports the stock roster, both shifts and eight basic training jobs. Installed
script and widget fixtures cover daytime and night results, the income panel,
and the second day’s management menu. Muku can keep her original night work
10000 and reach day two by cancelling the character selection. The schedule
supports ordinary paid facilities, room selection, roster dragging and manual
facility upgrades with owned materials. The daytime and night menus write
standalone v1040 slots. Title Continue and the daily Load panel restore those
slots through the saved KAG label. The panels also list slots, edit comments
and confirm deletion. An installed-data fixture saves the daytime menu,
reloads from the title and asserts the restored player, staff and schedule.
The last full opening replay reached the first daytime menu call.

Title Guest opens the campaign slot list followed by the selected campaign's
guest slot list. It can resume existing guest data or register a new guest.
The guest club's Save and Load buttons use the original five-page panel.
Guest writes preserve the surrounding campaign and casino records.

The standalone profile binds the 128 names registered by its retail
`ScriptManager.Initialize`. The [registration list](src/tjs/standalone.rs)
and [standalone handlers](src/tjs/retail.rs) distinguish the two Unity
profiles. The installation supplies the NEI tables, presets, Unity resources,
fonts and audio. Character, activity and save controllers also supply live
state to these methods. The player editor returns to the label supplied by the calling scenario.
See the [behavior and coverage record](../../docs/validation/com3d2.md)
and [remaining work](SATD.md#satd-com3d2-standalone).

The COM3D2 project client accepts an optional `linked_cm3d2` object for
the original `FileSystemOld` sources. Its `archives` list contains separate
mounted asset directories in the legacy load order. `paths_dat` and
`plugin` are client-relative asset paths to the linked installation's
`GameData/paths.dat` and native DLL. The old stage selector uses this source
for its stage catalogue, previews and BGM. The new stage selector continues
to use the standalone game's own tables and resources.

## Source provenance

Analysis inputs (not distributed): the installed game's
`CM3D2x64_Data/Managed/Assembly-CSharp*.dll` decompiled with ilspycmd 8.2,
and `Plugins/cm3d2_x64.dll` (SHA-256
`3d2a7db17d61cff21a25cc8722b31bc17ec482c11e19e1b614f101ae5c2ca658`) analysed
statically with IDA. `data/unity_schema.json` is generated by
`tools/unity_managed_schema.py` from the decompiled declarations and records
only script class/field names and types. No game data, key material or code
is embedded; the `.nei` table key is read from the installed DLL
(`src/native_key.rs`).

## Contracts implemented

- `GameUty.Init` archive order and flat `AddAutoPath` name lookup (`files.rs`).
- `.nei` tables: AES-128-CBC with the xorshift IV of `sub_180094270`, cell
  table of `sub_1800934D0` (`nei.rs`).
- Scene loading by `BuildSettings` order (`mainData` = index 0,
  `level<N-1>` = index N), the persistent `__GameMain__` singleton
  (`adapter.rs`, `unity_scene.rs`).
- NGUI widget layout, anchors, panel depth/alpha, `UISprite` (simple and
  sliced, padding as `UISprite.drawingDimensions`), `UI2DSprite`,
  `UITexture` as image layers blended on stored (gamma-encoded) bytes as
  Unity 4's gamma-space rendering does; `UILabel` `spacingX` and
  ClampContent line limits; `UIScrollView.showScrollBars` (`ngui.rs`).
- Level 3D content as world entities; `CameraMain`/`UltimateOrbitCamera`
  placement (`bg.rs`).
- `SceneFirst`, `SceneLogo`, `SceneWarning`, `SceneADV`, `TitleMgr`/`TitleCtrl`
  (Start, Continue, Config, End) (`scenes.rs`); `SceneTrophy` (`trophy.rs`),
  `SceneStaffRoll` (`staffroll.rs`), `SceneSingleEffect`/`startmvpeffect`
  (`scenes.rs`, `ops.rs`); the other scene controllers in their modules.
- `MessageWindowMgr` under-buttons (skip, auto, voice replay, backlog with
  `BacklogCtrl` units, config) and the `ADVKagManager` skip/auto advance
  (`msgwnd.rs`); `ConfigMgr`/`ConfigCtrl` panel with the message, model
  transparency and `SoundMgr` volume sliders (`config.rs`).
- `ADVKagManager` tag loop with `scenecall`, `wait`, `motionblendenabled`,
  `eval`, `talk`, `hitret`, `messagewindow`, `fade`, `fade3d` (`sync` waits),
  `cameracontrol`, `playbgm`/`stopbgm`/`playenv`/`stopenv`/`playse`
  (immediate)/`stopse`, `[s]`; text accumulation with `[SF]`/`[HF]`/`[HL]`
  kept for `ReplaceCharaName`; `SceneMgr.CloseScene` label return (`kag.rs`).
- `MessageWindowMgr` open/close, speaker and message labels, window alpha
  (`adapter.rs`); `UILabel` with the embedded TrueType data of Unity `Font`
  assets registered through the shared font library (`ngui.rs`).
- `SoundMgr` channels and default volumes, `PlaySystem` SE table,
  `UIButton.PlaySoundClick` (`sound.rs`). TJS: `startup.tjs`, `Scripts.execStorage`, man/tmp flags,
  `SetNewTrophy` (`tjs.rs`).
- `Maid`/`TBody`/`Menu.ProcScriptBin` (`chara.rs`): item menus, slots, masks,
  `node消去`/`node表示` replayed per slot over the body hierarchy and applied
  as `TMorph.FixVisibleFlag` triangle removal, part roots parented to the
  slot bone (`Bip01`→`ManBip` for men), `テクスチャ変更` with `*` stems and
  `PARTS_COLOR`, `テクスチャ合成` layers, `アタッチポイントの設定`,
  `リソース参照`/`半脱ぎ`, `commenttype`, `paramset`, `blendset`. The 1.00
  binary ignores `パーツnode消去`/`パーツnode表示`; `color_set`/`unsetitem`
  are SceneEdit catalogue commands.
- `MaidParts` colours and `_MultiColTex` tables, preset parts colours
  (`parts_color.rs`, `preset.rs`).
- Lighting: `LightMain` rotation/intensity, the disabled character light,
  shadow strength 0.1 and the level's `RenderSettings.ambientLight`
  (`adapter.rs`).

Unimplemented tags, scenes and functions fail visibly with their names. The
outstanding scope is tracked in [SATD.md](SATD.md).

## Validation

Owner-corpus tests (`CM3D2_ROOT=<game> cargo test --release -p crysirror-cm3d2 -- --ignored`)
convert all 427 models, 3058 animations and decrypt all 44 tables. Behaviour
evidence is recorded in [docs/validation/cm3d2.md](../../docs/validation/cm3d2.md).


### Title and daily-menu controls (1.00)

Use Up/Down to cycle Start, Continue, Config and End, and Enter to activate the
focused button. The desktop controller navigation mapping supplies the same
actions. Moving the mouse over a title button takes focus back. The title
binding is provided by CrySirror using the recovered TitleMgr commands and
order. The installed scene enables keyboard/controller input, but the original
physical command dispatcher has not been independently established. Settings
closes through OK. The load panel has its own Cancel button.

On the daytime menu, select Schedule, click a roster slot to choose a character,
then click the slot's daytime task to assign work. Right-click an occupied slot
to request removal. Cancel preserves the assignment. The sole leader cannot be
removed. Schedule OK returns to the daily menu. The verified day-2 route and
graphics-setting limitations are in [the UI validation record](../../docs/validation/cm3d2.md).
