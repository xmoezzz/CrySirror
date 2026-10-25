// SPDX-License-Identifier: MPL-2.0
//! Cross-package object graph for a cooked IoStore container: package IDs,
//! the container header's imported-package lists, import resolution through
//! public export hashes, and archetype (template) chains so that cooked
//! delta-serialized objects can be read with their inherited values.
use crate::iostore::Container;
use crate::props::{properties, Reader};
use crate::usmap::Usmap;
use crate::zen::{index_kind, parse_package, parse_script_objects, Package};
use anyhow::{ensure, Context, Result};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

pub struct Loaded {
    pub id: u64,
    pub path: String,
    pub bytes: Vec<u8>,
    pub pkg: Package,
}

/// A resolved object reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ObjRef {
    Null,
    /// `/Script/Module.Object` (native class, CDO or script struct).
    Script(String),
    Export { package: u64, index: usize },
    /// Import whose package is not in this container.
    Missing(u64),
}

pub struct Game {
    pub container: Container,
    pub script: HashMap<u64, String>,
    pub schema: Usmap,
    by_id: HashMap<u64, usize>,
    path_of: HashMap<usize, String>,
    imported: HashMap<u64, Vec<u64>>,
    cache: HashMap<u64, Rc<Loaded>>,
    props_cache: HashMap<(u64, usize), Rc<Value>>,
}

/// FIoContainerHeader (version 5): package IDs and imported-package views.
pub fn parse_container_header(b: &[u8]) -> Result<HashMap<u64, Vec<u64>>> {
    let u32at = |o: usize| -> Result<u32> { Ok(u32::from_le_bytes(b.get(o..o + 4).context("container header truncated")?.try_into()?)) };
    let u64at = |o: usize| -> Result<u64> { Ok(u64::from_le_bytes(b.get(o..o + 8).context("container header truncated")?.try_into()?)) };
    ensure!(b.get(..4) == Some(b"nCoI"), "bad container header magic");
    let version = u32at(4)?;
    ensure!(version == 5, "unsupported container header version {version}");
    let n = u32at(16)? as usize;
    let ids = (0..n).map(|i| u64at(20 + 8 * i)).collect::<Result<Vec<_>>>()?;
    let entries = 24 + 8 * n;
    ensure!(u32at(entries - 4)? as usize >= 16 * n, "store entries too small");
    let mut out = HashMap::with_capacity(n);
    for (i, id) in ids.into_iter().enumerate() {
        let e = entries + 16 * i;
        let (count, off) = (u32at(e)? as usize, u32at(e + 4)? as usize);
        let list = (0..count).map(|k| u64at(e + off + 8 * k)).collect::<Result<Vec<_>>>()?;
        out.insert(id, list);
    }
    Ok(out)
}

impl Game {
    /// Open `<game>.utoc` with its sibling `global.utoc`; `schema` is a
    /// `.usmap` or the game's Win64 executable.
    pub fn open(utoc: &Path, schema: &Path) -> Result<Self> {
        let mut container = Container::open(utoc)?;
        let mut global = Container::open(&utoc.with_file_name("global.utoc"))?;
        let so = global.chunks.iter().position(|k| k.kind == 5).context("ScriptObjects chunk")?;
        let script = parse_script_objects(&global.read(so)?)?;
        let hdr = container.chunks.iter().position(|k| k.kind == 6).context("container header chunk")?;
        let imported = parse_container_header(&container.read(hdr)?)?;
        let mut by_id = HashMap::new();
        for (e, k) in container.chunks.iter().enumerate() {
            if k.kind == 1 && k.index == 0 {
                by_id.insert(k.id, e);
            }
        }
        let path_of = container.files.iter().map(|(p, e)| (*e, p.clone())).collect();
        Ok(Self {
            container,
            script,
            schema: Usmap::load(schema)?,
            by_id,
            path_of,
            imported,
            cache: HashMap::new(),
            props_cache: HashMap::new(),
        })
    }

    pub fn package_ids(&self) -> impl Iterator<Item = (u64, &str)> {
        self.by_id.iter().filter_map(|(id, e)| Some((*id, self.path_of.get(e)?.as_str())))
    }

    pub fn package(&mut self, id: u64) -> Result<Rc<Loaded>> {
        if let Some(p) = self.cache.get(&id) {
            return Ok(p.clone());
        }
        let e = *self.by_id.get(&id).with_context(|| format!("package {id:016x} not in container"))?;
        let bytes = self.container.read(e)?;
        let pkg = parse_package(&bytes)?;
        let loaded = Rc::new(Loaded { id, path: self.path_of.get(&e).cloned().unwrap_or_default(), bytes, pkg });
        self.cache.insert(id, loaded.clone());
        Ok(loaded)
    }

    pub fn package_by_path(&mut self, suffix: &str) -> Result<Rc<Loaded>> {
        let id = self
            .package_ids()
            .find(|(_, p)| p.ends_with(suffix))
            .map(|(id, _)| id)
            .with_context(|| format!("no package ending with {suffix}"))?;
        self.package(id)
    }

