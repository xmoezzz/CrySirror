// SPDX-License-Identifier: MPL-2.0
//! Release gates for the display-layer modules: the running game's Ren'Py
//! version as a full tuple, so behaviour can follow intermediate releases
//! (6.99.11 vs 6.99.12) and not only `major.minor`.
use crysirror_python::Vm;

/// The game's `renpy.version_tuple` (`[major, minor, patch, build]`).
/// Without a loaded profile (unit VMs) the newest supported release.
pub fn version(vm: &Vm) -> [u32; 4] {
    super::state::try_rt(vm)
        .map(|r| r.profile.version)
        .unwrap_or([8, 6, 1, 26111705])
}

/// `renpy.version_tuple >= v`, comparing as many components as `v` has.
pub fn at_least(vm: &Vm, v: &[u32]) -> bool {
    let cur = version(vm);
    cur[..v.len()] >= *v
}

/// `renpy.version_tuple < v`.
pub fn before(vm: &Vm, v: &[u32]) -> bool {
    !at_least(vm, v)
}

/// The release lies in `[from, to)`.
pub fn between(vm: &Vm, from: &[u32], to: &[u32]) -> bool {
    at_least(vm, from) && before(vm, to)
}

/// A feature released in 8.`py3` and its Python 2 twin 7.`py2`
/// (`Profile::since_pair`).
pub fn state_since_pair(vm: &Vm, py2: u32, py3: u32) -> bool {
    let v = version(vm);
    at_least(vm, &[8, py3]) || (v[0] == 7 && v[1] >= py2)
}
