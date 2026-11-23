// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! KiriKiri (吉里吉里) adapter over CrySirror services (ADR 0053).
//!
//! Original TJS2 scripts execute through `crysirror-tjs2`; KAG scenarios use
//! `crysirror-kag`. Native objects map onto shared Layer/Canvas, clock, audio
//! and capability-storage services. The desktop links this adapter directly.
//!
//! Native implementation entry points:
//! - `natives.rs`: Layer operations, native faces and holdAlpha policy.
//! - `host.rs`: queued engine commands, layer lifetime and surface ownership.
//! - `image_api.rs` / `piled_copy.rs`: image dependencies and composed copies.
//! - `native_mouse.rs` / `pointer.rs` / `tooltip.rs`: pointer and hint policy.
//! - `storage.rs` / `storage_plugins.rs`: native names and text streams.
//! - `protect_plugin.rs`: protected archive registration over shared Resources.
//! - `video.rs` / `alpha_movie.rs`: native media state and event translation.
//! - `kag.rs` / `kag_save.rs`: scenario and parser-save semantics.
//! - `font_resources.rs` / `menus.rs`: font waits and menu-handle translation.
#![forbid(unsafe_code)]
// TjsError is the vendored VM's error type; its size is not ours to change.
#![allow(clippy::result_large_err)]
mod province;
mod native_pad;
mod fftgraph;
pub mod adapter;
mod constants;
mod async_trigger;
mod alpha_movie;
mod exe_options;
mod kag;
mod kag_save;
pub mod host;
mod menus;
pub mod natives;
mod native_members;
mod native_mouse;
mod plugins;
mod sound;
mod save_struct;
mod edit_dialog;
mod video;
mod pointer;
mod tooltip;
mod layer_movie;
pub mod storage;

pub use adapter::{Config, KrkrFactory, KrkrVm, register};

mod image_api;

mod json_plugin;

mod draw_plugin;
mod d2d_view;

mod protect_plugin;

mod text_render_plugin;

mod motion_plugin;

mod csv_plugin;
mod line_plugin;
mod varfile_plugin;
mod audio_plugin;
mod kaicho_plugin;
mod psbfile_plugin;
mod psd_plugin;
mod wf_plugins;
mod layer_ex;
mod province_ex;
mod system_ex;
mod wv_decoder;
mod multitrack;
mod msdf_plugin;

mod plugin_ports;

mod surfaces;
mod system_plugins;
mod offline_plugins;
mod get_sample;
mod http_plugin;
mod wrapper_plugins;

mod storage_plugins;

mod transitions;

mod piled_copy;

mod font_resources;
