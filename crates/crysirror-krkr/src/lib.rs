// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Optional KiriKiri (吉里吉里) adapter (ADR 0053).
//!
//! Original TJS2 scripts run on the vendored Kirakira VM (`krkr-tjs2`,
//! AGPL-3.0-or-later). KiriKiri's native classes are translated onto engine
//! Layer/Canvas services, the engine clock and capability storage; there is
//! no KiriKiri renderer, compositor or mixer in this crate. Because of the
//! upstream licence this crate is not part of the default desktop build.
#![forbid(unsafe_code)]
// TjsError is the vendored VM's error type; its size is not ours to change.
#![allow(clippy::result_large_err)]
pub mod adapter;
mod constants;
mod async_trigger;
mod kag;
mod kag_save;
pub mod host;
mod menus;
pub mod natives;
mod plugins;
mod sound;
mod save_struct;
mod video;
mod pointer;
pub mod storage;

pub use adapter::{Config, KrkrFactory, KrkrVm, register};

mod image_api;

mod json_plugin;

mod draw_plugin;

mod protect_plugin;

mod text_render_plugin;

mod motion_plugin;

mod csv_plugin;
mod psbfile_plugin;

mod kirakira;

mod surfaces;

mod storage_plugins;

mod transitions;

mod piled_copy;
