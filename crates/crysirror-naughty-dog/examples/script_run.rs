// SPDX-License-Identifier: MPL-2.0
//! Headless script run on the installed disc: boot the DC modules, start a
//! task subnode with the state-script spawners of its level packages and
//! tick the processes. Reports module loads, processes and their states,
//! script prints, errors and unimplemented natives.
//!
//! `script_run GAME_DIR [TASK|--all] [FRAMES]`
use anyhow::{Context, Result};
use crysirror_capfs::{ambient_authority, fs::Dir};
use crysirror_core::{service::AssetReader, state::ClientId, vfs::VfsProvider};
use crysirror_naughty_dog::{dc::sid, game::{Game, ModuleSource}, levels, mesh, package::Directory, scene, ss::{Property, World}};
use crysirror_resource::{ArchiveMount, DecodeContext, FormatRegistry, read_all};
use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
};

struct File(Mutex<crysirror_capfs::fs::File>, u64);
impl AssetReader for File {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.1)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let mut f = self.0.lock().map_err(|_| crysirror_core::Error::Io("file lock poisoned".into()))?;
        f.seek(SeekFrom::Start(offset)).and_then(|_| f.read(out)).map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
fn file(dir: &Dir, path: &str) -> Result<Arc<dyn AssetReader>> {
    let f = dir.open(path)?;
    let len = f.metadata()?.len();
    Ok(Arc::new(File(Mutex::new(f), len)))
}

struct Modules(ArchiveMount);
impl ModuleSource for Modules {
    fn names(&self) -> Vec<String> {
        self.0.list().unwrap_or_default().into_iter().filter_map(|p| p.strip_suffix(".bin").map(str::to_owned)).collect()
    }
    fn read(&self, name: &str) -> Result<Vec<u8>> {
        Ok(read_all(self.0.open(&format!("{name}.bin"))?.as_ref(), 32 << 20)?)
    }
}

#[derive(Default)]
struct Stub {
    time: f64,
    spawners: HashMap<u32, scene::SpawnerRecord>,
    spawned: std::collections::HashSet<u32>,
    agents: crysirror_naughty_dog::npc::Agents,
}
impl World for Stub {
    fn now(&self) -> f64 {
        self.time
    }
    fn print(&mut self, text: &str) {
        println!("  [print] {text}");
    }
    fn level_spawned(&self, _: u32) -> bool {
        true
    }
    fn object_spawned(&self, name: u32) -> bool {
        self.spawned.contains(&name)
    }
    fn spawn(&mut self, name: u32) -> bool {
        let Some(s) = self.spawners.get(&name) else { return false };
        if s.process_type == sid("Soldier") {
            self.agents.agents.insert(name, crysirror_naughty_dog::npc::Agent::new(s.position, s.rotation));
        }
        self.spawned.insert(name)
    }
    fn kill(&mut self, name: u32) -> bool {
        self.agents.agents.remove(&name);
        self.spawned.remove(&name)
    }
    fn object_type(&self, name: u32) -> Option<u32> {
        self.spawners.get(&name).map(|s| s.process_type)
    }
    fn agents(&mut self) -> Option<&mut crysirror_naughty_dog::npc::Agents> {
        Some(&mut self.agents)
    }
    fn agents_ref(&self) -> Option<&crysirror_naughty_dog::npc::Agents> {
        Some(&self.agents)
    }
    /// No nav meshes are loaded headless: a straight path.
    fn nav_path(&self, from: [f32; 3], to: [f32; 3]) -> Option<Vec<[f32; 3]>> {
        Some(vec![from, to])
    }
    fn locator(&self, name: u32) -> Option<([f32; 3], [f32; 4])> {
        self.spawners.get(&name).map(|s| (s.position, s.rotation))
    }
    fn property(&self, spawner: u32, prop: u32) -> Option<Property> {
        self.spawners.get(&spawner)?.properties.iter().find(|(k, _)| *k == prop).map(|(_, v)| v.clone())
    }
    fn fade(&mut self, color: [f32; 3], alpha: f32, seconds: f32) {
        println!("  [fade] {color:?} {alpha} over {seconds}s");
    }
    fn camera(&mut self, name: u32, _: f32) {
        println!("  [camera] {name:#010x}");
    }
    fn player_controls(&mut self, enabled: bool) {
        println!("  [controls] {enabled}");
    }
    fn sticks(&self, _: u32) -> [f32; 4] {
        [0.; 4]
    }
    fn animate(&mut self, object: u32, clip: u32, options: &crysirror_naughty_dog::ss::Animate) -> bool {
        println!("  [animate] {object:#010x} {clip:#010x} {options:?}");
        true
    }
}

