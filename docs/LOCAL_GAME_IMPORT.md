# Local God of War III / Ashen Vow import trial

Executed on 2026-10-25, macOS arm64, Apple M5 / Metal. These are offline
asset-conversion tools feeding CrySirror's existing desktop renderer. The
original Windows/PS3 executables were not run. This adds no console backend.
User files in Downloads were read only. All extracted assets, recovered schemas,
converted scenes and screenshots remain under the gitignored `local/` directory (see local/README.md), outside
this source export. The checked-in evidence contains metadata and test logs.

## Latest follow-up: GoW III OLY100 scene rendering (2026-10-25)

Actual CrySirror Metal capture:
`images/third-party/gow3/oly100-overview.png (the original preview was pruned in the 2026-10-25 cleanup)`.
The selected WAD now supplies 247 resource nodes / 1,474 surfaces / 548,191
triangles and 75 used low-mip texture previews. All selected first-table MG
geometry is retained. This is an encoded rest-space diagnostic: native transform
composition, moving-part poses, seven UV groups, original shaders and lighting
remain incomplete. It does not load the entire game or all Olympus resources.

WAD metadata framing and MG/MDL/material layout decoding stay in the external
developer frontend. Shared Rust CPU codecs decode the actual topology and DDS
images. `systems.environment` and the existing project camera/scene entity
produce a data-only project. No JS/TS or original executable is run to render it.

Reproduce after extracting `/WAD/PS3_BE/OLY100.WAD` with the PSARC tool:

```sh
cargo build -p crysirror-asset --example transcode
cargo build -p crysirror-desktop --release
# Python 3.11 is used for the complete tooling test suite on this host.
/opt/homebrew/bin/python3.11 tools/gow3_scene.py \
  "local/work/gow3/olympus.wad" \
  "local/work/gow3/olympus-engine-preview" \
  --codec target/debug/examples/transcode
target/release/crysirror render \
  "local/work/gow3/olympus-engine-preview/preview/project.json" \
  --frames 1 \
  --output "local/work/gow3/olympus-engine-preview/preview/scene.png" \
  --data-dir "local/work/gow3/olympus-engine-preview/preview/state"
```

Compilation and behavior are recorded separately in
[gow3-scene-results.json](gow3-scene-results.json): workspace/all-targets,
no-default, release and codec-example builds passed; Rust 170 passed / 13 ignored,
explicit Metal GPU 11 passed, Python 117 passed. Actual MCP stdio
validation/trace/capture also succeeded. Ignored tests are not passes.
[ADR 0017](adr/0017-gow3-scene-capabilities.md) details the observed native
profile, source provenance and proposed shared APIs. The earlier checkpoints
below retain their historical counts and limitations.

## Earlier follow-up: Fog shader preset (2026-10-25)

All 42 original fog-sheet instances now render through CrySirror's Fog preset:
procedural UV noise, flow, soft sheet edges and opaque-depth intersections,
combined with serialized source color/opacity/depth parameters. The explicit
external profile replaces the preceding self-authored mask, without interpreting
the original UE shader graph. The scene still has 7,709 nodes / 13,426,500 scene
triangles; only the source sky sphere is omitted. New actual capture:
`images/third-party/ashen/castle-shader-presets.png`.

Shader presets and developer-authored WGSL are shared engine capabilities;
`examples/shader-effects` demonstrates them with self-authored assets. Material
profiles can reference a custom effect code asset relative to the profile root;
the converter copies the code to a separate .wgsl output file and keeps only its
logical path/parameters in glTF material data. Escape/size checks apply. Shader
compilation remains a separate shader-check/render operation.

See [ADR 0016](adr/0016-shader-effects.md) and
[shader-effects-results.json](shader-effects-results.json) for current evidence:
Rust 166 passed / 13 ignored, explicit GPU 11 passed, Python 110 passed;
workspace/all-targets, no-default and release compilation passed. Original
material/lighting/gameplay fidelity and the GoW geometry gaps remain unresolved.

## Follow-up: shared engine materials (2026-10-25)

The first capture below omitted 42 fog sheets. After implementing the shared
material contract and transparent PBR composition (ADR 0015), a new capture
retains **all 42**: **7,709 nodes / 13,426,500 submitted triangles**, 31 original
base maps and one self-authored opacity map. Only the source sky sphere is
omitted. Screenshot:
`images/third-party/ashen/castle-engine-materials.png`.

The bridge now records serialized local scalar/vector parameters, including
`UMaterialInstanceDynamic` overrides. The external explicit profile maps source
Color, MaxOpacity (clamped to 0..1) and DepthFadeDistance (centimeters to meters)
onto ordinary engine material data. A self-authored elliptical opacity mask
substitutes for the unevaluated UE noise/projection/panning graph. This is an
approximate unlit fog-sheet material, not original shader execution. The renderer
contains no Ashen/FogSheet parameter-name checks. `examples/materials` uses the
same material system without a compatibility frontend or script.

