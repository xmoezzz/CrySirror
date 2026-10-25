// SPDX-License-Identifier: MPL-2.0
//! Static reflection recovery from a monolithic UE 5.7 Win64 shipping binary.
//!
//! Runtime dumpers (Dumper-7, UE4SS) walk `GUObjectArray` inside a running
//! game. We cannot run the Windows build, so instead we read the tables UHT
//! compiles into the image: `F*RegisterCompiledInInfo` entries (C++ name +
//! `Z_Construct_*` function), the `ClassParams` / `StructParams` /
//! `EnumParams` those functions pass to `UECodeGen_Private`, and the
//! `F*PropertyParams` arrays. Layouts were calibrated against this binary
//! (see docs/UE_IMPORT.md); anything that does not match fails visibly.
use crate::usmap::{Prop, PropType, Schema, Usmap};
use anyhow::{bail, ensure, Context, Result};
use std::collections::HashMap;

struct Section {
    name: String,
    va: u64,
    size: u64,
    raw: usize,
    raw_size: usize,
}

pub struct Image<'a> {
    b: &'a [u8],
    sections: Vec<Section>,
}

impl<'a> Image<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Self> {
        ensure!(b.get(..2) == Some(b"MZ"), "not a PE image");
        let u16at = |o: usize| -> Result<u16> {
            Ok(u16::from_le_bytes(
                b.get(o..o + 2).context("pe")?.try_into()?,
            ))
        };
        let u32at = |o: usize| -> Result<u32> {
            Ok(u32::from_le_bytes(
                b.get(o..o + 4).context("pe")?.try_into()?,
            ))
        };
        let pe = u32at(0x3c)? as usize;
        ensure!(b.get(pe..pe + 4) == Some(b"PE\0\0"), "bad PE signature");
        ensure!(
            u16at(pe + 24)? == 0x20b,
            "only PE32+ (x64) images are supported"
        );
        let count = u16at(pe + 6)? as usize;
        let opt = u16at(pe + 20)? as usize;
        let base = u64::from_le_bytes(b.get(pe + 48..pe + 56).context("pe")?.try_into()?);
        let mut sections = Vec::new();
        for i in 0..count {
            let o = pe + 24 + opt + i * 40;
            let name = b.get(o..o + 8).context("section")?;
            sections.push(Section {
                name: String::from_utf8_lossy(name)
                    .trim_end_matches('\0')
                    .to_owned(),
                size: u32at(o + 8)? as u64,
                va: base + u32at(o + 12)? as u64,
                raw_size: u32at(o + 16)? as usize,
                raw: u32at(o + 20)? as usize,
            });
        }
        Ok(Self { b, sections })
    }
    fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }
    fn in_section(&self, va: u64, name: &str) -> bool {
        self.section(name)
            .is_some_and(|s| va >= s.va && va < s.va + s.size)
    }
    fn is_code(&self, va: u64) -> bool {
        self.in_section(va, ".text")
    }
    fn is_data(&self, va: u64) -> bool {
        self.in_section(va, ".rdata") || self.in_section(va, ".data")
    }
    fn bytes(&self, va: u64, n: usize) -> Option<&'a [u8]> {
        let s = self
            .sections
            .iter()
            .find(|s| va >= s.va && va < s.va + s.size)?;
        let off = (va - s.va) as usize;
        if off + n > s.raw_size {
            return None;
        }
        self.b.get(s.raw + off..s.raw + off + n)
    }
    fn q(&self, va: u64) -> u64 {
        self.bytes(va, 8)
            .map_or(0, |x| u64::from_le_bytes(x.try_into().unwrap()))
    }
    fn d(&self, va: u64) -> u32 {
        self.bytes(va, 4)
            .map_or(0, |x| u32::from_le_bytes(x.try_into().unwrap()))
    }
    fn w(&self, va: u64) -> u16 {
        self.bytes(va, 2)
            .map_or(0, |x| u16::from_le_bytes(x.try_into().unwrap()))
    }
    fn byte(&self, va: u64) -> u8 {
        self.bytes(va, 1).map_or(0, |x| x[0])
    }
    /// A printable ASCII C string (identifiers, paths).
    fn cstr(&self, va: u64) -> Option<String> {
        if !self.is_data(va) {
            return None;
        }
        let mut out = String::new();
        for i in 0..512 {
            match self.byte(va + i) {
                0 => return (!out.is_empty()).then_some(out),
                c @ 0x20..=0x7e => out.push(c as char),
                _ => return None,
            }
        }
        None
    }
    fn wstr(&self, va: u64) -> Option<String> {
        if !self.in_section(va, ".rdata") {
            return None;
        }
        let mut out = String::new();
        for i in 0..256 {
            match self.w(va + i * 2) {
                0 => return (!out.is_empty()).then_some(out),
                c @ 0x20..=0x7e => out.push(c as u8 as char),
                _ => return None,
            }
        }
        None
    }
    /// Targets of RIP-relative `lea`/`mov r64, [rip+disp32]` in a short
    /// `Z_Construct_*` function, stopping at its `ret`.
    fn rip_refs(&self, mut f: u64) -> Vec<u64> {
        if self.byte(f) == 0xE9 {
            f = f
                .wrapping_add(5)
                .wrapping_add(self.d(f + 1) as i32 as i64 as u64);
        }
        let Some(code) = self.bytes(f, 192) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in 0..code.len() - 7 {
            if code[i] == 0xC3 && code[i + 1] == 0xCC {
                break;
            }
            if matches!(code[i], 0x48 | 0x4C)
                && matches!(code[i + 1], 0x8D | 0x8B)
                && code[i + 2] & 0xC7 == 0x05
            {
                let disp = i32::from_le_bytes(code[i + 3..i + 7].try_into().unwrap()) as i64;
                out.push((f + i as u64 + 7).wrapping_add(disp as u64));
            }
        }
        out
    }
}

