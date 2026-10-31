//! Emulation of the DLLs games load with `LoadDLL` / `#DLL.nnn`.
//!
//! * `EF00` (Little Busters!): scattered-sprite effects. Function 0 seeds
//!   eight tracks of 32 random control points; function 1 evaluates a
//!   uniform cubic B-spline through them at `F[1162 + track]` / 1000 and
//!   projects the point, writing `F[1151..=1153]`.
//! * `DT00` (Tomoyo After's dungeon RPG): the translation in
//!   [`crate::dt00`], reading its tables from the game's `dt00.dll`.
//! * `PT00` (Little Busters! baseball): the native port in [`crate::pt00`].
//!   It reads its data tables from the game's own `PT00.dll`, loaded on
//!   the first call. The DLL's integer array is the script's `intD`,
//!   copied in before and out after each call.

use std::collections::HashMap;

use anyhow::{bail, Result};

/// What the DLL emulation needs from the game's native system: the script
/// RNG and the game's own DLL file (CrySirror port change 2).
pub trait System {
    fn random(&mut self, low: i32, high: i32) -> i32;
    /// The bytes of a file in the game's root directory, by name.
    fn root_file(&mut self, name: &str) -> Result<Vec<u8>>;
}

/// Quadrant choices for the eight tracks (four patterns).
const DIRECTIONS: [usize; 32] = [
    0, 2, 1, 3, 0, 2, 1, 3, 1, 3, 2, 0, 1, 3, 2, 0, 0, 0, 0, 0, 3, 1, 2, 0, 3, 1, 3, 1, 0, 2, 3, 1,
];

const TRACK: usize = 0x60;

#[derive(Debug, Clone, PartialEq)]
pub enum Dll {
    Ef00(Vec<f64>),
    /// Built on the first call.
    Pt00(Option<Box<crate::pt00::Pt00>>),
    /// Built on the first call.
    Dt00(Option<Box<crate::dt00::Dt00>>),
    /// Named but unsupported; calls fail visibly.
    Unknown(String),
}

impl Dll {
    pub fn named(name: &str) -> Self {
        let stem = name
            .trim()
            .trim_end_matches(".dll")
            .trim_end_matches(".DLL")
            .to_ascii_uppercase();
        match stem.as_str() {
            "EF00" => Dll::Ef00(Vec::new()),
            "PT00" => Dll::Pt00(None),
            "DT00" => Dll::Dt00(None),
            _ => Dll::Unknown(name.to_owned()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Dll::Ef00(_) => "EF00",
            Dll::Pt00(_) => "PT00",
            Dll::Dt00(_) => "DT00",
            Dll::Unknown(name) => name,
        }
    }
}

/// Integer memory the DLLs read and write: whole banks, by the bank
/// number of [`crate::memory`] (A–F are 0–5, G 6, Z 7).
pub trait DllMemory {
    fn bank(&mut self, bank: u8) -> Result<&mut [i32]>;

    fn get(&mut self, bank: u8, index: i32) -> Result<i32> {
        let words = self.bank(bank)?;
        usize::try_from(index)
            .ok()
            .and_then(|i| words.get(i).copied())
            .ok_or_else(|| anyhow::anyhow!("reallive: bad DLL memory index {index}"))
    }

