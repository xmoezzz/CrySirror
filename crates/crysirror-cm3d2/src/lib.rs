// SPDX-License-Identifier: MPL-2.0
//! Custom Maid 3D 2 (KISS, Unity 4.6.6p4) native semantics for CrySirror.
//! Adopt mode: the game's C# logic is ported here; archives, images, models,
//! animation, KAG, TJS2 and Unity data readers are shared engine crates.
#![forbid(unsafe_code)]
pub mod adapter;
pub mod bg;
pub mod binio;
mod bone_shape;
pub mod chara;
pub mod config;
pub mod charselect;
pub mod daily;
pub mod files;
pub mod gamedata;
pub mod kag;
pub mod maid_management;
pub mod maid_param;
pub mod menu;
pub mod msgwnd;
pub mod native_key;
pub mod nei;
pub mod ngui;
pub mod face;
pub mod ops;
pub mod params;
pub mod parts_color;
pub mod preset;
pub mod scene_edit;
pub mod schedule;
pub mod schedule_data;
pub mod schedule_ui;
pub mod scenes;
pub mod save;
pub mod saveload;
pub mod shop;
pub mod sound;
pub mod staffroll;
pub mod text;
pub mod system_ui;
pub mod tjs;
pub mod trophy;
pub mod uiev;
pub mod unity_scene;
pub mod wf;
pub mod yotogi;
pub mod yotogi_data;

pub use adapter::{Config, ENGINE_ID};
#[cfg(feature = "runtime")]
pub use adapter::register;

#[cfg(feature = "runtime")]
mod com3d2;
