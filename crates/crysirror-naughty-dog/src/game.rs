// SPDX-License-Identifier: MPL-2.0
//! Native script boot and task start.
//!
//! Boot loads `startup.bin` and runs its `top-level` lambda, whose
//! `add-import-module`/`add-load-module` calls name the global modules in
//! order; each is then loaded from `dc1.psarc` (`NAME.bin`). Starting a
//! task subnode selects it in the task graph, loads the node's `ss-…`
//! module and starts a process for every spawner of the loaded level
//! packages that carries a `state-script` property (process name = the
//! spawner name).
use crate::dc::{Dc, TYPE_SCRIPT_LAMBDA, sid};
use crate::scene::SpawnerRecord;
use crate::ss::{Property, Runtime, World};
use crate::task::Graph;
use anyhow::{Context, Result, ensure};
use std::collections::HashMap;

/// Reads a module file (`NAME.bin`) of `dc1.psarc`.
pub trait ModuleSource {
    /// All module names (file stems).
    fn names(&self) -> Vec<String>;
    fn read(&self, name: &str) -> Result<Vec<u8>>;
}

pub struct Game {
    pub rt: Runtime,
    /// Module stem by `StringId`.
    names: HashMap<u32, String>,
    pub loaded: Vec<String>,
}

pub const PROP_STATE_SCRIPT: u32 = sid("state-script");

impl Game {
    pub fn boot(source: &dyn ModuleSource, world: &mut dyn World) -> Result<Self> {
        let names: HashMap<u32, String> = source.names().into_iter().map(|n| (sid(&n), n)).collect();
        let mut game = Self { rt: Runtime::new(Dc::default()), names, loaded: Vec::new() };
        game.load(source, world, "startup")?;
        let top = *game.rt.dc.symbols.get(&sid("top-level")).context("startup has no top-level")?;
        ensure!(game.rt.dc.mem.type_of(top)? == TYPE_SCRIPT_LAMBDA, "startup top-level type");
        game.rt.run_lambda(world, top, &[])?;
        game.load_requested(source, world)?;
        game.rt.inner.tasks = Graph::read(&game.rt.dc)?;
        Ok(game)
    }

    /// Load modules requested by `add-load-module` since the last call.
    pub fn load_requested(&mut self, source: &dyn ModuleSource, world: &mut dyn World) -> Result<()> {
        while !self.rt.inner.module_requests.is_empty() {
            let requests = std::mem::take(&mut self.rt.inner.module_requests);
            for r in requests {
                match self.names.get(&r).cloned() {
                    Some(n) => self.load(source, world, &n)?,
                    None => self.rt.inner.errors.push(format!("module {r:#010x} is not installed")),
                }
            }
        }
        Ok(())
    }

    /// Load module `name` and run its `top-level` lambda when it defines a
    /// new one.
    pub fn load(&mut self, source: &dyn ModuleSource, world: &mut dyn World, name: &str) -> Result<()> {
        if self.loaded.iter().any(|n| n == name) {
            return Ok(());
        }
        let bytes = source.read(name)?;
        let module = self.rt.dc.load(sid(name), &bytes).with_context(|| format!("DC module {name}"))?.clone();
        self.loaded.push(name.to_owned());
        if name != "startup" {
            if let Some((_, top)) = module.entries.iter().find(|(n, _)| *n == sid("top-level")) {
                if self.rt.dc.mem.type_of(*top)? == TYPE_SCRIPT_LAMBDA {
                    self.rt.run_lambda(world, *top, &[])?;
                }
            }
        }
        Ok(())
    }

    /// Select a task subnode (by name) and return `(node, subnode)`.
    pub fn select(&mut self, subnode: &str) -> Result<(usize, usize)> {
        let at = self.rt.inner.tasks.find(sid(subnode)).with_context(|| format!("unknown task {subnode}"))?;
        self.rt.select_task(at)?;
        Ok(at)
    }

    /// The first subnode of the boot sequence: the first node in task order
    /// (`pre-game`), followed by `main-menu`.
    pub fn default_task(&self) -> Option<String> {
        let g = &self.rt.inner.tasks;
        let node = g.order.iter().find_map(|id| g.nodes.iter().position(|n| n.id == *id && !n.subnodes.is_empty()))?;
        Some(g.nodes[node].subnodes[g.first(node)?].name.clone())
    }

    /// The first subnode of the story: the node that follows the menu
    /// nodes (`pre-game`, `main-menu`, `online`) in task order.
    pub fn story_task(&self) -> Option<String> {
        let g = &self.rt.inner.tasks;
        let online = g.order.iter().position(|id| *id == sid("online"))?;
        let next = *g.order.get(online + 1)?;
        let node = g.nodes.iter().position(|n| n.id == next)?;
        Some(g.nodes[node].subnodes[g.first(node)?].name.clone())
    }

    /// Load the current node's script module and start the processes of
    /// `spawners`.
    pub fn start(&mut self, source: &dyn ModuleSource, world: &mut dyn World, spawners: &[SpawnerRecord]) -> Result<usize> {
        let (node, _) = self.rt.inner.task.context("no task selected")?;
        let module = self.rt.inner.tasks.nodes[node].ss_module.clone();
        if !module.is_empty() && self.names.contains_key(&sid(&module)) {
            self.load(source, world, &module)?;
        }
        self.load_requested(source, world)?;
        let mut started = 0;
        for s in spawners {
            let Some((_, Property::Str(script))) = s.properties.iter().find(|(k, _)| *k == PROP_STATE_SCRIPT) else { continue };
            let name = sid(&s.name);
            if self.rt.process_by_name(name).is_some() {
                continue;
            }
            match self.rt.spawn(name, sid(script), Some(name)) {
                Ok(_) => started += 1,
                Err(e) => self.rt.inner.errors.push(format!("{}: {e:#}", s.name)),
            }
        }
        Ok(started)
    }
}

impl ModuleSource for crysirror_resource::ArchiveMount {
    fn names(&self) -> Vec<String> {
        use crysirror_core::vfs::VfsProvider;
        self.list().unwrap_or_default().into_iter().filter_map(|p| p.strip_suffix(".bin").map(str::to_owned)).collect()
    }
    fn read(&self, name: &str) -> Result<Vec<u8>> {
        use crysirror_core::vfs::VfsProvider;
        Ok(crysirror_resource::read_all(self.open(&format!("{name}.bin"))?.as_ref(), 32 << 20)?)
    }
}
