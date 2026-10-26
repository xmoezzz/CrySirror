# Frame pipeline and public demo verification

[ADR 0052](adr/0052-frame-pipeline-streaming-and-profiling.md) defines ownership,
budgets, offline fences and remaining synchronous work. Imported models use
ordinary CrySirror World/resource/render APIs.

## Resources

The examples use Khronos [DamagedHelmet](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/DamagedHelmet)
and [Sponza](https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Sponza).
Source assets and render evidence stay ignored. Keep the download manifests
with their URL/hash/license/credit records alongside the assets.

```sh
python3 tools/fetch_demo_assets.py aaa-helmet
python3 tools/fetch_demo_assets.py aaa-fox-sponza
```

## Shared BLAS and moving instances

Nine textured helmets rotate through ordinary World updates. Equal local
geometry shares BLAS; rigid motion changes TLAS without rebuilding shared BLAS.

```sh
cargo run -p crysirror-desktop -- render examples/aaa-helmet/instances.json \
  --renderer hardware-raytrace --movie-frames 24 \
  --movie-dir target/verification/helmet-frames \
  --output target/verification/helmet.png

cargo run -p crysirror-desktop -- run examples/aaa-helmet/instances.json \
  --renderer hardware-raytrace --no-audio --frames 120 \
  --profile target/verification/live-trace.json
```

`stats.hardware_rt_stats` reports encoded BLAS/TLAS builds, resident BLAS,
instances and CPU geometry-cache hits/misses. Counters are not GPU completion
evidence; captures provide separate pixels. Unsupported hardware/materials fail.

## Stream in, out and back in

The camera crosses load/unload radii. Background preparation feeds atomic owner
commits. Leases protect loaded cells and renderer references protect shared
models. Offline capture uses an explicit readiness fence without another script
or simulation tick. Live windows keep polling ordinary frames.

```sh
cargo run -p crysirror-desktop -- render examples/aaa-helmet/project.json \
  --script examples/aaa-helmet/streaming.ts --renderer hardware-raytrace \
  --movie-frames 56 --step-ms 100 \
  --movie-dir target/verification/streaming-frames \
  --output target/verification/streaming.png

cargo run -p crysirror-desktop -- render examples/aaa-fox-sponza/project.json \
  --script examples/aaa-fox-sponza/streaming.ts --renderer raster --asset-cache-mib 1024 \
  --movie-frames 12 --movie-dir target/verification/sponza-frames \
  --output target/verification/sponza.png
```

Sponza exceeds the default 256 MiB prepared-cache budget; the example explicitly
requests 1024 MiB. This is a host CPU-cache policy, separate from total RAM/VRAM.
This Sponza recipe uses the raster material path. Native alpha-tested RT
casters were added later in ADR 0055; this earlier capture recipe does not
establish that every Sponza material is supported by the RT hit-shading profile.

## Measurements

- `stats.profile.timings.stages`: sample counts and mean/p50/p95/max microseconds.
  CPU nested spans are inclusive.
- `stats.profile.cpu_trace`: Chrome CPU trace with worker threads/frame IDs.
  A `run --profile` file has `traceEvents` at its root for trace viewers.
- GPU timestamps use an uncalibrated separate clock. Check enabled, pending,
  dropped and failed counts. Invalid/zero samples are omitted.
- `stats.frame_samples`: up to 256 records of world counts, prepared CPU asset
  bytes, RT counters and capture wall time. Capture includes warmup/readback;
  PNG writing occurs afterward. These are not interactive FPS measurements.

```ts
const timings = profiler.snapshot();
const cpuTrace = profiler.trace();
profiler.clear();
```

Asset GUID databases/cooking, chunk/mip streaming, budgeted GPU uploads,
simulation parallelism, BLAS refit and an editor/profiler overlay remain work.
This iteration does not mark engine-wide ADR 0050 complete.

## Glossy reflections and per-pixel GI

The same Khronos model can exercise the newer lighting APIs (ADR 0060):

```sh
cargo run -p crysirror-desktop -- render examples/aaa-helmet/project.json \
  --script examples/aaa-helmet/ray-lighting.ts --renderer hardware-raytrace \
  --movie-frames 4 --movie-dir target/verification/ray-lighting-frames \
  --output target/verification/ray-lighting.png
```

The script keeps the glTF material maps, places three metal panels with
different roughness values, adds a local light and enables native diffuse GI.
Ray-traced reflections and GI share a two-layer sampled array at the existing
SSR binding. This does not increase the forward pass's sampled-texture count.

For the reproducible synthetic-room cost comparison:

```sh
cargo test -p crysirror-wgpu --test rt_lighting \
  compare_raster_probe_rt_probe_and_per_pixel_frame_cost -- --ignored --nocapture --test-threads=1
```

The JSON report records device, resolution, warmup, actual GPU frame samples,
probe update budget and rays per pixel. This measures the selected workload;
probe and per-pixel settings do not imply equal image quality. It is neither
a universal speedup claim nor an interactive FPS measurement.
