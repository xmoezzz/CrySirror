# ADR 0037: local BGI trial and a device-free CPU embedding experiment

Status: local experiment verified; CrySirror adapter not implemented.

## Requirement

Try the installed BGI game while auditing whether native VN functionality has
actually reached shared engine services. Preserve the original interpreter and
avoid editing the parallel renderer work or the owner's ethornell-rs changes.

## Decision and implementation

Run the installed `千の刃濤、桃花染の皇姫 体験版` (BGI 1.622) on an independent
APFS clone outside the repository. Its startup marker causes the original script
to exit; move that marker out of the clone only. Preserve the original game,
original source, and original VM instructions. Record that normalization.

The source is local ethornell-rs HEAD
`4c41dc3a566338d2ea8400635e23b062613e3782` **plus existing owner changes**.
This is not a clean pinned release. Capture the dirty patch, hashes and source
manifest outside the deliverable; do not vendor it as if it were a clean pin.

The ordinary CLI can exercise the game, but its headless path still attempts
audio device creation and does not install HostRuntime's file policy. Therefore
also copy source into cache and add an experimental HostRuntime CPU-frame hook.
This hook preserves native stepping, strict failures, budgets, injected input,
clock and CPU composition; silent mode creates no renderer or AudioSystem.
Resources remain read-only through HostRuntime policy and user data goes to a
separate private root. Pending host dialogs are errors requiring a host answer.
No answers, unsupported native operations or saves are fabricated.

The experiment exports owned RGBA bytes. It does not register a BGI VmFactory,
extract the native text renderer into crysirror-text, or connect native PCM to
crysirror-audio. The cached hook is an experimental seam, not a shipped API.

## Rejected alternatives

- Editing the original game/source, or committing game images/fonts.
- Treating standalone ethornell success as CrySirror integration.
- Replacing BGI instructions with the data VM or replaying pre-rendered images.
- Sharing upstream GPU pointers (wgpu 0.19) with CrySirror (wgpu 0.20).
- Silently confirming host dialogs or reporting muted output as audio fidelity.

## Compilation evidence

- Original current source: `cargo build --locked --release -p ethornell-cli`
  exited 0; warnings remain. Original dirty patch hash was unchanged afterward.
- Cached CPU experiment: release example built with `--locked`,
  `--no-default-features --features host-driven`; no VM opcode changes.
- Separate CrySirror regression: 42 Siglus/runtime tests, 126 Python tests and
  workspace all-target check passed (native-audit-results.json).

## Behavior evidence

- Rebuilt ordinary CLI: actual title and Japanese story captured at 1280×720.
  Strict `--fail-on-stub` run reached frame 3599, exited 0, and input advanced
  to another sentence. Background movement and native textbox/icons are visible.
- Cached strict HostRuntime CPU experiment: title, story and later input advance
  captured; 3600 native ticks, engine time 57,600 ms, renderer=false,
  audio=false, pending requests=0. It creates no window/GPU/audio device.
- Captured title, first story and later clicked sentence PNGs are byte-identical
  between the ordinary CLI baseline and CPU host experiment (SHA256 recorded).
  This compares three observed checkpoints, not every frame or save state.
- Actual local frames, game clone, logs, source patch and experiment stay under
  `~/.cache/crysirror/bgi-trial` and `~/.cache/crysirror/bgi-cpu-proof`.
  Machine-readable summary: ../bgi-local-trial-results.json.

This is bounded scene execution, not complete-game compatibility. Save/load,
movies, audible output, native GPU effect equivalence and names/ruby are not
validated. The embedding bootstrap explicitly reports silent/video limitations.

## Formal adapter acceptance still required

Provide dormant factory construction, client capability storage, budgeted
stepping, per-client input/time, and scoped present_surface publication. Reject
unsupported runtime snapshots explicitly until original save semantics are
covered. Audit the complete pinned source, separate device dependencies from
core/compat, and compare traces/pixels/saves against upstream before switching
from an experiment to a supported adapter. Migrate shared text/audio only with
native layout/wait/resource/save behavior preserved by differential tests.

Even the `host-driven` build still transitively compiles GPU/audio/Ruffle
dependencies, and the native VM contains unsafe code. A silent successful run
does not remove those dependency/ownership constraints. Do not put that graph
directly into device-free, safe core/compat crates. VmAdapter itself has no
Send requirement; the remaining issues include dormant construction and
bridging ClientStorage providers rather than bypassing them with OS paths.
