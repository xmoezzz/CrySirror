# CrySirror

A data-driven Rust runtime with a shared **wgpu + winit desktop backend**,
independent compatibility adapters, common rendering/audio/file services, and
local MCP authoring tools.

**This is the 0.2 desktop foundation, not a completed RFVP/Siglus merge.**
The workspace now compiles/tests locally and the offscreen GPU pixel test has
been executed. Complete upstream checkouts are pinned, but their VMs have not
been connected. The only executable adapter in this source tree is the explicit
`crysirror-script-v1` data-program VM. See [STATUS.md](STATUS.md) for measured
results rather than inferred compatibility.

The long-term target is a modern, data-driven third-person 3D engine with MCP
authoring/inspection. [ADR 0002](docs/adr/0002-modern-engine-and-optional-typescript.md)
sets the architecture and measurable milestones. Optional TS/JS authoring now
uses the Rust Oxc compiler and Boa JS engine to generate validated project data;
see [AUTHORING.md](docs/AUTHORING.md) and `examples/desktop/arena.ts`.

The monorepo now includes a separate `crysirror-siglus` resource frontend and
shared `crysirror-image` services. Native G00 pixels/metadata have been compared
with the executed original decoder. The renderer also has an executed GPU
compute ray tracer with directional shadows; hardware ray tracing is still
unsupported and fails explicitly. [ADR 0003](docs/adr/0003-siglus-monorepo-and-rendering-capabilities.md)
documents the boundaries, commands and desktop platform scope.

The engine owns its [Audio/SFX, Text, UI, Layer and World systems](docs/ENGINE_SYSTEMS.md).
Audio scheduling/buses and CPU text layout/rasterization now have separate engine
crates; desktop/wgpu provide lifecycle/output. UI, full layer composition and a
dynamic world remain future implementations, beyond today's flat visual/static
scene model. Compatibility frontends preserve native policies on those services.

## Architecture

```text
Original RFVP VM      Original Siglus VM      New data programs
     not linked            not linked         implemented source
         |                     |                      |
         +------- distinct VmAdapter factories ------+
                               |
                  Runtime / SharedHost / data model
                  |          |           |         |
              scene/text   files       audio     snapshots
                  |                      |
          crysirror-wgpu            desktop audio
                  |                      |
                  +---- crysirror-desktop ---------+
                             winit event loop
```

`crysirror-core` and `crysirror-compat` remain independent of window/GPU/audio
SDKs. The default desktop executable owns `winit`; the renderer uses `wgpu`
and does not depend on `winit`. Switch and PS Vita backends are deferred,
not conditional claims of support hidden inside the desktop implementation.

The default dependencies pin `wgpu = 0.20.1` and `winit = 0.30.12`. This is a
chosen API baseline, not a claim that these are the latest versions or that
upstream engines already compile against it. Cargo.lock now records a real,
successfully built dependency resolution. Resolved dependencies require Rust
1.88; the workspace declares that minimum, but only 1.98.1 was tested here.
A full dependency license audit is still pending.

## What is in the implementation

| Component | Implemented source | Important boundary |
|---|---|---|
| Window/input | winit ApplicationHandler, resize/suspend, keyboard/mouse, logical pointer mapping | Desktop only; interactive input not yet verified |
| 2D graphics | GPU textures, ordered alpha-blended quads, letterboxed canvas, bounded caches | PNG/JPEG/BMP, not native encrypted archives |
| Siglus resources | Separate native G00 decoder, shared image/JPEG/upload services | Original SceneVm/syscalls/saves not linked |
| Text | Supplied-font rasterization, simple wrapping/kerning, cached text surfaces | No bundled font, ruby/kinsoku/native layout not emulated |
| 3D | Camera, transformed cubes/indexed triangles, lighting, depth buffer | Static opaque prototype, not PBR/animation/physics |
| Ray tracing | Optional GPU compute primary/shadow rays, same 2D overlays | Software traversal; no hardware acceleration, GI or reflections |
| Audio | rodio output, namespaced channels, retrigger/loop/stop/gain | Native game codecs/semantics not verified |
| Files | Per-client capability directories, read-only assets, bounded atomic writes | Not a sandbox for arbitrary native Rust code |
| Saves | VM + shared-state snapshot, staged all-VM restore, version/identity/checksum checks | Not an importer for original saves; audio restarts |
| Data VM | Explicit service instructions, time/input waits, PC snapshots, budget | Not RFVP HCB or Siglus bytecode |
| TS/JS authoring | Oxc transform + Boa buildProject() -> validated JSON | Offline trusted recipes; no TS type checking or per-frame JS |
| MCP | stdio handshake, inspect/validate/replace, capture/package tools | Project authoring, not a live native VM debugger |
| Packaging | Existing host + explicit assets + hashes + own source/notices | Does not compile, sign, or bundle every OS library |

## Build and run

Use a Rust development machine with an appropriate graphics driver. The locked
workspace checks/tests have run on macOS with Rust 1.98.1. Other OSes and the
minimum Rust version still need validation. Interactive window/audio behavior
requires separate checks; see STATUS.md.

```sh
cargo check --locked --workspace --all-targets
cargo test --locked --workspace --all-targets
cargo run --locked -p crysirror-desktop -- run examples/desktop/project.json --no-audio
```

On Debian/Ubuntu, desktop builds typically need native development packages for
ALSA, X11/XKB and Wayland. Validate against the local platform libraries.
Build without audio when those audio libraries are not available:

