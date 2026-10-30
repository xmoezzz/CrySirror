//! Native Circus MES bytecode/resource policy, independent of rendering/devices.
//! Original instruction semantics are separate from shared engine services.
//! Nothing in this module fabricates successful execution of an unknown opcode.
#![forbid(unsafe_code)]
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
mod adapter;
pub mod machine;
pub mod names;
pub mod settings;
#[cfg(feature = "runtime")]
pub use adapter::register;
pub use adapter::{CircusVm, Config};

pub const MAX_SCRIPT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_INSTRUCTIONS: usize = 1_000_000;
pub const MAX_STRING_BYTES: usize = 8191;

/// Explicit bytecode ABI profiles; file extensions and game names are not ABIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    Dc4,
    Dc4PlusHarmony,
}
impl Profile {
    fn version(self) -> u16 {
        match self {
            Self::Dc4 => 0xaab6,
            Self::Dc4PlusHarmony => 0xabb6,
        }
    }
}

/// Original encoded arguments. Their native interpretation depends on opcode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arguments {
    Bytes([u8; 2]),
    SlotString { slot: u8, text: String },
    String(String),
    Words([i16; 4]),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// Native code offset, including the three-byte version prefix.
    pub offset: u32,
    pub next: u32,
    pub opcode: u8,
    pub arguments: Arguments,
    /// 0x77 is a four-word continuation, consumed by the preceding command.
    pub continuation: Option<[i16; 4]>,
}
#[derive(Debug, Clone)]
pub struct Program {
    pub profile: Profile,
    pub code_file_offset: usize,
    /// Original label words, including native high-byte metadata. Resolving
    /// their execution policy is deliberately separate from format validation.
    pub labels: Vec<u32>,
    /// Native auxiliary u16 per label, copied separately by sub_437FD0.
    pub label_metadata: Vec<u16>,
    pub instructions: Vec<Instruction>,
}
fn word(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .context("truncated Circus word")?
            .try_into()?,
    ))
}
fn long(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .context("truncated Circus header/index")?
            .try_into()?,
    ))
}
fn words(bytes: &[u8], at: usize) -> Result<[i16; 4]> {
    Ok([
        word(bytes, at)? as i16,
        word(bytes, at + 2)? as i16,
        word(bytes, at + 4)? as i16,
        word(bytes, at + 6)? as i16,
    ])
}
fn string(bytes: &[u8], at: usize, encrypted: bool) -> Result<(String, usize)> {
    let rest = bytes.get(at..).context("truncated Circus string")?;
    let end = rest
        .iter()
        .take(MAX_STRING_BYTES + 1)
        .position(|b| *b == 0)
        .context("unterminated/oversized Circus string")?;
    let raw = rest[..end]
        .iter()
        .map(|b| if encrypted { b.wrapping_add(0x20) } else { *b })
        .collect::<Vec<_>>();
    let (decoded, errors) = encoding_rs::SHIFT_JIS.decode_without_bom_handling(&raw);
    ensure!(!errors, "invalid Circus Shift-JIS string at {at:#x}");
    Ok((decoded.into_owned(), at + end + 1))
}
impl Program {
    /// Parse the offset2 native layout used by DC4/Plus Harmony. Arithmetic,
    /// allocation, string reads and continuation consumption are bounded.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_SCRIPT_BYTES, "Circus script byte limit");
        let count = long(bytes, 0)? as usize;
        ensure!(count <= 65535, "Circus label count limit");
        ensure!(long(bytes, 4)? == 3, "unsupported Circus MES header layout");
        let code_file_offset = count
            .checked_mul(6)
            .and_then(|v| v.checked_add(4))
            .context("Circus index overflow")?;
        ensure!(code_file_offset >= 8, "Circus index has no version region");
        let code = bytes
            .get(code_file_offset..)
            .context("Circus index exceeds script")?;
        let version = word(code, 0)?;
        let profile = match version {
            0xaab6 => Profile::Dc4,
            0xabb6 => Profile::Dc4PlusHarmony,
            _ => anyhow::bail!("unsupported Circus MES version {version:#06x}"),
        };
        ensure!(code.get(2) == Some(&0), "Circus version suffix");
        // The native runtime copies count words beginning at file offset 4:
        // first label 0 is the value 3 also used by format probes as a marker.
        // MesTextTool's editing view starts at 8 and intentionally treats the
        // combined index/metadata span differently; it is not VM indexing.
        let labels = (0..count)
            .map(|i| long(bytes, 4 + i * 4))
            .collect::<Result<Vec<_>>>()?;
        let label_metadata = (0..count)
            .map(|i| word(bytes, 4 + 4 * count + 2 * i))
            .collect::<Result<Vec<_>>>()?;
        let mut instructions = Vec::new();
        let mut pos = 3;
        while pos < code.len() {
            ensure!(
                instructions.len() < MAX_INSTRUCTIONS,
                "Circus instruction count limit"
            );
            let start = pos;
            let opcode = code[pos];
            ensure!(
                opcode != 0x77,
                "orphan Circus argument continuation at {pos:#x}"
            );
            let arguments = match opcode {
                0x00..=0x3a => {
                    let a = code
                        .get(pos + 1..pos + 3)
                        .context("truncated Circus byte operands")?;
                    pos += 3;
                    Arguments::Bytes(a.try_into()?)
                }
                0x3b..=0x47 => {
                    let slot = *code.get(pos + 1).context("truncated Circus string slot")?;
                    let (text, next) = string(code, pos + 2, false)?;
                    pos = next;
                    Arguments::SlotString { slot, text }
                }
                0x48..=0x6d => {
                    let (text, next) = string(code, pos + 1, opcode >= 0x69)?;
                    pos = next;
                    Arguments::String(text)
                }
                _ => {
                    let args = words(code, pos + 1)?;
                    pos += 9;
                    Arguments::Words(args)
                }
            };
            let continuation = if code.get(pos) == Some(&0x77) {
                let value = words(code, pos + 1)?;
                pos += 9;
                Some(value)
            } else {
                None
            };
            instructions.push(Instruction {
                offset: start as u32,
                next: pos as u32,
                opcode,
                arguments,
                continuation,
            });
        }
        Ok(Self {
            profile,
            code_file_offset,
            labels,
            label_metadata,
            instructions,
        })
    }
    /// Instruction lookup rejects entry into strings, arguments or extensions.
    pub fn instruction(&self, pc: u32) -> Result<&Instruction> {
        let index = self
            .instructions
            .binary_search_by_key(&pc, |op| op.offset)
            .map_err(|_| anyhow::anyhow!("Circus PC {pc:#x} is not an instruction boundary"))?;
        Ok(&self.instructions[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(code: &[u8]) -> Vec<u8> {
        let mut b = vec![];
        b.extend(2u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        b.extend(0x80000003u32.to_le_bytes());
        b.extend([0; 4]);
        b.extend(0xabb6u16.to_le_bytes());
        b.push(0);
        b.extend(code);
        b
    }
    #[test]
    fn original_operands_signedness_encrypted_strings_and_continuation_boundaries() {
        let mut code = vec![0x72];
        for n in [-1i16, 4, 8, 12] {
            code.extend(n.to_le_bytes());
        }
        code.push(0x77);
        for n in [2i16, 3, 4, 5] {
            code.extend(n.to_le_bytes());
        }
        code.extend([0x69, b'A'.wrapping_sub(0x20), 0, 0, 0, 0]);
        let p = Program::parse(&fixture(&code)).unwrap();
        assert_eq!(p.profile, Profile::Dc4PlusHarmony);
        assert_eq!(p.labels, [3, 0x80000003]);
        assert_eq!(p.label_metadata, [0, 0]);
        assert_eq!(p.instructions.len(), 3);
        assert_eq!(
            p.instruction(3).unwrap().arguments,
            Arguments::Words([-1, 4, 8, 12])
        );
        assert_eq!(p.instruction(3).unwrap().continuation, Some([2, 3, 4, 5]));
        assert!(p.instruction(12).is_err());
        assert_eq!(
            p.instruction(21).unwrap().arguments,
            Arguments::String("A".into())
        );
    }
    #[test]
    fn bad_headers_truncation_strings_and_profiles_fail_visibly() {
        for code in [
            &[0x77, 0][..],
            &[0x3d, 1, b'x'][..],
            &[0x72, 0][..],
            &[0, 0][..],
        ] {
            assert!(Program::parse(&fixture(code)).is_err());
        }
        let mut b = fixture(&[0, 0, 0]);
        b[16] = 0;
        assert!(Program::parse(&b).is_err());
        let mut b = fixture(&[0, 0, 0]);
        b[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Program::parse(&b).is_err());
    }
    #[test]
    #[ignore = "requires original locally installed Circus MES files"]
    fn installed_mes_corpus() {
        let root = std::env::var_os("CRYSIRROR_CIRCUS_MES").expect("CRYSIRROR_CIRCUS_MES");
        let mut count = 0;
        for f in std::fs::read_dir(root).unwrap() {
            let p = f.unwrap().path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mes")) {
                Program::parse(&std::fs::read(&p).unwrap())
                    .unwrap_or_else(|e| panic!("{}: {e:#}", p.display()));
                count += 1;
            }
        }
        assert!(count > 0);
        eprintln!("parsed {count} original Circus MES scripts");
    }
}
