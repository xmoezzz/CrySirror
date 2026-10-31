// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Independently written bounded System-NNN SPT data reader. Original numeric
//! identities, word PCs and raw text are preserved; no original engine code,
//! renderer or command callbacks are hosted. See SOURCE.json for research.
#![forbid(unsafe_code)]
#[cfg(feature = "runtime")]
pub mod installed;
pub mod machine;
pub mod bridge;
pub mod adapter;
pub mod metadata;
pub mod expression;
mod control;
pub mod config;
pub mod native_bootstrap;
pub mod native_control;
pub mod native_resources;
pub mod native_effects;
pub mod native_audio;
pub mod audio_bridge;
pub mod native_text;
pub mod native_ui;
pub mod native_save;
pub mod native_presentation;
pub mod native_overlap;
pub mod native_services_bootstrap;
pub mod resource_bridge;
pub mod image_bridge;
pub mod native_draw;
pub mod native_skip;
pub mod native_message_control;
pub mod native_shell;
pub mod native_geometry;
use anyhow::{ensure, Context, Result};
const MAX_SCRIPT_BYTES: usize = 16 * 1024 * 1024;
pub const IDENTIFY_DATA: u32 = 0x66660001;
/// Caller-selected original 32-bit rolling transform profile.
#[derive(Clone, Copy, Debug)]
pub struct XorProfile {
    pub xor: u32,
    pub xor_increment: u32,
    pub add_increment: u32,
}
impl Default for XorProfile {
    fn default() -> Self {
        Self {
            xor: u32::MAX,
            xor_increment: 0,
            add_increment: 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Program {
    words: Vec<u32>,
    bytes: Vec<u8>,
    pub header_words: u32,
    pub story_count: u32,
    pub integer_pool: u32,
    message_tables: [(u32, u32); 2],
    pub call_tables: Option<[u32; 4]>,
}
/// Immutable original instruction record. Arguments remain words, not guessed
/// evaluated parameters; expression/native execution is a separate stage.
#[derive(Clone, Copy, Debug)]
pub struct Instruction<'a> {
    pub pc: u32,
    pub next: u32,
    pub identify: u32,
    pub code: u32,
    pub operands: &'a [u32],
}
impl Program {
    pub fn parse(encoded: &[u8], profile: XorProfile) -> Result<Self> {
        ensure!(
            encoded.len() >= 80 && encoded.len() <= MAX_SCRIPT_BYTES && encoded.len() % 4 == 0,
            "SPT size/alignment"
        );
        let mut bytes = encoded.to_vec();
        crysirror_resource::legacy_stream::rolling_word_xor_add(
            &mut bytes,
            profile.xor,
            profile.xor_increment,
            profile.add_increment,
        )?;
        let words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let header_words = words[0];
        ensure!(
            words[1] == IDENTIFY_DATA
                && words[2] == 0x55550001
                && (20..=64).contains(&header_words)
                && header_words as usize <= words.len(),
            "unsupported SPT header/profile"
        );
        let message_tables = [(words[4], words[5]), (words[6], words[7])];
        for (count, start) in message_tables {
            ensure!(
                (start as usize)
                    .checked_add(count as usize)
                    .is_some_and(|e| e <= words.len()),
                "SPT message table bounds"
            );
        }
        let integer_pool = words[18];
        ensure!(
            integer_pool as usize <= words.len(),
            "SPT integer pool bounds"
        );
        let call_tables = if header_words >= 32 {
            Some([words[20], words[21], words[22], words[23]])
        } else {
            None
        };
        if let Some(tables) = call_tables {
            for table in tables {
                ensure!((table as usize) <= words.len(), "SPT call table bounds");
            }
        }
        let story_count = words[10];
        let p = Self {
            words,
            bytes,
            header_words,
            story_count,
            integer_pool,
            message_tables,
            call_tables,
        };
        // Validate every referenced text address and terminator before admission.
        for table in 0..2 {
            for id in 0..p.message_tables[table].0 {
                p.text(table, id)?;
            }
        }
        Ok(p)
    }
    pub fn word_count(&self) -> usize {
        self.words.len()
    }
    pub fn message_count(&self, table: usize) -> Result<u32> {
        Ok(self
            .message_tables
            .get(table)
            .context("SPT message table index")?
            .0)
    }
    pub fn text(&self, table: usize, id: u32) -> Result<&[u8]> {
        let &(count, start) = self
            .message_tables
            .get(table)
            .context("SPT message table index")?;
        ensure!(id < count, "SPT message id");
        let word_address = self.words[(start + id) as usize] as usize;
        let byte_address = word_address
            .checked_mul(4)
            .context("SPT text address overflow")?;
        let tail = self
            .bytes
            .get(byte_address..)
            .context("SPT text address bounds")?;
        let len = tail
            .iter()
            .take(1024 * 1024 + 1)
            .position(|b| *b == 0)
            .context("SPT unterminated/oversized text")?;
        Ok(&tail[..len])
    }
    pub fn instruction(&self, pc: u32) -> Result<Instruction<'_>> {
        let at = pc as usize;
        let header = self
            .words
            .get(at..at.checked_add(3).context("SPT pc overflow")?)
            .context("SPT instruction header bounds")?;
        let n = header[0] as usize;
        ensure!(n >= 3, "SPT instruction length");
        let end = at.checked_add(n).context("SPT instruction size overflow")?;
        let body = self
            .words
            .get(at..end)
            .context("SPT instruction body bounds")?;
        Ok(Instruction {
            pc,
            next: u32::try_from(end)?,
            identify: header[1],
            code: header[2],
            operands: &body[3..],
        })
    }
}

pub mod native_chara_trans;

pub mod native_old_lamp;

pub mod native_curve;

pub mod native_bura;
