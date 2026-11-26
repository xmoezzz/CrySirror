// SPDX-License-Identifier: MPL-2.0
//! Objects moving along splines: the game objects' spline trackers
//! (`follow-spline`, object +312: +0xD8 target speed, +0xDC current speed,
//! +0x38 follow distance), train cars (`DcTrain*`, PPU 0x7FB000; speeds
//! in mph × 0.44704) and rail vehicles following a rail mover (`DcRail*`,
//! 0x83E000; the mover manager at 0xFE2CDC, a mover's speed at +0x14, its
//! follow distance +0x1C and followed object +0x20). The host steps them
//! and places the objects; the native vehicle dynamics (careening, ground
//! hugging, physicalised cars) are not modelled.
use std::collections::BTreeMap;

/// `follow-spline` and `train-set-speed-mph` default acceleration when a
/// script sets none (m/s²; not native).
pub const DEFAULT_ACCELERATION: f32 = 4.;
/// A rail vehicle's speed range around its mover: `rail-set-speed`
/// multiplies its fraction by the vehicle's maximum, which `sub_80EBA0`
/// derives from the mover's speed (the settings term +188 is not decoded,
/// so the maximum is the mover's speed here).
pub const RAIL_CATCH_UP: f32 = 1.5;

#[derive(Clone, Debug, PartialEq)]
pub struct Follower {
    pub spline: Option<u32>,
    /// Arc distance along the spline (m).
    pub distance: f32,
    /// Current and target speed (m/s), acceleration (m/s², 0 instant).
    pub speed: f32,
    pub target: f32,
    pub acceleration: f32,
    /// Keep this far behind the followed point: the rail mover
    /// (`rail-set-follow-distance`), the player (`train-track-player`,
    /// `set-spline-follow-distance`) or the car ahead.
    pub gap: f32,
    pub lead: Lead,
    /// Stopped (`rail-deactivate`) without leaving the spline.
    pub active: bool,
    /// Train couplings: the car ahead of this one and the distance kept to
    /// it; cleared by `train-decouple`.
    pub coupled: Option<(u32, f32)>,
    /// `train-allow-unphysicalize`, `rail-override-sfx-state`,
    /// `rail-use-fixed-apref-in-igc`: recorded only.
    pub unphysicalize: bool,
    pub sfx_state: u32,
}

/// What a follower keeps its gap behind.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Lead {
    /// Drives at its own speed.
    #[default]
    None,
    /// A rail mover by index.
    Mover(u32),
    /// The player's projection onto the follower's spline.
    Player,
}

impl Follower {
    pub fn new(spline: Option<u32>, distance: f32) -> Self {
        Self { spline, distance, speed: 0., target: 0., acceleration: DEFAULT_ACCELERATION, gap: 0., lead: Lead::None, active: true, coupled: None, unphysicalize: false, sfx_state: 0 }
    }

    /// Set the target speed; `immediate` also sets the current speed
    /// (`set-spline-follow-speed`'s third argument writes +0xDC and clears
    /// +0xD4; `train-set-speed-mph`'s `sub_7F6DF8` vs `sub_7F6EE8`).
    pub fn set_speed(&mut self, speed: f32, immediate: bool) {
        self.target = speed;
        if immediate {
            self.speed = speed;
        }
    }

    /// Advance by `dt`: toward `chase` (the lead's arc distance less the
    /// gap) at up to [`RAIL_CATCH_UP`] × the target speed when leading
    /// something, else at the target speed reached by acceleration.
    /// Clamped to `0..=length`.
    fn advance(&mut self, dt: f32, length: f32, chase: Option<f32>) {
        if !self.active {
            self.speed = 0.;
            return;
        }
        match chase {
            Some(goal) => {
                let goal = goal - self.gap;
                let limit = (self.target.abs() * RAIL_CATCH_UP).max(1.);
                let step = (goal - self.distance).clamp(-limit * dt, limit * dt);
                self.speed = if dt > 0. { step / dt } else { 0. };
                self.distance += step;
            }
            None => {
                let dv = self.target - self.speed;
                self.speed = if self.acceleration <= 0. { self.target } else { self.speed + dv.clamp(-self.acceleration * dt, self.acceleration * dt) };
                self.distance += self.speed * dt;
            }
        }
        if length > 0. {
            self.distance = self.distance.clamp(0., length);
        }
    }
}

/// A rail mover: a point moving along its spline (default
/// `main-mover-spline-1`) that rail vehicles follow.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mover {
    pub spline: u32,
    pub distance: f32,
    /// m/s (+0x14).
    pub speed: f32,
    /// Stay this far ahead of `target`'s projection (+0x1C, +0x20) when
    /// one is set.
    pub follow: f32,
    pub target: Option<u32>,
    pub active: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Followers {
    pub objects: BTreeMap<u32, Follower>,
    pub movers: BTreeMap<u32, Mover>,
    /// `rail-set-igc-fade-out-time` (seconds), recorded only.
    pub igc_fade_out: f32,
}

/// The world queries stepping needs.
pub trait Track {
    fn length(&self, spline: u32) -> Option<f32>;
    /// The arc distance of the closest point on `spline` to `object`.
    fn project(&self, spline: u32, object: u32) -> Option<f32>;
}

