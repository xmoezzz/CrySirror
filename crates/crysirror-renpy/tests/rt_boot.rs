// SPDX-License-Identifier: MPL-2.0
//! Boots a game through the native runtime's `renpy.main.main()` on a
//! script thread and drives it with host ticks.
//!
//! The fixture is an unpacked Ren'Py game and the `renpy/common`
//! directory of its release, for example from the MIT reference checkout:
//! `git archive 8.3.7.25031702 the_question renpy/common | tar -x -C DIR`.
//! Set `RENPY_GAME_DIR` (the `game` directory), `RENPY_COMMON_DIR` and
//! `RENPY_VERSION` (e.g. `8.3.7.0`). Without them the test reports that
//! it did not run; that is not a pass.
use crysirror_renpy::rt::{
    boot,
    link::{self, FromScript, Tick, ToScript, Wait},
    state::{Files, Profile},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

struct DirFiles {
    game: PathBuf,
    common: PathBuf,
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

impl Files for DirFiles {
    fn list(&self) -> Vec<String> {
        let mut v = Vec::new();
        walk(&self.game, &self.game, &mut v);
        v.sort();
        v
    }
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.game.join(name)).ok()
    }
    fn common_list(&self) -> Vec<String> {
        let mut v = Vec::new();
        walk(&self.common, &self.common, &mut v);
        v.sort();
        v
    }
    fn common_read(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.common.join(name)).ok()
    }
}

fn fixture() -> Option<(DirFiles, [u32; 4])> {
    let game = std::env::var_os("RENPY_GAME_DIR")?;
    let common = std::env::var_os("RENPY_COMMON_DIR")?;
    let v = std::env::var("RENPY_VERSION").ok()?;
    let mut version = [0u32; 4];
    for (i, p) in v.split('.').take(4).enumerate() {
        version[i] = p.parse().ok()?;
    }
    Some((DirFiles { game: game.into(), common: common.into() }, version))
}

#[test]
fn game_boots_to_its_first_interaction() {
    let Some((files, version)) = fixture() else {
        eprintln!("game_boots_to_its_first_interaction: no fixture (RENPY_GAME_DIR/RENPY_COMMON_DIR/RENPY_VERSION); not run");
        return;
    };
    let (to_script, from_owner) = mpsc::sync_channel::<ToScript>(1);
    let (to_owner, from_script) = mpsc::sync_channel::<FromScript>(1);
    let files: Arc<dyn Files> = Arc::new(files);
    let first = Tick { now_ms: 0, ..Tick::default() };
    let thread = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let l = link::Link::new(from_owner, to_owner, first);
            boot::run_session(Profile::new(version), files, l, &|vm| {
                crysirror_renpy::rt::host(vm).borrow_mut().canvas = [1280, 720];
            });
        })
        .unwrap();
    let mut now = 0u64;
    let mut frames = 0;
    let mut last_frame = None;
    let mut exit = None;
    for _ in 0..240 {
        match from_script.recv().expect("script thread alive") {
            FromScript::Yield { out, wait } => {
                for d in &out.diagnostics {
                    eprintln!("diag: {d}");
                }
                if let Some(f) = out.frame {
                    frames += 1;
                    last_frame = Some(f);
                }
                now = match wait {
                    Wait::Until(t) => t.max(now + 1),
                    _ => now + 16,
                };
                if frames >= 3 {
                    break;
                }
                to_script.send(ToScript::Tick(Tick { now_ms: now, ..Tick::default() })).unwrap();
            }
            FromScript::Exit { out, result } => {
                for d in &out.diagnostics {
                    eprintln!("diag: {d}");
                }
                exit = Some(result);
                break;
            }
        }
    }
    if exit.is_none() {
        let _ = to_script.send(ToScript::Shutdown);
        while let Ok(m) = from_script.recv() {
            if let FromScript::Exit { result, .. } = m {
                exit = Some(result);
                break;
            }
            let _ = to_script.send(ToScript::Shutdown);
        }
    }
    thread.join().unwrap();
    if let Some(Err(e)) = &exit {
        panic!("the game ended with an error:\n{e}");
    }
    assert!(frames > 0, "no frame was drawn (exit: {exit:?})");
    let f = last_frame.unwrap();
    eprintln!("frames: {frames}; last frame has {} layer operations", f.len());
}
