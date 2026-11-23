# Desktop player and project guide

The default workspace target is `crysirror-desktop`, binary `crysirror`.
Build locally with the locked dependencies and release profile:

```sh
cargo build --release --locked
```

All native VM adapters, scripting and resource providers are linked by this
build. Default features include `audio`, `gamepad`, `smb` and `nfs`;
`acoustics-obvhs` is optional. `--no-default-features` disables these defaults;
it does not remove native adapters. The minimum Rust version is
declared in the root Cargo.toml. OS graphics/input/audio libraries remain native
build prerequisites. Windows calls the executable `crysirror.exe`.

## Play an installed game

```sh
./target/release/crysirror play /path/to/game
./target/release/crysirror play /path/to/game --detect-only
./target/release/crysirror play /path/to/game --headless --frames 1200 \
  --capture /tmp/game.png
```

The path may be a game directory or a file inside it. The player detects the
native profile, builds its project and installs capability-scoped resource
mounts internally. No hand-written bootstrap JSON or manual bulk extraction is
needed for supported layouts. Unknown formats, protection profiles and native
operations can still fail explicitly. See the [engine compatibility guide](NATIVE_ADAPTERS.md)
for supported formats, plug-ins and save-file compatibility.

`play` supports `--data-dir`, `--font`, `--no-audio`, `--input-track`, `--frames`,
`--capture`, `--headless`, `--detect-only` and `--allow-borderless`. Headless mode
creates no window/audio device but capture still requires the renderer.
Without a subcommand the executable attempts to play the game beside itself.

### Games on an SMB share

`play` accepts `smb://server/share/game-directory` directly, using a Rust SMB2/3
client. For example:

```sh
export CRYSIRROR_SMB_USER='your-user'
read -rs 'CRYSIRROR_SMB_PASSWORD?SMB password: '; echo
export CRYSIRROR_SMB_PASSWORD
./target/release/crysirror play 'smb://server/GameTest/game-Grisaia'
```

The password prompt above uses zsh. Credentials may also be supplied in URL
userinfo, with special characters percent-encoded. Diagnostic source names and
save-directory identities omit URL credentials. Without credentials, the client
attempts the `guest` account with an empty password.

Detection and archive access use the same network source. Files are read on
demand with random access, bounded I/O workers and a shared 32 MiB block cache;
the player does not download the whole game. Game assets are read-only, while
saves and cache remain local (`--data-dir` selects their location). A dropped
connection reports an error; automatic reconnection is not implemented.

The default desktop build enables SMB, including Siglus, BGI, SoftPAL and RFVP.
For a custom feature selection, enable the matching `smb` feature:

```sh
cargo build --release --locked -p crysirror-desktop --features smb
```

HTTP, WebDAV and SFTP URLs are not supported by `play`.

### Games on an NFS export

The default desktop build enables NFS, including Siglus, BGI, SoftPAL and RFVP.
For a custom feature selection, enable `nfs` when building:

```sh
cargo build --release --locked -p crysirror-desktop --features nfs
```

`play` also accepts `nfs://` URLs. The pure Rust client reads NFSv3, NFSv4.1 or
NFSv4.2 directly, without an operating-system mount or helper process:

```sh
./target/release/crysirror play 'nfs://server/data/GameTest/game-Grisaia'
./target/release/crysirror play 'nfs://server/data/GameTest/game-Grisaia?export=/data/GameTest&version=3'
./target/release/crysirror play 'nfs://server/GameTest/game-Grisaia?version=4.1'
```

The URL directory itself is mounted by default. If the server requires its exact
export path, supply `export=/data/GameTest`; the remaining URL path selects the
game directory or executable inside that export. NFSv4 paths use the server's
NFS namespace, which may differ from its local filesystem paths.

`version` defaults to `3`; select `4.1` or `4.2` explicitly. Authentication uses
the server's AUTH_SYS UID/GID mapping. Set `uid=1000&gid=1000` in the URL or use
`CRYSIRROR_NFS_UID` and `CRYSIRROR_NFS_GID`. Without either setting, the library
uses UID/GID 0; server root-squashing and file permissions still apply. Password
and URL-userinfo authentication are unsupported.

The client defaults to `noresvport=true`, so it does not need privileged source
ports. The server export must permit these connections (`insecure` in a Linux
export configuration). `noresvport=false` requests a reserved port and can fail
if the host account cannot bind one. `:port` selects the NFS TCP service port;
`mountport=` optionally selects the NFSv3 MOUNT service port.

Reads use four concurrent operations, a bounded queue and a shared 32 MiB block
cache. Files remain on the server; archives use random access. Game assets are
read-only, saves remain local, and symlinks/special files fail explicitly. An
operation timeout retires the connection; reopen the game to reconnect.
Integrated runtimes use their network I/O seam and local user overlay; they do
not require an OS-mounted share. See [network storage](NETWORK_STORAGE.md) for
provider ownership and platform validation boundaries.