/// Spawner records of a task subnode's levels (subnode level, caches,
/// world): only the `-ingame` packages hold spawners.
fn task_spawners(paks: &ArchiveMount, all: &std::collections::BTreeMap<String, crysirror_naughty_dog::Level>, level: &str, world: &str) -> Result<(usize, Vec<scene::SpawnerRecord>)> {
    let mut packages = Vec::new();
    for l in std::iter::once(level).chain(all.get(level).map(|l| l.caches.iter().map(String::as_str).collect::<Vec<_>>()).unwrap_or_default()).chain(std::iter::once(world)) {
        for p in all.get(l).map(|l| l.packages.as_slice()).unwrap_or_default() {
            if p.ends_with("-ingame") && !packages.contains(p) {
                packages.push(p.clone());
            }
        }
    }
    let mut records = Vec::new();
    for p in &packages {
        let exact = format!("{p}.pak");
        let path = if paks.exists(&exact) { Some(exact) } else { (0..100).map(|i| format!("{p}{i}.pak")).find(|x| paks.exists(x)) };
        let Some(path) = path else { continue };
        let package = paks.open(&path)?;
        let directory = Directory::read(package.as_ref())?;
        let items: Vec<_> = directory.items.iter().filter(|i| i.kind == "SPAWNER_GROUP").collect();
        if items.is_empty() {
            continue;
        }
        let loaded = mesh::Loaded::read(package.as_ref(), 256 << 20)?;
        for item in items {
            records.extend(scene::spawner_records(&loaded, item.payload_offset as usize)?);
        }
    }
    Ok((packages.len(), records))
}

