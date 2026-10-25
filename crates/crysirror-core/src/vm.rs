// SPDX-License-Identifier: MPL-2.0
use crate::{
    service::{HostServices, InputFrame},
    Result,
};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmYield {
    BudgetExhausted,
    WaitUntil(u64),
    WaitForInput,
    Halted,
}

/// A port each original VM must implement inside its own source tree.
/// The data VM and independent original Siglus interpreter implement this port.
/// Device handles remain outside private logical state.
pub trait VmAdapter {
    fn engine_id(&self) -> &'static str;
    fn snapshot_schema(&self) -> u32;
    fn step(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
        instruction_budget: u32,
    ) -> Result<VmYield>;
    /// Call only at an adapter-defined safepoint. Never serialize raw GPU handles.
    fn snapshot(&self) -> Result<Vec<u8>>;
    /// Implementations must validate before replacing live VM state.
    fn restore(&mut self, bytes: &[u8]) -> Result<()>;
    /// Rebuild immutable generated assets after validated restore, without
    /// advancing the VM. The supplied host rejects writes and scene mutations.
    fn restore_resources(&mut self, _host: &mut dyn HostServices) -> Result<()> {
        Ok(())
    }
    /// Bounded JSON diagnostics for inspection; an empty value means unavailable.
    fn diagnostics(&self) -> Vec<u8> {
        Vec::new()
    }
    fn export_original_save(&mut self) -> Result<Vec<u8>> {
        Err(crate::Error::Unsupported("original save export"))
    }
    fn import_original_save(&mut self, _bytes: &[u8]) -> Result<()> {
        Err(crate::Error::Unsupported("original save import"))
    }
    /// Opt in only when native import validates/stages all fallible work before
    /// committing. On error, logical VM state and queued host requests must be
    /// unchanged. Lets native savepoints work without claiming runtime snapshots.
    fn original_save_import_is_transactional(&self) -> bool {
        false
    }
}
