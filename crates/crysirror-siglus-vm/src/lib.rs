// SPDX-License-Identifier: MPL-2.0
#![forbid(unsafe_code)]
#![allow(dead_code, unexpected_cfgs)]
extern crate self as siglus_assets;
macro_rules! env_is_set {
    ($name:literal) => {{ std::env::var_os($name).is_some() }};
}
pub mod angou;
pub mod audio;
pub mod elm_code;
pub mod emote;
pub mod gameexe;
pub mod image_manager;
pub mod key_toml;
pub mod layer;
pub mod lzss;
pub mod mesh3d;
pub mod movie;
pub mod original_save;
pub mod platform_time;
pub mod resource;
pub mod runtime;
pub mod scene_pck;
pub mod scene_stream;
pub mod text_render;
mod util;
pub mod vm;
pub mod assets {
    pub use crysirror_siglus::RgbaImage;
}

mod serde_array;

pub mod io_fs;

pub mod adapter;
mod export;

pub mod fixture;
#[cfg(test)]
mod native_save_tests;
