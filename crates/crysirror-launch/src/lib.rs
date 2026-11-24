// SPDX-License-Identifier: MPL-2.0
//! Window-independent CrySirror player shared by the desktop and console
//! hosts: engine detection and launch plans, network/local game sources,
//! integrated-runtime roots, host providers and the runtime/renderer core.
#[cfg(not(target_arch = "wasm32"))]
mod compressed_archive;
pub mod core;
pub mod desktop_provider;
pub mod detect;
pub mod input_track;
pub mod integrated_roots;
pub mod nls;
pub mod source_path;
pub mod virtual_window;
