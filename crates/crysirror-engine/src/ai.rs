// SPDX-License-Identifier: MPL-2.0
//! Navigation and behaviour-tree execution (ADR 0014).
//!
//! Navigation: a 2.5D grid baked from level collision with downward rays
//! (walkable slope, agent headroom, step height), A* over 8-neighbours, then
//! line-of-sight string pulling. It is derived data: rebuilt on demand, never
//! saved. Behaviour trees run reactively (re-evaluated from the root every
//! fixed step); timers and blackboards are saved in the entity's AiState.
use crate::{physics::Physics, EngineContext};
use anyhow::{Context, Result};
use crysirror_scene::{
    ai::{AiState, BbValue, BtNode, BtStatus, Compare},
    world::{EntityHandle, Transform, WorldCommand},
    ServiceBatch,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavSettings {
    pub min: [f32; 3],
    pub max: [f32; 3],
    #[serde(default = "cell")]
    pub cell: f32,
    #[serde(default = "radius")]
    pub agent_radius: f32,
    #[serde(default = "height")]
    pub agent_height: f32,
    #[serde(default = "max_step")]
    pub max_step: f32,
    #[serde(default = "slope")]
    pub max_slope: f32,
}
fn cell() -> f32 {
    0.25
}
fn radius() -> f32 {
    0.3
}
fn height() -> f32 {
    1.6
}
fn max_step() -> f32 {
    0.35
}
fn slope() -> f32 {
    45.
}

pub struct NavGrid {
    pub settings: NavSettings,
    pub w: usize,
    pub h: usize,
    /// Walkable floor height per cell (NaN = blocked).
    pub heights: Vec<f32>,
}
impl NavGrid {
    pub fn bake(physics: &Physics, s: NavSettings) -> Result<Self> {
        anyhow::ensure!(
            s.cell >= 0.05 && s.max[0] > s.min[0] && s.max[2] > s.min[2],
            "invalid navigation bounds"
        );
        let w = ((s.max[0] - s.min[0]) / s.cell).ceil() as usize;
        let h = ((s.max[2] - s.min[2]) / s.cell).ceil() as usize;
        anyhow::ensure!(w * h <= 4_000_000, "navigation grid exceeds 4M cells");
        let cos = s.max_slope.to_radians().cos();
        let top = s.max[1];
        let mut heights = vec![f32::NAN; w * h];
        let mut steep = vec![f32::NAN; w * h];
        for z in 0..h {
            for x in 0..w {
                let p = Vec3::new(
                    s.min[0] + (x as f32 + 0.5) * s.cell,
                    top,
                    s.min[2] + (z as f32 + 0.5) * s.cell,
                );
                let Some(hit) = physics.raycast_static(p, Vec3::NEG_Y, (top - s.min[1]) * 1.01)
                else {
                    continue;
                };
                let steep_hit = hit.normal.y < cos;
                // Headroom: nothing above the floor within the agent height.
                let up = hit.point + Vec3::Y * 0.05;
                if physics
                    .raycast_static(up, Vec3::Y, s.agent_height)
                    .is_some()
                {
                    continue;
                }
                if steep_hit {
                    steep[z * w + x] = hit.point.y;
                } else {
                    heights[z * w + x] = hit.point.y;
                }
            }
        }
        // A steep hit is a small bump (decal, debris, trim) when the flat
        // cells around it are all within one step of it; the agent steps over
        // it. Without this, scattered bumps in imported levels erode most of
        // the floor away.
        for z in 0..h {
            for x in 0..w {
                let y = steep[z * w + x];
                if y.is_nan() {
                    continue;
                }
                let mut flat = 0;
                let mut ok = true;
                for (dx, dz) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                    let (nx, nz) = (x as i64 + dx, z as i64 + dz);
                    if nx < 0 || nz < 0 || nx >= w as i64 || nz >= h as i64 {
                        continue;
                    }
                    let n = heights[nz as usize * w + nx as usize];
                    if n.is_nan() {
                        continue;
                    }
                    flat += 1;
                    ok &= (n - y).abs() <= s.max_step;
                }
                if ok && flat >= 5 {
                    heights[z * w + x] = y;
                }
            }
        }
        // Erode by the agent radius: cells near blocked cells or ledges are unwalkable.
        let r = (s.agent_radius / s.cell).ceil() as isize;
        let base = heights.clone();
        for z in 0..h as isize {
            for x in 0..w as isize {
                let i = z as usize * w + x as usize;
                if base[i].is_nan() {
                    continue;
                }
                'n: for dz in -r..=r {
                    for dx in -r..=r {
                        let (nx, nz) = (x + dx, z + dz);
                        if nx < 0 || nz < 0 || nx >= w as isize || nz >= h as isize {
                            continue;
                        }
                        let nh = base[nz as usize * w + nx as usize];
                        if nh.is_nan() || (nh - base[i]).abs() > s.max_step * 2. {
                            heights[i] = f32::NAN;
                            break 'n;
                        }
                    }
                }
            }
        }
        Ok(Self {
            settings: s,
            w,
            h,
            heights,
        })
    }
    pub fn walkable(&self) -> usize {
        self.heights.iter().filter(|h| !h.is_nan()).count()
    }
    fn cell_of(&self, p: Vec3) -> Option<(usize, usize)> {
        let x = ((p.x - self.settings.min[0]) / self.settings.cell).floor();
        let z = ((p.z - self.settings.min[2]) / self.settings.cell).floor();
        (x >= 0. && z >= 0. && (x as usize) < self.w && (z as usize) < self.h)
            .then(|| (x as usize, z as usize))
    }
    fn nearest_walkable(&self, c: (usize, usize)) -> Option<(usize, usize)> {
        for r in 0..8isize {
            for dz in -r..=r {
                for dx in -r..=r {
                    let (x, z) = (c.0 as isize + dx, c.1 as isize + dz);
                    if x >= 0
                        && z >= 0
                        && (x as usize) < self.w
                        && (z as usize) < self.h
                        && !self.heights[z as usize * self.w + x as usize].is_nan()
                    {
                        return Some((x as usize, z as usize));
                    }
                }
            }
        }
        None
    }
    fn center(&self, c: (usize, usize)) -> Vec3 {
        Vec3::new(
            self.settings.min[0] + (c.0 as f32 + 0.5) * self.settings.cell,
            self.heights[c.1 * self.w + c.0],
            self.settings.min[2] + (c.1 as f32 + 0.5) * self.settings.cell,
        )
    }
    fn passable(&self, a: (usize, usize), b: (usize, usize)) -> bool {
        let (ha, hb) = (
            self.heights[a.1 * self.w + a.0],
            self.heights[b.1 * self.w + b.0],
        );
        !hb.is_nan() && (ha - hb).abs() <= self.settings.max_step
    }
    fn line_clear(&self, a: (usize, usize), b: (usize, usize)) -> bool {
        let (mut x, mut z) = (a.0 as isize, a.1 as isize);
        let (dx, dz) = ((b.0 as isize - x).abs(), -(b.1 as isize - z).abs());
        let (sx, sz) = (
            if (b.0 as isize) > x { 1 } else { -1 },
            if (b.1 as isize) > z { 1 } else { -1 },
        );
        let mut err = dx + dz;
        let mut prev = a;
        loop {
            let cur = (x as usize, z as usize);
            if !self.passable(prev, cur) {
                return false;
            }
            prev = cur;
            if x == b.0 as isize && z == b.1 as isize {
                return true;
            }
            let e2 = 2 * err;
            if e2 >= dz {
                err += dz;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                z += sz;
            }
        }
    }
    /// A* path (smoothed) from `from` to `to`, or None if unreachable.
    pub fn find_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let s = self.nearest_walkable(self.cell_of(from)?)?;
        let g = self.nearest_walkable(self.cell_of(to)?)?;
        let idx = |c: (usize, usize)| c.1 * self.w + c.0;
        let hcost = |c: (usize, usize)| {
            let (dx, dz) = (
                (c.0 as f32 - g.0 as f32).abs(),
                (c.1 as f32 - g.1 as f32).abs(),
            );
            dx.max(dz) + (std::f32::consts::SQRT_2 - 1.) * dx.min(dz)
        };
        #[derive(PartialEq)]
        struct Open(f32, usize);
        impl Eq for Open {}
        impl PartialOrd for Open {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Open {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                o.0.total_cmp(&self.0)
            }
        }
        let mut cost = HashMap::new();
        let mut came = HashMap::new();
        let mut open = BinaryHeap::new();
        cost.insert(idx(s), 0f32);
        open.push(Open(hcost(s), idx(s)));
        let mut expanded = 0;
        while let Some(Open(_, i)) = open.pop() {
            let c = (i % self.w, i / self.w);
            if c == g {
                break;
            }
            expanded += 1;
            if expanded > 500_000 {
                return None;
            }
            for (dx, dz) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
            ] {
                let (nx, nz) = (c.0 as isize + dx, c.1 as isize + dz);
                if nx < 0 || nz < 0 || nx >= self.w as isize || nz >= self.h as isize {
                    continue;
                }
                let n = (nx as usize, nz as usize);
                if !self.passable(c, n)
                    || (dx != 0
                        && dz != 0
                        && (!self.passable(c, (n.0, c.1)) || !self.passable(c, (c.0, n.1))))
                {
                    continue;
                }
                let step = if dx != 0 && dz != 0 {
                    std::f32::consts::SQRT_2
                } else {
                    1.
                };
                let nc = cost[&i] + step;
                if cost.get(&idx(n)).is_none_or(|old| nc < *old) {
                    cost.insert(idx(n), nc);
                    came.insert(idx(n), i);
                    open.push(Open(nc + hcost(n), idx(n)));
                }
            }
        }
        if !cost.contains_key(&idx(g)) {
            return None;
        }
        let mut cells = vec![g];
        let mut cur = idx(g);
        while let Some(p) = came.get(&cur) {
            cells.push((p % self.w, p / self.w));
            cur = *p;
        }
        cells.reverse();
        // String pulling: keep only corners that break line of sight.
        let mut out = vec![cells[0]];
        let mut anchor = 0;
        for i in 1..cells.len() {
            if !self.line_clear(cells[anchor], cells[i]) {
                out.push(cells[i - 1]);
                anchor = i - 1;
            }
        }
        out.push(*cells.last().unwrap());
        out.dedup();
        Some(out.into_iter().map(|c| self.center(c)).collect())
    }
}