impl Followers {
    pub fn get_or_insert(&mut self, name: u32, at: impl FnOnce() -> Follower) -> &mut Follower {
        self.objects.entry(name).or_insert_with(at)
    }

    /// The car at the front of `car`'s train.
    pub fn head(&self, mut car: u32) -> u32 {
        for _ in 0..64 {
            match self.objects.get(&car).and_then(|f| f.coupled) {
                Some((ahead, _)) => car = ahead,
                None => break,
            }
        }
        car
    }

    /// The cars coupled behind `head` (transitively).
    pub fn train(&self, head: u32) -> Vec<u32> {
        let mut out = vec![head];
        for k in 0..64 {
            let Some(&front) = out.get(k) else { break };
            out.extend(self.objects.iter().filter(|(_, f)| f.coupled.is_some_and(|(a, _)| a == front)).map(|(n, _)| *n));
        }
        out
    }

    /// Step movers, then free followers, then coupled cars at their
    /// distance behind the car ahead. Returns the objects that moved.
    pub fn step(&mut self, dt: f32, world: &dyn Track) -> Vec<u32> {
        let dt = dt.clamp(0., 0.25);
        for m in self.movers.values_mut().filter(|m| m.active) {
            let length = world.length(m.spline).unwrap_or(0.);
            m.distance = match m.target.and_then(|t| world.project(m.spline, t)) {
                Some(p) => p + m.follow,
                None => m.distance + m.speed * dt,
            };
            if length > 0. {
                m.distance = m.distance.clamp(0., length);
            }
        }
        let mut moved = Vec::new();
        let names: Vec<u32> = self.objects.keys().copied().collect();
        for name in &names {
            let f = &self.objects[name];
            let (Some(spline), None) = (f.spline, f.coupled) else { continue };
            let chase = match f.lead {
                Lead::None => None,
                Lead::Mover(m) => self.movers.get(&m).map(|m| m.distance),
                Lead::Player => world.project(spline, PLAYER),
            };
            let length = world.length(spline).unwrap_or(0.);
            let f = self.objects.get_mut(name).expect("follower");
            let before = f.distance;
            f.advance(dt, length, chase);
            if f.distance != before {
                moved.push(*name);
            }
        }
        // Coupled cars, front to back.
        for _ in 0..8 {
            let mut changed = false;
            for name in &names {
                let Some((ahead, spacing)) = self.objects[name].coupled else { continue };
                let Some(front) = self.objects.get(&ahead).cloned() else { continue };
                let f = self.objects.get_mut(name).expect("car");
                let (spline, distance) = (front.spline, front.distance - spacing);
                if f.spline != spline || f.distance != distance {
                    f.spline = spline;
                    f.distance = distance;
                    f.speed = front.speed;
                    if !moved.contains(name) {
                        moved.push(*name);
                    }
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        moved
    }
}

/// The player's object id (`sid("player")`).
pub const PLAYER: u32 = 0xcde4_b742;

#[cfg(test)]
mod tests {
    use super::*;

    struct Line;
    impl Track for Line {
        fn length(&self, _: u32) -> Option<f32> {
            Some(100.)
        }
        fn project(&self, _: u32, object: u32) -> Option<f32> {
            (object == PLAYER).then_some(40.)
        }
    }

    #[test]
    fn player_id() {
        assert_eq!(PLAYER, crate::dc::sid("player"));
    }

    #[test]
    fn accelerates_to_target_and_clamps() {
        let mut fs = Followers::default();
        let f = fs.get_or_insert(1, || Follower::new(Some(9), 0.));
        f.set_speed(10., false);
        fs.step(0.25, &Line);
        assert_eq!(fs.objects[&1].speed, 1.);
        for _ in 0..200 {
            fs.step(0.25, &Line);
        }
        assert_eq!(fs.objects[&1].distance, 100.);
        assert_eq!(fs.objects[&1].speed, 10.);
    }

    #[test]
    fn coupled_cars_keep_spacing_and_decouple() {
        let mut fs = Followers::default();
        fs.get_or_insert(1, || Follower::new(Some(9), 50.)).set_speed(4., true);
        fs.get_or_insert(2, || Follower::new(Some(9), 30.)).coupled = Some((1, 20.));
        fs.get_or_insert(3, || Follower::new(Some(9), 10.)).coupled = Some((2, 20.));
        assert_eq!(fs.train(1), vec![1, 2, 3]);
        assert_eq!(fs.head(3), 1);
        fs.step(0.25, &Line);
        assert_eq!((fs.objects[&2].distance, fs.objects[&3].distance), (31., 11.));
        // Decoupled, car 2 coasts down from 4 m/s (target 0, 4 m/s²) and
        // car 3 stays coupled to it.
        fs.objects.get_mut(&2).unwrap().coupled = None;
        fs.step(0.25, &Line);
        assert_eq!((fs.objects[&2].distance, fs.objects[&3].distance), (31.75, 11.75));
    }

    #[test]
    fn tracks_player_with_gap() {
        let mut fs = Followers::default();
        let f = fs.get_or_insert(1, || Follower::new(Some(9), 0.));
        f.lead = Lead::Player;
        f.gap = -5.;
        f.set_speed(20., false);
        for _ in 0..40 {
            fs.step(0.25, &Line);
        }
        assert_eq!(fs.objects[&1].distance, 45.);
    }
}
