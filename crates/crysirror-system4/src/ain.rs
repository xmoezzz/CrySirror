// Copyright (c) 2026 CrySirror contributors.
// Format definitions researched against libsys4, Copyright (C) 2019 Nunuhara
// Cabbage; GPL-2.0-or-later. See SOURCE.json and LICENSE.
//! Bounded AIN v5 program reader. Byte strings retain original encoding.
use anyhow::{Context, Result, bail, ensure};
use std::{collections::BTreeSet, io::Read};
pub const MAX_AIN_BYTES: usize = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 1_000_000;
#[derive(Clone, Debug)]
pub struct Variable {
    pub name: Vec<u8>,
    pub kind: i32,
    pub structure: i32,
    pub rank: i32,
    pub group: Option<i32>,
}
#[derive(Clone, Debug)]
pub struct Function {
    pub address: u32,
    pub name: Vec<u8>,
    pub is_label: bool,
    pub return_kind: i32,
    pub return_structure: i32,
    pub arguments: u32,
    pub crc: u32,
    pub variables: Vec<Variable>,
}
#[derive(Clone, Debug)]
pub struct Structure {
    pub name: Vec<u8>,
    pub constructor: i32,
    pub destructor: i32,
    pub members: Vec<Variable>,
}
#[derive(Clone, Debug)]
pub enum InitialValue {
    Integer { global: u32, kind: i32, value: i32 },
    String { global: u32, value: Vec<u8> },
}
#[derive(Clone, Debug)]
pub struct HllFunction {
    pub name: Vec<u8>,
    pub return_kind: i32,
    pub arguments: Vec<(Vec<u8>, i32)>,
}
#[derive(Clone, Debug)]
pub struct Library {
    pub name: Vec<u8>,
    pub functions: Vec<HllFunction>,
}
#[derive(Clone, Debug)]
pub struct Switch {
    pub kind: i32,
    pub default: i32,
    pub cases: Vec<(i32, u32)>,
}
#[derive(Clone, Debug)]
pub struct FunctionType {
    pub name: Vec<u8>,
    pub return_kind: i32,
    pub return_structure: i32,
    pub arguments: u32,
    pub variables: Vec<Variable>,
}
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub version: u32,
    pub keycode: u32,
    pub code: Vec<u8>,
    pub functions: Vec<Function>,
    pub globals: Vec<Variable>,
    pub initial_values: Vec<InitialValue>,
    pub structures: Vec<Structure>,
    pub messages: Vec<Vec<u8>>,
    pub strings: Vec<Vec<u8>>,
    pub filenames: Vec<Vec<u8>>,
    pub main: u32,
    pub message_function: i32,
    pub libraries: Vec<Library>,
    pub switches: Vec<Switch>,
    pub game_version: i32,
    pub labels: Vec<(Vec<u8>, u32)>,
    pub object_jump: Option<i32>,
    pub function_types: Vec<FunctionType>,
    pub delegates: Vec<FunctionType>,
    pub global_groups: Vec<Vec<u8>>,
    pub function_type_size_metadata: Option<u32>,
    pub delegate_size_metadata: Option<u32>,
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    records: usize,
    strings: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).context("AIN offset overflow")?;
        let out = self
            .bytes
            .get(self.at..end)
            .context("truncated AIN field")?;
        self.at = end;
        Ok(out)
    }
    fn word(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn int(&mut self) -> Result<i32> {
        Ok(self.word()? as i32)
    }
    fn count(&mut self) -> Result<usize> {
        let n = self.word()? as usize;
        self.records = self.records.checked_add(n).context("AIN record overflow")?;
        ensure!(
            self.records <= MAX_RECORDS && n <= self.bytes.len().saturating_sub(self.at),
            "AIN record budget"
        );
        Ok(n)
    }
    fn string(&mut self) -> Result<Vec<u8>> {
        let len = self.bytes[self.at..]
            .iter()
            .position(|b| *b == 0)
            .context("unterminated AIN string")?;
        ensure!(len <= 1024 * 1024, "AIN individual string budget");
        self.strings = self
            .strings
            .checked_add(len)
            .context("AIN string overflow")?;
        ensure!(self.strings <= MAX_AIN_BYTES, "AIN string budget");
        let out = self.take(len)?.to_vec();
        self.take(1)?;
        Ok(out)
    }
    fn strings(&mut self) -> Result<Vec<Vec<u8>>> {
        let n = self.count()?;
        (0..n).map(|_| self.string()).collect()
    }
    fn variable(&mut self, global: bool) -> Result<Variable> {
        Ok(Variable {
            name: self.string()?,
            kind: self.int()?,
            structure: self.int()?,
            rank: self.int()?,
            group: if global { Some(self.int()?) } else { None },
        })
    }
    fn variables(&mut self, global: bool) -> Result<Vec<Variable>> {
        let n = self.count()?;
        (0..n).map(|_| self.variable(global)).collect()
    }
    fn function_types(&mut self) -> Result<(u32, Vec<FunctionType>)> {
        let declared = self.word()?;
        let n = self.count()?;
        let types = (0..n)
            .map(|_| {
                let name = self.string()?;
                let return_kind = self.int()?;
                let return_structure = self.int()?;
                let arguments = self.word()?;
                let variables = self.variables(false)?;
                ensure!(
                    arguments as usize <= variables.len(),
                    "AIN function-type argument count"
                );
                Ok(FunctionType {
                    name,
                    return_kind,
                    return_structure,
                    arguments,
                    variables,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        // The original reader retains this field but does not seek or size the
        // serialized records by it. Preserve it as metadata; records remain
        // bounded by their actual typed fields and the next section tag.
        Ok((declared, types))
    }
}
/// Decode legacy MT-XOR or AI2 zlib envelopes without executing native code.
/// The caller must schedule this CPU work off the owner thread for large assets.
pub fn decode(bytes: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        bytes.len() <= MAX_AIN_BYTES && bytes.len() >= 8,
        "AIN input size"
    );
    let out = if bytes.starts_with(b"AI2\0\0\0\0\0") {
        ensure!(bytes.len() >= 16, "truncated AI2 header");
        let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let compressed = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        ensure!(
            size <= MAX_AIN_BYTES && compressed == bytes.len() - 16,
            "AI2 envelope bounds"
        );
        let mut decoder = flate2::read::ZlibDecoder::new(&bytes[16..]);
        let mut out = Vec::new();
        decoder
            .by_ref()
            .take(size as u64 + 1)
            .read_to_end(&mut out)?;
        ensure!(
            out.len() == size && decoder.total_in() == compressed as u64,
            "AI2 decompressed size/trailing data"
        );
        out
    } else if bytes.starts_with(b"VERS") {
        bytes.to_vec()
    } else {
        let mut out = bytes.to_vec();
        crysirror_resource::legacy_stream::mt19937_1998_xor(&mut out, 0x5d3e3);
        out
    };
    ensure!(out.starts_with(b"VERS"), "unrecognized AIN envelope");
    Ok(out)
}
impl Program {
    pub fn parse(input: &[u8]) -> Result<Self> {
        let bytes = decode(input)?;
        let mut r = Reader {
            bytes: &bytes,
            at: 0,
            records: 0,
            strings: 0,
        };
        let mut p = Self::default();
        let mut seen = BTreeSet::new();
        while r.at < bytes.len() {
            let at = r.at;
            let tag: [u8; 4] = r.take(4)?.try_into().unwrap();
            ensure!(seen.insert(tag), "duplicate AIN section at {at:#x}");
            if at == 0 {
                ensure!(&tag == b"VERS", "AIN VERS must be first");
            }
            match &tag {
                b"VERS" => {
                    p.version = r.word()?;
                    ensure!(p.version == 5, "unsupported AIN version {}", p.version);
                }
                b"KEYC" => p.keycode = r.word()?,
                b"CODE" => {
                    let n = r.word()? as usize;
                    p.code = r.take(n)?.to_vec();
                }
                b"FUNC" => {
                    let n = r.count()?;
                    for _ in 0..n {
                        let address = r.word()?;
                        let name = r.string()?;
                        let label = r.int()?;
                        ensure!(label == 0 || label == 1, "AIN function label flag");
                        let return_kind = r.int()?;
                        let return_structure = r.int()?;
                        let arguments = r.word()?;
                        let vars = r.count()?;
                        let crc = r.word()?;
                        let variables = (0..vars)
                            .map(|_| r.variable(false))
                            .collect::<Result<Vec<_>>>()?;
                        ensure!(arguments as usize <= vars, "AIN function arguments");
                        p.functions.push(Function {
                            address,
                            name,
                            is_label: label == 1,
                            return_kind,
                            return_structure,
                            arguments,
                            crc,
                            variables,
                        });
                    }
                }
                b"GLOB" => p.globals = r.variables(true)?,
                b"GSET" => {
                    let n = r.count()?;
                    for _ in 0..n {
                        let global = r.word()?;
                        let kind = r.int()?;
                        p.initial_values.push(if kind == 12 {
                            InitialValue::String {
                                global,
                                value: r.string()?,
                            }
                        } else {
                            InitialValue::Integer {
                                global,
                                kind,
                                value: r.int()?,
                            }
                        });
                    }
                }
                b"STRT" => {
                    let n = r.count()?;
                    for _ in 0..n {
                        p.structures.push(Structure {
                            name: r.string()?,
                            constructor: r.int()?,
                            destructor: r.int()?,
                            members: r.variables(false)?,
                        });
                    }
                }
                b"MSG0" => p.messages = r.strings()?,
                b"STR0" => p.strings = r.strings()?,
                b"FNAM" => p.filenames = r.strings()?,
                b"OBJG" => p.global_groups = r.strings()?,
                b"MAIN" => p.main = r.word()?,
                b"MSGF" => p.message_function = r.int()?,
                b"GVER" => p.game_version = r.int()?,
                b"OJMP" => p.object_jump = Some(r.int()?),
                b"HLL0" => {
                    let n = r.count()?;
                    for _ in 0..n {
                        let name = r.string()?;
                        let funcs = r.count()?;
                        let mut functions = Vec::new();
                        for _ in 0..funcs {
                            let name = r.string()?;
                            let return_kind = r.int()?;
                            let args = r.count()?;
                            ensure!(args <= 100, "AIN HLL argument budget");
                            let arguments = (0..args)
                                .map(|_| Ok((r.string()?, r.int()?)))
                                .collect::<Result<Vec<_>>>()?;
                            functions.push(HllFunction {
                                name,
                                return_kind,
                                arguments,
                            });
                        }
                        p.libraries.push(Library { name, functions });
                    }
                }
                b"SWI0" => {
                    let n = r.count()?;
                    for _ in 0..n {
                        let kind = r.int()?;
                        let default = r.int()?;
                        let cases = r.count()?;
                        let cases = (0..cases)
                            .map(|_| Ok((r.int()?, r.word()?)))
                            .collect::<Result<Vec<_>>>()?;
                        p.switches.push(Switch {
                            kind,
                            default,
                            cases,
                        });
                    }
                }
                b"SLBL" => {
                    let n = r.count()?;
                    p.labels = (0..n)
                        .map(|_| Ok((r.string()?, r.word()?)))
                        .collect::<Result<Vec<_>>>()?;
                }
                b"FNCT" => {
                    let (size, types) = r.function_types()?;
                    p.function_type_size_metadata = Some(size);
                    p.function_types = types;
                }
                b"DELG" => {
                    let (size, types) = r.function_types()?;
                    p.delegate_size_metadata = Some(size);
                    p.delegates = types;
                }
                _ => bail!(
                    "unsupported AIN section {:?} at {at:#x}",
                    String::from_utf8_lossy(&tag)
                ),
            }
        }
        ensure!(
            seen.contains(b"CODE") && seen.contains(b"FUNC") && seen.contains(b"MAIN"),
            "AIN missing program sections"
        );
        ensure!(
            !p.code.is_empty() && (p.main as usize) < p.functions.len(),
            "AIN main function"
        );
        for f in &p.functions {
            ensure!(
                (f.address as usize) < p.code.len(),
                "AIN function address outside code"
            );
        }
        for init in &p.initial_values {
            let global = match init {
                InitialValue::Integer { global, .. } | InitialValue::String { global, .. } => {
                    *global
                }
            };
            ensure!(
                (global as usize) < p.globals.len(),
                "AIN initializer global index"
            );
        }
        Ok(p)
    }
}
