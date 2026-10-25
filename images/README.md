# Images

Representative renders for documentation and promotion.

- `engine/`: renders of CrySirror's own test scenes and demos. Some use
  public sample assets: Sponza, DamagedHelmet and Fox are from the Khronos
  glTF samples; see `tools/fetch_demo_assets.py` for their licences and
  credits.
- `third-party/`: renders and movies made from the owner's local game data
  (God of War III, Ashen Vow, Siglus titles). Each title's rights holder
  owns that content. The folder is gitignored. Get permission before
  publishing any of it.

Contents of `third-party/` (2026-10-25):
- **gow3:** the OLY100 hall with:
  - lights, GI, LOD, native and engine probes and atmosphere;
  - the walking-character slice (`hall-walk.mp4` and sheets);
  - the level sequence (`hall-sequence.mp4`).
- **ashen:** Fantasy Castle renders: lit with shadows, VSM, night
  atmosphere, unlit, engine materials and shader presets.
- **siglus:** native Siglus resource renders.