/// `EPropertyGenFlags` (low 6 bits; 0x40 marks ObjectPtr / native bool).
mod gen {
    pub const BYTE: u8 = 0x00;
    pub const INT8: u8 = 0x01;
    pub const INT16: u8 = 0x02;
    pub const INT: u8 = 0x03;
    pub const INT64: u8 = 0x04;
    pub const UINT16: u8 = 0x05;
    pub const UINT32: u8 = 0x06;
    pub const UINT64: u8 = 0x07;
    pub const UNSIZED_INT: u8 = 0x08;
    pub const UNSIZED_UINT: u8 = 0x09;
    pub const FLOAT: u8 = 0x0A;
    pub const DOUBLE: u8 = 0x0B;
    pub const BOOL: u8 = 0x0C;
    pub const SOFT_CLASS: u8 = 0x0D;
    pub const WEAK_OBJECT: u8 = 0x0E;
    pub const LAZY_OBJECT: u8 = 0x0F;
    pub const SOFT_OBJECT: u8 = 0x10;
    pub const CLASS: u8 = 0x11;
    pub const OBJECT: u8 = 0x12;
    pub const INTERFACE: u8 = 0x13;
    pub const NAME: u8 = 0x14;
    pub const STR: u8 = 0x15;
    pub const ARRAY: u8 = 0x16;
    pub const MAP: u8 = 0x17;
    pub const SET: u8 = 0x18;
    pub const STRUCT: u8 = 0x19;
    pub const DELEGATE: u8 = 0x1A;
    pub const INLINE_MULTICAST: u8 = 0x1B;
    pub const SPARSE_MULTICAST: u8 = 0x1C;
    pub const TEXT: u8 = 0x1D;
    pub const ENUM: u8 = 0x1E;
    pub const FIELD_PATH: u8 = 0x1F;
    pub const LWC_REAL: u8 = 0x20;
    pub const OPTIONAL: u8 = 0x21;
    pub const VERSE_VALUE: u8 = 0x22;
    pub const UTF8_STR: u8 = 0x23;
    pub const ANSI_STR: u8 = 0x24;
}

pub struct Reflection<'a> {
    img: Image<'a>,
    /// `Z_Construct_UClass_*` → class name without prefix.
    class_fns: HashMap<u64, String>,
    struct_cache: HashMap<u64, Option<String>>,
    enum_cache: HashMap<u64, Option<String>>,
    pub usmap: Usmap,
    /// Unrecognized property kinds, for diagnostics.
    pub unknown_kinds: HashMap<u8, Vec<String>>,
}

fn strip_prefix(name: &str) -> &str {
    let mut c = name.chars();
    match (c.next(), c.next()) {
        (Some('U' | 'A' | 'I' | 'F'), Some(n)) if n.is_ascii_uppercase() => &name[1..],
        _ => name,
    }
}

impl<'a> Reflection<'a> {
    pub fn read(bytes: &'a [u8]) -> Result<Self> {
        let img = Image::parse(bytes)?;
        let mut r = Self {
            img,
            class_fns: HashMap::new(),
            struct_cache: HashMap::new(),
            enum_cache: HashMap::new(),
            usmap: Usmap::default(),
            unknown_kinds: HashMap::new(),
        };
        let classes = r.find_classes()?;
        ensure!(
            !classes.is_empty(),
            "no UHT class registrations found (unknown layout)"
        );
        for (fn_va, name, _) in &classes {
            r.class_fns.insert(*fn_va, name.clone());
        }
        for (fn_va, name, params) in classes {
            r.build_class(fn_va, &name, params)?;
        }
        Ok(r)
    }

