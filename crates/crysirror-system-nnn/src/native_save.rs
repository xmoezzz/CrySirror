// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Reviewed Mugen2 legacy save records. This is the original native binary
//! envelope, not the framework snapshot format. Opaque status/effect sections
//! retain their bytes; full VM/resource restoration requires separate mapping.
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{ClientStorage, StorageWrite},
};
use std::sync::Arc;
const HEADER: usize = 32;
const LIMIT: usize = 64 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveBlock {
    pub code: u32,
    pub label: [u8; 16],
    pub payload: Vec<u8>,
}
impl SaveBlock {
    pub fn new(code: u32, label: &[u8], payload: Vec<u8>) -> Result<Self> {
        ensure!(
            label.len() <= 15 && !label.contains(&0),
            "NNN native save label"
        );
        let mut out = [0; 16];
        out[..label.len()].copy_from_slice(label);
        Ok(Self {
            code,
            label: out,
            payload,
        })
    }
    fn word(&self, index: usize) -> Result<u32> {
        let at = index.checked_mul(4).context("NNN save word overflow")?;
        Ok(u32::from_le_bytes(
            self.payload
                .get(at..at + 4)
                .context("NNN save word bounds")?
                .try_into()?,
        ))
    }
}
#[derive(Clone, Debug)]
pub struct LegacySaveProfile {
    pub header_name: Vec<u8>,
    pub version: u32,
    pub extension_sizes: Vec<u32>,
    pub omake: bool,
    pub cutin: bool,
}
#[derive(Clone, Debug)]
pub struct NativeSaveFile {
    pub blocks: Vec<SaveBlock>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSavePosition {
    pub run_mode: i32,
    pub script_number: i32,
    pub script_sub_number: i32,
    pub script_sub_sub_number: i32,
    pub last_message_id: i32,
    pub message_rows: u32,
    pub message_mode: u32,
    pub last_select_id: i32,
    pub choice_rows: u32,
    pub extra_rows: u32,
    pub music_number: i32,
    pub music_repeat_count: i32,
}
impl NativeSaveFile {
    pub fn parse(bytes: &[u8], profile: &LegacySaveProfile) -> Result<Self> {
        ensure!(
            (96..=LIMIT).contains(&bytes.len()),
            "NNN native save byte budget"
        );
        let mut blocks = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            ensure!(blocks.len() < 1024, "NNN native save block budget");
            let head = bytes
                .get(at..at + HEADER)
                .context("NNN truncated native save block header")?;
            let size = u32::from_le_bytes(head[..4].try_into()?) as usize;
            ensure!(
                size >= HEADER && size <= LIMIT,
                "NNN native save block length"
            );
            let end = at
                .checked_add(size)
                .context("NNN native save block overflow")?;
            let raw = bytes
                .get(at..end)
                .context("NNN truncated native save block")?;
            ensure!(
                head[8..16] == [0; 8],
                "NNN unknown native save header flags"
            );
            let code = u32::from_le_bytes(head[4..8].try_into()?);
            let label = head[16..32].try_into()?;
            blocks.push(SaveBlock {
                code,
                label,
                payload: raw[HEADER..].to_vec(),
            });
            at = end;
        }
        let out = Self { blocks };
        out.validate(profile)?;
        Ok(out)
    }
    pub fn validate(&self, profile: &LegacySaveProfile) -> Result<()> {
        ensure!(
            profile.header_name.len() <= 128 && profile.extension_sizes.len() <= 256,
            "NNN save profile budget"
        );
        let n = 8
            + usize::from(profile.omake)
            + usize::from(profile.cutin)
            + profile.extension_sizes.len();
        ensure!(self.blocks.len() == n, "NNN native save block count");
        let header = &self.blocks[0];
        ensure!(
            header.code == 0 && header.payload.is_empty(),
            "NNN native save HEADER block"
        );
        let mut expected = [0; 16];
        let name = &profile.header_name[..profile.header_name.len().min(15)];
        expected[..name.len()].copy_from_slice(name);
        ensure!(
            header.label == expected,
            "NNN original save belongs to another game"
        );
        let info = &self.blocks[1];
        ensure!(
            info.code == 1 && info.payload.len() == 32,
            "NNN legacy save INFO size"
        );
        ensure!(
            info.word(0)? as usize == n
                && info.word(1)? == profile.version
                && info.word(2)? as usize == profile.extension_sizes.len()
                && info.word(3)? == u32::from(profile.omake)
                && info.word(4)? == u32::from(profile.cutin),
            "NNN native save version/extension profile"
        );
        // These sizes come from complete legacy Save* static functions. Modern
        // GAMEMESSAGE is larger and must not be substituted for this ABI.
        for (at, code, size) in [
            (2, 2, 7088),
            (4, 4, 4128),
            (5, 5, 16416),
            (6, 6, 4128),
            (7, 7, 16416),
        ] {
            ensure!(
                self.blocks[at].code == code && self.blocks[at].payload.len() + HEADER == size,
                "NNN legacy save section {code} size/order"
            );
        }
        let mini = &self.blocks[3];
        ensure!(
            mini.code == 3 && mini.payload.len() >= 16,
            "NNN save MINICG header"
        );
        let (w, h) = (mini.word(0)?, mini.word(1)?);
        ensure!(
            w > 0 && h > 0 && w <= 8192 && h <= 8192,
            "NNN native save thumbnail dimensions"
        );
        ensure!(
            u64::from(w) * u64::from(h) * 4 + 16 == mini.payload.len() as u64,
            "NNN native save thumbnail length"
        );
        let mut at = 8;
        if profile.omake {
            ensure!(
                self.blocks[at].code == 8 && self.blocks[at].payload.len() + HEADER == 12832,
                "NNN legacy OMAKE block"
            );
            at += 1;
        }
        if profile.cutin {
            ensure!(
                self.blocks[at].code == 9 && self.blocks[at].payload.len() + HEADER == 1056,
                "NNN legacy CUTIN block"
            );
            at += 1;
        }
        for (i, size) in profile.extension_sizes.iter().enumerate() {
            let b = &self.blocks[at + i];
            ensure!(
                b.code == 100 + i as u32 && b.payload.len() == *size as usize,
                "NNN native save EXT block"
            );
        }
        let p = self.position()?;
        ensure!(
            p.message_rows <= 64
                && p.choice_rows
                    .checked_add(p.extra_rows)
                    .is_some_and(|n| n <= 64),
            "NNN saved message count"
        );
        Ok(())
    }
    /// Native variable callback writes exactly1000 values, although the record
    /// reserves1024 slots. The remaining96 padding bytes are preserved.
    pub fn variables(&self) -> Result<Vec<i32>> {
        let b = self.blocks.get(4).context("NNN variable block absent")?;
        ensure!(
            b.code == 4 && b.payload.len() == 4096,
            "NNN legacy variable record"
        );
        (0..1000).map(|i| Ok(b.word(i)? as i32)).collect()
    }
    pub fn replace_variables(&mut self, values: &[i32]) -> Result<()> {
        ensure!(
            values.len() == 1000,
            "NNN legacy save needs1000 native variables"
        );
        let b = self
            .blocks
            .get_mut(4)
            .context("NNN variable block absent")?;
        ensure!(
            b.code == 4 && b.payload.len() == 4096,
            "NNN legacy variable record"
        );
        for (i, v) in values.iter().enumerate() {
            b.payload[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        Ok(())
    }
    pub fn position(&self) -> Result<NativeSavePosition> {
        let b = self.blocks.get(2).context("NNN status block absent")?;
        ensure!(
            b.code == 2 && b.payload.len() == 7056,
            "NNN legacy status record"
        );
        let word = |native: usize| b.word(native - 8);
        Ok(NativeSavePosition {
            run_mode: word(24)? as i32,
            script_number: word(26)? as i32,
            script_sub_number: word(27)? as i32,
            script_sub_sub_number: word(31)? as i32,
            last_message_id: word(28)? as i32,
            message_rows: word(29)?,
            message_mode: word(30)?,
            last_select_id: word(32)? as i32,
            choice_rows: word(33)?,
            extra_rows: word(34)?,
            music_number: word(36)? as i32,
            music_repeat_count: word(37)? as i32,
        })
    }
    pub fn messages(&self, count: usize) -> Result<Vec<Vec<u8>>> {
        ensure!(count <= 64, "NNN saved message row count");
        let b = self.blocks.get(7).context("NNN message block absent")?;
        ensure!(
            b.code == 7 && b.payload.len() == 16384,
            "NNN legacy message record"
        );
        b.payload
            .chunks_exact(256)
            .take(count)
            .map(|row| {
                let len = row
                    .iter()
                    .position(|v| *v == 0)
                    .context("NNN unterminated saved message")?;
                ensure!(
                    len <= 254 && row[len + 1] == 0,
                    "NNN saved message native double-NUL"
                );
                Ok(row[..len].to_vec())
            })
            .collect()
    }
    /// Serialize only after exact original profile validation. This preserves
    /// untouched fields; it cannot manufacture missing VM/effect/save state.
    pub fn encode(&self, profile: &LegacySaveProfile) -> Result<Vec<u8>> {
        self.validate(profile)?;
        let mut out = Vec::new();
        for b in &self.blocks {
            let size = b
                .payload
                .len()
                .checked_add(HEADER)
                .context("NNN save block size")?;
            ensure!(out.len() + size <= LIMIT, "NNN native save byte budget");
            out.extend(u32::try_from(size)?.to_le_bytes());
            out.extend(b.code.to_le_bytes());
            out.extend([0; 8]);
            out.extend(b.label);
            out.extend(&b.payload);
        }
        Ok(out)
    }
    /// Admission on capability storage; the returned completion must become
    /// Ready before native save UI/VM policy can acknowledge the write.
    pub fn begin_write(
        &self,
        profile: &LegacySaveProfile,
        storage: &dyn ClientStorage,
        slot: u32,
        trial: bool,
    ) -> Result<Box<dyn StorageWrite>> {
        ensure!(slot < 10000, "NNN original save slot budget");
        let name = format!("{}data{slot}.sav", if trial { "t" } else { "" });
        Ok(storage.begin_write_atomic(
            &VfsPath::new(Mount::Save, &name)?,
            Arc::from(self.encode(profile)?),
        )?)
    }
}
