# Engine-owned material gallery

A self-authored, script-free project. Every material lives in `systems.entities`;
no game files, compatibility VM, font or downloaded asset is required. The one
1×1 opacity PNG is an authored 50% red-channel data sample.

```sh
cargo run --release -p crysirror-desktop -- render examples/materials/project.json \
  --output dist/engine-materials.png --data-dir /tmp/crysirror-materials-state
```

Read left to right:

| Row | Left | Middle | Right |
|---|---|---|---|
| Top | Opaque | Alpha mask (checker cells, cutoff 0.5) | Alpha blend |
| Bottom | Additive | Premultiplied output from straight color inputs | Independent opacity × soft depth intersection |

The last panel fades more over the closer bars than over the backplate. The
unlit colors make the compositing visible independently of environment lighting.
MCP `project_validate`, `engine_trace` and `scene_capture` work on this project.

Optional code uses exactly the same contract:

```ts
const pane = world.spawn({name: "pane", geometry: "cube"});
pane.setMaterial({
  alpha_mode: "blend", base_color_factor: [0.2, 0.5, 0.8, 0.4],
  unlit: true, double_sided: true, depth_fade: 2
});
pane.setMaterial(null); // restore the geometry's original material
```

Rust: `crysirror_scene::material::MaterialSpec` and `WorldCommand::SetMaterial`.
All texture paths resolve inside that client's asset root. Raw cube/triangle UVs
are zero; authored glTF UVs are needed for spatially varying texture detail.
The complete contract, evidence and limits are in
[ADR 0015](../../docs/adr/0015-engine-materials-transparency.md).
