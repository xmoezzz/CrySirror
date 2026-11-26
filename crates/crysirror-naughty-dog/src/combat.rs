// SPDX-License-Identifier: MPL-2.0
//! Combat data and rules: weapons, NPC archetypes, damage and hit tests.
//!
//! Data (`dc1.psarc`):
//! - `weapons.bin`: a map of `weapon-def` records. +0x08 automatic, +0x0c
//!   weapon class, +0x14 seconds between shots (assault rifle 0.1, semi
//!   pistol 0.22, shotgun 0.3), +0x18 clip (30, 15, 6), +0xe0 bullet
//!   `StringId`, +0xe4 pellets (shotgun 10), +0xe8 pellet spread degrees.
//! - `ai.bin`: maps of `ai-archetype` records (the spawner tag `archetype`
//!   names one): +0x10 vision range (60 m), +0x14 class name, +0x18 damage
//!   class (`light-class`, `medium-class`, …), +0x3c hit points (100),
//!   +0x50 the `ai-weapon-skill`; `ai-weapon-skill` +0x00 weapon
//!   `StringId`, +0x28/+0x2c the pause between bursts (0.4–0.8 s), +0x30
//!   and +0x34 the burst length in shots.
//! - `damage-settings.bin`: `*weapon-damage-settings*`, a map of weapon →
//!   map of damage class → boxed `int32` hit points per bullet
//!   (`pistol-chloe`: light 50, medium 25, heavy 9).
//! - `dda-settings.bin`: per difficulty a `{low, mid, high}` triple; the
//!   normal record's first triple (2.5, 5, 8) is taken as the hit points an
//!   NPC bullet takes from the player (inferred, SATD-nd-retail-runtime).
use crate::dc::{Dc, sid};
use anyhow::{Result, ensure};
use glam::Vec3;
use std::collections::HashMap;

const TYPE_MAP: u32 = sid_const(b"map");
const TYPE_INT32: u32 = 0xa45e_4e75;