    fn set(&mut self, bank: u8, index: i32, value: i32) -> Result<()> {
        let words = self.bank(bank)?;
        let slot = usize::try_from(index)
            .ok()
            .and_then(|i| words.get_mut(i))
            .ok_or_else(|| anyhow::anyhow!("reallive: bad DLL memory index {index}"))?;
        *slot = value;
        Ok(())
    }
}

const BANK_D: u8 = 3;
const BANK_F: u8 = 5;

fn ef00_configure(params: &mut Vec<f64>, sys: &mut dyn System, args: [i32; 4]) {
    if params.is_empty() {
        params.resize(TRACK * 8, 0.0);
    }
    let (top, size) = if args[0] == 1 {
        (0, 0x20)
    } else {
        let top = args[1].clamp(0, 0x20);
        (top, args[2].min(0x20 - top).max(0))
    };
    let mut random = |modulus: i32| f64::from(sys.random(0, modulus - 1));
    for track in 0..8 {
        let at = track * TRACK + top as usize * 3;
        for point in 0..size as usize {
            params[at + point * 3] = random(800) - 400.0;
            params[at + point * 3 + 1] = random(600) - 300.0;
            params[at + point * 3 + 2] = random(700) - 350.0;
        }
    }
    if args[3] != 1 {
        return;
    }
    let pattern = sys.random(0, 3) as usize * 8;
    for track in 0..8 {
        let p = &mut params[track * TRACK..];
        let mut x = f64::from(sys.random(0, 599)) - 300.0;
        let mut y = f64::from(sys.random(0, 479)) - 240.0;
        x += if x < 0.0 { -80.0 } else { 80.0 };
        y += if y < 0.0 { -80.0 } else { 80.0 };
        // Force the start into one quadrant.
        let (sx, sy) = match DIRECTIONS[pattern + track] {
            0 => (1.0, 1.0),
            1 => (-1.0, 1.0),
            2 => (1.0, -1.0),
            _ => (-1.0, -1.0),
        };
        x = x.abs() * sx;
        y = y.abs() * sy;
        p[9] = x * 1.2;
        p[10] = y * 1.2;
        p[11] *= 1.2;
        p[12] *= -0.08;
        p[13] *= -0.08;
        p[14] *= -0.08;
        p[15] = -p[9];
        p[16] = -p[10];
        p[17] = -p[11];
    }
}

fn ef00_calculate(params: &[f64], memory: &mut dyn DllMemory, track: i32) -> Result<()> {
    if params.is_empty() {
        bail!("reallive: EF00 calculation before configuration");
    }
    let track = track.clamp(0, 7);
    let step = memory.get(BANK_F, 1154 + track)?;
    let point = |offset: i32| ((((step + offset) & 0x1f) + track * 0x20) * 3) as usize;
    let (j, k, l, m) = (point(0), point(1), point(2), point(3));
    let x = f64::from(memory.get(BANK_F, 1162 + track)?) * 0.001;
    // The DLL reads each control point one element late; kept for
    // fidelity with the original.
    let at = |i: usize| params.get(i).copied().unwrap_or(0.0);
    let blend = |o: usize| {
        crysirror_vfx::spatial::uniform_cubic_bspline(
            [at(j + o), at(k + o), at(l + o), at(m + o)],
            x,
        )
    };
    let depth = blend(3);
    let (mut px, mut py) = (blend(2), blend(1));
    if depth != 400.0 {
        [px, py] = crysirror_vfx::spatial::perspective_xy([px, py], [800.0, 700.0], 400.0 - depth);
    }
    memory.set(BANK_F, 1151, px as i32)?;
    memory.set(BANK_F, 1152, py as i32)?;
    memory.set(BANK_F, 1153, depth as i32)?;
    Ok(())
}

fn pt00_call(
    state: &mut Option<Box<crate::pt00::Pt00>>,
    sys: &mut dyn System,
    memory: &mut dyn DllMemory,
    args: [i32; 5],
) -> Result<i32> {
    use crate::pt00::{Pt00, F_LEN};
    let pt = match state {
        Some(pt) => pt,
        None => {
            let dll = sys.root_file("PT00.dll")?;
            let seed = sys.random(0, i32::MAX) as u32;
            state.insert(Box::new(Pt00::load(&dll, seed)?))
        }
    };
    let words = memory.bank(BANK_D)?;
    let len = words.len().min(F_LEN);
    pt.write_f(&words[..len]);
    let result = pt.call(args[0], args[1], args[2], args[3], args[4]);
    pt.read_f(&mut memory.bank(BANK_D)?[..len]);
    Ok(result)
}

fn dt00_call(
    state: &mut Option<Box<crate::dt00::Dt00>>,
    sys: &mut dyn System,
    memory: &mut dyn DllMemory,
    args: [i32; 5],
) -> Result<i32> {
    use crate::dt00::{Dt00, BANK_COUNT};
    let dt = match state {
        Some(dt) => dt,
        None => {
            let dll = sys.root_file("dt00.dll")?;
            let seed = sys.random(0, i32::MAX) as u32;
            state.insert(Box::new(Dt00::load(&dll, seed)?))
        }
    };
    // intA–intF, intG, intZ, in the order of RealLiveState.
    for bank in 0..BANK_COUNT {
        let words = memory.bank(bank as u8)?;
        dt.write_bank(bank, words);
    }
    let result = dt.call(args[0], args[1], args[2], args[3], args[4]);
    for bank in 0..BANK_COUNT {
        let words = memory.bank(bank as u8)?;
        dt.read_bank(bank, words);
    }
    Ok(result)
}

/// DLL slots of a running game.
#[derive(Debug, Default, Clone)]
pub struct Dlls {
    pub slots: HashMap<i32, Dll>,
}

impl Dlls {
    pub fn load(&mut self, slot: i32, name: &str) {
        self.slots.insert(slot, Dll::named(name));
    }

