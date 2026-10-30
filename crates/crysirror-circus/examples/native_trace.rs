//! Static native execution diagnostic. Does not render and is not a game host.
use anyhow::{bail, Context, Result};
use crysirror_circus::{
    machine::{Event, Machine, NativeHost, Outcome},
    Program,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
struct Host {
    root: PathBuf,
    scenes: BTreeMap<String, Arc<Program>>,
    events: usize,
}
impl NativeHost for Host {
    fn scene_index(&mut self, name: &str) -> Result<Option<i32>> {
        let names = crysirror_circus::names::Names::parse(&std::fs::read(
            self.root.join("Advdata/GRP/NAMES.DAT"),
        )?)?;
        Ok(Some(
            names
                .scenes
                .iter()
                .position(|s| s.eq_ignore_ascii_case(name.split('.').next().unwrap_or(name)))
                .map_or(-1, |i| i as i32),
        ))
    }
    fn scene(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        if let Some(p) = self.scenes.get(name) {
            return Ok(Some(p.clone()));
        }
        let namepath = format!("{}.mes", name.trim_end_matches(".mes"));
        let path = self.root.join("Advdata/MES").join(namepath);
        let p = Arc::new(Program::parse(
            &std::fs::read(&path).with_context(|| path.display().to_string())?,
        )?);
        self.scenes.insert(name.into(), p.clone());
        eprintln!("scene {name}");
        Ok(Some(p))
    }
    fn emit(&mut self, e: &Event) -> Result<bool> {
        if matches!(e, Event::PlayerInput) {
            bail!("native_trace has no UI; use the real runtime headless_boot for player input");
        }
        self.events += 1;
        if let Event::LoadImage { slot, name } = e {
            let path = self
                .root
                .join("Advdata/GRP")
                .join(format!("{}.crx", name.replace('\\', "/")));
            let bytes = std::fs::read(&path).with_context(|| path.display().to_string())?;
            if !bytes.starts_with(b"CRXG") {
                bail!("unknown image {name}");
            }
            eprintln!("image slot={slot} {name}");
        }
        Ok(true)
    }
    fn save_exists(&mut self, _slot: u16) -> Result<bool> {
        Ok(false)
    } // Diagnostic is an explicitly fresh empty save namespace.
    fn sound_name(&mut self, id: u16) -> Result<Option<String>> {
        let names = crysirror_circus::names::Names::parse(&std::fs::read(
            self.root.join("Advdata/GRP/NAMES.DAT"),
        )?)?;
        Ok(Some(
            names
                .sfx
                .get(usize::from(id))
                .context("native sound ID")?
                .clone(),
        ))
    }
}
fn main() -> Result<()> {
    let root = std::env::args()
        .nth(1)
        .context("native_trace GAME_ROOT [ENTRY]")?;
    let entry = std::env::args().nth(2).unwrap_or("start".into());
    let mut machine = Machine::new(entry);
    let mut host = Host {
        root: root.into(),
        scenes: BTreeMap::new(),
        events: 0,
    };
    for n in 0..100_000 {
        match machine.step(&mut host, n * 16, false) {
            Ok(Outcome::Advanced) => {}
            Ok(o) => {
                println!(
                    "{o:?}, scene={}, pc={:#x}, instructions={}, native groups={}, events={}",
                    machine.state.scene,
                    machine.state.pc,
                    machine.state.instructions,
                    machine.state.groups.len(),
                    host.events
                );
                if matches!(o, Outcome::Halted | Outcome::WaitForInput) {
                    break;
                }
            }
            Err(e) => {
                eprintln!("{e:#}");
                return Err(e);
            }
        }
    }
    Ok(())
}
