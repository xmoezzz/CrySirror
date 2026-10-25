// SPDX-License-Identifier: MPL-2.0
//! All resource access must be supplied through the scoped adapter boundary.
use anyhow::{Result, bail};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
pub fn read_file_bytes(p: &Path) -> Result<Vec<u8>> {
    Ok(crate::io_fs::read(p)?)
}
pub fn open_game_file(p: &Path) -> Result<GameFile> {
    Ok(GameFile(Cursor::new(read_file_bytes(p)?)))
}
pub fn invalidate_game_path_cache(_: &Path) {}
pub fn find_scene_pck_path_for_append(_: &Path, _: &str) -> Result<PathBuf> {
    bail!("Siglus append pack must be loaded through client capability")
}
pub fn load_scene_pck_decode_options(_: &Path) -> Result<crate::scene_pck::ScenePckDecodeOptions> {
    bail!("Siglus decode options must be supplied explicitly; no bundled game keys")
}

pub struct GameFile(Cursor<Vec<u8>>);
impl Read for GameFile {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(b)
    }
}
impl Seek for GameFile {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(p)
    }
}
pub struct Metadata(u64);
impl Metadata {
    pub fn len(&self) -> u64 {
        self.0
    }
}
impl GameFile {
    pub fn metadata(&self) -> std::io::Result<Metadata> {
        Ok(Metadata(self.0.get_ref().len() as u64))
    }
}
pub fn game_file_exists(p: &Path) -> bool {
    crate::io_fs::read(p).is_ok()
}
pub fn game_file_len(p: &Path) -> Result<u64> {
    Ok(read_file_bytes(p)?.len() as u64)
}
pub fn resolve_game_file(p: &Path) -> Result<Option<PathBuf>> {
    Ok(crate::io_fs::read(p).ok().map(|_| p.to_path_buf()))
}
