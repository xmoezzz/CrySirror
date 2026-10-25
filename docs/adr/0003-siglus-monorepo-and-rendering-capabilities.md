# ADR 0003: Siglus resource compatibility and desktop rendering capabilities

Status: resource slice and compute ray tracer implemented and executed locally;
original VM integration and hardware acceleration remain incomplete.

## Monorepo split

All production crates are local workspace members with path dependencies. Building
the player does not require an external Siglus checkout. Complete upstream source
is retained separately for read-only reference/differential validation, not as a
second workspace or as an unpinned Git dependency.

| Shared CrySirror implementation | Siglus-specific compatibility |
|---|---|
| core::binary checked range/endian reads | G00 header/cut/chip layouts |
| crysirror-image RGBA surfaces, channel conversion, PNG/JPEG/BMP decoding, decoder registry and bounds | Native LZSS, palette semantics, type-3 format scrambling |
| Runtime per-client capability roots and asset byte caches | PATNO slot identity, missing/reused cuts, pivots and original dimensions |
| wgpu texture uploads, compositing, raster/compute rendering | Future original SceneVm, values, forms/syscalls, waits and original saves |
| Desktop window/device/audio lifecycle | Native object/text/audio policies translated at audited service boundaries |

The new crysirror-siglus crate has no window/GPU/audio SDK dependencies and forbids
unsafe code. Its full normal dependency tree was checked with cargo tree. The
existing core remains dependency-free/no_std; crysirror-compat depends only on it.

The desktop installs the G00 decoder through the default `siglus-resources`
feature. The shared runtime/image/renderer crates do not depend on Siglus. The
wgpu crate uses Siglus only as a dev-dependency for the integration test.
`--no-default-features` removes the desktop audio and G00 plugins.

## Executable Siglus resource slice

The port derives from the actual runtime decoder at siglus_rs commit
`5f358ef1360b0439e92aed2b749e8f25d8721954`. Complete relevant resource source files
were inspected; source hashes, provenance and port changes are recorded in
crates/crysirror-siglus/UPSTREAM.md and upstream-files.json.

The original runtime and the format-tool G00 decoders are different. The runtime
preserves empty PATNO slots and signed reused-cut lengths and crops by the display
rectangle. These behaviors are retained; selecting the format-tool parser instead
would have changed them. Common RGBA/JPEG/endian utilities were extracted, while
native framing, LZSS and metadata remain in the compatibility crate.

Native decode returns all frames plus centers and original dimensions. The generic
sprite decoder accepts one frame with no native pivot/crop policy. Multi-frame or
metadata-bearing files error rather than dropping that information. Future native
OBJECT/PCT code must explicitly select and transform frames before submission.
Decoded/input buffers have 64 MiB limits, dimensions 8192 and cut count 4096;
out-of-profile files fail visibly. Total frame pixels are also bounded.

The original decoder was compiled in a separate temporary resource harness and
executed against the same synthetic fixtures as the port. Type 0 overlapping
matches, type 1 palette/alpha, and type 2 reused/missing/cropped cuts matched in
pixels and metadata. Type 3 JPEG is separately tested against shared JPEG output.
No user game/font data is needed or distributed. **This does not execute SceneVm,
and no native Siglus VM factory is registered.**

Reproduce:

```sh
python3 tools/compare_siglus_resources.py --source-root /path/to/pinned/siglus_rs
cargo test --locked -p crysirror-siglus
python3 examples/siglus/create_resource_fixture.py /tmp/crysirror-siglus-demo
cargo run --locked -p crysirror-desktop -- render /tmp/crysirror-siglus-demo/project.json --output dist/siglus-resource.png
```

The demonstration intentionally uses `crysirror-script-v1` to submit a native G00
resource. Its name/output reports that fact. It is not a converted Siglus program.

## Desktop platforms

The user selected Windows, Linux and macOS first. Mobile and Web are later work;
Switch and PS Vita remain deferred, with no other console scope.

wgpu backend selection belongs to the host: DX12/Vulkan/Metal are desktop backend
routes, not fields in VM state. The renderer remains independent of winit. This
iteration was executed only on macOS/Apple M5/Metal. Existing CI defines all three
desktop jobs but has not run here; Windows/Linux behavior is not claimed proven.

## Ray tracing as an explicit capability

`core::graphics` defines SDK-independent RenderingPath and GraphicsCapabilities.
Required capabilities either succeed or error; no implicit fallback is performed.
The desktop `gpu-info` command distinguishes adapter-advertised features from
renderer implementations. Device pointers/acceleration structures never enter
project data, native VM values or saves.

| Path | Implemented behavior |
|---|---|
| raster | Existing depth-tested opaque triangles, then ordered 2D/text overlays |
| compute-raytrace | Real GPU compute primary rays, triangle intersections and directional hard-shadow rays; then the same overlays |
| hardware-raytrace | Explicit unsupported error; no image or silent raster substitution |

Compute traversal is a bounded linear triangle loop, not a BVH/path tracer or
hardware acceleration. It uses transformed scene geometry and camera clipping,
supports up to 4096 triangles, 1,048,576 pixels and 256 million pixel/triangle
pairs, and checks device dimensions. Work exceeding those limits errors. Color
is shaded in linear space, stored as RGBA16F and composited to the sRGB
target. It does not provide PBR, reflections, GI, denoising, motion blur or
production performance. The current implementation rebuilds ray buffers/textures
per frame, so it is a correctness foundation, not an optimized frame graph.

```sh
cargo run --locked -p crysirror-desktop -- gpu-info
cargo run --locked -p crysirror-desktop -- render examples/desktop/arena.json --renderer compute-raytrace --output dist/arena-raytrace.png
cargo run --locked -p crysirror-desktop -- run examples/desktop/arena.json --renderer compute-raytrace --no-audio
```

wgpu 0.20's type definitions expose native ray query/acceleration feature flags,
but this engine does not wire hardware acceleration resource creation or dispatch.
The local Metal adapter advertises neither of those flags through this API.
Future hardware work must select and validate a suitable backend/API generation;
an upgrade and real device tests are prerequisites. See the upstream
[ray tracing design](https://github.com/gfx-rs/wgpu/blob/trunk/docs/api-specs/ray_tracing.md)
for its explicitly experimental API direction; names there are not assumed to be
APIs available in this pinned generation.

## Next Siglus execution work

The complete VM/CommandContext dependency seam remains the next integration task.
CommandContext currently owns image/layer/font/audio/movie state and synchronous
capture callbacks; SiglusHost owns redraw/pump/wait order. Split these through
tested services before linking the original SceneVm. Do not replace its values,
opcode dispatch, proc switching, clocks or native save layouts with the data VM.
Original VM compilation, legal native scene execution, logical/pixel/audio trace
comparison and save/restore validation remain separate required gates.
