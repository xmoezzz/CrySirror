# 0042: Need for Speed: Carbon (PC) as an import target, starting with the canyons

Date: 2026-10-25. Status: the canyon renders in the engine with decoded
geometry and textures (see "Geometry and textures" below). Scenery
instancing, normal and specular maps, lighting and vehicles are pending.

## Context

The owner supplied `NFSCarbonCE_JP.iso` (PC, Collector's Edition, Japanese)
and wants the canyon areas loaded, to drive more engine capability:
- large open-world streaming;
- vehicles;
- road surfaces;
- night lighting.

The ISO is kept at `local/sources/nfs-carbon/` (gitignored; local/README.md).

Community research the owner pointed to, cloned as references into
`local/reference/` with their commits pinned:

| Project | Commit | Licence | Use |
|---|---|---|---|
| MaxHwoy/hyperlinked | f1f3fbfd | BSD-3 | Chunk ids, runtime structs (streamer, scenery, geometry, textures); can be ported with attribution |
| berkayylmao/NFSPluginSDK | d238bdff | BSD-3 | Game object types (Attrib, PVehicle, …) |
| xan1242/NFSC_CustomVT | 0142b4bf | MIT | Visual Treatment (bloom/HDR post) shaders |
| MaxHwoy/Binary | 799adf9f | **GPL-3.0** | Format documentation only; no code may enter the MPL code base |

Not cloned: nfs-carbon-recomp, which recompiles the Xbox 360 executable;
it is not needed for PC data import.

## Findings (tool output on the owner's disc)

- **Disc layout.** The disc is an EA installer. Game files are inside
  `0compressed.zip` and `1compressed.zip`, which are deflated and readable
  directly, with no extraction. Contents include:
  - `TRACKS/L5RA.BUN` (1.1 MB track header) and `TRACKS/STREAML5RA.BUN`
    (1.24 GB of streamed sections), together the Palmont world including
    the canyons;
  - `TRACKS/L5RA/*` (track maps, trough boundary);
  - `GLOBAL/*` (global textures and attributes);
  - `CARS/*/GEOMETRY.BIN` and `NIS/*` (cinematics).
- **Streaming sections.** `tools/nfsc_bun.py sections` parses the 92-byte
  32-bit `streamer::section` records from chunk `0x00034110`:
  - 962 sections with names like `A41` (number 141) or `T51` (2051);
  - each record holds the stream offset and size, the centre and radius in
    world XZ, and a checksum;
  - offsets increase and all fall inside the 1,237 MB stream, with
    1,176 MB in total.
- **Section content.** `section-stats` walks the chunk tree of a section
  without overruns. A41 and T51 each contain:
  - solid lists (`0x80134000`/`0x80134010`/`0x80134100`);
  - vertex and index data (`0x00134B01`/`0x00134B02`/`0x00134B03`);
  - texture packs (`0xB3300000` plus VRAM data `0x33320002`);
  - scenery sections (infos, instances, tree nodes);
  - event triggers.

  The geometry ids are not in the hyperlinked enum. They are recorded as
  unknown until decoded.

## Decision (plan)

1. **Geometry.** Decode solid lists into meshes: vertex formats, index
   streams and material/texture references. Use the hyperlinked runtime
   structs as BSD reference; read Binary only as documentation. Validate
   on decoded triangles, for example closed road surfaces and continuity
   across neighbouring sections.
2. **Textures.** Decode texture packs to DDS/BCn through the shared codecs.
3. **Scenery placement.** Instances, matrices and the solid hash → mesh
   mapping produce a glTF scene per section, assembled by section centre.
4. **Canyon selection.** Identify the canyon sections from section centres
   and radii, scenery names, and `TRACKS/L5RA/TrackMaps.bin`. Do not
   guess; list the evidence.
5. **Engine.** Stream sections through the existing streaming-level API,
   with mesh collision for the road, then a vehicle controller as an engine
   capability (ADR 0041 lists vehicles as missing).

Unknown chunk ids and layouts fail visibly. No game data enters the
repository.

## Validation so far

- Python `tests/test_nfsc_bun.py` (synthetic): nested chunk walking,
  overrun rejection and the section record layout.
- Local runs on the disc as listed above. These are reported findings, not
  decoded content.

## Geometry and textures (2026-10-25)

`tools/nfsc_geometry.py` (solids → glTF) and `tools/nfsc_textures.py` (TPK →
DDS → PNG through the engine's `transcode` codec) are our own decoders.
Field offsets follow the BSD-3 hyperlinked runtime structs; everything else
below was measured on the data and is checked while decoding.

- **Solids.** Each chunk's data starts on a 16-byte file boundary with 0x11
  fill, except texture keys and material names.
  - The header has the name at +160, the bounding box and the pivot.
  - Mesh entries are 144 bytes: texture slots, shader type, vertex count,
    triangle count, index start and index count.
  - Vertex buffers are consumed in entry order. Indices are u16 and
    relative to the entry's buffer.
- **Vertex strides per shader type.** Measured from buffers that hold one
  entry, across all 962 sections:

  | Shader type | Stride (bytes) |
  |---|---|
  | 0, 25, 26 | 36 |
  | 1, 3 | 52 |
  | 4, 5 | 48 |
  | 15 | 44 |

  There are two layouts:
  - Layout A (types 0/1/3/4/15): position, normal, BGRA colour, UV.
  - Layout B (types 5/25/26: water and decals): position, colour, UV,
    normal.

  The choice is evidence-based: layout B turns 1.8% (type 5) and 0%
  (types 25/26) unit normals into 100%.
- **Whole-stream run.** All 962 sections decode: 40,677 solids and
  4,618,241 triangles. Every exported vertex normal is unit length.
- **Coordinates.** World coordinates are Z-up metres and are exported as
  (x, z, −y). That mapping is a rotation, so the winding is kept: 90.9% of
  triangles agree with their vertex normals. Nearly all of the rest are
  two-sided foliage cards. Reversing the winding (the first attempt) agreed
  for only 21%.
- **Textures.**
  - A texture record is 88 bytes plus a length-prefixed name. Image data
    sits at a 128-byte-aligned VRAM base.
  - The index covers 3,496 textures from 621 section packs and the global
    packs: DXT1 3,984, DXT5 2,399, DXT3 198, RGBA8 49.
  - A few GLOBAL variants (DYNTEX, track maps) use another layout. They are
    reported and not needed for the canyon.
  - Transparency follows the texture's blend type, not its alpha usage:
    terrain and cliffs are "modulated" but blend type 0, so they are drawn
    opaque. Punch-through textures become alpha-masked and double-sided.
- **Canyon identification.** Evidence, not a guess:
  - Solid names `RD_TNDRIFTB_ROADB` and `TN_TNDRIFTB_ROCK_*` match the disc
    movie `canyon_tndriftb`.
  - Section pairs P1–P9 (road and vegetation) and P41–P52 (terrain) share
    centres.
  - Other canyons: MUDRIFT in P13–P22 and P53–P64; CTDRIFT and EXDRIFT in
    the Q, R and S groups.
- **Export.** TNDRIFTB, 17 sections:
  - 1,315 solids, 98,431 triangles and 55 textured materials;
  - all 2,060 diffuse keys resolved;
  - rendered by the engine with sun, sky, fog, cascaded shadows and TAA
    (`images/third-party/nfsc/canyon-tndriftb.png`, gitignored).
- **Tests.** Python synthetic tests cover the solid decode (Y-up mapping,
  BGRA colours, materials, texture keys), unknown shader types failing,
  TPK records, and the DDS header with mip dropping.

### Next

- Scenery instances (props placed by matrices) and the X/Y/Z shared
  sections.
- Normal and specular slots.
- Car geometry (`CARS/*/GEOMETRY.BIN`).
- A vehicle controller in the engine.
- Streaming the canyon sections through the streaming-level API.