Re-export with the current bridge into an external source directory, then run
`tools/ue_preview.py <source> <output> --material-profile <external-profile.json>`
without `--omit-mesh`. Profile format:

```json
{"schema":1,"materials":{"<exact source material path>":{
  "material":{"alpha_mode":"blend","unlit":true,"depth_fade":4,
    "base_color_factor":[0.4,0.5,0.6,0.5],"opacity_texture":"soft-sheet.png"},
  "note":"Explicit approximate conversion; authored opacity mask"
}}}
```

All profile PNGs resolve relative to the profile directory. Unknown schemas,
invalid material values, escaping paths and unmatched material paths fail
visibly. The converter writes a versioned engine-owned material envelope in glTF
extras; it does not compile UE shaders. Source-specific profiles and user assets
remain external. The original material/lighting/gameplay limits below still apply.
God of War III has no new mesh/scene result in this follow-up.

Compilation and behavior are separately recorded in
[ADR 0015](adr/0015-engine-materials-transparency.md),
[engine-materials-results.json](engine-materials-results.json) and
[engine-materials-mcp.json](engine-materials-mcp.json).
Current verification: Rust **162 passed / 11 ignored**, explicit Metal GPU
**9 passed**, Python 3.11 **109 passed**; all failed counts zero. Workspace,
no-default and release compilation passed. Historical results below are retained.

## Actual behavior

| Input | Result | Boundary |
|---|---|---|
| Ashen Vow Windows demo, FantasyCastle map | Actual original static geometry rendered by CrySirror's PBR raster path on Metal | Partial material preview, different lighting, no gameplay or original-image comparison |
| God of War III (Japan), PS3 assets | 16 PSARC directories / 6,041 names read; selected hero WAD decompressed and its 885 records walked completely | No mesh decoder. The Olympus level's unframed `EntityCount` block at `0xd60` is explicitly unsupported; no scene rendered |

Machine-readable results: [local-game-import-results.json](local-game-import-results.json).

Ashen details:

- The shipping EXE contains the UTF-16 string `Unreal Engine 5.7.4`. The
  bridge explicitly uses CUE4Parse's UE 5.7 profile.
- IoStore v8: 6,702 chunks, 4,811 indexed paths, Oodle compression, no
  container encryption. The pre-existing Rust reader handles its package data.
- Existing static-reflection recovery reads the EXE's UHT registration tables.
  The new deterministic `.usmap` writer exports 7,312 schemas and 1,276 enums.
  No game code is put into runtime project data. The recovered schema parsed
  100/100 sampled StaticMeshComponents with the expected UObject tail.
- The CUE4Parse bridge visits all 10,436 exports in `L_FantasyCastle`, finds
  4,816 static mesh components and 132 distinct conventional-LOD meshes,
  and reconstructs 7,710 instances with no caught geometry conversion errors.
  Attachment/template transforms and ISM/HISM instances are included.
- glTF conversion changes centimeters/Z-up/left-handed coordinates into
  meters/Y-up/right-handed coordinates, reverses triangle winding and preserves
  normals, UVs, sections and per-component material overrides.
- The explicit preview profile removes one sky sphere (the engine draws its
  own sky) and 42 instances of the unsupported fog-sheet mesh. Every omission
  is recorded. The final capture submits 7,667 nodes and 13,426,416 conventional
  LOD triangles. This number is submission count, not a GPU timing measurement.
- 31 original base-color textures are decoded to 512-pixel mips. Selection is
  explicitly heuristic: material parameters or a unique base-color suffix in
  the material's TextureStreamingData. Ambiguous names are recorded and fall
  back to neutral material (38 failed material-slot attempts; repeated names
  included). UE shader graphs, texture transforms, ORM/normal/emissive maps,
  exact transparency and lighting are not reproduced. Source alpha is treated
  as an alpha mask and mapped textures use two-sided preview materials.
- Conventional fallback LODs are used; Nanite clusters, skeletal characters,
  landscape, PCG/Niagara, runtime spawning and gameplay are not converted.
  Thus the successful capture does **not** establish normal full-game rendering.

Actual screenshot on this host:
`images/third-party/ashen/castle-close-lit-shadows.png (the textured preview was pruned in the 2026-10-25 cleanup)`.
The preview project/script are in the same directory. The original sky and fog
exclusions, different camera, lighting and approximate materials are intentional
and visible in the report; there is no image-equivalence claim.

## Implemented tools and reproducibility

- `ue-import schemas <shipping.exe> <output.usmap>`: export the existing
  reflection recovery into standard uncompressed v4 schemas. Byte-enum metadata,
  inheritance, static-array dimensions, nested types and explicit enum values
  are retained. Two new Rust tests verify encoding/decoding.
- `tools/ue_scene_bridge`: developer-only C# exporter over an external pinned
  CUE4Parse checkout. It does not add .NET or CUE4Parse to the Rust player.
- `tools/ue_preview.py`: convert bridge geometry, optional base-color textures
  and instance data into a glTF plus an executable CrySirror preview project.
