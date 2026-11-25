// SPDX-License-Identifier: MPL-2.0
//! Print a particle module's effect groups and, per descriptor, the shared
//! emitter its native programs translate to (unit placement).
//!
//! `particle_effects part-menu.bin [GROUP-SUBSTRING]`
use anyhow::Result;
use crysirror_naughty_dog::particle::{Library, Placement};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = &args[0];
    let name = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("part");
    let library = Library::parse(name, &std::fs::read(path)?)?;
    println!("{} groups, {} effects", library.groups.len(), library.effects.len());
    let placement = Placement { position: glam::Vec3::ZERO, rotation: glam::Quat::IDENTITY, color: [1.; 3], alpha: 1., sprite_scale: 1., effect_scale: glam::Vec3::ONE };
    for (group, emitters) in &library.groups {
        for e in emitters.iter().filter_map(|e| library.effects.get(e)) {
            if args.get(1).is_some_and(|f| f != "--check" && !e.name.contains(f.as_str())) {
                continue;
            }
            let (spec, frame) = crysirror_naughty_dog::particle::emitter(e, &e.name, &placement, None, crysirror_vfx::BlendMode::Additive, 0., 1);
            let system = crysirror_vfx::SystemSpec { emitters: vec![spec.clone()], ..Default::default() };
            if let Err(err) = system.compile() {
                println!("{group:08x} {}: does not compile: {err:#} (life {:?} size {:?} rotation {:?} colour {:?} rate {:?} shape {:?})", e.name, spec.initialize.lifetime, spec.initialize.size, spec.initialize.rotation, spec.initialize.color, spec.spawn.rate, spec.initialize.shape);
            }
            if args.get(1).is_some_and(|f| f == "--check") {
                continue;
            }
            println!("{group:08x} {} orientation {} flags {:08x} shape {:?}", e.name, e.orientation, e.flags, e.shape);
            println!("  spawn {:?} capacity {}", spec.spawn, spec.capacity);
            println!("  init life {:?} size {:?} rotation {:?} colour {:?}", spec.initialize.lifetime, spec.initialize.size, spec.initialize.rotation, spec.initialize.color);
            for m in &spec.update {
                if let crysirror_vfx::Module::ColorOverLife { curve } = m {
                    println!("  colour over life {:?}", curve.keys.iter().map(|k| (k.time, k.value)).collect::<Vec<_>>());
                }
            }
            // Motion probe: position and velocity at birth and after the
            // update program at ages 0.5 s and 1 s.
            let probe = |p: &crysirror_naughty_dog::particle::Particle, n: &str| e.slot(n).and_then(|s| p.attrs.get(s)).copied().unwrap_or(f32::NAN);
            let vec = |p: &crysirror_naughty_dog::particle::Particle, k: &str| [probe(p, &format!("{k}X")), probe(p, &format!("{k}Y")), probe(p, &format!("{k}Z"))];
            for seed in [1u16, 777] {
                let born = e.birth(seed);
                let mut at = Vec::new();
                for age in [0.5f32, 1.] {
                    let mut q = born.clone();
                    e.run(&e.update, &mut q, crysirror_naughty_dog::particle::Clock { age, effect_time: age, dt: 1. / 30. });
                    at.push((age, vec(&q, "pos"), vec(&q, "vel")));
                }
                println!("  motion seed {seed}: born pos {:?} vel {:?}; {:?}", vec(&born, "pos"), vec(&born, "vel"), at);
            }
            println!("  frame x {:?} y {:?} t {:?}", frame.x_axis.truncate(), frame.y_axis.truncate(), frame.w_axis.truncate());
        }
    }
    Ok(())
}
