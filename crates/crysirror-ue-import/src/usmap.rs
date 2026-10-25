// SPDX-License-Identifier: MPL-2.0
//! `.usmap` class/struct schemas (format versions 0–4, uncompressed) used to
//! decode cooked UE5 unversioned property data.
use anyhow::{bail, ensure, Context, Result};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum PropType {
    Byte { enum_name: Option<String> },
    Bool,
    Int8,
    Int16,
    Int,
    Int64,
    UInt16,
    UInt32,
    UInt64,
    Float,
    Double,
    Object,
    Interface,
    Weak,
    Lazy,
    SoftObject,
    Name,
    Str,
    Utf8Str,
    AnsiStr,
    Text,
    Delegate,
    MulticastDelegate,
    FieldPath,
    Struct(String),
    Enum { inner: Box<PropType>, name: String },
    Array(Box<PropType>),
    Set(Box<PropType>),
    Map(Box<PropType>, Box<PropType>),
    Optional(Box<PropType>),
}
#[derive(Clone, Debug)]
pub struct Prop {
    pub index: u16,
    pub array_dim: u8,
    pub name: String,
    pub ty: PropType,
}
#[derive(Clone, Debug)]
pub struct Schema {
    pub name: String,
    pub super_name: Option<String>,
    /// Total property slots including array dimensions.
    pub count: u16,
    pub props: Vec<Prop>,
}
#[derive(Default)]
pub struct Usmap {
    pub schemas: HashMap<String, Schema>,
    pub enums: HashMap<String, Vec<(u64, String)>>,
}

struct R<'a> {
    b: &'a [u8],
    p: usize,
}
impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        ensure!(self.p + n <= self.b.len(), "usmap truncated");
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into()?))
    }
}

impl Usmap {
    /// Load a `.usmap`, or recover the schemas from a UE Win64 binary.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        if bytes.starts_with(b"MZ") {
            return Ok(crate::reflect::Reflection::read(&bytes)?.usmap);
        }
        Self::parse(&bytes)
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = R { b: bytes, p: 0 };
        ensure!(r.u16()? == 0x30C4, "not a usmap");
        let version = r.u8()?;
        ensure!(version <= 4, "unsupported usmap version {version}");
        // bHasVersioning is a 32-bit bool.
        if version >= 1 && r.u32()? != 0 {
            // FPackageFileVersion + custom versions + net CL.
            r.take(8)?;
            let n = r.u32()? as usize;
            r.take(n * 20)?;
            r.take(4)?;
        }
        let method = r.u8()?;
        let compressed = r.u32()? as usize;
        let size = r.u32()? as usize;
        let raw = r.take(compressed)?;
        let owned;
        let body: &[u8] = match method {
            0 => raw,
            1 => {
                let mut out = vec![0u8; size];
                oozextract::Extractor::new()
                    .read_from_slice(raw, &mut out)
                    .map_err(|e| anyhow::anyhow!("oodle usmap: {e:?}"))?;
                owned = out;
                &owned
            }
            3 => {
                use std::io::Read;
                let mut out = Vec::with_capacity(size);
                ruzstd::StreamingDecoder::new(raw)
                    .map_err(|e| anyhow::anyhow!("zstd usmap: {e}"))?
                    .read_to_end(&mut out)?;
                owned = out;
                &owned
            }
            m => bail!("unsupported usmap compression {m} (brotli)"),
        };
        ensure!(body.len() == size, "usmap size mismatch");
        let mut r = R { b: body, p: 0 };
        let n = r.u32()? as usize;
        let mut names = Vec::with_capacity(n);
        for _ in 0..n {
            let len = if version >= 2 {
                r.u16()? as usize
            } else {
                r.u8()? as usize
            };
            names.push(String::from_utf8_lossy(r.take(len)?).into_owned());
        }
        let name = |i: u32| -> Result<String> {
            names.get(i as usize).cloned().context("usmap name index")
        };
        let mut out = Usmap::default();
        for _ in 0..r.u32()? {
            let en = name(r.u32()?)?;
            let count = if version >= 3 {
                r.u16()? as usize
            } else {
                r.u8()? as usize
            };
            let mut vals = Vec::with_capacity(count);
            for i in 0..count {
                if version >= 4 {
                    let v = r.u64()?;
                    vals.push((v, name(r.u32()?)?));
                } else {
                    vals.push((i as u64, name(r.u32()?)?));
                }
            }
            out.enums.insert(en, vals);
        }
        fn ty(r: &mut R, name: &dyn Fn(u32) -> Result<String>) -> Result<PropType> {
            Ok(match r.u8()? {
                0 => PropType::Byte { enum_name: None },
                1 => PropType::Bool,
                2 => PropType::Int,
                3 => PropType::Float,
                4 => PropType::Object,
                5 => PropType::Name,
                6 => PropType::Delegate,
                7 => PropType::Double,
                8 => PropType::Array(Box::new(ty(r, name)?)),
                9 => PropType::Struct(name(r.u32()?)?),
                10 => PropType::Str,
                11 => PropType::Text,
                12 => PropType::Interface,
                13 => PropType::MulticastDelegate,
                14 => PropType::Weak,
                15 => PropType::Lazy,
                16 | 17 => PropType::SoftObject,
                18 => PropType::UInt64,
                19 => PropType::UInt32,
                20 => PropType::UInt16,
                21 => PropType::Int64,
                22 => PropType::Int16,
                23 => PropType::Int8,
                24 => {
                    let k = ty(r, name)?;
                    PropType::Map(Box::new(k), Box::new(ty(r, name)?))
                }
                25 => PropType::Set(Box::new(ty(r, name)?)),
                26 => {
                    let inner = ty(r, name)?;
                    PropType::Enum {
                        inner: Box::new(inner),
                        name: name(r.u32()?)?,
                    }
                }
                27 => PropType::FieldPath,
                28 => PropType::Optional(Box::new(ty(r, name)?)),
                29 => PropType::Utf8Str,
                30 => PropType::AnsiStr,
                t => bail!("unknown usmap property type {t}"),
            })
        }
        for _ in 0..r.u32()? {
            let sname = name(r.u32()?)?;
            let sup = r.u32()?;
            let count = r.u16()?;
            let serializable = r.u16()?;
            let mut props = Vec::with_capacity(serializable as usize);
            for _ in 0..serializable {
                let index = r.u16()?;
                let array_dim = r.u8()?;
                let pname = name(r.u32()?)?;
                let t = ty(&mut r, &name)?;
                props.push(Prop {
                    index,
                    array_dim,
                    name: pname,
                    ty: t,
                });
            }
            out.schemas.insert(
                sname.clone(),
                Schema {
                    name: sname,
                    super_name: (sup != u32::MAX).then(|| name(sup)).transpose()?,
                    count,
                    props,
                },
            );
        }
        Ok(out)
    }
    /// Flattened property slots of a struct and its supers, indexed by schema slot.
    pub fn slots(&self, name: &str) -> Result<Vec<Option<(String, PropType)>>> {
        let mut chain = Vec::new();
        let mut cur = Some(name.to_owned());
        while let Some(n) = cur {
            let s = self
                .schemas
                .get(&n)
                .with_context(|| format!("no schema for '{n}'"))?;
            chain.push(s);
            cur = s.super_name.clone();
            ensure!(chain.len() < 64, "schema chain too deep");
        }
        let mut slots = Vec::new();
        // Unversioned schemas follow `PropertyLink` (TFieldIterator order):
        // the class's own properties first, then each super in turn.
        for s in &chain {
            let base = slots.len();
            slots.resize(base + s.count as usize, None);
            for p in &s.props {
                for d in 0..p.array_dim.max(1) as usize {
                    let i = base + p.index as usize + d;
                    if i < slots.len() {
                        slots[i] = Some((p.name.clone(), p.ty.clone()));
                    }
                }
            }
        }
        Ok(slots)
    }
}

