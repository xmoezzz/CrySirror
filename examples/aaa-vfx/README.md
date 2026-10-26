# Modular VFX laboratory

This example authors engine effects through `Vfx` and `Entity`, using the
public Khronos DamagedHelmet model and Poly Haven HDR from `aaa-helmet` as a
real PBR scene. It exercises resident GPU flipbook fire, CPU soft smoke, mesh
shards, collision-triggered child sparks and a moving world-space ribbon.
There is no script-side particle loop. Press **Q** to pause/resume fire and
**E** to restart it. The script updates a typed wind parameter without resetting
particles. Assets and generated images are ignored, never embedded in source.

From the repository root:

```sh
python3 tools/fetch_demo_assets.py aaa-helmet
python3 tools/prepare_vfx_demo.py
cargo run --locked -p crysirror-desktop -- render examples/aaa-vfx/project.json --script examples/aaa-vfx/game.ts --frames 121 --output target/vfx/lab.png
cargo run --locked -p crysirror-desktop -- run examples/aaa-vfx/project.json --script examples/aaa-vfx/game.ts --no-audio
```

The procedural eight-frame flame texture is generated from our Python source.
The preparation command verifies the existing source manifest hashes before
copying the model/HDR and preserves their attribution in `THIRD_PARTY.json`.
If `aaa-helmet/assets` is already prepared, downloading again is unnecessary.

`stress.ts` initializes 100,000 GPU-resident particles in a time-zero burst and
executes noise, drag and orbit modules. It needs no external assets (create the
empty `assets` directory first). For a timeline longer than the 32-tick command
window, use:

```sh
mkdir -p examples/aaa-vfx/assets
cargo run --locked -p crysirror-desktop -- render examples/aaa-vfx/project.json --script examples/aaa-vfx/stress.ts --frames 121 --output target/vfx/100k.png
cargo run --locked -p crysirror-desktop -- run examples/aaa-vfx/project.json --script examples/aaa-vfx/stress.ts --frames 60 --capture target/vfx/live-100k.png --profile target/vfx/live-profile.json --no-audio
```

The JSON output reports CPU simulation separately from GPU capacity, encoded
work and draw counters. Live GPU particle counts remain `null`: reading an
indirect buffer in an explicit device test is not a live telemetry service.
Offline capture includes asset preparation, temporal warmup, GPU completion and
image readback, so capture wall time is not an interactive frame benchmark.
For isolated GPU simulation measurements run the explicit tests in
`crysirror-wgpu/tests/vfx_compute.rs`; timestamp support is checked on device.

See [ADR 0064](../../docs/adr/0064-modular-vfx-cpu-gpu.md) for ownership,
backpressure and the remaining Niagara capability gaps. No Unreal code or
Niagara assets are imported, and this example is not proof of full parity.
