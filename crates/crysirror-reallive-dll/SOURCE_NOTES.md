# First-class RealLive native DLL semantics

`crysirror-reallive-dll` is a maintained CrySirror workspace crate, adopted from
https://github.com/xmoezzz/siglus_rs at
`5f358ef1360b0439e92aed2b749e8f25d8721954`, `crates/reallive/src/dll.rs`,
`pt00/` and `dt00/`. The licence is MPL-2.0; `LICENSE` is preserved.
`PRE_ADOPTION_SOURCE.json` preserves the earlier adoption inventory;
`SOURCE.json` records the current seams, original and maintained-file hashes.

## Ownership

- This crate retains EF00/PT00/DT00 native argument conventions, script-bank
  numbers, RNG/state, original actor/dungeon branches and native table layouts.
  PT00 baseball and DT00 dungeon code are original game-specific computations
  over numeric memory, not another visual-novel engine or renderer.
- EF00 B-spline sampling and perspective arithmetic, PT00 planar geometry,
  parabolic sampling and f64 matrix operations now call reusable
  `crysirror-vfx::spatial` kernels. Native constants, one-element-late EF00
  control-point addressing, integer truncation and singularity policy stay here.
- The former private PE parser is removed. `crysirror-resource::pe::PeData`
  validates caller-owned PE bytes and lends selected data sections. Preferred
  addresses are numeric layout metadata. `.text` switch tables are data only;
  no PE code is mapped, relocated, loaded by the OS or executed.
- The two-method `System` trait requests the native RNG and capability-relative
  game files. `DllMemory` borrows native script banks. There is no window, GPU,
  renderer, bitmap compositor, text renderer, mixer or audio device dependency.
- DT00 `image()` / `image_hash()` refer to its native table/data memory, not a
  picture or composed game screen. Production environment-selected trace and
  memory-dump filesystem writes have been removed. Only opt-in tests read
  original owner-supplied files through `std::fs`.

## Validation boundaries

Unknown DLL names fail on invocation. The native translated switch defaults
and integer semantics remain original policy. This source adoption does not
establish full minigame gameplay, native-save coverage or complete asynchronous
PE-table preparation. The native file/table ABI still returns synchronously;
shared PE parsing never acquires an executable-code capability.

Default tests report owner-file cases as ignored. To run those cases, supply
`REALLIVE_TEST_GAME` (directory containing PT00.dll) and
`REALLIVE_TEST_TOMOYO` (directory containing dt00.dll), and use
`cargo test --release -p crysirror-reallive-dll -- --ignored`.
They call the Rust translation against original static tables; they never run
original DLL code. Independent shared spatial tests need no game files or VM.