Windowed games retain their OS title bar unless borderless behavior is allowed.
macOS menus use the screen menu bar; Windows uses native window menus; Linux
currently uses the engine menu fallback. An offscreen capture cannot verify OS
menu presentation or physical input.

## Run and inspect an engine project

```sh
./target/release/crysirror run examples/desktop/project.json --no-audio
./target/release/crysirror inspect examples/desktop/project.json
./target/release/crysirror trace examples/desktop/project.json --frames 120 --press-at 1
./target/release/crysirror render examples/desktop/project.json --output /tmp/scene.png
./target/release/crysirror gpu-info
```

| Command | Purpose |
| --- | --- |
| `run` | Windowed project; supports font/state overrides, scripts, capture and profiling. |
| `render` | Offscreen GPU PNG; supports bounded warm-up frames, step/input tracks, scripts and consecutive movie-frame captures. |
| `trace` | Engine/VM ticks and service diagnostics without a window/GPU/audio device. |
| `validate` | Project schema/data validation; not complete asset or game compatibility. |
| `inspect` | Project data and the executable's actually registered adapters. |
| `gpu-info` | Adapter features, enabled device capabilities and implemented rendering selections. |
| `script-types` | Generate project-bound script declarations from the class registry. |
| `shader-check` | Validate a scoped custom WGSL surface asset without a GPU. |
| `puppet-import` | Convert supplied E-mote data to engine puppet assets; native format limits apply. |

Use `crysirror <command> --help` for that build's option definitions. `render`
and `trace` accept repeatable zero-based `--press-at` indices and JSON
`--input-track` input. `run`, `render` and `trace` accept JS/TS `--script` sources
with `--script-client` selecting their project client. Offline `buildProject()`
recipes use the separate [authoring tool](AUTHORING.md).

`run` and `render` select `--renderer raster`, `compute-raytrace` or
`hardware-raytrace`. Hardware queries require supported device features/limits;
unsupported devices fail with a reason rather than silently switching paths.
Capabilities, encoded work, successful pixels and performance measurements are
separate evidence. See [architecture](ARCHITECTURE.md) and the
[frame/profiling guide](FRAME_PIPELINE.md).

F5/F9 request framework save/restore, F12 captures, and Escape exits the host.
In a KiriKiri session, pressing Escape sends the game's window-close request.
The game can display its own confirmation and keep running after cancellation.
Native adapters may reject unsupported snapshot states. These shortcuts do not
promise original-game save compatibility; see [save boundaries](SAVE_FORMAT.md).

## Project data and fonts

Schema-2 projects name an asset root and client instances. Each client's resource
paths remain scoped to its capability root. `crysirror-script-v1` data actions
are separate from original native script formats; native adapters run their own
instruction sets. `ClientSpec.systems` declares shared World/Layer/UI state.
See [engine systems](ENGINE_SYSTEMS.md) and [generated data/API declarations](api/README.md).

Projects can declare licensed fonts in `project.fonts`. Development `--font`
reads a caller-supplied face; it is not a portable asset declaration. G00 and
other native image formats use shared decoder registries; adapter-specific
cuts/pivots remain native policy. No game data or fonts are bundled.

`--data-dir` overrides state storage. Otherwise the host selects the platform's
user-data location under CrySirror; project fingerprints or installed-game
identity distinguish state. Change `content_revision` when asset semantics
change. Logical relative asset references survive moving the project together.
Budgets and supported data fields are defined by code/API validation, rather
than a second independently maintained limit table here.

## Packaging

```sh
python3 tools/desktop_project.py examples/desktop/project.json --assets
python3 tools/package_game.py --project examples/desktop/project.json \
  --executable target/release/crysirror --target linux --output dist/CrySirror-linux.zip
```

Use an executable built for the requested target. The packager checks its format
and explicit resources; it does not compile/sign the host, test target execution
or prove dependency/asset licensing completeness. `--data-only` replaces
`--executable`/`--target` when distributing project data without a runtime.
See [NOTICE.md](../NOTICE.md) for component notices and [icons](../icon/README.md)
for prepared platform icon assets.

## Local validation

`python3 tools/verify.py` runs Python tests plus release Cargo checks/tests.
`--gpu` explicitly includes the ignored GPU smoke test. Test commands, failures,
platform evidence and bounded game milestones belong in focused
[validation notes](validation).

## GitHub build

[The build workflow](../.github/workflows/build.yml) compiles and links the full
desktop player on Linux, Windows and macOS for pushes, pull requests and manual
runs. It uses `cargo build --release --locked -p crysirror-desktop --bin crysirror`
with default audio/gamepad features. It does not run tests, games or demos.
CI disables LTO and uses 16 codegen units to limit build memory; dependency
sources are cached, while `target/` and game data are not cached or uploaded.
Compilation does not establish device, rendering, audio or game compatibility.