    /// FClassRegisterCompiledInInfo { OuterRegister, InnerRegister, Name (TCHAR*), Info, … }
    /// whose OuterRegister references a plausible FClassParams.
    fn find_classes(&self) -> Result<Vec<(u64, String, u64)>> {
        let rdata = self.img.section(".rdata").context("no .rdata")?;
        let raw = &self.img.b[rdata.raw..rdata.raw + rdata.raw_size];
        let q = |i: usize| u64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap());
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for i in 2..raw.len() / 8 - 1 {
            let (outer, inner, name, info) = (q(i - 2), q(i - 1), q(i), q(i + 1));
            if !(self.img.is_code(outer)
                && self.img.is_code(inner)
                && self.img.in_section(name, ".rdata")
                && self.img.in_section(info, ".data"))
            {
                continue;
            }
            let Some(w) = self.img.wstr(name) else {
                continue;
            };
            if !seen.insert(outer) {
                continue;
            }
            if let Some(params) = self
                .img
                .rip_refs(outer)
                .into_iter()
                .find(|t| self.is_class_params(*t))
            {
                out.push((outer, strip_prefix(&w).to_owned(), params));
            }
        }
        Ok(out)
    }

    fn is_class_params(&self, t: u64) -> bool {
        let i = &self.img;
        if !i.is_data(t) || !i.is_code(i.q(t)) {
            return false;
        }
        let bits = i.d(t + 56);
        let (deps, props) = (bits & 0xF, (bits >> 15) & 0x7FF);
        let deps_ok = deps == 0 || i.is_data(i.q(t + 24));
        let props_ok =
            props == 0 || (i.is_data(i.q(t + 40)) && i.cstr(i.q(i.q(i.q(t + 40)))).is_some());
        deps_ok && props_ok
    }

    fn build_class(&mut self, _fn_va: u64, name: &str, t: u64) -> Result<()> {
        let bits = self.img.d(t + 56);
        let (deps, nprops) = ((bits & 0xF) as u64, ((bits >> 15) & 0x7FF) as usize);
        // DependencySingletonFuncArray[0] is the super class constructor.
        let super_name = (deps > 0)
            .then(|| self.img.q(self.img.q(t + 24)))
            .and_then(|f| self.class_fns.get(&f).cloned());
        let props = self.read_props(self.img.q(t + 40), nprops, name)?;
        self.insert(name, super_name, props);
        Ok(())
    }

    fn insert(
        &mut self,
        name: &str,
        super_name: Option<String>,
        list: Vec<(String, PropType, u16)>,
    ) {
        let mut props = Vec::with_capacity(list.len());
        let mut index = 0u16;
        for (n, ty, dim) in list {
            props.push(Prop {
                index,
                array_dim: dim.min(255) as u8,
                name: n,
                ty,
            });
            index += dim.max(1);
        }
        self.usmap.schemas.insert(
            name.to_owned(),
            Schema {
                name: name.to_owned(),
                super_name,
                count: index,
                props,
            },
        );
    }

    /// Mirror of `UECodeGen_Private::ConstructFProperties`: walk the array
    /// backwards; containers consume the params just before them.
    fn read_props(
        &mut self,
        arr: u64,
        n: usize,
        owner: &str,
    ) -> Result<Vec<(String, PropType, u16)>> {
        let ptrs: Vec<u64> = (0..n as u64).map(|k| self.img.q(arr + 8 * k)).collect();
        let mut left = ptrs.len();
        let mut out = Vec::new();
        while left > 0 {
            out.push(self.construct(&ptrs, &mut left, owner)?);
        }
        out.reverse();
        Ok(out)
    }

    fn construct(
        &mut self,
        ptrs: &[u64],
        left: &mut usize,
        owner: &str,
    ) -> Result<(String, PropType, u16)> {
        ensure!(
            *left > 0,
            "{owner}: container property without inner params"
        );
        *left -= 1;
        let p = ptrs[*left];
        let name = self
            .img
            .cstr(self.img.q(p))
            .with_context(|| format!("{owner}: property name at {p:#x}"))?;
        let kind = self.img.byte(p + 24) & 0x3F;
        let dim = self.img.w(p + 48).max(1);
        let func = self.img.q(p + 56);
        use gen::*;
        let ty = match kind {
            BYTE => PropType::Byte {
                enum_name: (func != 0).then(|| self.enum_name(func)).flatten(),
            },
            INT8 => PropType::Int8,
            INT16 => PropType::Int16,
            INT | UNSIZED_INT => PropType::Int,
            INT64 => PropType::Int64,
            UINT16 => PropType::UInt16,
            UINT32 | UNSIZED_UINT => PropType::UInt32,
            UINT64 => PropType::UInt64,
            FLOAT => PropType::Float,
            DOUBLE | LWC_REAL => PropType::Double,
            BOOL => PropType::Bool,
            SOFT_CLASS | SOFT_OBJECT => PropType::SoftObject,
            WEAK_OBJECT => PropType::Weak,
            LAZY_OBJECT => PropType::Lazy,
            CLASS | OBJECT => PropType::Object,
            INTERFACE => PropType::Interface,
            NAME => PropType::Name,
            STR => PropType::Str,
            UTF8_STR => PropType::Utf8Str,
            ANSI_STR => PropType::AnsiStr,
            TEXT => PropType::Text,
            DELEGATE => PropType::Delegate,
            INLINE_MULTICAST | SPARSE_MULTICAST => PropType::MulticastDelegate,
            FIELD_PATH => PropType::FieldPath,
            STRUCT => PropType::Struct(
                self.struct_name(func)
                    .with_context(|| format!("{owner}.{name}: unresolved struct"))?,
            ),
            ARRAY => PropType::Array(Box::new(self.construct(ptrs, left, owner)?.1)),
            SET => PropType::Set(Box::new(self.construct(ptrs, left, owner)?.1)),
            OPTIONAL => PropType::Optional(Box::new(self.construct(ptrs, left, owner)?.1)),
            MAP => {
                let key = self.construct(ptrs, left, owner)?.1;
                let value = self.construct(ptrs, left, owner)?.1;
                PropType::Map(Box::new(key), Box::new(value))
            }
            ENUM => {
                let inner = self.construct(ptrs, left, owner)?.1;
                let name = self.enum_name(func).unwrap_or_default();
                PropType::Enum {
                    inner: Box::new(inner),
                    name,
                }
            }
            VERSE_VALUE => bail!("{owner}.{name}: Verse values are not supported"),
            k => {
                self.unknown_kinds
                    .entry(k)
                    .or_default()
                    .push(format!("{owner}.{name}"));
                bail!("{owner}.{name}: unknown property kind {k:#x}")
            }
        };
        Ok((name, ty, dim))
    }

    /// FStructParams { OuterFunc, SuperFunc, StructOpsFunc, NameUTF8, PropertyArray, u16 NumProperties, … }
    fn struct_name(&mut self, f: u64) -> Option<String> {
        if let Some(v) = self.struct_cache.get(&f) {
            return v.clone();
        }
        self.struct_cache.insert(f, None); // break recursion
        let i = &self.img;
        let t = i.rip_refs(f).into_iter().find(|&t| {
            i.is_data(t)
                && i.is_code(i.q(t))
                && i.cstr(i.q(t + 24)).is_some()
                && (i.q(t + 32) == 0 || i.is_data(i.q(t + 32)))
        })?;
        let name = i.cstr(i.q(t + 24))?;
        let (super_fn, arr, n) = (i.q(t + 8), i.q(t + 32), i.w(t + 40) as usize);
        self.struct_cache.insert(f, Some(name.clone()));
        let super_name = (super_fn != 0)
            .then(|| self.struct_name(super_fn))
            .flatten();
        match self.read_props(arr, n, &name) {
            Ok(props) => self.insert(&name, super_name, props),
            Err(_) => {
                self.usmap.schemas.remove(&name);
            }
        }
        Some(name)
    }

    /// FEnumParams { OuterFunc, DisplayNameFunc, NameUTF8, CppTypeUTF8, Enumerators, u32 ObjectFlags, u16 Num, … }
    fn enum_name(&mut self, f: u64) -> Option<String> {
        if let Some(v) = self.enum_cache.get(&f) {
            return v.clone();
        }
        let i = &self.img;
        let t = i.rip_refs(f).into_iter().find(|&t| {
            i.is_data(t)
                && i.is_code(i.q(t))
                && i.cstr(i.q(t + 16)).is_some()
                && i.cstr(i.q(t + 24)).is_some()
        });
        let name = t.and_then(|t| {
            let name = i.cstr(i.q(t + 16))?;
            let (arr, n) = (i.q(t + 32), i.w(t + 44) as u64);
            let values = (0..n)
                .filter_map(|k| {
                    let e = arr + 16 * k;
                    let full = i.cstr(i.q(e))?;
                    let short = full.rsplit("::").next().unwrap_or(&full).to_owned();
                    Some((i.q(e + 8), short))
                })
                .collect();
            Some((name, values))
        });
        let out = name.map(|(n, values)| {
            self.usmap.enums.insert(n.clone(), values);
            n
        });
        self.enum_cache.insert(f, out.clone());
        out
    }
}
