// SPDX-License-Identifier: MPL-2.0
//! Print player animation states and their transitions from an extracted
//! `anim-player.bin`. `anim_states ANIM_PLAYER_BIN [STATE…]`
use anyhow::{Context, Result};
use crysirror_naughty_dog::{anim_state, dc::sid};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let table = anim_state::parse(&std::fs::read(args.first().context("ANIM_PLAYER_BIN")?)?)?;
    let name = |id: u32| table.get(id).map_or(format!("{id:08x}"), |s| s.name.clone());
    println!("{} states, initial {}", table.states.len(), name(table.initial));
    if args.get(1).is_some_and(|a| a == "--all") {
        for s in &table.states {
            println!("{}", s.name);
        }
        return Ok(());
    }
    for want in &args[1..] {
        let s = table.get(sid(want)).context("no such state")?;
        println!("{} animation {:08x} (= sid of state name? {})", s.name, s.animation, s.animation == sid(want));
        for t in &s.transitions {
            println!("  {:08x} -> {} fade {}", t.request, name(t.target), t.fade);
        }
    }
    Ok(())
}