const fn sid_const(s: &[u8]) -> u32 {
    let mut c: u32 = 0;
    let mut i = 0;
    while i < s.len() {
        c ^= (s[i] as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04c1_1db7 } else { c << 1 };
            k += 1;
        }
        i += 1;
    }
    c
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponDef {
    pub automatic: bool,
    pub class: u32,
    pub interval: f32,
    pub clip: u32,
    pub bullet: u32,
    pub pellets: u32,
    pub spread_degrees: f32,
    /// Name `StringId`s (0 when absent): +0x28 the fire sound (`p-fire`),
    /// +0x2C reload, +0x30 empty-click, +0x50 the shell-ejection effect,
    /// +0x54 the shell sound, +0x8C the muzzle-flash effect.
    pub fire_sound: u32,
    pub reload_sound: u32,
    pub empty_sound: u32,
    pub shell_effect: u32,
    pub shell_sound: u32,
    pub muzzle_effect: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Archetype {
    pub vision: f32,
    pub damage_class: u32,
    pub health: u32,
    pub weapon_skill: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeaponSkill {
    pub weapon: u32,
    pub burst_pause: [f32; 2],
    pub burst_shots: [u32; 2],
}

/// Player hit points and the default player loadout.
pub const PLAYER_HEALTH: f32 = 100.;
/// Seconds without damage before the player's health recovers, and the
/// recovery rate (hit points per second). Not decoded from the player
/// settings.
pub const REGEN_DELAY: f32 = 4.;
pub const REGEN_RATE: f32 = 30.;
/// Fallback damage of an unlisted weapon and class.
pub const DEFAULT_DAMAGE: i32 = 25;
/// Capsule of a character for hit tests: radius and height (metres).
pub const CHARACTER_RADIUS: f32 = 0.4;
pub const CHARACTER_HEIGHT: f32 = 1.8;
/// Fraction of the player's head height at which NPCs aim.
const AIM_HEIGHT: f32 = 0.75;

#[derive(Clone, Debug, Default)]
pub struct CombatData {
    pub weapons: HashMap<u32, WeaponDef>,
    pub archetypes: HashMap<u32, Archetype>,
    pub skills: HashMap<u32, WeaponSkill>,
    /// `(weapon, damage class)` → hit points per bullet.
    pub damage: HashMap<(u32, u32), i32>,
    /// NPC bullet damage to the player: low, mid, high.
    pub player_damage: [f32; 3],
}

/// Every map object a module's pointers reach: `(key, value address)`.
fn maps(dc: &Dc, module: u32) -> Result<Vec<Vec<(u32, u32)>>> {
    let Some(m) = dc.module(module) else { return Ok(Vec::new()) };
    let (base, len) = (m.base, m.len);
    let mut out = Vec::new();
    let mut at = base;
    while at + 8 <= base + len {
        // Candidate objects follow their type word.
        if dc.mem.u32(at)? == TYPE_MAP {
            let p = at + 4;
            let (count, pairs) = (dc.mem.u32(p)?, dc.mem.u32(p + 4)?);
            if count > 0 && count < 4096 && pairs >= base && pairs + 8 * count <= base + len {
                out.push((0..count).map(|i| Ok((dc.mem.u32(pairs + 8 * i)?, dc.mem.u32(pairs + 8 * i + 4)?))).collect::<Result<Vec<_>>>()?);
            }
        }
        at += 4;
    }
    Ok(out)
}

impl CombatData {
    /// Read the modules by name (any may be missing).
    pub fn parse(modules: &[(&str, &[u8])]) -> Result<Self> {
        let mut dc = Dc::default();
        for (name, bytes) in modules {
            dc.load(sid(name), bytes)?;
        }
        let mut data = Self { player_damage: [2.5, 5., 8.], ..Default::default() };
        let (def, arch, skill) = (sid("weapon-def"), sid("ai-archetype"), sid("ai-weapon-skill"));
        for module in ["weapons", "ai"] {
            for map in maps(&dc, sid(module))? {
                for (key, v) in map {
                    let Ok(kind) = dc.mem.type_of(v) else { continue };
                    let f = |o: u32| dc.mem.f32(v + o);
                    let w = |o: u32| dc.mem.u32(v + o);
                    if kind == def {
                        let name = |o: u32| -> u32 { w(o).ok().filter(|p| *p != 0).and_then(|p| dc.mem.cstr(p).ok()).filter(|s| !s.is_empty()).map_or(0, |s| sid(&s)) };
                        data.weapons.insert(key, WeaponDef {
                            automatic: w(0x8)? != 0,
                            class: w(0xc)?,
                            interval: f(0x14)?,
                            clip: w(0x18)?,
                            bullet: w(0xe0)?,
                            pellets: w(0xe4)?.max(1),
                            spread_degrees: f(0xe8)?,
                            fire_sound: name(0x28),
                            reload_sound: name(0x2c),
                            empty_sound: name(0x30),
                            shell_effect: name(0x50),
                            shell_sound: name(0x54),
                            muzzle_effect: name(0x8c),
                        });
                    } else if kind == arch {
                        data.archetypes.insert(key, Archetype { vision: f(0x10)?, damage_class: w(0x18)?, health: w(0x3c)?, weapon_skill: w(0x50)? });
                    } else if kind == skill {
                        data.skills.insert(key, WeaponSkill { weapon: w(0)?, burst_pause: [f(0x28)?, f(0x2c)?], burst_shots: [w(0x30)?, w(0x34)?] });
                    }
                }
            }
        }
        if let Some(m) = dc.module(sid("damage-settings")) {
            if let Some(&(_, root)) = m.entries.iter().find(|(n, _)| *n == sid("*weapon-damage-settings*")) {
                let (count, pairs) = (dc.mem.u32(root)?, dc.mem.u32(root + 4)?);
                ensure!(count < 4096, "damage table size");
                for i in 0..count {
                    let (weapon, classes) = (dc.mem.u32(pairs + 8 * i)?, dc.mem.u32(pairs + 8 * i + 4)?);
                    if dc.mem.type_of(classes)? != TYPE_MAP {
                        continue;
                    }
                    let (n, inner) = (dc.mem.u32(classes)?, dc.mem.u32(classes + 4)?);
                    ensure!(n < 4096, "damage class count");
                    for k in 0..n {
                        let (class, value) = (dc.mem.u32(inner + 8 * k)?, dc.mem.u32(inner + 8 * k + 4)?);
                        if dc.mem.type_of(value)? == TYPE_INT32 {
                            data.damage.insert((weapon, class), dc.mem.u32(value)? as i32);
                        }
                    }
                }
            }
        }
        if let Some(m) = dc.module(sid("dda-settings")) {
            if let Some(&(_, normal)) = m.entries.iter().find(|(n, _)| *n == sid("*normal-difficulty-settings*")) {
                let t = [dc.mem.f32(normal)?, dc.mem.f32(normal + 4)?, dc.mem.f32(normal + 8)?];
                if t.iter().all(|v| v.is_finite() && *v >= 0.) {
                    data.player_damage = t;
                }
            }
        }
        Ok(data)
    }

    /// Hit points one bullet of `weapon` takes from damage class `class`.
    pub fn damage(&self, weapon: u32, class: u32) -> i32 {
        self.damage.get(&(weapon, class)).copied().unwrap_or(DEFAULT_DAMAGE)
    }

    /// The weapon an archetype's skill fires.
    pub fn npc_weapon(&self, archetype: u32) -> Option<(&Archetype, Option<&WeaponSkill>, Option<&WeaponDef>)> {
        let a = self.archetypes.get(&archetype)?;
        let skill = self.skills.get(&a.weapon_skill);
        Some((a, skill, skill.and_then(|s| self.weapons.get(&s.weapon))))
    }
}

/// Distance along the ray `origin + t·dir` (unit `dir`) to a vertical
/// capsule standing at `foot`, if hit within `max`.
pub fn ray_capsule(origin: Vec3, dir: Vec3, foot: Vec3, radius: f32, height: f32, max: f32) -> Option<f32> {
    let a = foot + Vec3::Y * radius;
    let v = Vec3::Y * (height - 2. * radius).max(0.);
    let w0 = origin - a;
    let (b, c, d, e) = (dir.dot(v), v.dot(v), dir.dot(w0), v.dot(w0));
    // Closest points of the ray and the axis segment.
    let mut s = if c > 1e-9 { let den = c - b * b; if den.abs() > 1e-9 { ((e - b * d) / den).clamp(0., 1.) } else { 0. } } else { 0. };
    let mut t = (s * b - d).max(0.);
    if c > 1e-9 {
        s = ((t * b + e) / c).clamp(0., 1.);
        t = (s * b - d).max(0.);
    }
    let p = origin + dir * t;
    let q = a + v * s;
    let dist = p.distance(q);
    if dist > radius || t > max {
        return None;
    }
    Some((t - (radius * radius - dist * dist).max(0.).sqrt()).max(0.))
}

/// The chance an NPC shot hits the player: base 0.35 at 10 m, falling
/// to a quarter at the vision range, × the NPC's accuracy multiplier;
/// halved while the player hangs or climbs. Not decoded from the AI
/// shooting curves (`*ai-shooting-dda-settings*`).
pub fn hit_chance(distance: f32, vision: f32, accuracy: f32, exposed: bool) -> f32 {
    let range = vision.max(10.);
    let falloff = 1. - 0.75 * ((distance - 10.) / (range - 10.)).clamp(0., 1.);
    let base = 0.35 * falloff * accuracy.max(0.);
    (if exposed { base } else { base * 0.5 }).clamp(0., 0.95)
}

/// Where NPCs aim at a player standing at `foot`.
pub fn aim_point(foot: Vec3) -> Vec3 {
    foot + Vec3::Y * CHARACTER_HEIGHT * AIM_HEIGHT
}

/// The player's health: damage, recovery and death.
#[derive(Clone, Debug, PartialEq)]
pub struct Health {
    pub value: f32,
    pub since_damage: f32,
    pub dead: bool,
}

impl Default for Health {
    fn default() -> Self {
        Self { value: PLAYER_HEALTH, since_damage: f32::INFINITY, dead: false }
    }
}

impl Health {
    /// Take `amount`; true when this kills.
    pub fn hurt(&mut self, amount: f32, invincible: bool) -> bool {
        if self.dead || invincible || amount <= 0. {
            return false;
        }
        self.value -= amount;
        self.since_damage = 0.;
        if self.value <= 0. {
            self.value = 0.;
            self.dead = true;
            return true;
        }
        false
    }
    pub fn step(&mut self, dt: f32) {
        if self.dead {
            return;
        }
        self.since_damage += dt;
        if self.since_damage >= REGEN_DELAY {
            self.value = (self.value + REGEN_RATE * dt).min(PLAYER_HEALTH);
        }
    }
}

/// An NPC's gun: burst state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gun {
    pub cooldown: f32,
    pub burst_left: u32,
    pub ammo: u32,
    pub reloading: f32,
}

impl Gun {
    /// Advance by `dt`; true when a shot fires now. `rand` draws in [0, 1).
    pub fn step(&mut self, dt: f32, weapon: Option<&WeaponDef>, skill: Option<&WeaponSkill>, rand: &mut dyn FnMut() -> f32) -> bool {
        let interval = weapon.map_or(0.5, |w| w.interval.max(0.05));
        let clip = weapon.map_or(12, |w| w.clip.max(1));
        if self.reloading > 0. {
            self.reloading -= dt;
            if self.reloading <= 0. {
                self.ammo = clip;
            }
            return false;
        }
        self.cooldown -= dt;
        if self.cooldown > 0. {
            return false;
        }
        if self.ammo == 0 {
            // Reload time is not decoded: 2 s.
            self.reloading = 2.;
            return false;
        }
        if self.burst_left == 0 {
            let (lo, hi) = skill.map_or((1, 3), |s| (s.burst_shots[0].max(1), s.burst_shots[1].max(s.burst_shots[0]).max(1)));
            self.burst_left = lo + ((hi - lo + 1) as f32 * rand()) as u32 % (hi - lo + 1);
        }
        self.burst_left -= 1;
        self.ammo -= 1;
        self.cooldown = if self.burst_left == 0 {
            let [a, b] = skill.map_or([0.6, 1.2], |s| s.burst_pause);
            interval + a + (b - a).max(0.) * rand()
        } else {
            interval
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsule_hits_and_misses() {
        let foot = Vec3::new(0., 0., 10.);
        let hit = ray_capsule(Vec3::new(0., 1., 0.), Vec3::Z, foot, 0.4, 1.8, 100.).unwrap();
        assert!((hit - 9.6).abs() < 1e-3, "{hit}");
        assert!(ray_capsule(Vec3::new(1., 1., 0.), Vec3::Z, foot, 0.4, 1.8, 100.).is_none());
        assert!(ray_capsule(Vec3::new(0., 3., 0.), Vec3::Z, foot, 0.4, 1.8, 100.).is_none());
        assert!(ray_capsule(Vec3::new(0., 1., 0.), Vec3::Z, foot, 0.4, 1.8, 5.).is_none());
    }

    #[test]
    fn health_regenerates_after_the_delay() {
        let mut h = Health::default();
        assert!(!h.hurt(30., false));
        h.step(1.);
        assert_eq!(h.value, 70.);
        h.step(REGEN_DELAY);
        assert!(h.value > 70.);
        assert!(!h.hurt(500., true));
        assert!(h.hurt(500., false));
        assert!(h.dead);
    }

    #[test]
    fn gun_fires_bursts_then_reloads() {
        let w = WeaponDef { automatic: true, class: 1, interval: 0.1, clip: 4, bullet: 0, pellets: 1, spread_degrees: 0. };
        let s = WeaponSkill { weapon: 0, burst_pause: [0.5, 0.5], burst_shots: [2, 2] };
        let mut g = Gun { ammo: 4, ..Default::default() };
        let mut r = || 0.;
        let shots: Vec<bool> = (0..40).map(|_| g.step(0.05, Some(&w), Some(&s), &mut r)).collect();
        assert_eq!(shots.iter().filter(|s| **s).count(), 4);
        assert!(g.reloading > 0.);
    }

    #[test]
    fn hit_chance_falls_with_distance() {
        assert!(hit_chance(5., 60., 1., true) > hit_chance(50., 60., 1., true));
        assert_eq!(hit_chance(5., 60., 0., true), 0.);
        assert!(hit_chance(5., 60., 1., false) < hit_chance(5., 60., 1., true));
    }
}
