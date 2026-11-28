// SPDX-License-Identifier: MPL-2.0
//! Loose RLD v3 initialization from the installed layouts of RetouchSystem::liteInit.
use anyhow::{Context, Result, ensure};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use crysirror_resource::{
    legacy_stream::Mt19937Split,
    pe::{MAX_PE_BYTES, PeData, PeRawResource, PeResourceName},
};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Sources {
    pub exe: String,
    pub ini: String,
    pub resident: String,
}

/// Neither the export immediate nor the per-installation bitmap word is
/// written into the launch plan, diagnostics or snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Resident {
    Midori2011,
    LateVol1,
    LateVol2,
}

impl Resident {
    pub(super) fn late(self) -> bool {
        matches!(self, Self::LateVol1 | Self::LateVol2)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Seeds {
    pub(super) resident: Resident,
    pub(super) checkpoint_layout: bool,
    normal: Option<u32>,
    definition: u32,
}

fn setting_fields(bytes: &[u8]) -> std::collections::BTreeMap<Vec<u8>, &[u8]> {
    let mut fields = std::collections::BTreeMap::new();
    let mut setting = false;
    for line in bytes.split(|b| *b == b'\n') {
        let line = line.trim_ascii();
        if line.starts_with(b"[") {
            setting = line.eq_ignore_ascii_case(b"[setting]");
            continue;
        }
        if !setting || line.starts_with(b";") {
            continue;
        }
        if let Some(eq) = line.iter().position(|b| *b == b'=') {
            let key = line[..eq].trim_ascii().to_ascii_lowercase();
            let v = line[eq + 1..].trim_ascii();
            fields.entry(key).or_insert(v);
        }
    }
    fields
}

fn default_marker_settings(bytes: &[u8]) -> bool {
    let fields = setting_fields(bytes);
    [b"hit".as_slice(), b"hitpos"].iter().all(|key| fields.get(*key).is_none_or(|value| value.is_empty()))
}

fn settings_sum(bytes: &[u8], resident: Resident) -> Result<u32> {
    let fields = setting_fields(bytes);
    let mut lanes = [0u8; 4];
    let mut index = 0;
    let flags = fields
        .get(b"flags".as_slice())
        .and_then(|v| std::str::from_utf8(v).ok())
        .unwrap_or("0")
        .parse::<u32>()?;
    let omit_title = resident.late()
        && fields
            .get(b"sysver".as_slice())
            .is_some_and(|v| crate::native_numbers::integer2011(v) != 0);
    let keys = [
        "class",
        "title",
        "w_view",
        "h_view",
        "n_reg",
        "n_strreg",
        "n_sysreg",
        "n_locreg",
        "n_usave",
        "n_asave",
        "n_qsave",
        "n_cg",
        "n_message",
        "n_scene",
        "n_sound",
        "flags",
        "guid",
        "svdata",
    ];
    for key in keys {
        if key == "flags" && flags & 4 == 0 {
            break;
        }
        if key == "title" && omit_title || key == "svdata" && !resident.late() {
            continue;
        }
        for &byte in fields.get(key.as_bytes()).copied().unwrap_or_default() {
            lanes[index] = lanes[index].wrapping_add(byte);
            index = (index + 1) % 4;
        }
    }
    Ok(u32::from_le_bytes(lanes))
}

fn bitmap_word(dib: &[u8]) -> Result<u32> {
    let header = dib
        .get(..40)
        .context("truncated ExHibit initialization bitmap")?;
    let field = |at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    let (width, height) = (field(4) as usize, field(8) as usize);
    let depth = u16::from_le_bytes([header[14], header[15]]);
    ensure!(
        field(0) == 40
            && (32..=4096).contains(&width)
            && (32..=4096).contains(&height)
            && header[12..14] == [1, 0]
            && matches!(depth, 24 | 32)
            && field(16) == 0
            && field(32) == 0,
        "unsupported ExHibit initialization DIB"
    );
    let pixel = usize::from(depth) / 8;
    let stride = (width * pixel).next_multiple_of(4);
    let rows = dib
        .get(40..40 + height * stride)
        .context("truncated ExHibit initialization pixels")?;
    let mut value = 0;
    for y in height - 32..height {
        value = (value << 1) | u32::from(rows[y * stride + 31 * pixel] & 1)
    }
    Ok(value)
}

// The neutral initialization resource and localized artwork can share an ID.
// In the 2011 package only the neutral entry initializes the ordinary scenarios.
fn initialization_word(bitmaps: &[PeRawResource]) -> Result<u32> {
    let matching = bitmaps.iter().filter(|r| r.name == PeResourceName::Id(152));
    let neutral = matching.clone().any(|r| r.language == 0);
    let mut candidates = matching.filter(|r| !neutral || r.language == 0);
    let first = candidates
        .next()
        .context("missing ExHibit initialization bitmap 152")?;
    let word = bitmap_word(&first.bytes)?;
    for variant in candidates {
        ensure!(
            bitmap_word(&variant.bytes)? == word,
            "ambiguous language variants of ExHibit initialization bitmap"
        );
    }
    Ok(word)
}

impl Seeds {
    pub fn midori_system_path(storage: &dyn ClientStorage, sources: &Sources) -> Result<VfsPath> {
        let ini = VfsPath::new(Mount::Asset, &sources.ini)?;
        let reader = storage.open_read(&ini)?;
        let data = crysirror_resource::read_all(reader.as_ref(), 65536)?;
        let fields = setting_fields(&data);
        let class = fields.get(b"class".as_slice()).copied().unwrap_or(b"ExHIBIT");
        let class = crysirror_resource::text_stream::decode_shift_jis(class, 512)?;
        ensure!(!class.is_empty() && !class.contains(['/', '\\']), "invalid ExHibit system class");
        Ok(VfsPath::new(Mount::Save, &format!("{class}.system.dat"))?)
    }

    pub fn read(storage: &dyn ClientStorage, s: &Sources) -> Result<Self> {
        let read = |path: &str, limit| -> Result<Vec<u8>> {
            let r = storage.open_read(&VfsPath::new(Mount::Asset, path)?)?;
            crysirror_resource::read_all(r.as_ref(), limit)
        };
        let resident = read(&s.resident, MAX_PE_BYTES as u64)?;
        let pe = PeData::parse(&resident)?;
        let export = pe.export_rva("?liteInit@RetouchSystem@@QAEXXZ")?;
        let code = pe.rva_slice(export, 0xaf)?;
        let (resident, immediate) = if code[0x84..0x8a] == [0xc7, 0x86, 0xc8, 0x11, 0, 0]
            && code[0x8e..0x98] == [0xc7, 0x86, 0x38, 0x80, 0, 0, 255, 255, 255, 255]
        {
            (Resident::Midori2011, 0x8a)
        } else {
            ensure!(
                code[0x9b..0xa1] == [0xc7, 0x83, 0x44, 0x1e, 0, 0]
                    && code[0xa5..0xaf] == [0xc7, 0x83, 0x48, 0x98, 0, 0, 255, 255, 255, 255],
                "unsupported ExHibit RLD v3 initialization ABI"
            );
            let layout = match export {
                0x1d2b00 => Resident::LateVol1,
                0x1d3080 => Resident::LateVol2,
                _ => anyhow::bail!("unverified ExHibit late resident layout"),
            };
            (layout, 0xa1)
        };
        let definition = u32::from_le_bytes(code[immediate..immediate + 4].try_into()?);
        let ini = read(&s.ini, 65536)?;
        let checkpoint_layout = default_marker_settings(&ini)
            && matches!(storage.metadata(Mount::Asset, "res/g/gn/hit.gyu"), Ok(Some(_)))
            && matches!(storage.metadata(Mount::Asset, "res/g/gn/default.cur"), Ok(None));
        let exe = read(&s.exe, MAX_PE_BYTES as u64)?;
        let image = PeData::parse(&exe)?;
        let bitmaps = image.raw_resources(&PeResourceName::Id(2), &|| Ok(()))?;
        let normal =
            if resident.late() && !bitmaps.iter().any(|r| r.name == PeResourceName::Id(152)) {
                let rcg = image.raw_resources(&PeResourceName::Name("RCG".into()), &|| Ok(()))?;
                if rcg.iter().any(|r| r.name == PeResourceName::Id(152)) {
                    None
                } else {
                    // FCDib's missing image converts to a 32x32 red surface.
                    // Every blue sample in column 31 is zero, and liteInit
                    // leaves the ordinary transform disabled.
                    Some(0)
                }
            } else {
                let value = initialization_word(&bitmaps)?;
                Some(if value == 0 {
                    0
                } else {
                    settings_sum(&ini, resident)? ^ value
                })
            };
        Ok(Self {
            resident,
            checkpoint_layout,
            normal,
            definition,
        })
    }
    #[cfg(test)]
    pub(super) fn unencoded_fixture() -> Self {
        Self {
            resident: Resident::Midori2011,
            checkpoint_layout: false,
            normal: Some(0),
            definition: 0,
        }
    }
    pub fn decode(self, name: &str, bytes: &mut [u8]) -> Result<()> {
        let payload = bytes.get_mut(16..).context("truncated RLD payload")?;
        let seed = if name.eq_ignore_ascii_case("def") || name.starts_with('$') {
            self.definition
        } else {
            self.normal.with_context(|| format!("ExHibit {:?} {name}.rld uses an unsupported RCG/152 initialization resource", self.resident))?
        };
        if seed != 0 {
            let mut generator = Mt19937Split::new(seed);
            let table: [u32; 256] = std::array::from_fn(|_| seed ^ generator.next_u32());
            for (index, word) in payload.chunks_exact_mut(4).take(0x3ff0).enumerate() {
                let v = u32::from_le_bytes((*word).try_into()?);
                word.copy_from_slice(&(v ^ table[index & 255]).to_le_bytes());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_initialization_bitmap_wins_over_localized_artwork() -> Result<()> {
        let variant = |language, marker| {
            let mut bytes = vec![0; 40 + 32 * 96];
            bytes[0] = 40;
            bytes[4] = 32;
            bytes[8] = 32;
            bytes[12] = 1;
            bytes[14] = 24;
            bytes[40 + 31 * 96 + 93] = marker;
            PeRawResource {
                name: PeResourceName::Id(152),
                language,
                bytes,
            }
        };
        assert_eq!(initialization_word(&[variant(1041, 0), variant(0, 1)])?, 1);
        assert!(initialization_word(&[variant(1041, 0), variant(1033, 1)]).is_err());
        Ok(())
    }
}
