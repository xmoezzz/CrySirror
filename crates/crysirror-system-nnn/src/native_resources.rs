// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Original resource naming policy. Readers/codecs/mounts remain host-owned.
use anyhow::{ensure, Result};

#[derive(Clone, Debug)]
pub struct ResourceNames {
    root: String,
}
/// Host resolves these against its shared archive registry. Entries are searched
/// in order; a missing container/entry is distinct from corrupt archive data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedMember {
    pub archive: String,
    pub table: String,
    pub member: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageLookup {
    pub packed: Vec<PackedMember>,
    pub loose: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicLookup {
    pub control: String,
    pub audio: String,
}
impl ResourceNames {
    fn decoded_name(raw: &[u8]) -> Result<String> {
        ensure!(
            !raw.is_empty() && raw.len() <= 254,
            "System-NNN resource name length"
        );
        let (text, _, errors) = encoding_rs::SHIFT_JIS.decode(raw);
        ensure!(!errors, "System-NNN resource Shift_JIS encoding");
        let text = text.replace('\\', "/");
        ensure!(
            !text.starts_with('/')
                && !text.contains([':', '\0'])
                && !text.split('/').any(|p| matches!(p, "." | ".." | "")),
            "System-NNN resource logical name"
        );
        Ok(text)
    }
    /// Native LOADDWQ supplies a basename and prefixes its first two original
    /// bytes as a directory. This is separate from CPicture's general names.
    pub fn script_image(&self, basename: &[u8], pack_levels: u32) -> Result<ImageLookup> {
        ensure!(basename.len() >= 2, "System-NNN LOADDWQ basename too short");
        let name = Self::decoded_name(basename)?;
        ensure!(!name.contains('/'), "System-NNN LOADDWQ expects basename");
        let prefix = Self::decoded_name(&basename[..2])?;
        self.picture_decoded(format!("{prefix}/{name}"), pack_levels)
    }
    /// General original CPicture names may include sys/sc or sys/sm. Pack tag
    /// prefix selection and descending patch-level precedence come from the
    /// reviewed legacy OpenDWQFile 0x4d4260; decoding belongs to shared archives.
    pub fn picture(&self, logical: &[u8], pack_levels: u32) -> Result<ImageLookup> {
        self.picture_decoded(Self::decoded_name(logical)?, pack_levels)
    }
    fn picture_decoded(&self, logical: String, pack_levels: u32) -> Result<ImageLookup> {
        ensure!(pack_levels <= 1024, "System-NNN picture pack-level budget");
        let lower = logical.to_ascii_lowercase();
        let tags = [
            ("ta", "ta"),
            ("bg", "bg"),
            ("ev", "ev"),
            ("sys/sc", "sc"),
            ("sys/sm", "sm"),
            ("sys", "sys"),
        ];
        let mut packed = Vec::new();
        if let Some((_, tag)) = tags.iter().find(|(prefix, _)| lower.starts_with(prefix)) {
            let member = format!("{}.dwq", lower.rsplit('/').next().unwrap());
            for level in (0..pack_levels).rev() {
                packed.push(PackedMember {
                    archive: self.path(&format!("dwq/{tag}{level}.gpk"))?,
                    table: self.path(&format!("dwq/{tag}{level}.gtb"))?,
                    member: member.clone(),
                });
            }
        }
        Ok(ImageLookup {
            packed,
            loose: self.path(&format!("dwq/{logical}.dwq"))?,
        })
    }
    pub fn packed_voice(&self, basename: &[u8]) -> Result<PackedMember> {
        ensure!(basename.len() >= 2, "System-NNN packed voice basename");
        let name = Self::decoded_name(basename)?;
        ensure!(
            !name.contains('/'),
            "System-NNN packed voice expects basename"
        );
        let prefix = Self::decoded_name(&basename[..2])?;
        Ok(PackedMember {
            archive: self.path(&format!("cdvaw/{prefix}.vpk"))?,
            table: self.path(&format!("cdvaw/{prefix}.vtb"))?,
            member: format!("{name}.vaw"),
        })
    }
    pub fn loose_voice(&self, basename: &[u8]) -> Result<String> {
        ensure!(basename.len() >= 2, "System-NNN voice basename");
        let name = Self::decoded_name(basename)?;
        ensure!(!name.contains('/'), "System-NNN voice expects basename");
        let prefix = Self::decoded_name(&basename[..2])?;
        self.path(&format!("cdvaw/{prefix}/{name}.vaw"))
    }
    pub fn system_wave(&self, directory: Option<&[u8]>, basename: &[u8]) -> Result<String> {
        let name = Self::decoded_name(basename)?;
        ensure!(
            !name.contains('/'),
            "System-NNN system wave expects basename"
        );
        let relative = if let Some(dir) = directory.filter(|d| !d.is_empty()) {
            format!("vaw/{}/{name}.vaw", Self::decoded_name(dir)?)
        } else {
            format!("vaw/{name}.vaw")
        };
        self.path(&relative)
    }
    pub fn music(&self, basename: &[u8]) -> Result<String> {
        let name = Self::decoded_name(basename)?;
        self.path(&format!("wgq/{name}.wgq"))
    }
    pub fn music_lookup(&self, basename: &[u8]) -> Result<MusicLookup> {
        let name = Self::decoded_name(basename)?;
        Ok(MusicLookup {
            control: self.path(&format!("wgq/{name}.mml"))?,
            audio: self.path(&format!("wgq/{name}.wgq"))?,
        })
    }
    pub fn new(root: &str) -> Result<Self> {
        let root = root.replace('\\', "/");
        ensure!(
            !root.starts_with('/')
                && !root.contains(':')
                && !root.split('/').any(|p| matches!(p, ".." | ".")),
            "System-NNN resource capability root"
        );
        Ok(Self {
            root: root.trim_end_matches('/').to_owned(),
        })
    }
    pub fn path(&self, relative: &str) -> Result<String> {
        let relative = relative.replace('\\', "/");
        ensure!(
            !relative.is_empty()
                && !relative.starts_with('/')
                && !relative.contains(':')
                && !relative.contains('\0')
                && !relative.split('/').any(|p| matches!(p, ".." | "." | "")),
            "System-NNN relative resource path"
        );
        Ok(if self.root.is_empty() {
            relative
        } else {
            format!("{}/{relative}", self.root)
        })
    }
    pub fn script(&self, name: &[u8]) -> Result<String> {
        ensure!(
            !name.is_empty() && name.len() <= 128,
            "System-NNN script basename length"
        );
        let (decoded, _, error) = encoding_rs::SHIFT_JIS.decode(name);
        ensure!(
            !error
                && !decoded.contains(['/', '\\', ':', '\0'])
                && !matches!(decoded.as_ref(), "." | ".."),
            "System-NNN script basename/encoding"
        );
        self.path(&format!("spt/{decoded}.spt"))
    }
    pub fn encoded_path(&self, path: &[u8]) -> Result<String> {
        let (decoded, _, error) = encoding_rs::SHIFT_JIS.decode(path);
        ensure!(!error, "System-NNN resource Shift_JIS encoding");
        self.path(&decoded)
    }
}
