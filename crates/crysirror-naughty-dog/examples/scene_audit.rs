// SPDX-License-Identifier: MPL-2.0
//! Inventory of one scene: the packages of the given levels and the art
//! groups their spawners place, with each package's item kinds, what the
//! player consumes, decoded mesh size and skinning/animation data.
//!
//! `scene_audit GAME_DIR LEVEL…`
use anyhow::{Context, Result};
use crysirror_capfs::{ambient_authority, fs::Dir};
use crysirror_core::{service::AssetReader, state::ClientId, vfs::VfsProvider};
use crysirror_naughty_dog::{levels, mesh, package::Directory, scene};
use crysirror_resource::{ArchiveMount, DecodeContext, FormatRegistry, read_all};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

struct File(Mutex<crysirror_capfs::fs::File>, u64);
impl AssetReader for File {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.1)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let mut f = self.0.lock().map_err(|_| crysirror_core::Error::Io("lock".into()))?;
        f.seek(SeekFrom::Start(offset)).and_then(|_| f.read(out)).map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}

/// Item kinds the player reads.
const USED: &[&str] = &["PM_COLLECTION_2", "VRAM_DESC", "LIGHT_TABLE_1", "SPAWNER_GROUP", "CAMERA_TABLE_1", "REGION_TREE", "MATERIAL_TABLE_1"];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args.first().context("GAME_DIR")?;
    let disc = Dir::open_ambient_dir(root, ambient_authority())?;
    let main = "PS3_GAME/USRDIR/build/main";
    let open_file = |p: &str| -> Result<Arc<dyn AssetReader>> {
        let f = disc.open(format!("{main}/{p}"))?;
        let len = f.metadata()?.len();
        Ok(Arc::new(File(Mutex::new(f), len)))
    };
    let mut registry = FormatRegistry::default();
    crysirror_formats_arc::register(&mut registry)?;
    let mut context = DecodeContext::new(ClientId(1));
    context.limits.max_entry_size = 256 << 20;
    let mount = |name: &str| -> Result<ArchiveMount> {
        let (_, a) = registry.open(open_file(name)?, Some("sony/psarc"), &context)?;
        Ok(ArchiveMount::new(a, context.clone())?)
    };
    let (paks, actors) = (mount("pak22.psarc")?, mount("actor27.psarc")?);
    let all = levels(std::str::from_utf8(&read_all(open_file("pak22.txt")?.as_ref(), 4 << 20)?)?)?;
    let mut unused: BTreeMap<String, usize> = BTreeMap::new();
    let mut arts: BTreeMap<String, usize> = BTreeMap::new();
    let mut flags: BTreeMap<u32, usize> = BTreeMap::new();
    let mut ptypes: BTreeMap<u32, usize> = BTreeMap::new();
    let mut report = |label: &str, package: &dyn AssetReader| -> Result<(Vec<scene::SpawnerRecord>, BTreeMap<String, usize>, usize, Option<([f32; 3], [f32; 3])>)> {
        let directory = Directory::read(package)?;
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for i in &directory.items {
            *kinds.entry(i.kind.clone()).or_default() += 1;
            if !USED.contains(&i.kind.as_str()) {
                *unused.entry(i.kind.clone()).or_default() += 1;
            }
        }
        let loaded = mesh::Loaded::read(package, 256 << 20)?;
        let mut meshes = scene::meshes_of(&loaded, &directory)?;
        let tris: usize = meshes.iter().flat_map(|m| &m.fragments).map(|f| f.triangles.len()).sum();
        let empty = meshes.iter().flat_map(|m| &m.fragments).filter(|f| f.triangles.is_empty()).count();
        let no_uv = meshes.iter().flat_map(|m| &m.fragments).filter(|f| !f.triangles.is_empty() && f.uvs.is_empty()).count();
        scene::place_art_group(&mut meshes);
        let bounds = scene::placed_bounds(&meshes);
        let mut spawners = Vec::new();
        for item in directory.items.iter().filter(|i| i.kind == "SPAWNER_GROUP") {
            spawners.extend(scene::spawner_records(&loaded, item.payload_offset as usize)?);
        }
        println!("{label}: {tris} tris, {empty} empty fragments, {no_uv} without UVs, kinds {kinds:?}");
        Ok((spawners, kinds, tris, bounds))
    };
    let mut seen = BTreeSet::new();
    for level in &args[1..] {
        for p in all.get(level).map(|l| l.packages.clone()).unwrap_or_default() {
            let exact = format!("{p}.pak");
            let Some(path) = (if paks.exists(&exact) { Some(exact) } else { (0..100).map(|i| format!("{p}{i}.pak")).find(|x| paks.exists(x)) }) else {
                println!("{p}: not installed");
                continue;
            };
            let (spawners, ..) = report(&path, paks.open(&path)?.as_ref())?;
            for s in spawners {
                *flags.entry(s.flags).or_default() += 1;
                *ptypes.entry(s.process_type).or_default() += 1;
                if let Some(a) = s.art_group {
                    *arts.entry(a).or_default() += 1;
                }
            }
        }
    }
    println!("\nart groups: {}", arts.len());
    for (art, n) in &arts {
        if !seen.insert(art.clone()) {
            continue;
        }
        let path = format!("{art}.pak");
        if !actors.exists(&path) {
            println!("  {art} ×{n}: NOT INSTALLED");
            continue;
        }
        let pkg = actors.open(&path)?;
        let (_, kinds, tris, bounds) = report(&format!("  {art} ×{n}"), pkg.as_ref())?;
        let h = bounds.map(|(l, h)| [h[0] - l[0], h[1] - l[1], h[2] - l[2]]);
        println!("    size {h:?} skeleton {} anims {} cloth/havok {}", kinds.contains_key("JOINT_HIERARCHY"), kinds.get("ANIM").copied().unwrap_or(0), kinds.keys().filter(|k| k.contains("HAVOK") || k.contains("CLOTH")).count());
        let _ = tris;
    }
    println!("\nspawner flags {flags:x?}\nprocess types {ptypes:x?}\nitem kinds not read by the player: {unused:?}");
    Ok(())
}
