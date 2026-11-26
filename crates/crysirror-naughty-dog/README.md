# Naughty Dog engine resources

Onboarding mode is **adopt**. This crate reads the installed PS3 disc profile for *Uncharted 2: Among Thieves*, title ID BCAS20097, application/disc version 01.00. The identifiers come from the installation's `PS3_GAME/PARAM.SFO`. The Rust code keeps package data in client-readable streams and uses CrySirror's shared archive and image services. This resource profile is available independently of a game VM.

## Inspect installed resources

From the repository root, supply the directory containing `PS3_GAME` and an optional native level name.

```sh
cargo run --release --offline --locked -p crysirror-naughty-dog --example level_resources -- "$HOME/Downloads/game-Uncharted2" train-cliff
```

The command reads the level declaration, mounts the eight archives in place, decodes one installed controller image into RGBA, and reports the resource types and SHA-256 of exact package matches. It reads the localization tables and prints two language variants of the same UI string. All results go to the terminal. Some package references have numbered siblings on disc. The command reports a missing exact match until native version selection is established.

| Input | Current behavior |
| --- | --- |
| Plain PSARC 1.3, zlib, 65,536-byte blocks, flags zero | The shared `sony/psarc` archive format exposes seekable files under their original case-sensitive paths. It validates the manifest path hashes, physical and logical bounds, and zlib end/checksum. Uncompressed and compressed blocks may alternate. File positions use wide offsets. |
| `pak22.txt` | Reads 403 level declarations, including 660 package references. Retains the level ID, package, actor, sound bank and cache references in native order, including repeated cache edges. |
| PAK 0xa79 | Reads the original page and named item directories, including item type and page-relative location. Package payloads remain accessible through the shared archive mount. |
| `text1.psarc` | Validates sorted IDs and UTF-8 string boundaries in the installed common, networking and subtitles catalogs. Lookups support multiple IDs referring to one string. |
| `VRAM_DESC` | Reads the RSX format, size, level count, face count and data offset relative to the package's VRAM base (file length minus header data size). Level 0 of DXT1/DXT3/DXT5 decodes through the shared block decoder; swizzled A8R8G8B8 is de-Mortonized. |
| Background mesh indices | `geometry::decode_indices` expands the compressed fragment triangle stream (explicit references, 2-bit strip codes, emission flags). Every fragment of `menu-temple` decodes with all counts and references consistent and no degenerate triangles. |
| Background mesh positions | `mesh::prototype_meshes` relocates the package and dequantizes fragment positions (bounds mapping; offset fragments on the mesh grid with two placement bits). `examples/background_census` decodes every installed package. |
| Mesh materials and UVs | Mesh groups bind fragment ranges to materials; pass-0 samplers name textures by `VRAM_DESC` ID. Texture coordinates are 1/256 of the texture. `level_resources LEVEL OUT.glb` writes a textured inspection GLB outside the repository. |
| XVAG codec 6 | `crysirror_audio::xvag` decodes PS-ADPCM streams (music, ambience, speech) with frame-interleaved channels to WAVE. |
| Loose boot DDS | Uses the existing CrySirror image decoder. The local sample includes original 1280 × 720 DXT1 controller and gallery assets. |

## Evidence and outstanding work

The 0xa79 page/item layout was independently checked on the local installation against the [package layout research by icemesh at `3e3025c`](https://github.com/icemesh/big2-ps3-pak-research/blob/3e3025cc9afc2cbf8e6c0e5a36466d6a72004244/big2-ps3-pak-research/src/package/package-def.h). The PSARC layout was checked against its eight on-disc manifests and [libPSARC format notes](https://github.com/GaticusHax/libPSARC/wiki/Documentation-libPSARC-PSARC-File-Format). The implementation treats all offsets as checked integer data.

Native PPU behaviour is analyzed from an owner-supplied decrypted `EBOOT.elf` beside the installed `EBOOT.BIN`. Neither file is distributed. The script ABI, opcode table and native entry points are recorded in [NATIVE_ABI.md](NATIVE_ABI.md).

## Player

`crysirror play GAME_DIR` detects the installation and starts the `naughty-dog` adapter (`src/adapter.rs`) in task mode:

1. `src/game.rs` loads `dc1.psarc/startup.bin`, runs its `top-level` lambda in the script interpreter (`src/script.rs`) and loads the 86 modules it requests.
2. The task graph (`src/task.rs`) starts at the first boot subnode (`introduction`; `{"task": NAME}` selects another). Each subnode runs its start lambda and presents its task-manager record: `TaskManagerPlayMovie` plays `movie1/NAME.m2v`, `TaskManagerMessageDialog` shows the catalog text of its key from `text1.psarc` (`{"language": N}` selects `N.common`, default 2, English) until Enter, `TaskManagerMainMenu` shows New Game and Continue over the menu level; both start the story because native saves are not read yet. The boot runs `introduction`, `ndi`, `kiosk-trailer` (skipped by its start lambda), `autosave-warning` and `menu`. Its level, the level's `cache` levels and the node's world level load from `pak22.psarc`. The world package's load regions (`src/region.rs`) run their scheme lambda in the interpreter and add the displayed levels.
3. Spawners are placed in world space (parent spawners applied) with their art groups from `actor27.psarc`. Spawners flagged 0x40000000 start hidden. The player model stands at the subnode's continue spawner.
4. Every spawner with a `state-script` property runs as a process (`src/ss.rs`), and each frame advances these processes. Natives act on spawned entities, the screen fade, task state and process variables. Unimplemented natives return 0 and are listed in the session diagnostics.
5. `task-subnode-complete`, finished movies and dismissed dialogs advance the task. Movies play through the shared MPEG decoder. Enter or Escape skips it. A movie over the platform large-asset limit (1 GiB on desktop and mobile, 64 MiB on PS Vita and Switch) is skipped.

Controls emulate a PS3 pad on the keyboard with RPCS3's default keyboard layout (`src/pad.rs`): left stick W/A/S/D, right stick Home/End/Delete/Page Down, d-pad arrows, square Z, circle C, triangle V, L1 Q, L2 R, L3 F, R1 E, R2 T, start Enter; the host key mask has no X or G, so cross is Space, select Tab and R3 B. In game: cross jumps (and confirms in menus), circle takes cover or drops from a hang (and goes back in menus), square strikes, triangle reloads, L1 aims, R1 fires, L2 throws a grenade, d-pad left/right picks a pistol or long gun. F1 toggles a free camera. `{"level": NAME}` without a task opens the free-camera level viewer (WASD/QE, arrows, Page Down/Up through `pak22.txt`).

Rendering uses each material's colour map (by texture source name) and the levels' baked light probes, evaluated per vertex. Native animation, collision, AI, audio banks, HUD, saves and most of the 833 natives are not executed. Desktop, browser and console gameplay are unverified. See [validated samples](../../docs/validation/uncharted2.md) and [SATD-nd-retail-runtime](SATD.md).