    pub fn unload(&mut self, slot: i32) {
        self.slots.remove(&slot);
    }

    pub fn call(
        &mut self,
        slot: i32,
        sys: &mut dyn System,
        memory: &mut dyn DllMemory,
        args: [i32; 5],
    ) -> Result<i32> {
        let Some(dll) = self.slots.get_mut(&slot) else {
            bail!("reallive: no DLL is loaded in slot {slot}");
        };
        match dll {
            Dll::Ef00(params) => match args[0] {
                0 => ef00_configure(params, sys, [args[1], args[2], args[3], args[4]]),
                1 => ef00_calculate(params, memory, args[1])?,
                other => bail!("reallive: EF00 has no function {other}"),
            },
            Dll::Pt00(state) => return pt00_call(state, sys, memory, args),
            Dll::Dt00(state) => return dt00_call(state, sys, memory, args),
            Dll::Unknown(name) => bail!("reallive: DLL {name} is not implemented"),
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoFiles;
    impl System for NoFiles {
        fn random(&mut self, _low: i32, _high: i32) -> i32 {
            0
        }
        fn root_file(&mut self, _name: &str) -> Result<Vec<u8>> {
            bail!("no test file authority")
        }
    }
    struct Memory(Vec<i32>);
    impl DllMemory for Memory {
        fn bank(&mut self, bank: u8) -> Result<&mut [i32]> {
            if bank == BANK_F {
                Ok(&mut self.0)
            } else {
                bail!("unexpected bank")
            }
        }
    }
    #[test]
    fn extracted_ef00_sampler_preserves_native_wrap_offset_and_bank_results() {
        let params: Vec<f64> = (0..TRACK * 8)
            .map(|i| ((i * 7) % 113) as f64 - 56.)
            .collect();
        let mut dlls = Dlls::default();
        dlls.slots.insert(1, Dll::Ef00(params));
        let mut memory = Memory(vec![0; 2000]);
        // Golden integer outputs from the pinned scalar EF00 arithmetic,
        // including its one-element-late points and unclamped time values.
        for (track, step, time, expected) in [
            (0, 31, 375, [-56, -60, -22]),
            (5, 7, 1500, [30, 13, 21]),
            (7, 30, -250, [-5, -17, 0]),
        ] {
            memory.0[1154 + track] = step;
            memory.0[1162 + track] = time;
            assert_eq!(
                dlls.call(1, &mut NoFiles, &mut memory, [1, track as i32, 0, 0, 0])
                    .unwrap(),
                0
            );
            assert_eq!(&memory.0[1151..1154], &expected);
        }
    }
    #[test]
    fn unknown_dll_names_fail_without_file_or_executable_access() {
        let mut dlls = Dlls::default();
        dlls.load(0, "unknown.dll");
        let error = dlls
            .call(0, &mut NoFiles, &mut Memory(vec![0; 2000]), [0; 5])
            .unwrap_err();
        assert!(error.to_string().contains("not implemented"));
    }
}