```sh
cargo run -p crysirror-desktop --no-default-features -- run examples/desktop/project.json --no-audio
```

The fixture intentionally gives two clients the same local visual ID and asset
filename. Separate client asset roots should draw a green panel on the left
and an orange panel on the right, over one shared static 3D scene. That is a
test of the new service implementation, not proof of native compatibility.

| Input | Action |
|---|---|
| Enter / Space / mouse click | Advance a data-script input wait |
| F5 | Save whole-runtime snapshot |
| F9 | Restore snapshot transactionally |
| F12 | Capture through the GPU renderer |
| Escape | Exit |

The `text.json` fixture needs a font you provide locally:

```sh
cargo run -p crysirror-desktop -- run examples/desktop/text.json --font /path/to/your-font.ttf --no-audio
cargo run -p crysirror-desktop -- run examples/desktop/audio.json
```

For a distributable text project, declare a properly licensed font in
`project.fonts` rather than relying on a developer's absolute `--font` path.
No fonts are distributed in this repository or its source artifacts.

## Inspect, trace and capture

```sh
cargo run -p crysirror-desktop -- inspect examples/desktop/project.json
cargo run -p crysirror-desktop -- trace examples/desktop/project.json --frames 120 --press-at 1
cargo run -p crysirror-desktop -- render examples/desktop/project.json --output dist/capture.png
```

`trace` executes VM ticks without opening a window or emitting sound. `render`
uses the actual wgpu pipeline offscreen and requires a working GPU/backend,
including a software Vulkan adapter where available. It never substitutes a
fabricated PNG when GPU initialization fails.

The former bootstrap CLI remains available under a different binary name:

```sh
cargo run -p crysirror-cli --bin crysirror-contract -- replay examples/fusion.json
```

Schema 1 bootstrap transcripts and schema 2 desktop projects are separate.

## MCP authoring tools

Read-only project tooling works without Rust and was exercised in a real stdio
subprocess test:

```sh
python3 tools/mcp_server.py --project examples/desktop/project.json
```

After compiling the host, enable project replacement and captures explicitly:

```sh
cargo build --release -p crysirror-desktop
python3 tools/mcp_server.py --project examples/desktop/project.json \
  --executable target/release/crysirror --allow-write
```

Tools: `project_inspect`, `project_validate`, optional `project_replace`,
`scene_capture`, `game_package`. The last two need a configured executable.
The server implements newline-delimited stdio JSON-RPC with protocol
`2025-06-18`, initialization and tool errors. It has no shell-execution tool
and no HTTP listener. stdout is reserved for protocol responses.

A capture starts a fresh host from project data, not the currently running
window. Replacing JSON does not hot-reload an existing process. There is no
external generative-model provider or automatic 3D asset generator included.

## Packaging

```sh
python3 tools/desktop_project.py examples/desktop/project.json --assets
python3 tools/package_game.py --project examples/desktop/project.json \
  --executable target/release/crysirror --target linux --output dist/CrySirror-linux.zip
```

Use a host built for the requested OS. Windows uses `crysirror.exe`. The tool
checks executable format but does not prove execution, ABI compatibility,
code signing or third-party-license completeness. It includes the current
CrySirror engine sources and own notices. The distributor must verify those
sources correspond to the executable and audit dependency/asset notices.

For asset transfer only, use `--data-only` in place of `--executable/--target`.
The resulting archive explicitly says `runtime_included: false`.

## Verification

```sh
python3 -m unittest discover -s tests -v
python3 tools/verify.py
python3 tools/verify.py --gpu
```

`verify.py` exits with an error if cargo is absent. Python tests are never
reported as Rust or GPU tests. Validation is local by default. GitHub CI has
been removed at the owner's request; do not add or enable remote build automation
without explicit authorization. Cross-platform results require actual evidence.

[ADR 0050](docs/adr/0050-engine-wide-async-execution.md) requires asynchronous,
multithreaded support throughout the engine. Current asset/pose workers are
only stage one.

## Connecting the original VMs

[docs/NATIVE_ADAPTERS.md](docs/NATIVE_ADAPTERS.md) describes the exact new
factory/VM/host interfaces and the remaining work. Existing VM semantics stay
in their respective adapters. Do not substitute the demo interpreter for an
unknown native opcode or silently approximate a native text layout.

```sh
python3 tools/upstreams.py fetch --source-root ../CrySirror-references
python3 tools/upstreams.py audit --source-root ../CrySirror-references > upstream-audit.json
```

Fetching records real commits. It does not automatically refactor, merge or
register either native VM. Original source is now pinned and locally available;
original-game behavior remains unverified. Keep reference checkouts outside the
deliverable because their font/game fixtures must not be redistributed.

## Repository publication

This workspace is currently a source export without a .git directory. No remote
was created in this iteration. The publication helper requires a separately
prepared clean Git checkout and an authorized local GitHub CLI session:

```sh
python3 tools/publish_github.py --private
```

The script checks that the account is `xmoezzz`, refuses an existing origin and
dirty worktree, does not force-push, and defaults to private visibility.

## License

New code and self-authored demo assets: MPL-2.0. See [LICENSE](LICENSE) and
[NOTICE.md](NOTICE.md). Upstream/third-party code and user assets keep their own
licenses. No proprietary console SDK or game material is bundled.
