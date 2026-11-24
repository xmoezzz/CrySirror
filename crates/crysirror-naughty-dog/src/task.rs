// SPDX-License-Identifier: MPL-2.0
//! The game task graph (`tasks.bin` `*task-graph*`, type `game-task-graph`).
//!
//! The graph is `{node map, node edges}`; a map is `{count, [(name, value)]}`
//! sorted by `StringId` and an edge list `{count, [(from, to)]}` uses the
//! pseudo-tasks `%start` and `%end`. A `game-task-node` is `{name, world,
//! ss-module}` (each as string pointer and `StringId`), its subnode map and
//! subnode edges. A `game-task-subnode` holds name, title, level part
//! (`pak22.txt` level), a combined name, checkpoint data and at +0x28 a
//! `continue` record whose first string names the player's continue
//! spawner and +0x38 the companion's. The subnode's +0x38 is a start
//! lambda and +0x40 its task-manager record ([`Manager`]). Completion is derived from the
//! edges: everything ordered before the current subnode (and every node
//! before the current node) is complete. The PPU tracks completion as
//! progress state; starting from a checkpoint equals this ordering.
use crate::dc::{Dc, sid};
use anyhow::{Context, Result, ensure};
use std::collections::{HashMap, HashSet, VecDeque};

/// What a subnode presents (+0x40 record `{?, type, flag, params, name
/// string, name}`).
#[derive(Clone, Debug, PartialEq)]
pub enum Manager {
    None,
    /// `TaskManagerPlayMovie`: `movie1/NAME.m2v`.
    PlayMovie(String),
    /// `TaskManagerMessageDialog`: a text catalog key.
    MessageDialog(String),
    /// `TaskManagerMainMenu`.
    MainMenu,
    Other(u32),
}

#[derive(Clone, Debug)]
pub struct Subnode {
    pub name: String,
    pub id: u32,
    /// `pak22.txt` level of the subnode.
    pub level: String,
    pub continue_player: Option<String>,
    pub continue_buddy: Option<String>,
    pub manager: Manager,
    /// Script lambda run when the subnode starts (+0x38).
    pub on_start: Option<u32>,
    pub address: u32,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub id: u32,
    pub world: String,
    /// The node's state-script module (`ss-…`).
    pub ss_module: String,
    pub subnodes: Vec<Subnode>,
    /// Subnode ids in `%start`-first order.
    pub order: Vec<u32>,
    /// Subnode `(from, to)` edges.
    pub edges: Vec<(u32, u32)>,
    pub address: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    /// Node ids in `%start`-first order.
    pub order: Vec<u32>,
}

fn string(dc: &Dc, ptr_at: u32) -> Result<String> {
    match dc.mem.u32(ptr_at)? {
        0 => Ok(String::new()),
        p => dc.mem.cstr(p),
    }
}

fn map(dc: &Dc, at: u32) -> Result<Vec<(u32, u32)>> {
    let (n, e) = (dc.mem.u32(at)?, dc.mem.u32(at + 4)?);
    ensure!(n < 4096, "task map size");
    (0..n).map(|i| Ok((dc.mem.u32(e + 8 * i)?, dc.mem.u32(e + 8 * i + 4)?))).collect()
}

/// Breadth-first order of an edge list from `%start`, excluding the
/// pseudo-tasks.
fn order(edges: &[(u32, u32)]) -> Vec<u32> {
    let (start, end) = (sid("%start"), sid("%end"));
    let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
    for (a, b) in edges {
        next.entry(*a).or_default().push(*b);
    }
    let (mut out, mut seen, mut queue) = (Vec::new(), HashSet::new(), VecDeque::from([start]));
    while let Some(t) = queue.pop_front() {
        for n in next.get(&t).into_iter().flatten() {
            if seen.insert(*n) {
                if *n != end {
                    out.push(*n);
                }
                queue.push_back(*n);
            }
        }
    }
    out
}

