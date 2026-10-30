//! Read-only PE32/PE32+ data views over caller-owned bytes. Parsing is backed
//! by pelite0.10 safe file APIs; nothing is mapped, relocated or executed.
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};

/// Maximum admitted executable-as-data file (no process/device authority).
pub const MAX_PE_BYTES: usize = 64 * 1024 * 1024;
/// One file-backed PE section. Addresses are numeric file metadata only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeSection {
    /// Original eight-byte section name, including padding.
    pub name: [u8; 8],
    /// Relative virtual address from the preferred image base.
    pub rva: u32,
    /// Virtual extent declared by the file, including unbacked padding.
    pub virtual_size: u32,
    /// First byte in the original file.
    pub file_offset: u32,
    /// Bytes actually backed by the original file.
    pub file_size: u32,
    /// Original IMAGE_SCN flags; no protection changes are performed.
    pub characteristics: u32,
}
/// Validated immutable file view, freely shareable across resource workers.
pub struct PeData<'a> {
    bytes: &'a [u8],
    sections: Vec<PeSection>,
    image_base: u64,
    header_size: u32,
    sha256: [u8; 32],
}
impl<'a> PeData<'a> {
    /// Validate PE headers and every section's backed range before admission.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_PE_BYTES, "PE data file budget");
        let image = pelite::PeFile::from_bytes(bytes)
            .map_err(|e| anyhow::anyhow!("PE data header: {e}"))?;
        let (image_base, header_size) = match image.optional_header() {
            pelite::Wrap::T32(header) => (u64::from(header.ImageBase), header.SizeOfHeaders),
            pelite::Wrap::T64(header) => (header.ImageBase, header.SizeOfHeaders),
        };
        ensure!(
            header_size as usize <= bytes.len(),
            "PE headers exceed file"
        );
        let mut sections = Vec::new();
        for section in image.section_headers() {
            let end = section
                .PointerToRawData
                .checked_add(section.SizeOfRawData)
                .context("PE section file extent overflow")?;
            ensure!(end as usize <= bytes.len(), "PE section exceeds file");
            section
                .VirtualAddress
                .checked_add(section.VirtualSize.max(section.SizeOfRawData))
                .context("PE section RVA extent overflow")?;
            sections.push(PeSection {
                name: section.Name,
                rva: section.VirtualAddress,
                virtual_size: section.VirtualSize,
                file_offset: section.PointerToRawData,
                file_size: section.SizeOfRawData,
                characteristics: section.Characteristics,
            });
        }
        let sha256 = Sha256::digest(bytes).into();
        Ok(Self {
            bytes,
            sections,
            image_base,
            header_size,
            sha256,
        })
    }
    /// Preferred image base from the optional header, never a process pointer.
    pub fn image_base(&self) -> u64 {
        self.image_base
    }
    /// Validated section metadata in original order.
    pub fn sections(&self) -> &[PeSection] {
        &self.sections
    }
    /// SHA-256 of the immutable original bytes for exact data-profile matching.
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
    /// Borrow a fully file-backed RVA range. Virtual zero-fill, overlapping
    /// section mappings and cross-section ranges reject explicitly.
    pub fn rva_slice(&self, rva: u32, length: usize) -> Result<&'a [u8]> {
        ensure!(length <= MAX_PE_BYTES, "PE data slice budget");
        let end = u64::from(rva)
            .checked_add(length as u64)
            .context("PE RVA slice overflow")?;
        if end <= u64::from(self.header_size) {
            return self
                .bytes
                .get(rva as usize..end as usize)
                .context("PE header slice");
        }
        let mut found = None;
        for section in &self.sections {
            if rva >= section.rva && end <= u64::from(section.rva) + u64::from(section.file_size) {
                ensure!(found.is_none(), "ambiguous overlapping PE RVA mapping");
                let offset = u64::from(section.file_offset) + u64::from(rva - section.rva);
                found = Some(
                    self.bytes
                        .get(offset as usize..offset as usize + length)
                        .context("PE backed slice")?,
                );
            }
        }
        found.context("PE RVA range has no contiguous file backing")
    }
    /// Read a preferred virtual-address range as file data. This never invokes
    /// an export/function or grants a native-code execution capability.
    pub fn va_slice(&self, address: u64, length: usize) -> Result<&'a [u8]> {
        let rva = u32::try_from(
            address
                .checked_sub(self.image_base)
                .context("PE address below image base")?,
        )?;
        self.rva_slice(rva, length)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(wide: bool) -> Vec<u8> {
        let mut b = vec![0; 1024];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        let opt = coff + 20;
        let size = if wide { 240u16 } else { 224 };
        b[coff..coff + 2].copy_from_slice(&(if wide { 0x8664u16 } else { 0x14c }).to_le_bytes());
        b[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        b[coff + 16..coff + 18].copy_from_slice(&size.to_le_bytes());
        b[opt..opt + 2].copy_from_slice(&(if wide { 0x20bu16 } else { 0x10b }).to_le_bytes());
        if wide {
            b[opt + 24..opt + 32].copy_from_slice(&0x140000000u64.to_le_bytes());
        } else {
            b[opt + 28..opt + 32].copy_from_slice(&0x400000u32.to_le_bytes());
        }
        b[opt + 56..opt + 60].copy_from_slice(&0x2000u32.to_le_bytes());
        b[opt + 60..opt + 64].copy_from_slice(&512u32.to_le_bytes());
        let section = opt + size as usize;
        b[section..section + 5].copy_from_slice(b".data");
        b[section + 8..section + 12].copy_from_slice(&0x400u32.to_le_bytes());
        b[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        b[section + 16..section + 20].copy_from_slice(&512u32.to_le_bytes());
        b[section + 20..section + 24].copy_from_slice(&512u32.to_le_bytes());
        b[512..516].copy_from_slice(b"data");
        b
    }
    #[test]
    fn pe32_and_pe64_backed_ranges_are_read_only_and_thread_safe() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<PeData<'static>>();
        for wide in [false, true] {
            let bytes = fixture(wide);
            let image = PeData::parse(&bytes).unwrap();
            assert_eq!(image.rva_slice(0x1000, 4).unwrap(), b"data");
            assert_eq!(
                image.va_slice(image.image_base() + 0x1000, 4).unwrap(),
                b"data"
            );
            assert!(image.rva_slice(0x1200, 1).is_err());
            assert!(image.rva_slice(0x11ff, 2).is_err());
            assert!(image.va_slice(image.image_base() - 1, 1).is_err());
            assert_eq!(image.rva_slice(0, 2).unwrap(), b"MZ");
        }
    }
    #[test]
    fn malformed_or_truncated_sections_and_headers_fail() {
        let mut bytes = fixture(false);
        assert!(PeData::parse(&bytes[..200]).is_err());
        let section = 0x84 + 20 + 224;
        bytes[section + 20..section + 24].copy_from_slice(&4096u32.to_le_bytes());
        assert!(PeData::parse(&bytes).is_err());
        assert!(PeData::parse(b"MZ").is_err());
    }
}

