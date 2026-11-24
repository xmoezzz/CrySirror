// SPDX-License-Identifier: MPL-2.0
//! Native key material of the installed `cm3d2_x64.dll`.
//!
//! The `.nei` table key is a 16-byte AES key that `sub_180001000` copies from
//! `xmmword_180155A40` into `xmmword_18015BF38` at load. It is never embedded
//! in CrySirror: it is read from the user's DLL at a file offset recorded per
//! DLL digest, and an unknown DLL fails visibly.
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};

struct Profile {
    /// SHA-256 of the plug-in DLL.
    digest: &'static str,
    /// File offset of the key (VA 0x180155A40 in `.data`, raw 0x153000).
    key_offset: usize,
    name: &'static str,
}

const PROFILES: &[Profile] = &[Profile {
    digest: "3d2a7db17d61cff21a25cc8722b31bc17ec482c11e19e1b614f101ae5c2ca658",
    key_offset: 0x154a40,
    name: "CM3D2 1.00 x64 (Unity 4.6.6p4)",
}, Profile {
    digest: "0317add504a0c2640e792c99a5e5422840c0d9ba51086120cc850aeebd2b43cd",
    key_offset: 0x186b60,
    name: "COM3D2 x64 (Unity 5.6.4p2)",
}];

pub const DLL_PATH: &str = "CM3D2x64_Data/Plugins/cm3d2_x64.dll";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeKeys {
    pub profile: &'static str,
    pub table_key: [u8; 16],
}

impl NativeKeys {
    pub fn from_dll(bytes: &[u8]) -> Result<Self> {
        let digest: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        let profile = PROFILES
            .iter()
            .find(|p| p.digest == digest)
            .with_context(|| format!("unsupported cm3d2 plug-in DLL (sha256 {digest})"))?;
        let key: [u8; 16] = bytes
            .get(profile.key_offset..profile.key_offset + 16)
            .context("cm3d2 plug-in DLL is shorter than its profile")?
            .try_into()?;
        ensure!(key.iter().any(|b| *b != 0), "cm3d2 table key is empty");
        Ok(Self { profile: profile.name, table_key: key })
    }
}
