// SPDX-License-Identifier: MPL-2.0
//! Embedded Python shells of the `renpy` package (see `build.rs`).

include!(concat!(env!("OUT_DIR"), "/py_sources.rs"));

pub fn source(name: &str) -> Option<&'static str> {
    SOURCES.iter().find(|(n, _, _)| *n == name).map(|(_, s, _)| *s)
}

pub fn is_package(name: &str) -> bool {
    SOURCES.iter().any(|(n, _, p)| *n == name && *p)
}

/// Shell of the native runtime: its own tree, then the shared generated
/// configuration profiles (`renpy._profiles`).
pub fn rt_source(name: &str) -> Option<&'static str> {
    RT_SOURCES.iter().find(|(n, _, _)| *n == name).map(|(_, s, _)| *s).or_else(|| name.starts_with("renpy._profiles").then(|| source(name)).flatten())
}

pub fn rt_is_package(name: &str) -> bool {
    RT_SOURCES.iter().any(|(n, _, p)| *n == name && *p) || (name == "renpy._profiles" && is_package(name))
}