/// A bounded RT_MENU resource, read as immutable data from the PE file.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeMenuResource {
    /// Original resource identifier, or named resource key.
    pub name: PeResourceName,
    /// Original Windows language identifier.
    pub language: u16,
    /// Shared MENU/MENUEX format tree; no native code or window handles.
    pub template: crate::pe_menu::MenuTemplate,
}
/// Owned PE resource key; does not identify executable code.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PeResourceName {
    /// Numeric resource name.
    Id(u32),
    /// Named resource in Unicode.
    Name(String),
}
impl PeData<'_> {
    /// Read all bounded RT_MENU language variants. Resources are parsed on the
    /// CPU; this never loads the module as executable code or creates a menu.
    pub fn menu_templates(&self) -> Result<Vec<PeMenuResource>> {
        use pelite::resources::{FindError, Name};
        let image = pelite::PeFile::from_bytes(self.bytes).context("PE resource header")?;
        let resources = image.resources().context("PE resource directory")?;
        let directory = match resources
            .root()?
            .get_dir(Name::Id(pelite::image::RT_MENU as u32))
        {
            Ok(dir) => dir,
            Err(FindError::NotFound) => return Ok(Vec::new()),
            Err(e) => return Err(e).context("PE MENU directory"),
        };
        let mut rows = Vec::new();
        let mut bytes_total = 0usize;
        for resource in directory.entries() {
            let name = match resource.name()? {
                Name::Id(id) => PeResourceName::Id(id),
                Name::Wide(s) => {
                    PeResourceName::Name(String::from_utf16(s).context("PE menu name encoding")?)
                }
                Name::Str(s) => PeResourceName::Name(s.into()),
            };
            let languages = resource
                .entry()?
                .dir()
                .context("PE menu language directory missing")?;
            for language in languages.entries() {
                ensure!(rows.len() < 32, "PE menu resource count budget");
                let language_id = match language.name()? {
                    Name::Id(id) => u16::try_from(id).context("PE menu language range")?,
                    _ => anyhow::bail!("named PE menu language is unsupported"),
                };
                let data = language.entry()?.data().context("PE menu data missing")?;
                bytes_total = bytes_total
                    .checked_add(data.size())
                    .context("PE menu byte overflow")?;
                ensure!(
                    bytes_total <= 4 * 1024 * 1024,
                    "PE cumulative menu byte budget"
                );
                let template = crate::pe_menu::MenuTemplate::parse(data.bytes()?)?;
                rows.push(PeMenuResource {
                    name: name.clone(),
                    language: language_id,
                    template,
                });
            }
        }
        Ok(rows)
    }
}
