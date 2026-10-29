// SPDX-License-Identifier: MPL-2.0
//! Test-only: the subset of siglus_rs `game_fs` the vendored DLL tests call.
pub fn read(path: impl AsRef<std::path::Path>) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}
