// SPDX-License-Identifier: MPL-2.0
//! Gameplay framework (ADR 0014): attributes, gameplay effects, hierarchical
//! tags, abilities and cue events — the data model of a GAS-style system.
//! Definitions live in the world; per-entity state is an entity component.
//! Both are plain saved data and advance on the world's fixed step.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_ACTIVE_EFFECTS: usize = 256;
pub const MAX_EVENTS: usize = 256;

/// `a` matches `query` when equal or a descendant: "State.Stunned.Hard" ⊂ "State".
pub fn tag_matches(tag: &str, query: &str) -> bool {
    tag == query
        || (tag.len() > query.len()
            && tag.starts_with(query)
            && tag.as_bytes()[query.len()] == b'.')
}
fn valid_tag(t: &str) -> bool {
    !t.is_empty()
        && t.len() <= 128
        && t.split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModOp {
    Add,
    Multiply,
    Override,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modifier {
    pub attribute: String,
    pub op: ModOp,
    pub magnitude: f32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectDuration {
    /// Modifies base values once.
    Instant,
    /// Modifies current values while active.
    Duration {
        seconds: f32,
    },
    Infinite,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDef {
    pub duration: EffectDuration,
    /// Periodic effects apply their modifiers to base values every period.
    #[serde(default)]
    pub period: Option<f32>,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    /// Tags owned while the effect is active.
    #[serde(default)]
    pub grant_tags: Vec<String>,
    /// Refuse application when the target owns any of these.
    #[serde(default)]
    pub blocked_by_tags: Vec<String>,
    /// Maximum stacks; reapplying refreshes duration and adds a stack.
    #[serde(default = "one")]
    pub max_stacks: u32,
    /// Cue emitted on apply/remove (e.g. "Cue.Combat.Hit").
    #[serde(default)]
    pub cue: Option<String>,
}
fn one() -> u32 {
    1
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbilityDef {
    /// Instant effect applied on activation; activation fails if it would
    /// drive an attribute below its minimum.
    #[serde(default)]
    pub cost: Option<String>,
    /// Duration effect applied on activation; its grant_tags block reuse.
    #[serde(default)]
    pub cooldown: Option<String>,
    #[serde(default)]
    pub required_tags: Vec<String>,
    #[serde(default)]
    pub blocked_tags: Vec<String>,
    /// Owned while the ability is active (e.g. "State.Attacking").
    #[serde(default)]
    pub active_tags: Vec<String>,
    /// Activating this ability cancels active abilities owning these tags.
    #[serde(default)]
    pub cancel_abilities_with_tags: Vec<String>,
    /// Automatically end after this many seconds (None = until ended).
    #[serde(default)]
    pub duration: Option<f32>,
    #[serde(default)]
    pub cue: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameplayDefs {
    #[serde(default)]
    pub effects: BTreeMap<String, EffectDef>,
    #[serde(default)]
    pub abilities: BTreeMap<String, AbilityDef>,
}
impl GameplayDefs {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.effects.len() <= 4096 && self.abilities.len() <= 4096,
            "gameplay definition limit"
        );
        for (n, e) in &self.effects {
            ensure!(valid_tag(n), "effect name '{n}'");
            let finite = |v: f32| v.is_finite() && v.abs() <= 1e9;
            if let EffectDuration::Duration { seconds } = e.duration {
                ensure!(finite(seconds) && seconds > 0., "effect '{n}' duration");
            }
            if let Some(p) = e.period {
                ensure!(
                    finite(p) && p >= 0.01 && e.duration != EffectDuration::Instant,
                    "effect '{n}' period"
                );
            }
            ensure!(
                e.modifiers
                    .iter()
                    .all(|m| valid_tag(&m.attribute) && finite(m.magnitude)),
                "effect '{n}' modifiers"
            );
            ensure!(
                e.grant_tags
                    .iter()
                    .chain(&e.blocked_by_tags)
                    .chain(&e.cue)
                    .all(|t| valid_tag(t)),
                "effect '{n}' tags"
            );
            ensure!((1..=1000).contains(&e.max_stacks), "effect '{n}' stacks");
        }
        for (n, a) in &self.abilities {
            ensure!(valid_tag(n), "ability name '{n}'");
            for r in a.cost.iter().chain(&a.cooldown) {
                ensure!(
                    self.effects.contains_key(r),
                    "ability '{n}' references missing effect '{r}'"
                );
            }
            if let Some(c) = &a.cost {
                ensure!(
                    self.effects[c].duration == EffectDuration::Instant,
                    "ability '{n}' cost must be instant"
                );
            }
            ensure!(
                a.required_tags
                    .iter()
                    .chain(&a.blocked_tags)
                    .chain(&a.active_tags)
                    .chain(&a.cancel_abilities_with_tags)
                    .chain(&a.cue)
                    .all(|t| valid_tag(t)),
                "ability '{n}' tags"
            );
            if let Some(d) = a.duration {
                ensure!(d.is_finite() && d > 0., "ability '{n}' duration");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attribute {
    pub base: f32,
    #[serde(default)]
    pub min: Option<f32>,
    #[serde(default)]
    pub max: Option<f32>,
    /// Clamp to another attribute's current value (e.g. Health ≤ MaxHealth).
    #[serde(default)]
    pub max_attribute: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveEffect {
    pub effect: String,
    pub remaining: Option<f32>,
    pub period_left: Option<f32>,
    pub stacks: u32,
    /// Entity id of the instigator, if any.
    pub source: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameplayEvent {
    /// "cue", "effect_applied", "effect_removed", "ability_activated",
    /// "ability_ended", "attribute_depleted".
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub value: f32,
    #[serde(default)]
    pub source: Option<u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameplayState {
    #[serde(default)]
    pub attributes: BTreeMap<String, Attribute>,
    #[serde(default)]
    pub effects: Vec<ActiveEffect>,
    /// Loose tags with reference counts (added by scripts).
    #[serde(default)]
    pub loose_tags: BTreeMap<String, u32>,
    #[serde(default)]
    pub granted: Vec<String>,
    /// Active ability → seconds since activation.
    #[serde(default)]
    pub active: BTreeMap<String, f32>,
    #[serde(default)]
    pub events: Vec<GameplayEvent>,
}

impl GameplayState {
    fn push(&mut self, kind: &str, name: &str, value: f32, source: Option<u32>) {
        if self.events.len() >= MAX_EVENTS {
            self.events.remove(0);
        }
        self.events.push(GameplayEvent {
            kind: kind.into(),
            name: name.into(),
            value,
            source,
        });
    }
    /// All tags owned: loose, effect-granted and active-ability tags.
    pub fn tags(&self, defs: &GameplayDefs) -> Vec<String> {
        let mut t: Vec<String> = self.loose_tags.keys().cloned().collect();
        for e in &self.effects {
            if let Some(d) = defs.effects.get(&e.effect) {
                t.extend(d.grant_tags.iter().cloned());
            }
        }
        for a in self.active.keys() {
            if let Some(d) = defs.abilities.get(a) {
                t.extend(d.active_tags.iter().cloned());
            }
        }
        t.sort();
        t.dedup();
        t
    }
    pub fn has_tag(&self, defs: &GameplayDefs, query: &str) -> bool {
        self.tags(defs).iter().any(|t| tag_matches(t, query))
    }
    fn has_any(&self, defs: &GameplayDefs, queries: &[String]) -> bool {
        let tags = self.tags(defs);
        queries
            .iter()
            .any(|q| tags.iter().any(|t| tag_matches(t, q)))
    }
    /// Current value: (base + Σadd) · Πmul, overrides last; then clamped.
    pub fn value(&self, defs: &GameplayDefs, name: &str) -> Option<f32> {
        self.value_depth(defs, name, 0)
    }
    fn value_depth(&self, defs: &GameplayDefs, name: &str, depth: u32) -> Option<f32> {
        let a = self.attributes.get(name)?;
        let (mut add, mut mul, mut ovr) = (0f32, 1f32, None);
        for e in &self.effects {
            let Some(d) = defs.effects.get(&e.effect) else {
                continue;
            };
            if d.duration == EffectDuration::Instant || d.period.is_some() {
                continue;
            }
            for m in d.modifiers.iter().filter(|m| m.attribute == name) {
                match m.op {
                    ModOp::Add => add += m.magnitude * e.stacks as f32,
                    ModOp::Multiply => mul *= m.magnitude.powi(e.stacks as i32),
                    ModOp::Override => ovr = Some(m.magnitude),
                }
            }
        }
        let v = ovr.unwrap_or((a.base + add) * mul);
        Some(self.clamp(defs, a, v, depth))
    }
    fn clamp(&self, defs: &GameplayDefs, a: &Attribute, mut v: f32, depth: u32) -> f32 {
        if let Some(min) = a.min {
            v = v.max(min);
        }
        if let Some(max) = a.max {
            v = v.min(max);
        }
        if let (Some(m), true) = (&a.max_attribute, depth < 4) {
            if let Some(mv) = self.value_depth(defs, m, depth + 1) {
                v = v.min(mv);
            }
        }
        v
    }
    fn modify_base(&mut self, defs: &GameplayDefs, m: &Modifier, stacks: u32, source: Option<u32>) {
        let Some(a) = self.attributes.get(&m.attribute).cloned() else {
            return;
        };
        let raw = match m.op {
            ModOp::Add => a.base + m.magnitude * stacks as f32,
            ModOp::Multiply => a.base * m.magnitude.powi(stacks as i32),
            ModOp::Override => m.magnitude,
        };
        let before = a.base;
        let new = self.clamp(defs, &a, raw, 0);
        self.attributes.get_mut(&m.attribute).unwrap().base = new;
        if new <= a.min.unwrap_or(f32::MIN) && before > new {
            self.push("attribute_depleted", &m.attribute, new, source);
        }
    }

    pub fn apply_effect(
        &mut self,
        defs: &GameplayDefs,
        name: &str,
        source: Option<u32>,
    ) -> Result<bool> {
        let d = defs
            .effects
            .get(name)
            .with_context(|| format!("unknown effect '{name}'"))?
            .clone();
        if self.has_any(defs, &d.blocked_by_tags) {
            return Ok(false);
        }
        if let Some(c) = &d.cue {
            self.push("cue", c, 1., source);
        }
        match d.duration {
            EffectDuration::Instant => {
                for m in &d.modifiers {
                    self.modify_base(defs, m, 1, source);
                }
            }
            _ => {
                let remaining = match d.duration {
                    EffectDuration::Duration { seconds } => Some(seconds),
                    _ => None,
                };
                if let Some(e) = self.effects.iter_mut().find(|e| e.effect == name) {
                    e.stacks = (e.stacks + 1).min(d.max_stacks);
                    e.remaining = remaining;
                    e.source = source;
                } else {
                    ensure!(
                        self.effects.len() < MAX_ACTIVE_EFFECTS,
                        "active effect limit"
                    );
                    self.effects.push(ActiveEffect {
                        effect: name.into(),
                        remaining,
                        period_left: d.period,
                        stacks: 1,
                        source,
                    });
                }
            }
        }
        self.push("effect_applied", name, 1., source);
        Ok(true)
    }
    pub fn remove_effect(&mut self, defs: &GameplayDefs, name: &str) -> bool {
        let before = self.effects.len();
        self.effects.retain(|e| e.effect != name);
        let removed = self.effects.len() != before;
        if removed {
            if let Some(c) = defs.effects.get(name).and_then(|d| d.cue.clone()) {
                self.push("cue", &c, 0., None);
            }
            self.push("effect_removed", name, 0., None);
        }
        removed
    }

    /// Activation checks in GAS order: granted, not active, tags, cooldown, cost.
    pub fn try_activate(
        &mut self,
        defs: &GameplayDefs,
        name: &str,
    ) -> Result<std::result::Result<(), String>> {
        let a = defs
            .abilities
            .get(name)
            .with_context(|| format!("unknown ability '{name}'"))?
            .clone();
        if !self.granted.iter().any(|g| g == name) {
            return Ok(Err("not granted".into()));
        }
        if self.active.contains_key(name) {
            return Ok(Err("already active".into()));
        }
        let tags = self.tags(defs);
        if let Some(r) = a
            .required_tags
            .iter()
            .find(|q| !tags.iter().any(|t| tag_matches(t, q)))
        {
            return Ok(Err(format!("missing tag {r}")));
        }
        if let Some(b) = a
            .blocked_tags
            .iter()
            .find(|q| tags.iter().any(|t| tag_matches(t, q)))
        {
            return Ok(Err(format!("blocked by tag {b}")));
        }
        if let Some(cd) = &a.cooldown {
            if self.effects.iter().any(|e| &e.effect == cd) {
                return Ok(Err("on cooldown".into()));
            }
        }
        if let Some(cost) = &a.cost {
            for m in &defs.effects[cost].modifiers {
                let Some(attr) = self.attributes.get(&m.attribute) else {
                    return Ok(Err(format!("missing attribute {}", m.attribute)));
                };
                if m.op == ModOp::Add && attr.base + m.magnitude < attr.min.unwrap_or(0.) {
                    return Ok(Err(format!("cannot afford {}", m.attribute)));
                }
            }
        }
        // Cancel abilities owning any listed tag.
        let cancel: Vec<String> = self
            .active
            .keys()
            .filter(|n| {
                defs.abilities.get(*n).is_some_and(|d| {
                    d.active_tags.iter().any(|t| {
                        a.cancel_abilities_with_tags
                            .iter()
                            .any(|q| tag_matches(t, q))
                    })
                })
            })
            .cloned()
            .collect();
        for c in cancel {
            self.end_ability(defs, &c);
        }
        if let Some(cost) = &a.cost {
            self.apply_effect(defs, cost, None)?;
        }
        if let Some(cd) = &a.cooldown {
            self.apply_effect(defs, cd, None)?;
        }
        self.active.insert(name.into(), 0.);
        if let Some(c) = &a.cue {
            self.push("cue", c, 1., None);
        }
        self.push("ability_activated", name, 0., None);
        Ok(Ok(()))
    }
    pub fn end_ability(&mut self, defs: &GameplayDefs, name: &str) -> bool {
        if self.active.remove(name).is_none() {
            return false;
        }
        if let Some(c) = defs.abilities.get(name).and_then(|a| a.cue.clone()) {
            self.push("cue", &c, 0., None);
        }
        self.push("ability_ended", name, 0., None);
        true
    }

    /// Advance durations, periods and timed abilities by `dt` seconds.
    pub fn tick(&mut self, defs: &GameplayDefs, dt: f32) {
        let mut periodic = Vec::new();
        let mut expired = Vec::new();
        for e in &mut self.effects {
            if let Some(r) = &mut e.remaining {
                *r -= dt;
                if *r <= 0. {
                    expired.push(e.effect.clone());
                }
            }
            if let (Some(p), Some(d)) = (&mut e.period_left, defs.effects.get(&e.effect)) {
                *p -= dt;
                while *p <= 0. {
                    *p += d.period.unwrap_or(1.);
                    periodic.push((e.effect.clone(), e.stacks, e.source));
                }
            }
        }
        for (name, stacks, source) in periodic {
            if let Some(d) = defs.effects.get(&name).cloned() {
                for m in &d.modifiers {
                    self.modify_base(defs, m, stacks, source);
                }
            }
        }
        for name in expired {
            self.remove_effect(defs, &name);
        }
        let mut ended = Vec::new();
        for (name, t) in &mut self.active {
            *t += dt;
            if defs
                .abilities
                .get(name)
                .and_then(|a| a.duration)
                .is_some_and(|d| *t >= d)
            {
                ended.push(name.clone());
            }
        }
        for n in ended {
            self.end_ability(defs, &n);
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.attributes.len() <= 256
                && self.effects.len() <= MAX_ACTIVE_EFFECTS
                && self.events.len() <= MAX_EVENTS,
            "gameplay state limits"
        );
        for (n, a) in &self.attributes {
            ensure!(valid_tag(n) && a.base.is_finite(), "attribute '{n}'");
        }
        for t in self.loose_tags.keys() {
            if !valid_tag(t) {
                bail!("invalid tag '{t}'");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn defs() -> GameplayDefs {
        serde_json::from_value(serde_json::json!({
            "effects": {
                "Damage": {"duration": {"kind": "instant"}, "modifiers": [{"attribute": "Health", "op": "add", "magnitude": -30}], "cue": "Cue.Hit"},
                "Poison": {"duration": {"kind": "duration", "seconds": 3}, "period": 1, "modifiers": [{"attribute": "Health", "op": "add", "magnitude": -5}], "grant_tags": ["State.Poisoned"]},
                "Fortify": {"duration": {"kind": "infinite"}, "modifiers": [{"attribute": "MaxHealth", "op": "multiply", "magnitude": 1.5}], "max_stacks": 2},
                "StaminaCost": {"duration": {"kind": "instant"}, "modifiers": [{"attribute": "Stamina", "op": "add", "magnitude": -40}]},
                "AttackCooldown": {"duration": {"kind": "duration", "seconds": 0.5}, "grant_tags": ["Cooldown.Attack"]},
                "Stun": {"duration": {"kind": "duration", "seconds": 1}, "grant_tags": ["State.Stunned"]}
            },
            "abilities": {
                "Attack": {"cost": "StaminaCost", "cooldown": "AttackCooldown", "blocked_tags": ["State.Stunned"], "active_tags": ["State.Attacking"], "duration": 0.4, "cue": "Cue.Swing"},
                "Parry": {"active_tags": ["State.Parrying"], "cancel_abilities_with_tags": ["State.Attacking"], "duration": 0.2}
            }
        })).unwrap()
    }
    fn knight() -> GameplayState {
        let mut s = GameplayState::default();
        for (n, base) in [("Health", 100.), ("MaxHealth", 100.), ("Stamina", 100.)] {
            s.attributes.insert(
                n.into(),
                Attribute {
                    base,
                    min: Some(0.),
                    max: None,
                    max_attribute: (n == "Health").then(|| "MaxHealth".to_owned()),
                },
            );
        }
        s.granted = vec!["Attack".into(), "Parry".into()];
        s
    }
    #[test]
    fn effects_modify_attributes_with_stacking_periods_and_clamps() {
        let d = defs();
        d.validate().unwrap();
        let mut s = knight();
        s.apply_effect(&d, "Damage", Some(7)).unwrap();
        assert_eq!(s.value(&d, "Health"), Some(70.));
        assert!(s
            .events
            .iter()
            .any(|e| e.kind == "cue" && e.name == "Cue.Hit" && e.source == Some(7)));
        s.apply_effect(&d, "Fortify", None).unwrap();
        s.apply_effect(&d, "Fortify", None).unwrap();
        s.apply_effect(&d, "Fortify", None).unwrap(); // capped at 2 stacks
        assert_eq!(s.value(&d, "MaxHealth"), Some(225.));
        s.apply_effect(&d, "Poison", None).unwrap();
        assert!(s.has_tag(&d, "State"));
        s.tick(&d, 3.0);
        assert_eq!(s.value(&d, "Health"), Some(55.));
        assert!(!s.has_tag(&d, "State.Poisoned"), "expired");
        for _ in 0..5 {
            s.apply_effect(&d, "Damage", None).unwrap();
        }
        assert_eq!(s.value(&d, "Health"), Some(0.));
        assert!(s
            .events
            .iter()
            .any(|e| e.kind == "attribute_depleted" && e.name == "Health"));
    }
    #[test]
    fn abilities_check_tags_cost_cooldown_and_cancel() {
        let d = defs();
        let mut s = knight();
        assert_eq!(s.try_activate(&d, "Attack").unwrap(), Ok(()));
        assert_eq!(s.value(&d, "Stamina"), Some(60.));
        assert!(s.has_tag(&d, "State.Attacking"));
        assert_eq!(
            s.try_activate(&d, "Attack").unwrap(),
            Err("already active".into())
        );
        s.tick(&d, 0.45); // ability ends, cooldown (0.5) still running
        assert!(!s.active.contains_key("Attack"));
        assert_eq!(
            s.try_activate(&d, "Attack").unwrap(),
            Err("on cooldown".into())
        );
        s.tick(&d, 0.1);
        assert_eq!(s.try_activate(&d, "Attack").unwrap(), Ok(()));
        // Parry cancels the attack.
        assert_eq!(s.try_activate(&d, "Parry").unwrap(), Ok(()));
        assert!(!s.active.contains_key("Attack") && s.active.contains_key("Parry"));
        s.tick(&d, 0.6);
        assert_eq!(s.value(&d, "Stamina"), Some(20.));
        assert_eq!(
            s.try_activate(&d, "Attack").unwrap(),
            Err("cannot afford Stamina".into())
        );
        s.apply_effect(&d, "Stun", None).unwrap();
        s.attributes.get_mut("Stamina").unwrap().base = 100.;
        assert_eq!(
            s.try_activate(&d, "Attack").unwrap(),
            Err("blocked by tag State.Stunned".into())
        );
        assert!(tag_matches("A.B.C", "A.B") && !tag_matches("A.BC", "A.B"));
    }
}
