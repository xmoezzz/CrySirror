# Migration gates

The default implementation path is now **wgpu + winit desktop first**.
Console-specific implementations are deferred as requested.

## 0. Establish the desktop implementation baseline

Compile the supplied desktop/runtime/rendering source, resolve Cargo.lock,
run all Rust tests, run the ignored real-GPU test explicitly, and run a window
smoke test. Python/tooling tests alone do not satisfy this gate. Record failures
instead of treating CI configuration as proof of a passing run.

## 1. Acquire and pin original source

Fetch rfvp and siglus_rs, inspect actual VM-to-host seams, and capture legal
reference fixtures. Preserve upstream source/license notices. See
NATIVE_ADAPTERS.md for precise destination APIs and required behavior evidence.

## 2. Connect RFVP, then Siglus

Keep each original interpreter, instruction/data model and private save state.
Route compatible operations to one shared host, renderer and audio system.
Keep behavior differences in adapters/profiles. Do not silently approximate
unknown text layouts or promote demonstration scripts into native-VM claims.

## 3. Validate the shared implementations

The source now includes textures/compositing, simple text, audio, VFS and staged
whole-runtime restore. Validate their runtime behavior, then extend the seams
for real native image/text/audio/archive formats. Original save import remains
a separate feature with golden files. Audio sample-exact resume is not present.

## 4. Broaden 3D and agent tooling from real use cases

The static 3D renderer and project-level MCP server are initial implementations.
Add logical 3D resources, animation/physics and eventually road/vehicle recipes
when a game requires them. Live session control, arbitrary asset generation
providers and a racing game do not exist in this snapshot.

## 5. Switch and PS Vita only

After desktop native behavior is validated, use audited platform work and
legitimate SDKs to build independent window/render/audio/package backends.
Do not assume a desktop wgpu configuration maps onto either console. No other
console is currently in scope. Only hardware/emulator evidence establishes
console support.
