# Desktop fixtures

These are self-authored service/program fixtures, not original game data.
Both clients intentionally use local visual 0 and `panel.png`, but separate
asset prefixes. They share one 3D scene and one compositor.

- `project.json`: no font or audio device needed. Enter/Space/click advances both clients.
- `text.json`: use a local licensed font via `--font /path/to/font.ttf`. No fonts are distributed.
- `audio.json`: first advance also plays a short self-generated WAV tone.

No fixture executes RFVP HCB or Siglus bytecode. The offscreen GPU test and TS
arena capture were executed on macOS/Metal; see STATUS.md for evidence.

## Optional TypeScript arena

`arena.ts` generates `arena.json` using the Rust-only Oxc/Boa authoring tool.
It contains a static floor and nine pillars. Build/run instructions and current
limitations are in [AUTHORING.md](../../docs/AUTHORING.md).

