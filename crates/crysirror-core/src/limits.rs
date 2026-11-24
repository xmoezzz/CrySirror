// SPDX-License-Identifier: MPL-2.0
//! Per-platform size limits for single large assets (game files read whole,
//! generated models, imported glTF). PS Vita and Switch keep 64 MiB; the
//! browser allows 256 MiB; desktop and mobile allow 1 GiB.

#[cfg(any(target_os = "vita", target_os = "horizon"))]
pub const LARGE_ASSET_BYTES: u64 = 64 << 20;
#[cfg(all(target_arch = "wasm32", not(any(target_os = "vita", target_os = "horizon"))))]
pub const LARGE_ASSET_BYTES: u64 = 256 << 20;
#[cfg(not(any(target_os = "vita", target_os = "horizon", target_arch = "wasm32")))]
pub const LARGE_ASSET_BYTES: u64 = 1 << 30;