/// Non-saved per-entity execution state.
#[derive(Default)]
pub struct AiRuntime {
    pub nav: Option<NavGrid>,
    vertical: HashMap<u32, f32>,
    paths: HashMap<u32, (Vec3, Vec<Vec3>)>,
}

struct Ctx<'a> {
    cx: &'a mut EngineContext,
    id: u32,
    dt: f32,
    state: AiState,
    pos: Vec3,
    yaw: f32,
    moved: bool,
    leaf: u32,
}

fn bb_target(cx: &mut EngineContext, state: &AiState, key: &str) -> Result<Option<Vec3>> {
    Ok(match state.blackboard.get(key) {
        Some(BbValue::Vec3(v)) => Some(Vec3::from_array(*v)),
        Some(BbValue::Number(id)) => {
            let s = cx.state()?;
            let world = &s.runtime.host.engine.world;
            let id = *id as u32;
            world
                .handle(s.client, id)
                .ok()
                .and_then(|h| world.global_matrix(h).ok())
                .map(|m| m.w_axis.truncate())
        }
        _ => None,
    })
}

fn tick(c: &mut Ctx, node: &BtNode, index: &mut u32) -> Result<BtStatus> {
    let me = *index;
    *index += 1;
    let skip = |i: &mut u32, n: &BtNode| *i += n.count() as u32 - 1;
    use BtStatus::*;
    Ok(match node {
        BtNode::Selector { children } | BtNode::Sequence { children } => {
            let sel = matches!(node, BtNode::Selector { .. });
            let mut result = if sel { Failure } else { Success };
            let mut done = false;
            for ch in children {
                if done {
                    *index += ch.count() as u32;
                    continue;
                }
                let r = tick(c, ch, index)?;
                if (sel && r != Failure) || (!sel && r != Success) {
                    result = r;
                    done = true;
                }
            }
            result
        }
        BtNode::Invert { child } => match tick(c, child, index)? {
            Success => Failure,
            Failure => Success,
            Running => Running,
        },
        BtNode::Cooldown { seconds, child } => {
            let left = c.state.memory.get(&me).copied().unwrap_or(0.) - c.dt;
            if left > 0. {
                c.state.memory.insert(me, left);
                skip(index, child);
                *index += 1;
                Failure
            } else {
                c.state.memory.remove(&me);
                let r = tick(c, child, index)?;
                if r == Success {
                    c.state.memory.insert(me, *seconds);
                }
                r
            }
        }
        BtNode::Check { key, op, value } => {
            let v = c.state.blackboard.get(key);
            let num = match v {
                Some(BbValue::Number(n)) => Some(*n),
                Some(BbValue::Bool(b)) => Some(f32::from(u8::from(*b))),
                _ => None,
            };
            let ok = match op {
                Compare::Set => v.is_some(),
                Compare::NotSet => v.is_none(),
                Compare::Greater => num.is_some_and(|n| n > *value),
                Compare::Less => num.is_some_and(|n| n < *value),
                Compare::Equal => num.is_some_and(|n| (n - value).abs() < 1e-5),
                Compare::True => num.is_some_and(|n| n != 0.),
                Compare::False => num.is_none_or(|n| n == 0.),
            };
            if ok {
                Success
            } else {
                Failure
            }
        }
        BtNode::HasTag { tag, target } => {
            let id = match target {
                Some(k) => match c.state.blackboard.get(k) {
                    Some(BbValue::Number(n)) => *n as u32,
                    _ => return Ok(Failure),
                },
                None => c.id,
            };
            let s = c.cx.state()?;
            let world = &s.runtime.host.engine.world;
            let owned = world
                .entities(s.client)
                .into_iter()
                .find(|e| e.0 == id)
                .and_then(|e| {
                    e.2.gameplay
                        .as_ref()
                        .map(|g| g.has_tag(&world.gameplay, tag))
                })
                .unwrap_or(false);
            if owned {
                Success
            } else {
                Failure
            }
        }
        BtNode::Near { target, less_than } => match bb_target(c.cx, &c.state, target)? {
            Some(t) if (t - c.pos).length() < *less_than => Success,
            _ => Failure,
        },
        BtNode::Wait { seconds } => {
            c.leaf = me;
            let t = c.state.memory.get(&me).copied().unwrap_or(0.) + c.dt;
            if t >= *seconds {
                c.state.memory.remove(&me);
                Success
            } else {
                c.state.memory.insert(me, t);
                Running
            }
        }
        BtNode::Face {
            target,
            degrees_per_second,
        } => {
            c.leaf = me;
            let Some(t) = bb_target(c.cx, &c.state, target)? else {
                return Ok(Failure);
            };
            let d = t - c.pos;
            let want = d.x.atan2(d.z).to_degrees();
            let mut delta = (want - c.yaw + 540.).rem_euclid(360.) - 180.;
            let max = degrees_per_second * c.dt;
            if delta.abs() <= max {
                c.yaw = want;
                c.moved = true;
                return Ok(Success);
            }
            delta = delta.signum() * max;
            c.yaw += delta;
            c.moved = true;
            Running
        }
        BtNode::MoveTo {
            target,
            speed,
            acceptance,
        } => {
            c.leaf = me;
            let Some(goal) = bb_target(c.cx, &c.state, target)? else {
                return Ok(Failure);
            };
            let flat = |v: Vec3| Vec3::new(v.x, 0., v.z);
            if flat(goal - c.pos).length() <= *acceptance {
                return Ok(Success);
            }
            let rt = &mut c.cx.state()?.ai;
            let need = match rt.paths.get(&c.id) {
                Some((g, p)) => (*g - goal).length() > 0.5 || p.is_empty(),
                None => true,
            };
            if need {
                let path = match &rt.nav {
                    Some(nav) => nav.find_path(c.pos, goal),
                    None => Some(vec![goal]),
                };
                let Some(mut path) = path else {
                    return Ok(Failure);
                };
                if path.len() > 1 {
                    path.remove(0);
                }
                rt.paths.insert(c.id, (goal, path));
            }
            let (_, path) = rt.paths.get_mut(&c.id).unwrap();
            while path.len() > 1 && flat(path[0] - c.pos).length() < 0.3 {
                path.remove(0);
            }
            let next = path.first().copied().unwrap_or(goal);
            let dir = flat(next - c.pos).normalize_or_zero();
            let vy = rt.vertical.get(&c.id).copied().unwrap_or(0.) - 18. * c.dt;
            let desired = dir * *speed * c.dt + Vec3::Y * vy * c.dt;
            let key = (c.cx.state()?.client.0, c.id);
            let s = c.cx.state()?;
            let mv = s.physics()?.move_character(key, desired, c.dt, 0.35, 50.)?;
            s.ai.vertical
                .insert(c.id, if mv.grounded { -1. } else { vy });
            c.pos += mv.translation;
            if dir != Vec3::ZERO {
                c.yaw = dir.x.atan2(dir.z).to_degrees();
            }
            c.moved = true;
            Running
        }
        BtNode::Ability { name } => {
            c.leaf = me;
            let s = c.cx.state()?;
            let defs = s.runtime.host.engine.world.gameplay.clone();
            let client = s.client;
            let Some((_, _, spec)) = s
                .runtime
                .host
                .engine
                .world
                .entities(client)
                .into_iter()
                .find(|e| e.0 == c.id)
            else {
                return Ok(Failure);
            };
            let mut g = spec.gameplay.clone().unwrap_or_default();
            let ok = g.try_activate(&defs, name)?.is_ok();
            if ok {
                s.runtime.host.engine.submit(
                    client,
                    &ServiceBatch {
                        world: vec![WorldCommand::SetGameplay {
                            id: c.id,
                            gameplay: Some(g),
                        }],
                        ..Default::default()
                    },
                )?;
            }
            if ok {
                Success
            } else {
                Failure
            }
        }
        BtNode::Set { key, value } => {
            c.state.blackboard.insert(key.clone(), value.clone());
            Success
        }
        BtNode::Perceive {
            tag,
            radius,
            into,
            min_interval,
        } => {
            let t = c.state.memory.get(&me).copied().unwrap_or(0.) - c.dt;
            if t > 0. {
                c.state.memory.insert(me, t);
                return Ok(if c.state.blackboard.contains_key(into) {
                    Success
                } else {
                    Failure
                });
            }
            c.state.memory.insert(me, *min_interval);
            let s = c.cx.state()?;
            let world = &s.runtime.host.engine.world;
            let mut best: Option<(f32, u32)> = None;
            for (id, generation, spec) in world.entities(s.client) {
                if id == c.id
                    || !spec
                        .gameplay
                        .as_ref()
                        .is_some_and(|g| g.has_tag(&world.gameplay, tag))
                {
                    continue;
                }
                let p = world
                    .global_matrix(EntityHandle {
                        client: s.client.0,
                        id,
                        generation,
                    })?
                    .w_axis
                    .truncate();
                let d = (p - c.pos).length();
                if d <= *radius && best.is_none_or(|b| d < b.0) {
                    best = Some((d, id));
                }
            }
            match best {
                Some((_, id)) => {
                    c.state
                        .blackboard
                        .insert(into.clone(), BbValue::Number(id as f32));
                    Success
                }
                None => {
                    c.state.blackboard.remove(into);
                    Failure
                }
            }
        }
        BtNode::Succeed => Success,
        BtNode::Fail => Failure,
    })
}

