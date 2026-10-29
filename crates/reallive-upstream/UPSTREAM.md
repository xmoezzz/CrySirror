# Vendored RealLive extension DLLs

- Source: https://github.com/xmoezzz/siglus_rs @ 5f358ef1360b0439e92aed2b749e8f25d8721954,
  `crates/reallive/src/dll.rs`, `pt00/` and `dt00/`. License: MPL-2.0 (`LICENSE`).
- EF00 (Little Busters! scattered sprites), PT00 (Little Busters! baseball,
  reads the game's PT00.dll tables) and DT00 (Tomoyo After dungeon, reads the
  game's dt00.dll). Pure computation over the script's integer banks.
- `SOURCE.json` records every file's SHA-256; `pt00/` and `dt00/` are
  byte-identical to the pin.

## Port changes (complete list)

1. New `reallive-dll/Cargo.toml` and `lib.rs`. The unmodified upstream tests
   run against `test-shim/game_fs` (a test-only `read` = `std::fs::read`);
   the PT00/DT00 game-DLL tests return early unless `REALLIVE_TEST_GAME` /
   `REALLIVE_TEST_TOMOYO` name an original install.
2. `dll.rs`: `crate::system::System` is replaced by a two-method trait
   (`random`, `root_file`), implemented by `crysirror-reallive` over its RNG
   and capability storage; `game_fs` reads become `root_file` /
   `std::fs::write` (DT00 image dump, env-gated diagnostic only).
3. `dll.rs`: an unknown DLL name fails visibly (upstream returned 0).
4. `dll.rs`: `Banks` (DllMemory over the upstream Machine) moved to the
   adapter, which owns the VM memory.
