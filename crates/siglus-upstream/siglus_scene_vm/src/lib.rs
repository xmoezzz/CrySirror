//! Siglus BG stage: g00 decoding + Siglus-like resource lookup + wgpu rendering.
//!
//! Code comments are intentionally in English.

/// Whether a debug environment variable is set, read once per call site:
/// these checks sit on per-object and per-frame paths, where reading the
/// environment each time (a lock, a scan and an allocation) is measurable
/// on consoles.
macro_rules! env_is_set {
    ($name:literal) => {{
        static SET: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *SET.get_or_init(|| std::env::var_os($name).is_some())
    }};
}

pub mod app_path;
pub mod platform_time;

pub mod assets;
pub mod audio;
pub mod emote;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub mod emote_key;
pub mod image_manager;
#[cfg(all(feature = "native-host", not(any(target_os = "horizon", target_os = "vita"))))]
pub mod ime;
pub mod layer;
pub mod mesh3d;
pub mod movie;
pub mod original_save;
pub mod render_math;
pub(crate) mod render_plan;
pub mod resource;
pub mod runtime;
pub mod text_render;
#[cfg(target_arch = "wasm32")]
pub mod wasm_entry;
#[cfg(target_arch = "wasm32")]
pub mod wasm_vfs;

pub mod elm_code;

pub mod scene_stream;
pub mod vm;

// Re-export the format-first asset crate so higher layers (VM/app) can share
// parsers/decoders without wiring a second direct dependency.
pub use siglus_assets as formats;

#[cfg(all(feature = "native-render", not(any(target_os = "horizon", target_os = "vita"))))]
pub mod render;
#[cfg(target_os = "horizon")]
#[path = "render/horizon/mod.rs"]
pub mod render;
#[cfg(target_os = "vita")]
#[path = "render/vita/mod.rs"]
pub mod render;

pub mod input;

#[cfg(target_os = "android")]
pub mod android_host;
#[cfg(all(feature = "native-host", any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub mod desktop_chihaya_bench;
#[cfg(all(feature = "native-host", any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub mod desktop_messagebox;
#[cfg(all(feature = "native-host", any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub mod desktop_twitter;
// CrySirror: the host's semantic pump/syscom logic needs only a Renderer.
#[cfg(feature = "native-render")]
pub mod host;
#[cfg(target_os = "ios")]
pub mod ios_host;
#[cfg(all(feature = "native-host", any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub mod pump_host;
#[cfg(target_os = "horizon")]
pub mod switch_host;

#[cfg(feature = "native-host")]
pub mod display_ffi;

#[cfg(all(feature = "native-host", any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub mod desktop_config;
#[cfg(all(feature = "native-host", any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub mod desktop_icon;

/// CrySirror port addition: an always-failing stand-in for the `ureq` calls
/// used by game network syscalls when the `native-net` feature is off.
#[cfg(not(feature = "native-net"))]
pub(crate) mod ureq_disabled {
    #[derive(Debug)]
    pub struct Response;
    impl Response {
        pub fn into_reader(self) -> std::io::Empty {
            std::io::empty()
        }
        pub fn status(&self) -> u16 {
            0
        }
    }
    #[derive(Debug)]
    pub enum Error {
        Status(u16, Response),
        Disabled,
    }
    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("network access is disabled in the CrySirror embedding")
        }
    }
    impl std::error::Error for Error {}
    pub struct Request;
    impl Request {
        pub fn set(self, _: &str, _: &str) -> Self {
            self
        }
        pub fn call(self) -> Result<Response, Error> {
            Err(Error::Disabled)
        }
        pub fn send_bytes(self, _: &[u8]) -> Result<Response, Error> {
            Err(Error::Disabled)
        }
    }
    pub fn get(_: &str) -> Request {
        Request
    }
    pub fn post(_: &str) -> Request {
        Request
    }
}
