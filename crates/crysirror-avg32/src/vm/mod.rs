// SPDX-License-Identifier: MPL-2.0
//! The original AVG32 interpreter and its native policies, retained from
//! siglus_rs `crates/avg32/src` at commit
//! 5f358ef1360b0439e92aed2b749e8f25d8721954 (MPL-2.0). See `SOURCE.json`.
//!
//! Raster buffers and transitions are the shared `crysirror_image::avg32`
//! device; files, saves, clock, text glyphs and audio are engine services
//! reached through the adapter (`fs`, `sound::AudioCommand`, `text`).
#![allow(dead_code, clippy::all)]

pub mod animation;
pub mod archive;
pub mod ard;
pub mod cgmode;
pub mod cursor;
pub mod engine;
pub mod flags;
pub mod font;
pub mod fs;
pub mod game;
pub mod ini;
pub mod menu;
pub mod meswin;
pub mod movie;
pub mod nls;
pub mod resource;
pub mod savedata;
pub mod scenario;
pub mod scene;
pub mod sound;
pub mod text;
pub mod system;
pub mod voicepatch;