impl Usmap {
    /// Export the recovered schema to the interoperable uncompressed v4 format.
    /// This contains reflection metadata, never executable bytes or game payloads.
    pub fn encode(&self) -> Result<Vec<u8>> {
        use std::collections::{BTreeMap, BTreeSet};
        fn collect(t: &PropType, n: &mut BTreeSet<String>) {
            match t {
                PropType::Struct(s) => {
                    n.insert(s.clone());
                }
                PropType::Byte { enum_name: Some(s) } => {
                    n.insert(s.clone());
                }
                PropType::Enum { inner, name } => {
                    n.insert(name.clone());
                    collect(inner, n)
                }
                PropType::Array(t) | PropType::Set(t) | PropType::Optional(t) => collect(t, n),
                PropType::Map(k, v) => {
                    collect(k, n);
                    collect(v, n)
                }
                _ => {}
            }
        }
        let mut names = BTreeSet::new();
        for s in self.schemas.values() {
            names.insert(s.name.clone());
            if let Some(n) = &s.super_name {
                names.insert(n.clone());
            }
            for p in &s.props {
                names.insert(p.name.clone());
                collect(&p.ty, &mut names);
            }
        }
        for (n, v) in &self.enums {
            names.insert(n.clone());
            for (_, n) in v {
                names.insert(n.clone());
            }
        }
        let names: Vec<_> = names.into_iter().collect();
        let ids: BTreeMap<_, _> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i as u32))
            .collect();
        fn u32v(out: &mut Vec<u8>, v: u32) {
            out.extend(v.to_le_bytes());
        }
        fn ty(out: &mut Vec<u8>, t: &PropType, ids: &BTreeMap<&str, u32>) -> Result<()> {
            use PropType::*;
            if let Byte {
                enum_name: Some(name),
            } = t
            {
                out.push(26);
                out.push(0);
                u32v(out, ids[name.as_str()]);
                return Ok(());
            }
            let code = match t {
                Byte { .. } => 0,
                Bool => 1,
                Int => 2,
                Float => 3,
                Object => 4,
                Name => 5,
                Delegate => 6,
                Double => 7,
                Array(_) => 8,
                Struct(_) => 9,
                Str => 10,
                Text => 11,
                Interface => 12,
                MulticastDelegate => 13,
                Weak => 14,
                Lazy => 15,
                SoftObject => 16,
                UInt64 => 18,
                UInt32 => 19,
                UInt16 => 20,
                Int64 => 21,
                Int16 => 22,
                Int8 => 23,
                Map(..) => 24,
                Set(_) => 25,
                Enum { .. } => 26,
                FieldPath => 27,
                Optional(_) => 28,
                Utf8Str => 29,
                AnsiStr => 30,
            };
            out.push(code);
            match t {
                Struct(n) => u32v(out, ids[n.as_str()]),
                Enum { inner, name } => {
                    ty(out, inner, ids)?;
                    u32v(out, ids[name.as_str()]);
                }
                Array(t) | Set(t) | Optional(t) => ty(out, t, ids)?,
                Map(k, v) => {
                    ty(out, k, ids)?;
                    ty(out, v, ids)?;
                }
                _ => {}
            }
            Ok(())
        }
        let mut body = Vec::new();
        u32v(&mut body, names.len() as u32);
        for n in &names {
            ensure!(n.len() <= u16::MAX as usize, "schema name too long");
            body.extend((n.len() as u16).to_le_bytes());
            body.extend(n.as_bytes());
        }
        let mut enums: Vec<_> = self.enums.iter().collect();
        enums.sort_by_key(|(n, _)| *n);
        u32v(&mut body, enums.len() as u32);
        for (n, values) in enums {
            u32v(&mut body, ids[n.as_str()]);
            ensure!(values.len() <= u16::MAX as usize, "enum size");
            body.extend((values.len() as u16).to_le_bytes());
            for (v, n) in values {
                body.extend(v.to_le_bytes());
                u32v(&mut body, ids[n.as_str()]);
            }
        }
        let mut schemas: Vec<_> = self.schemas.values().collect();
        schemas.sort_by_key(|s| &s.name);
        u32v(&mut body, schemas.len() as u32);
        for s in schemas {
            u32v(&mut body, ids[s.name.as_str()]);
            u32v(
                &mut body,
                s.super_name.as_ref().map_or(u32::MAX, |n| ids[n.as_str()]),
            );
            body.extend(s.count.to_le_bytes());
            ensure!(s.props.len() <= u16::MAX as usize, "property count");
            body.extend((s.props.len() as u16).to_le_bytes());
            for p in &s.props {
                body.extend(p.index.to_le_bytes());
                body.push(p.array_dim);
                u32v(&mut body, ids[p.name.as_str()]);
                ty(&mut body, &p.ty, &ids)?;
            }
        }
        ensure!(body.len() <= 64 * 1024 * 1024, "schema file too large");
        let mut out = vec![0xc4, 0x30, 4, 0, 0, 0, 0, 0];
        u32v(&mut out, body.len() as u32);
        u32v(&mut out, body.len() as u32);
        out.extend(body);
        Ok(out)
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;
    #[test]
    fn encoding_roundtrips_inheritance_arrays_and_enum_values() {
        let mut m = Usmap::default();
        m.enums
            .insert("Mode".into(), vec![(0, "Idle".into()), (270, "Run".into())]);
        m.schemas.insert(
            "Base".into(),
            Schema {
                name: "Base".into(),
                super_name: None,
                count: 1,
                props: vec![Prop {
                    index: 0,
                    array_dim: 1,
                    name: "Label".into(),
                    ty: PropType::Str,
                }],
            },
        );
        m.schemas.insert(
            "Child".into(),
            Schema {
                name: "Child".into(),
                super_name: Some("Base".into()),
                count: 3,
                props: vec![
                    Prop {
                        index: 0,
                        array_dim: 2,
                        name: "Value".into(),
                        ty: PropType::Int,
                    },
                    Prop {
                        index: 2,
                        array_dim: 1,
                        name: "Mode".into(),
                        ty: PropType::Enum {
                            inner: Box::new(PropType::UInt16),
                            name: "Mode".into(),
                        },
                    },
                ],
            },
        );
        let encoded = m.encode().unwrap();
        let restored = Usmap::parse(&encoded).unwrap();
        assert_eq!(restored.slots("Child").unwrap(), m.slots("Child").unwrap());
        assert_eq!(restored.enums, m.enums);
        assert_eq!(encoded, restored.encode().unwrap());
    }
    #[test]
    fn byte_enum_metadata_is_preserved_as_usmap_enum() {
        let mut m = Usmap::default();
        m.enums.insert("Mode".into(), vec![(1, "Active".into())]);
        m.schemas.insert(
            "S".into(),
            Schema {
                name: "S".into(),
                super_name: None,
                count: 1,
                props: vec![Prop {
                    index: 0,
                    array_dim: 1,
                    name: "Mode".into(),
                    ty: PropType::Byte {
                        enum_name: Some("Mode".into()),
                    },
                }],
            },
        );
        let r = Usmap::parse(&m.encode().unwrap()).unwrap();
        assert_eq!(
            r.schemas["S"].props[0].ty,
            PropType::Enum {
                inner: Box::new(PropType::Byte { enum_name: None }),
                name: "Mode".into()
            }
        );
    }
}
