// SPDX-License-Identifier: MPL-2.0
//! Host-boundary helpers for future native RFVP and Siglus adapters.
//!
//! These helpers do NOT interpret HCB/SS bytecode and do NOT claim native
//! syscall or save compatibility. Original VMs must invoke them at audited seams.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
#[cfg(test)]
extern crate std;

use alloc::{string::String, vec::Vec};

use crysirror_core::{
    path::{Mount, VfsPath},
    service::HostServices,
    state::{ClientId, Command},
    Error, Result,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineKind {
    Rfvp,
    Siglus,
}
impl EngineKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Rfvp => "rfvp",
            Self::Siglus => "siglus",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CasePolicy {
    Exact,
    AsciiInsensitive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatibilityProfile {
    pub engine: EngineKind,
    pub path_case: CasePolicy,
    /// This profile belongs to the adapter. The core does not assume that RFVP
    /// and Siglus share line breaking, ruby or glyph metrics.
    pub layout_profile: String,
}

impl CompatibilityProfile {
    pub fn new(engine: EngineKind, layout_profile: String) -> Result<Self> {
        if layout_profile.is_empty() {
            return Err(Error::Invalid("layout profile"));
        }
        Ok(Self {
            engine,
            path_case: CasePolicy::Exact,
            layout_profile,
        })
    }
    /// Called AFTER original engine-specific filename decoding. Does not
    /// pretend that Shift-JIS bytes are UTF-8 or that lowercasing is universal.
    pub fn asset_path(&self, decoded_name: &str) -> Result<VfsPath> {
        let mut name = decoded_name.replace('\\', "/");
        if self.path_case == CasePolicy::AsciiInsensitive {
            name.make_ascii_lowercase();
        }
        VfsPath::new(Mount::Asset, &name)
    }
}

/// An instance-scoped boundary. Never put native numeric opcode dispatch in core.
pub struct Bridge {
    client: ClientId,
    profile: CompatibilityProfile,
}
impl Bridge {
    pub fn new(client: ClientId, profile: CompatibilityProfile) -> Self {
        Self { client, profile }
    }
    pub fn client(&self) -> ClientId {
        self.client
    }
    pub fn profile(&self) -> &CompatibilityProfile {
        &self.profile
    }
    pub fn submit(&self, host: &mut dyn HostServices, commands: &[Command]) -> Result<()> {
        host.submit(self.client, commands)
    }
    pub fn read_asset(&self, host: &mut dyn HostServices, decoded_name: &str) -> Result<Vec<u8>> {
        host.read(self.client, &self.profile.asset_path(decoded_name)?)
    }
    pub fn write_save(
        &self,
        host: &mut dyn HostServices,
        relative_name: &str,
        bytes: &[u8],
    ) -> Result<()> {
        // Unlike legacy asset paths, writes use canonical engine paths only.
        host.write_atomic(
            self.client,
            &VfsPath::new(Mount::Save, relative_name)?,
            bytes,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_asset_names_are_normalized_at_adapter_boundary() {
        let p = CompatibilityProfile::new(EngineKind::Rfvp, "legacy-rfvp".into()).unwrap();
        assert_eq!(
            p.asset_path("image\\A.png").unwrap().relative(),
            "image/A.png"
        );
        assert!(p.asset_path("..\\outside").is_err());
        assert!(p.asset_path("C:\\outside").is_err());
    }
    #[test]
    fn case_folding_is_opt_in() {
        let mut p = CompatibilityProfile::new(EngineKind::Siglus, "legacy-siglus".into()).unwrap();
        assert_eq!(p.asset_path("A.PNG").unwrap().relative(), "A.PNG");
        p.path_case = CasePolicy::AsciiInsensitive;
        assert_eq!(p.asset_path("A.PNG").unwrap().relative(), "a.png");
    }
}
