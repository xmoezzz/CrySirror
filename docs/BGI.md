# BGI adapter

The default desktop build registers `bgi`. It runs the original BGI interpreter,
with CrySirror providing image composition, glyph rasterization, channel audio,
time/input and capability storage. No TypeScript/data-VM rewrite is required.
The attributed CPU source and port changes are in crates/bgi-upstream.

Keep the game and licensed font outside this repository. A project directory
can contain `project.json`, `assets/game/` (game files) and `assets/font.ttc`.
Supply the font yourself; no font is bundled or downloaded by the adapter.

```json
{
  "schema": 2,
  "name": "BGI project",
  "content_revision": "1",
  "assets": "assets",
  "canvas": [1280, 720],
  "clients": [{
    "name": "game", "adapter": "bgi", "game": "your-game-id",
    "config": {
      "root": "game", "font": "font.ttc", "font_face_index": 0,
      "gbk": false, "strict": true, "silent": false
    }
  }]
}
```

All config paths are relative to this client's asset capability. Use the game's
native canvas size. `gbk:false` uses CP932/Shift-JIS; `gbk:true` selects GBK.
Strict unknown-operation errors cannot be disabled. `silent:true` explicitly
discards native audio for diagnostics; normal audio uses shared CPU decoding and
mixing and the desktop output device. `--no-audio` mutes device output while
allowing non-silent decoding/control to be checked.

```sh
cargo run --release -p crysirror-desktop -- inspect /absolute/project.json
cargo run --release -p crysirror-desktop -- run /absolute/project.json \
  --data-dir /absolute/private-state
```

Native archive `system.arc:ipl._bp` boots through client-scoped streaming reads.
Original in-game save slots use the save mount under the selected state root;
they do not write back into the game directory. Existing saves inside resource
UserData are not silently imported. One writer lease protects that namespace;
close an existing session before reopening the same game/state root. Lease files
may remain as empty files; their OS locks release on drop/process exit.

For offline time/input/capture checks, the adapter example drives the same VM:

```sh
cargo run --release -p crysirror-bgi --example native_trace -- \
  /absolute/project.json /absolute/private-state 3600 \
  --click 240 85 455 --enter 1200 --click 1800 650 620 \
  --capture-at 1799 /absolute/before.png --capture /absolute/after.png
```

Click coordinates and frame timings above belong to the locally tested trial,
not a general automation protocol for every BGI game. A click has a held frame
and a release frame. The example reports VM/draw/PCM checkpoints, creates no
devices and writes captures only outside the repository. Render/trace commands
also run original VMs; CPU service tests are distinct from actual GPU behavior.

The validated profile is the local BGI 1.622 senmomo trial opening. It does not
establish compatibility for all BGI games, complete trial completion, every
native effect or movie container. Runtime snapshots/general native-save import
staging remain unsupported; use the original in-game save UI. Host modal dialogs,
installers, IME and native operations without a service fail explicitly. Native
rename and persistent empty-directory semantics are not implemented. Desktop
Windows/Linux are targets but were not tested by the local macOS run.

The display compositor preserves the original Rust port's float coverage for
known selectors 1/0x20; packed native bitmap operations use the separate exact
integer kernels. Do not infer commercial-renderer equivalence from that mapping.
CPU composition is a correctness path; performance/complete GPU-native effect
migration remains future work. See ADR 0039 and bgi-integration-results.json for
source provenance, actual commands, tests and the limits of each claim.
