// SPDX-License-Identifier: MPL-2.0
//! Spline-follow, train and rail-vehicle script natives; see
//! `crate::follow`.
use super::{Ctx, Wait};
use crate::dc::{Dc, sid};
use crate::follow::{Follower, Lead, Mover};
use crate::script::{MAX_ARGS, Status};
use anyhow::Result;

type Reply = Option<(u32, Status)>;

/// mph → m/s (TOC 0x3EE4E26E).
const MPH: f32 = 0.447_04;
/// The process type of train cars (`TrainCar`).
pub const TYPE_TRAIN_CAR: u32 = 0xdfdd_f753;
/// Cars on one spline within this distance of the car ahead are taken as
/// coupled (the native coupling data is not decoded).
const COUPLING_REACH: f32 = 30.;

/// A frame `{position, w 1, rotation}` in script memory.
pub(super) fn alloc_frame(dc: &mut Dc, position: [f32; 3], rotation: [f32; 4]) -> Result<u32> {
    let at = dc.mem.alloc(32)?;
    for (k, v) in position.iter().chain(&[1.]).chain(&rotation).enumerate() {
        dc.mem.write_u32(at + 4 * k as u32, v.to_bits())?;
    }
    Ok(at)
}

impl Ctx<'_> {
    fn rail_object(&self, name: u32) -> u32 {
        let name = if name == sid("self") { self.proc.map_or(name, |p| self.inner.procs[p].name) } else { name };
        self.inner.process(name).and_then(|p| self.inner.procs[p].spawner).unwrap_or(name)
    }

    /// The object's follower, created on the closest spline when it has
    /// none (the spline tracker's own spline assignment is not decoded).
    fn follower(&mut self, name: u32) -> Option<&mut Follower> {
        let known = self.world.followers_ref().is_some_and(|f| f.objects.contains_key(&name));
        if !known {
            let (p, _) = self.world.locator(name)?;
            let on = self.world.closest_spline(p);
            let f = Follower::new(on.map(|(s, _)| s), on.map_or(0., |(_, d)| d));
            self.world.followers()?.objects.insert(name, f);
        }
        self.world.followers()?.objects.get_mut(&name)
    }

    /// Couple the train cars on `head`'s spline behind it in distance
    /// order when the train has no couplings yet.
    fn couple_train(&mut self, head: u32) {
        let Some(spline) = self.follower(head).and_then(|f| f.spline) else { return };
        let has = self.world.followers_ref().is_some_and(|f| f.objects.values().any(|o| o.coupled.is_some_and(|(a, _)| a == head)) || f.objects.get(&head).is_some_and(|o| o.coupled.is_some()));
        if has {
            return;
        }
        let cars: Vec<u32> = self.world.spawned_of_type(TYPE_TRAIN_CAR).into_iter().filter(|c| *c != head).collect();
        let mut behind: Vec<(f32, u32)> = Vec::new();
        for car in cars {
            if let Some(f) = self.follower(car) {
                if f.spline == Some(spline) && f.coupled.is_none() {
                    behind.push((f.distance, car));
                }
            }
        }
        let Some(front) = self.world.followers_ref().and_then(|f| f.objects.get(&head)).map(|f| f.distance) else { return };
        behind.retain(|(d, _)| *d < front);
        behind.sort_by(|a, b| b.0.total_cmp(&a.0));
        let (mut ahead, mut at) = (head, front);
        for (d, car) in behind {
            if at - d > COUPLING_REACH {
                break;
            }
            if let Some(f) = self.world.followers().and_then(|f| f.objects.get_mut(&car)) {
                f.coupled = Some((ahead, at - d));
            }
            (ahead, at) = (car, d);
        }
    }

    fn mover_key(index: u32) -> u32 {
        if index <= 1 { 0 } else { index }
    }

    fn mover(&mut self, index: u32) -> Option<&mut Mover> {
        let key = Self::mover_key(index);
        let followers = self.world.followers()?;
        Some(followers.movers.entry(key).or_insert_with(|| Mover { spline: sid("main-mover-spline-1"), ..Default::default() }))
    }

    pub(super) fn rail_native(&mut self, dc: &mut Dc, name: &str, a: &[u32; MAX_ARGS], n: usize) -> Result<Reply> {
        let ok = |v: u32| Ok(Some((v, Status::Continue)));
        let okf = |v: f32| Ok(Some((v.to_bits(), Status::Continue)));
        let f = |i: usize, default: f32| if n > i { f32::from_bits(a[i]) } else { default };
        let flag = |i: usize| n > i && a[i] != 0;
        let who = if n > 0 { self.rail_object(a[0]) } else { self.rail_object(sid("self")) };
        match name {
            // (set-spline-follow-speed object speed [immediate]): +0xD8,
            // and +0xDC with the third argument (`sub_167094`).
            "set-spline-follow-speed" => {
                let Some(fo) = self.follower(who) else {
                    self.inner.error(format!("set-spline-follow-speed: Unable to find game object '{who:#010x}'"));
                    return ok(0);
                };
                fo.set_speed(f(1, 0.), flag(2));
                ok(1)
            }
            // (follow-spline object [spline]): attach the tracker to the
            // spline from the object's closest point, or detach.
            "follow-spline" => {
                let spline = if n > 1 && a[1] != 0 { Some(a[1]) } else { None };
                if let Some(s) = spline {
                    if self.world.spline_length(s).is_none() {
                        self.inner.error(format!("follow-spline: Unable to find spline '{s:#010x}'"));
                        return ok(0);
                    }
                }
                let at = self.world.locator(who).map(|(p, _)| p);
                let distance = spline.zip(at).and_then(|(s, p)| self.world.spline_distance(s, p));
                let Some(fo) = self.follower(who) else {
                    self.inner.error(format!("follow-spline: Unable to find game object '{who:#010x}'"));
                    return ok(0);
                };
                fo.spline = spline;
                fo.distance = distance.unwrap_or(0.);
                ok(1)
            }
            // (wait-spline-distance object metres): wait until the object is
            // that far along its spline; no wait when already there or not
            // on a spline (`sub_165E9C`).
            "wait-spline-distance" => {
                if n <= 1 {
                    self.inner.error("wait-spline-distance: Expected at least two arguments: (object-name string) (distance-meters float).".into());
                    return ok(0);
                }
                if self.proc.is_none() {
                    self.inner.error("wait-spline-distance: Not called from script context -- ignored.".into());
                    return ok(0);
                }
                let metres = f(1, 0.);
                let waits = self.follower(who).is_some_and(|fo| fo.spline.is_some() && fo.distance < metres);
                if waits {
                    let (_, status) = self.suspend(Wait::Spline { object: who, distance: metres });
                    return Ok(Some((0, status)));
                }
                ok(0)
            }
            // (set-spline-follow-distance object metres): +0x38, then the
            // tracker recomputes (`sub_16842C`, not decoded): taken as
            // keeping that far ahead of the player along the spline.
            "set-spline-follow-distance" => {
                let Some(fo) = self.follower(who) else { return ok(0) };
                fo.gap = -f(1, 0.);
                fo.lead = Lead::Player;
                ok(1)
            }
            // (transfer-to-spline object spline [mode 1|2]): event
            // 0xF5157C98 to the object; continues on the new spline from its
            // closest point.
            "transfer-to-spline" => {
                if n > 1 && a[1] != 0 {
                    let at = self.world.locator(who).map(|(p, _)| p);
                    let d = at.and_then(|p| self.world.spline_distance(a[1], p));
                    if let Some(fo) = self.follower(who) {
                        fo.spline = Some(a[1]);
                        fo.distance = d.unwrap_or(0.);
                    }
                }
                ok(0)
            }
            // (get-spline-length spline) (`sub_343FC`).
            "get-spline-length" => {
                let length = if n > 0 { self.world.spline_length(a[0]) } else { None };
                if length.is_none() {
                    self.inner.error(format!("spline-length?: Unable to find spline '{:#010x}'", if n > 0 { a[0] } else { 0 }));
                }
                okf(length.unwrap_or(0.))
            }

            // Trains.
            // (train-spawn spawner): spawn the car (`sub_7F8E40`).
            "train-spawn" | "rail-spawn" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error(format!("{name}: NULL or empty name"));
                    return ok(0);
                }
                self.world.spawn(a[0]);
                if name == "rail-spawn" {
                    if let Some(fo) = self.follower(a[0]) {
                        fo.lead = Lead::Mover(0);
                    }
                    return ok(1);
                }
                ok(0)
            }
            // (train-spawn-on-track spawner car offset [exclusion-region]
            // [exclusion-region]): spawn at the car's spline distance +
            // offset (`spline-locator-at-distance`). The exclusion regions
            // are not tested.
            "train-spawn-on-track" => {
                if n < 2 || a[0] == 0 || a[1] == 0 {
                    self.inner.error("train-spawn-on-track: Expected (spawner-name) (car-name) (dist-offset)".into());
                    return ok(0);
                }
                let car = self.rail_object(a[1]);
                let Some((Some(spline), d)) = self.follower(car).map(|fo| (fo.spline, fo.distance)) else {
                    self.inner.error(format!("train-spawn-on-track: Cannot find spline for train car '{car:#010x}'"));
                    return ok(0);
                };
                let Some((p, t)) = self.world.spline_at(spline, d + f(2, 0.)) else { return ok(0) };
                let q = glam::Quat::from_rotation_arc(glam::Vec3::Z, glam::Vec3::from_array(t).normalize_or(glam::Vec3::Z));
                self.world.spawn(a[0]);
                self.world.place(a[0], p, q.to_array());
                ok(1)
            }
            // (train-set-speed-mph head-car mph [immediate]).
            "train-set-speed-mph" => {
                if n < 2 {
                    self.inner.error("train-set-speed-mph: Expected two arguments (head-car-name string) (speed-mph float)".into());
                    return ok(0);
                }
                self.couple_train(who);
                let head = self.world.followers_ref().map_or(who, |fs| fs.head(who));
                match self.follower(head) {
                    Some(fo) => fo.set_speed(f(1, 0.) * MPH, flag(2)),
                    None => self.inner.error(format!("train-set-speed-mph: Cannot find train car '{who:#010x}'")),
                }
                ok(0)
            }
            // (train-set-acceleration car a): every car of the train
            // (`sub_7F6D38`, +916).
            "train-set-acceleration" => {
                if n < 2 {
                    self.inner.error("train-set-acceleration: Expected two arguments (head-car-name string) (acceleration float)".into());
                    return ok(0);
                }
                self.couple_train(who);
                if let Some(fs) = self.world.followers() {
                    let head = fs.head(who);
                    for car in fs.train(head) {
                        if let Some(fo) = fs.objects.get_mut(&car) {
                            fo.acceleration = f(1, 0.).max(0.);
                        }
                    }
                }
                ok(0)
            }
            "train-get-acceleration" | "train-get-default-acceleration" => {
                let acc = if name == "train-get-acceleration" { self.follower(who).map_or(0., |fo| fo.acceleration) } else { crate::follow::DEFAULT_ACCELERATION };
                okf(acc)
            }
            "train-get-distance" => okf(self.follower(who).map_or(0., |fo| fo.distance)),
            "train-get-spline" | "rail-get-spline" => ok(self.follower(who).and_then(|fo| fo.spline).unwrap_or(0)),
            "train-get-head-car" => {
                self.follower(who);
                ok(self.world.followers_ref().map_or(who, |fs| fs.head(who)))
            }
            // (train-teleport car spline [distance]).
            "train-teleport" => {
                if n < 2 {
                    self.inner.error("train-teleport: Expected at least two arguments (car-name string) (spline-name string)".into());
                    return ok(0);
                }
                if self.world.spline_length(a[1]).is_none() {
                    self.inner.error(format!("train-teleport: Can't find spline '{:#010x}'", a[1]));
                    return ok(0);
                }
                match self.follower(who) {
                    Some(fo) => {
                        fo.spline = Some(a[1]);
                        fo.distance = f(2, 0.);
                    }
                    None => self.inner.error(format!("train-teleport: Can't find train car '{who:#010x}'")),
                }
                ok(0)
            }
            // (train-decouple car which-coupling): `front` detaches the car
            // from the one ahead, otherwise the cars behind detach.
            "train-decouple" => {
                if n < 2 {
                    self.inner.error("train-decouple: Expected at least two arguments (car-name string) (which-coupling train-coupling)".into());
                    return ok(0);
                }
                self.couple_train(self.world.followers_ref().map_or(who, |fs| fs.head(who)));
                if let Some(fs) = self.world.followers() {
                    let speed = fs.objects.get(&fs.head(who)).map_or(0., |h| h.speed);
                    let detach: Vec<u32> = if a[1] == sid("front") { vec![who] } else { fs.objects.iter().filter(|(_, o)| o.coupled.is_some_and(|(x, _)| x == who)).map(|(k, _)| *k).collect() };
                    for car in detach {
                        if let Some(o) = fs.objects.get_mut(&car) {
                            o.coupled = None;
                            o.speed = speed;
                            o.target = 0.;
                        }
                        for k in fs.train(car).into_iter().skip(1) {
                            if let Some(o) = fs.objects.get_mut(&k) {
                                o.speed = speed;
                            }
                        }
                    }
                }
                ok(0)
            }
            // (train-track-player car track?).
            "train-track-player" => {
                if n < 2 {
                    self.inner.error("train-track-player: Expected two arguments (car-name string) (track boolean)".into());
                    return ok(0);
                }
                if let Some(fo) = self.follower(who) {
                    fo.lead = if flag(1) { Lead::Player } else { Lead::None };
                }
                ok(0)
            }
            "train-allow-unphysicalize" => {
                if let Some(fo) = self.follower(who) {
                    fo.unphysicalize = flag(1);
                }
                ok(0)
            }
            // (event-train-car?): the event's sender is a train car.
            "event-train-car?" => {
                let sender = self.proc.and_then(|p| self.inner.procs[p].groups.iter().rev().find(|g| g.kind == super::KIND_EVENT)).map(|g| g.sender);
                match sender {
                    None => {
                        self.inner.error("event-train-car?: Called from outside event context, ignored".into());
                        ok(0)
                    }
                    Some(None) => {
                        self.inner.error("event-train-car?: event didn't have a sender, ignored".into());
                        ok(0)
                    }
                    Some(Some(s)) => {
                        let s = self.rail_object(s);
                        ok(u32::from(self.world.object_type(s) == Some(TYPE_TRAIN_CAR)))
                    }
                }
            }
            // (player-under-train?): a train car above the player within
            // 2.5 m horizontally and 6 m up (the native ground probe is
            // not decoded).
            "player-under-train?" => {
                let Some((p, _)) = self.world.locator(sid("player")) else { return ok(0) };
                let cars = self.world.spawned_of_type(TYPE_TRAIN_CAR);
                let under = cars.into_iter().filter_map(|c| self.world.locator(c)).any(|(c, _)| {
                    let (dx, dz, dy) = (c[0] - p[0], c[2] - p[2], c[1] - p[1]);
                    dx * dx + dz * dz < 2.5 * 2.5 && (0. ..6.).contains(&dy)
                });
                ok(u32::from(under))
            }

            // Rail vehicles.
            "rail-activate" | "rail-deactivate" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error(format!("{name}: Argument (vehicle-name string) is NULL"));
                    return ok(0);
                }
                let active = name == "rail-activate";
                match self.follower(who) {
                    Some(fo) => fo.active = active,
                    None => self.inner.error(format!("{name}: Vehicle {who:#010x} not found!")),
                }
                ok(1)
            }
            // (rail-set-speed vehicle fraction [immediate]): fraction of the
            // vehicle's maximum (the mover's speed here).
            "rail-set-speed" => {
                if n < 2 {
                    return ok(0);
                }
                let lead = self.follower(who).map(|fo| fo.lead);
                let index = match lead {
                    Some(Lead::Mover(m)) => m,
                    _ => 0,
                };
                let max = self.mover(index).map_or(0., |m| m.speed);
                match self.follower(who) {
                    Some(fo) => fo.set_speed(max * f(1, 0.), flag(2)),
                    None => self.inner.error(format!("rail-set-speed: Unable to find RailVehicle '{who:#010x}'.")),
                }
                ok(0)
            }
            "rail-set-acceleration" => {
                if let Some(fo) = self.follower(who) {
                    fo.acceleration = f(1, 0.).max(0.);
                }
                ok(0)
            }
            // (rail-set-follow-distance vehicle metres …): keep that far
            // behind the mover (`sub_80FC00`); returns a float constant.
            "rail-set-follow-distance" => {
                if n <= 2 {
                    self.inner.error("rail-set-follow-distance: expected (vehicle distance …)".into());
                    return okf(0.);
                }
                if let Some(fo) = self.follower(who) {
                    fo.gap = f(1, 0.);
                    if fo.lead == Lead::None {
                        fo.lead = Lead::Mover(0);
                    }
                }
                okf(0.)
            }
            // (rail-follow-mover vehicle [mover]) (`sub_80B7AC`).
            "rail-follow-mover" => {
                let m = if n > 1 { a[1] } else { 0 };
                if let Some(fo) = self.follower(who) {
                    fo.lead = Lead::Mover(Self::mover_key(m));
                }
                ok(0)
            }
            // (rail-animate-to-mover vehicle …): snap to the mover less the gap.
            "rail-animate-to-mover" => {
                let lead = self.follower(who).map(|fo| (fo.lead, fo.gap));
                if let Some((Lead::Mover(m), gap)) = lead {
                    let at = self.mover(m).map(|m| m.distance);
                    if let (Some(d), Some(fo)) = (at, self.follower(who)) {
                        fo.distance = d - gap;
                    }
                }
                ok(0)
            }
            "rail-has-vehicle?" => ok(u32::from(self.world.followers_ref().is_some_and(|fs| fs.objects.contains_key(&who)))),
            "rail-override-sfx-state" => {
                if let Some(fo) = self.follower(who) {
                    fo.sfx_state = if n > 1 { a[1] } else { 0 };
                }
                ok(0)
            }
            "rail-use-fixed-apref-in-igc" | "rail-enable-truck-jump" => ok(0),
            "rail-set-igc-fade-out-time" => {
                if let Some(fs) = self.world.followers() {
                    fs.igc_fade_out = f(0, 0.);
                }
                ok(0)
            }
            "rail-set-all-vehicles-to-mover-speed" => {
                let speed = self.mover(0).map_or(0., |m| m.speed);
                if let Some(fs) = self.world.followers() {
                    for fo in fs.objects.values_mut().filter(|fo| matches!(fo.lead, Lead::Mover(_))) {
                        fo.set_speed(speed, true);
                    }
                }
                ok(0)
            }
            // (rail-activate-mover index [spline]) (`sub_824E84`).
            "rail-activate-mover" | "rail-deactivate-mover" => {
                let index = if n > 0 { a[0] } else { 0 };
                let spline = if n > 1 && a[1] != 0 { a[1] } else { sid("main-mover-spline-1") };
                let active = name == "rail-activate-mover";
                let known = self.world.spline_length(spline).is_some();
                if active && !known {
                    self.inner.error(format!("rail-activate-mover {index} ('{spline:#010x}') failed - ask a programmer."));
                    return ok(0);
                }
                if let Some(m) = self.mover(index) {
                    if active {
                        m.spline = spline;
                    }
                    m.active = active;
                }
                ok(0)
            }
            // (rail-set-mover-speed m/s [mover]).
            "rail-set-mover-speed" => {
                let index = if n > 1 { a[1] } else { 0 };
                if let Some(m) = self.mover(index) {
                    m.speed = f(0, 0.);
                }
                ok(0)
            }
            // (rail-set-mover-follow-distance metres [mover] [object]):
            // `sub_8212B4` (+0x1C, +0x20).
            "rail-set-mover-follow-distance" => {
                let index = if n > 1 { a[1] } else { 1 };
                let target = if n > 2 && a[2] != 0 { Some(self.rail_object(a[2])) } else { None };
                if let Some(m) = self.mover(index) {
                    m.follow = f(0, 0.);
                    m.target = target;
                }
                ok(0)
            }
            // (rail-set-mover-initial-distance spawner [mover]): the
            // spawner's projection onto the mover's spline (`sub_821860`).
            "rail-set-mover-initial-distance" => {
                if n == 0 || a[0] == 0 {
                    self.inner.error("set-rail-mover-initial-distance: Missing spawner name".into());
                    return ok(0);
                }
                let Some((p, _)) = self.world.locator(a[0]) else {
                    self.inner.error(format!("set-rail-mover-initial-distance: Unknown spawner {:#010x}", a[0]));
                    return ok(0);
                };
                let index = if n > 1 { a[1] } else { 0 };
                let spline = self.mover(index).map(|m| m.spline);
                let d = spline.and_then(|s| self.world.spline_distance(s, p));
                if let (Some(d), Some(m)) = (d, self.mover(index)) {
                    m.distance = d;
                }
                ok(0)
            }
            // (rail-get-mover-locator [mover]).
            "rail-get-mover-locator" => {
                let index = if n > 0 { a[0] } else { 0 };
                let at = self.mover(index).map(|m| (m.spline, m.distance));
                let Some((p, t)) = at.and_then(|(s, d)| self.world.spline_at(s, d)) else { return ok(0) };
                let q = glam::Quat::from_rotation_arc(glam::Vec3::Z, glam::Vec3::from_array(t).normalize_or(glam::Vec3::Z));
                ok(alloc_frame(dc, p, q.to_array())?)
            }
            _ => Ok(None),
        }
    }
}
