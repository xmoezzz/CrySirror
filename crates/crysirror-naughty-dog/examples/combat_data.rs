// SPDX-License-Identifier: MPL-2.0
//! Parse the combat tables from extracted DC modules and print a summary:
//! weapons, archetypes, skills, damage entries, bullets, the player damage
//! and, when `camera.bin` is present, the follow and aim camera settings.
//!
//! `combat_data DIR` (holding `weapons.bin`, `ai.bin`,
//! `damage-settings.bin`, `dda-settings.bin`)
use anyhow::{Context, Result};
use crysirror_naughty_dog::{combat::CombatData, dc::sid};

fn main() -> Result<()> {
    let dir = std::env::args().nth(1).context("DIR")?;
    let names = ["weapons", "ai", "damage-settings", "dda-settings"];
    let bytes: Vec<Vec<u8>> = names.iter().map(|n| std::fs::read(format!("{dir}/{n}.bin"))).collect::<std::io::Result<_>>()?;
    let modules: Vec<(&str, &[u8])> = names.iter().copied().zip(bytes.iter().map(Vec::as_slice)).collect();
    let d = CombatData::parse(&modules)?;
    println!("{} weapons, {} archetypes, {} skills, {} damage entries, player damage {:?}", d.weapons.len(), d.archetypes.len(), d.skills.len(), d.damage.len(), d.player_damage);
    for w in ["assault-rifle-c", "pistol-semi-b", "shotgun-b"] {
        println!("{w}: {:?}", d.weapons.get(&sid(w)));
    }
    for a in ["medium-mg-burst", "light-pistol-upgrade", "armor-soldier"] {
        println!("{a}: {:?}", d.npc_weapon(sid(a)).map(|(a, s, w)| (a.clone(), s.cloned(), w.map(|w| w.interval))));
    }
    let resolved = d.weapons.values().filter(|w| d.bullets.contains_key(&w.bullet)).count();
    println!("{} bullets; {resolved} of {} weapons name one", d.bullets.len(), d.weapons.len());
    if let Some(b) = d.weapons.get(&sid("pistol-semi-b")).and_then(|w| d.bullets.get(&w.bullet)) {
        println!("pistol-semi-b bullet: {} impacts, stone {:#x} (stone-pistol-squib {:#x}), tracer {:#x} (rifle-beam-tracer {:#x})", b.impacts.len(), b.impact("stone"), sid("stone-pistol-squib"), b.tracer, sid("rifle-beam-tracer"));
    }
    if let Ok(b) = std::fs::read(format!("{dir}/camera.bin")) {
        println!("camera: {:?}", crysirror_naughty_dog::camera_settings::CameraSettings::parse(&b)?);
    }
    println!("pistol-chloe vs light {} medium {}", d.damage(sid("pistol-chloe"), sid("light-class")), d.damage(sid("pistol-chloe"), sid("medium-class")));
    Ok(())
}