/// Run every AI entity of the App's client for one fixed step.
pub fn step(cx: &mut EngineContext, dt: f32) -> Result<()> {
    let (client, agents, defs) = {
        let s = cx.state()?;
        let w = &s.runtime.host.engine.world;
        let agents: Vec<(u32, u32, AiState, Transform)> = w
            .entities(s.client)
            .into_iter()
            .filter_map(|(id, g, e)| e.ai.clone().map(|a| (id, g, a, e.transform.clone())))
            .collect();
        (s.client, agents, w.ai.clone())
    };
    for (id, generation, state, transform) in agents {
        let Some(tree) = defs.trees.get(&state.tree).cloned() else {
            continue;
        };
        let pos = {
            let s = cx.state()?;
            s.runtime
                .host
                .engine
                .world
                .global_matrix(EntityHandle {
                    client: client.0,
                    id,
                    generation,
                })?
                .w_axis
                .truncate()
        };
        let mut c = Ctx {
            cx,
            id,
            dt,
            state,
            pos,
            yaw: transform.rotation_degrees[1],
            moved: false,
            leaf: 0,
        };
        let mut index = 0;
        let status = tick(&mut c, &tree, &mut index)
            .with_context(|| format!("behaviour tree on entity {id}"))?;
        let (mut state, moved, yaw, leaf, new_pos) = (c.state, c.moved, c.yaw, c.leaf, c.pos);
        state.status = Some(status);
        state.active_leaf = Some(leaf);
        let s = cx.state()?;
        let mut cmds = vec![WorldCommand::SetAi {
            id,
            ai: Some(state),
        }];
        if moved {
            // moveCharacter already updated translation through physics; fetch the latest.
            let cur = s
                .runtime
                .host
                .engine
                .world
                .entity(EntityHandle {
                    client: client.0,
                    id,
                    generation,
                })?
                .transform
                .clone();
            let mut t = cur;
            // Apply the world-space displacement from the character controller.
            let delta = new_pos - pos;
            for k in 0..3 {
                t.translation[k] += delta[k];
            }
            t.rotation_degrees[1] = yaw;
            cmds.push(WorldCommand::SetTransform { id, transform: t });
        }
        s.runtime.host.engine.submit(
            client,
            &ServiceBatch {
                world: cmds,
                ..Default::default()
            },
        )?;
    }
    Ok(())
}