- `tools/psarc.py`: bounded read-only PSARC 1.4/zlib listing and selected-file
  extraction. Output is explicit, external to the source tree, staged and
  published without replacing existing files. It never auto-extracts archive
  paths, which may have an absolute-path flag.
- `tools/gow3_wad.py`: bounded inspection of the observed PS3 big-endian WAD
  record framing. Tags remain opaque. This does not decode meshes, EDGE streams,
  textures or level placement. Unknown level blocks fail with offsets/context.

External references inspected in full checkouts:

- [CUE4Parse](https://github.com/FabianFG/CUE4Parse/tree/460fc477bad0f9db3cfc387365a6699ee246377e),
  Apache-2.0, commit `460fc477bad0f9db3cfc387365a6699ee246377e`.
  Its core and texture-conversion libraries run externally with managed
  decompression/texture decoding. Native CMake builds are disabled.
- [psarc-tool](https://github.com/AlexAltea/psarc-tool/tree/9e8e2e5ce4c58155e8be06c812cd72794997f396),
  commit `9e8e2e5ce4c58155e8be06c812cd72794997f396`, inspected as a format
  reference. No implementation was copied or linked. The new Python reader
  uses the observed big-endian TOC and per-block size table.

On this machine .NET SDK 10.0.402 is installed only in
`local/toolchains/dotnet`, with no PATH/system changes. Build products and
NuGet dependencies are external. The upstream .NET build reports warnings;
this is a local developer trial, not a vetted distributable dependency bundle.

The executed workflow, with the external references already at those revisions:

```sh
cargo build --release -p crysirror-ue-import

target/release/ue-import schemas \
  "$HOME/Downloads/ashen-vow-windows-demo/Portfolio/Binaries/Win64/Portfolio-Win64-Shipping.exe" \
  "local/work/ashen/recovered.usmap"

# Keep .NET's intermediate products outside the source export.
mkdir -p "local/work/ashen/bridge"
cp tools/ue_scene_bridge/Program.cs tools/ue_scene_bridge/Bridge.csproj \
  "local/work/ashen/bridge/"
"local/toolchains/dotnet/dotnet" build \
  "local/work/ashen/bridge/Bridge.csproj" -c Release \
  -p:CUE4PARSE_SKIP_NATIVE=true \
  -p:Cue4ParseRoot="local/reference/CUE4Parse"

"local/toolchains/dotnet/dotnet" \
  "local/work/ashen/bridge/bin/Release/net10.0/Bridge.dll" \
  "$HOME/Downloads/ashen-vow-windows-demo/Portfolio/Content/Paks" \
  "local/work/ashen/recovered.usmap" \
  Portfolio/Content/Maps/FantasyCastle/Map/L_FantasyCastle \
  "local/work/ashen/castle-textured"

python3 tools/ue_preview.py \
  "local/work/ashen/castle-textured" \
  "local/work/ashen/castle-textured-preview" \
  --eye 110 85 85 --target -30 25 -65 \
  --omit-mesh /Game/Maps/FantasyCastle/Meshes/SM_Fog_Plane.SM_Fog_Plane

target/release/crysirror render \
  "local/work/ashen/castle-textured-preview/project.json" \
  --script "local/work/ashen/castle-textured-preview/preview.ts" \
  --frames 1 \
  --output "local/work/ashen/castle-textured-preview/castle.png" \
  --data-dir "local/work/ashen/castle-textured-preview/state"
```

For God of War III the supplied extracted `PS3_GAME/USRDIR` directory was used;
the redundant 37 GB ISO did not need to be mounted or modified. For example:

```sh
python3 tools/psarc.py list \
  "$HOME/Downloads/God of War III (Japan)/God of War III (Japan)/PS3_GAME/USRDIR/RES.PSARC"
python3 tools/psarc.py extract \
  "$HOME/Downloads/God of War III (Japan)/God of War III (Japan)/PS3_GAME/USRDIR/RES.PSARC" \
  --entry /WAD/PS3_BE/R_HERO00.WAD \
  --output "local/work/gow3/another-hero.wad"
python3 tools/gow3_wad.py "local/work/gow3/another-hero.wad"
```

## Compilation and tests

- `cargo check --workspace --all-targets`: passed.
- `cargo test --workspace`: **158 passed, 0 failed, 8 ignored**. The ignored
  six GPU, one caller-font and one RFVP-game tests are not passes in this run.
- Python tools: **106 passed**. New tests use synthetic resources only and
  cover compressed/raw/empty PSARC entries, invalid ranges/corruption, WAD framing
  and unknown level blocks, glTF topology/placement, material overrides, pixel
  channel order and path escapes.
- External .NET bridge: build succeeded; actual schema export and scene export
  succeeded. The actual CrySirror Metal capture succeeded. The GPU smoke suite
  was not rerun for these developer-tool changes.

Logs: `local-game-import-{cargo-check,rust-tests,python-tests}.txt`.
No Windows/Linux execution, original executable compatibility, native save/game
logic, full material fidelity, hardware RT, or frame-rate result is claimed.