    /// Resolve an FPackageObjectIndex as stored in a package of `owner`.
    pub fn resolve_object(&mut self, owner: &Loaded, idx: u64) -> Result<ObjRef> {
        Ok(match index_kind(idx) {
            0 => ObjRef::Export { package: owner.id, index: (idx & ((1 << 62) - 1)) as usize },
            1 => ObjRef::Script(self.script.get(&idx).cloned().unwrap_or_else(|| format!("script:{idx:x}"))),
            2 => {
                let raw = idx & ((1 << 62) - 1);
                let (pi, hi) = ((raw >> 32) as usize, (raw & 0xFFFF_FFFF) as usize);
                let pid = *self.imported.get(&owner.id).and_then(|l| l.get(pi)).context("imported package index")?;
                let hash = *owner.pkg.import_hashes.get(hi).context("imported export hash index")?;
                if !self.by_id.contains_key(&pid) {
                    return Ok(ObjRef::Missing(pid));
                }
                let target = self.package(pid)?;
                let index = target.pkg.exports.iter().position(|e| e.public_hash == hash).with_context(|| format!("{}: no export with hash {hash:x}", target.path))?;
                ObjRef::Export { package: pid, index }
            }
            _ => ObjRef::Null,
        })
    }

    /// Resolve an FPackageIndex (as serialized in property data).
    pub fn resolve_index(&mut self, owner: &Loaded, i: i64) -> Result<ObjRef> {
        if i > 0 {
            Ok(ObjRef::Export { package: owner.id, index: (i - 1) as usize })
        } else if i < 0 {
            let idx = *owner.pkg.imports.get((-i - 1) as usize).context("import index out of range")?;
            self.resolve_object(owner, idx)
        } else {
            Ok(ObjRef::Null)
        }
    }

    /// Human-readable `package.ObjectName` for a reference.
    pub fn describe(&mut self, r: &ObjRef) -> Result<String> {
        Ok(match r {
            ObjRef::Null => String::new(),
            ObjRef::Script(s) => s.clone(),
            ObjRef::Missing(id) => format!("<missing {id:016x}>"),
            ObjRef::Export { package, index } => {
                let p = self.package(*package)?;
                format!("{}:{}", p.path, p.pkg.exports.get(*index).map(|e| e.name.as_str()).unwrap_or("?"))
            }
        })
    }

    /// Native class name (without `/Script/Module.`) of an export, following
    /// Blueprint class supers until a native class is reached.
    pub fn native_class(&mut self, package: u64, index: usize) -> Result<String> {
        let p = self.package(package)?;
        let mut r = self.resolve_object(&p, p.pkg.exports[index].class)?;
        for _ in 0..32 {
            match r {
                ObjRef::Script(s) => return Ok(s.rsplit('.').next().unwrap_or(&s).to_owned()),
                ObjRef::Export { package, index } => {
                    let p = self.package(package)?;
                    r = self.resolve_object(&p, p.pkg.exports[index].super_)?;
                }
                _ => break,
            }
        }
        anyhow::bail!("no native class for export {index} of {}", p.path)
    }

    /// Whether the export's class is directly a native class.
    pub fn is_native_instance(&mut self, package: u64, index: usize) -> Result<bool> {
        let p = self.package(package)?;
        Ok(index_kind(p.pkg.exports[index].class) == 1)
    }

    /// The export's own (delta) properties.
    pub fn own_properties(&mut self, package: u64, index: usize) -> Result<Rc<Value>> {
        if let Some(v) = self.props_cache.get(&(package, index)) {
            return Ok(v.clone());
        }
        let p = self.package(package)?;
        anyhow::ensure!(self.is_native_instance(package, index)?, "Blueprint-class instance (schema not recovered)");
        let class = self.native_class(package, index)?;
        let range = p.pkg.export_range(&p.pkg.exports[index]);
        let data = p.bytes.get(range).context("export range")?;
        let mut r = Reader { b: data, p: 0, names: &p.pkg.names };
        let v = Rc::new(properties(&mut r, &self.schema, &class).with_context(|| format!("{}:{}", p.path, p.pkg.exports[index].name))?);
        self.props_cache.insert((package, index), v.clone());
        Ok(v)
    }

    /// Resolve an object value produced by [`Game::properties`] (`{index, package}`).
    pub fn resolve_value(&mut self, v: &Value) -> Result<ObjRef> {
        let (Some(i), Some(pk)) = (v.get("index").and_then(Value::as_i64), v.get("package").and_then(Value::as_str)) else {
            return Ok(ObjRef::Null);
        };
        let owner = self.package(u64::from_str_radix(pk, 16)?)?;
        self.resolve_index(&owner, i)
    }

    /// Properties merged over the archetype chain (template first, own last).
    /// Native CDO templates contribute nothing: their values are code defaults.
    pub fn properties(&mut self, package: u64, index: usize) -> Result<Value> {
        let mut chain = vec![(package, index)];
        let mut cur = (package, index);
        for _ in 0..16 {
            let p = self.package(cur.0)?;
            match self.resolve_object(&p, p.pkg.exports[cur.1].template)? {
                ObjRef::Export { package, index } if (package, index) != cur => {
                    chain.push((package, index));
                    cur = (package, index);
                }
                _ => break,
            }
        }
        let mut out = Map::new();
        for (pk, ix) in chain.into_iter().rev() {
            match self.own_properties(pk, ix) {
                Ok(v) => {
                    if let Value::Object(m) = v.as_ref() {
                        for (k, v) in m {
                            let mut v = v.clone();
                            tag_package(&mut v, pk);
                            out.insert(k.clone(), v);
                        }
                    }
                }
                // Archetypes we cannot decode (Blueprint CDOs) only lose inherited values.
                Err(e) if (pk, ix) != (package, index) => {
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(Value::Object(out))
    }
}

/// Record which package an object index belongs to, since merged archetype
/// values come from different packages.
fn tag_package(v: &mut Value, package: u64) {
    match v {
        Value::Object(m) => {
            if m.len() == 1 && m.get("index").is_some_and(Value::is_i64) {
                m.insert("package".into(), Value::String(format!("{package:016x}")));
            } else {
                m.values_mut().for_each(|x| tag_package(x, package));
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| tag_package(x, package)),
        _ => {}
    }
}