impl Graph {
    pub fn read(dc: &Dc) -> Result<Self> {
        let g = *dc.symbols.get(&sid("*task-graph*")).context("tasks module is not loaded")?;
        ensure!(dc.mem.type_of(g)? == sid("game-task-graph"), "*task-graph* type");
        let mut nodes = Vec::new();
        for (id, n) in map(dc, dc.mem.u32(g)?)? {
            let mut subnodes = Vec::new();
            for (sid_, s) in map(dc, dc.mem.u32(n + 0x18)?)? {
                let cont = dc.mem.u32(s + 0x28)?;
                let opt = |at: u32| -> Result<Option<String>> { Ok(if cont == 0 { None } else { Some(string(dc, cont + at)?).filter(|s| !s.is_empty()) }) };
                let manager = match dc.mem.u32(s + 0x40)? {
                    0 => Manager::None,
                    r => match dc.mem.u32(r + 4)? {
                        t if t == sid("none") => Manager::None,
                        t if t == sid("TaskManagerPlayMovie") => Manager::PlayMovie(string(dc, r + 0x10)?),
                        t if t == sid("TaskManagerMessageDialog") => Manager::MessageDialog(string(dc, r + 0x10)?),
                        t if t == sid("TaskManagerMainMenu") => Manager::MainMenu,
                        t => Manager::Other(t),
                    },
                };
                let on_start = match dc.mem.u32(s + 0x38)? {
                    0 => None,
                    l => (dc.mem.type_of(l)? == crate::dc::TYPE_SCRIPT_LAMBDA).then_some(l),
                };
                subnodes.push(Subnode { name: string(dc, s)?, id: sid_, level: string(dc, s + 0x10)?, continue_player: opt(0)?, continue_buddy: opt(0x38)?, manager, on_start, address: s });
            }
            let edges = map(dc, dc.mem.u32(n + 0x1c)?)?;
            nodes.push(Node { name: string(dc, n)?, id, world: string(dc, n + 8)?, ss_module: string(dc, n + 0x10)?, subnodes, order: order(&edges), edges, address: n });
        }
        let edges = map(dc, dc.mem.u32(g + 4)?)?;
        Ok(Self { nodes, order: order(&edges) })
    }

    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// `(node, subnode)` of a subnode name.
    pub fn find(&self, subnode: u32) -> Option<(usize, usize)> {
        self.nodes.iter().enumerate().find_map(|(i, n)| n.subnodes.iter().position(|s| s.id == subnode).map(|j| (i, j)))
    }

    /// The first subnode of a node in task order.
    pub fn first(&self, node: usize) -> Option<usize> {
        let n = &self.nodes[node];
        n.order.iter().find_map(|id| n.subnodes.iter().position(|s| s.id == *id))
    }

    /// Whether subnode `(node, sub)` is ordered before `current`.
    pub fn before(&self, (node, sub): (usize, usize), current: (usize, usize)) -> bool {
        if node != current.0 {
            let pos = |n: usize| self.order.iter().position(|id| *id == self.nodes[n].id);
            return matches!((pos(node), pos(current.0)), (Some(a), Some(b)) if a < b);
        }
        let n = &self.nodes[node];
        let pos = |s: usize| n.order.iter().position(|id| *id == n.subnodes[s].id);
        matches!((pos(sub), pos(current.1)), (Some(a), Some(b)) if a < b)
    }

    /// The subnode that follows `at`: among its successors (subnodes run
    /// in parallel after a fork, such as weather tasks) the first with a
    /// continue spawner, else the first; at the node's `%end` the first
    /// subnode of the next node.
    pub fn next(&self, (node, sub): (usize, usize)) -> Option<(usize, usize)> {
        let n = &self.nodes[node];
        let id = n.subnodes[sub].id;
        let successors: Vec<usize> = n.edges.iter().filter(|(a, _)| *a == id).filter_map(|(_, b)| n.subnodes.iter().position(|s| s.id == *b)).collect();
        if let Some(j) = successors.iter().find(|j| n.subnodes[**j].continue_player.is_some()).or(successors.first()) {
            return Some((node, *j));
        }
        let at = self.order.iter().position(|x| *x == n.id)?;
        self.order[at + 1..].iter().find_map(|x| {
            let i = self.nodes.iter().position(|y| y.id == *x)?;
            Some((i, self.first(i)?))
        })
    }
}