/// `--all`: every task subnode in task order for `FRAMES` frames each;
/// prints per-task process counts and errors and the game-wide missing
/// natives.
fn all_tasks(modules: &Modules, paks: &ArchiveMount, all: &std::collections::BTreeMap<String, crysirror_naughty_dog::Level>, frames: u32) -> Result<()> {
    let mut world = Stub::default();
    let names: Vec<String> = {
        let g = Game::boot(modules, &mut world)?;
        let t = &g.rt.inner.tasks;
        t.order.iter().filter_map(|id| t.node(*id)).flat_map(|n| n.order.iter().filter_map(|s| n.subnodes.iter().find(|x| x.id == *s)).map(|s| s.name.clone())).collect()
    };
    let mut missing: HashMap<&'static str, u32> = HashMap::new();
    let (mut faults, mut total_procs) = (0, 0);
    for task in &names {
        let mut world = Stub::default();
        let mut game = Game::boot(modules, &mut world)?;
        let (node, sub) = game.select(task)?;
        let (level, wl) = { let n = &game.rt.inner.tasks.nodes[node]; (n.subnodes[sub].level.clone(), n.world.clone()) };
        let (packages, records) = task_spawners(paks, all, &level, &wl)?;
        for r in &records {
            world.spawners.insert(sid(&r.name), r.clone());
            if r.flags & 0x4000_0000 == 0 {
                world.spawn(sid(&r.name));
            }
        }
        let started = game.start(modules, &mut world, &records)?;
        let mut fault = None;
        for f in 0..frames {
            world.time = f64::from(f) / 30.;
            world.agents.step(1. / 30.);
            if let Err(e) = game.rt.tick(&mut world).and_then(|_| game.load_requested(modules, &mut world)) {
                fault = Some(format!("{e:#}"));
                break;
            }
        }
        let vm_errors = game.rt.inner.errors.iter().filter(|e| e.contains("not mapped") || e.contains("opcode") || e.contains("depth") || e.contains("instructions")).count();
        faults += usize::from(fault.is_some()) + vm_errors;
        total_procs += started;
        for (n, c) in game.rt.missing() {
            *missing.entry(n).or_default() += c;
        }
        println!("{task:48} level {level:28} ingame {packages:2} procs {started:4} errors {:4} vm {vm_errors:3}{}", game.rt.inner.errors.len(), fault.map(|f| format!(" FAULT {f}")).unwrap_or_default());
    }
    let mut missing: Vec<_> = missing.into_iter().collect();
    missing.sort_by(|a, b| b.1.cmp(&a.1));
    println!("{} tasks, {total_procs} processes, {faults} VM faults/errors; {} unimplemented natives called:", names.len(), missing.len());
    for (n, c) in missing.iter().take(200) {
        println!("  {c:8} {n}");
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args.first().context("GAME_DIR")?;
    let frames: u32 = args.get(2).map(|f| f.parse()).transpose()?.unwrap_or(300);
    let disc = Dir::open_ambient_dir(root, ambient_authority())?;
    let main = "PS3_GAME/USRDIR/build/main";
    let mut registry = FormatRegistry::default();
    crysirror_formats_arc::register(&mut registry)?;
    let mut context = DecodeContext::new(ClientId(1));
    context.limits.max_entry_size = 256 << 20;
    let open = |name: &str| -> Result<ArchiveMount> {
        let (_, a) = registry.open(file(&disc, &format!("{main}/{name}"))?, Some("sony/psarc"), &context)?;
        Ok(ArchiveMount::new(a, context.clone())?)
    };
    let modules = Modules(open("dc1.psarc")?);
    if args.get(1).is_some_and(|a| a == "--all") {
        let manifest = read_all(file(&disc, &format!("{main}/pak22.txt"))?.as_ref(), 4 << 20)?;
        let all = levels(std::str::from_utf8(&manifest)?)?;
        return all_tasks(&modules, &open("pak22.psarc")?, &all, frames);
    }
    let mut world = Stub::default();
    let mut game = Game::boot(&modules, &mut world)?;
    println!("boot: {} modules: {}", game.loaded.len(), game.loaded.join(" "));
    let task = args.get(1).cloned().or_else(|| game.default_task()).context("no task")?;
    let (node, sub) = game.select(&task)?;
    let n = &game.rt.inner.tasks.nodes[node];
    let s = &n.subnodes[sub];
    println!("task {} (node {}, world {}, module {}) level {} continue {:?}", s.name, n.name, n.world, n.ss_module, s.level, s.continue_player);
    // Level packages: the subnode's level, its caches and the world level.
    let manifest = read_all(file(&disc, &format!("{main}/pak22.txt"))?.as_ref(), 4 << 20)?;
    let all = levels(std::str::from_utf8(&manifest)?)?;
    let mut packages = Vec::new();
    for l in std::iter::once(&s.level).chain(all.get(&s.level).map(|l| &l.caches).into_iter().flatten()).chain(std::iter::once(&n.world)) {
        for p in all.get(l).map(|l| l.packages.as_slice()).unwrap_or_default() {
            if !packages.contains(p) {
                packages.push(p.clone());
            }
        }
    }
    let paks = open("pak22.psarc")?;
    let mut records = Vec::new();
    for p in &packages {
        let exact = format!("{p}.pak");
        let path = if paks.exists(&exact) { Some(exact) } else { (0..100).map(|i| format!("{p}{i}.pak")).find(|x| paks.exists(x)) };
        let Some(path) = path else { continue };
        let package = paks.open(&path)?;
        let directory = Directory::read(package.as_ref())?;
        let items: Vec<_> = directory.items.iter().filter(|i| i.kind == "SPAWNER_GROUP").collect();
        if items.is_empty() {
            continue;
        }
        let loaded = mesh::Loaded::read(package.as_ref(), 256 << 20)?;
        for item in items {
            records.extend(scene::spawner_records(&loaded, item.payload_offset as usize)?);
        }
    }
    for r in &records {
        world.spawners.insert(sid(&r.name), r.clone());
        if r.flags & 0x4000_0000 == 0 {
            world.spawned.insert(sid(&r.name));
        }
    }
    let started = game.start(&modules, &mut world, &records)?;
    println!("packages {} spawners {} processes {started}; modules now {}", packages.len(), records.len(), game.loaded.len());
    for f in 0..frames {
        world.time = f64::from(f) / 30.;
        world.agents.step(1. / 30.);
        game.rt.tick(&mut world)?;
        game.load_requested(&modules, &mut world)?;
    }
    let names: HashMap<u32, &str> = records.iter().map(|r| (sid(&r.name), r.name.as_str())).collect();
    let mut states: HashMap<String, usize> = HashMap::new();
    for (i, p) in game.rt.inner.procs.iter().enumerate() {
        let state = game.rt.state_name(i).map(|s| format!("{s:#010x}")).unwrap_or_default();
        *states.entry(format!("{} {state}{}", names.get(&p.name).copied().unwrap_or("?"), if p.dead { " dead" } else { "" })).or_default() += 1;
    }
    let mut states: Vec<_> = states.into_iter().collect();
    states.sort();
    for (s, n) in states.iter().take(40) {
        println!("  proc {s} ×{n}");
    }
    println!("errors {}:", game.rt.inner.errors.len());
    for e in game.rt.inner.errors.iter().take(20) {
        println!("  {e}");
    }
    let mut missing: Vec<_> = game.rt.missing().iter().collect();
    missing.sort_by(|a, b| b.1.cmp(a.1));
    println!("unimplemented natives {} ({} calls):", missing.len(), missing.iter().map(|m| m.1).sum::<u32>());
    for (n, c) in missing.iter().take(60) {
        println!("  {c:6} {n}");
    }
    Ok(())
}
