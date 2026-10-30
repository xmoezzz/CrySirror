// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Original EXCALL process boundaries retain their caller's native wait.
//! No new clock, worker, callback interpreter or rendering host lives here.
use super::*;
impl SiglusVm {
    pub(super) fn sync_native_proc_requests(&mut self) -> Result<()> {
        let Some(vm) = self.state.vm.as_mut() else {
            return Ok(());
        };
        if vm.take_script_proc_pop_request() {
            vm.ctx.wait = vm
                .ctx
                .native_proc_waits
                .pop()
                .context("native EXCALL popped without a suspended caller wait")?;
            vm.ctx.input.clear_all();
            vm.ctx.script_input.clear_all();
        }
        if vm.take_script_proc_request() {
            ensure!(
                vm.ctx.native_proc_waits.len() < 64,
                "native EXCALL process depth budget"
            );
            let wait = std::mem::take(&mut vm.ctx.wait);
            vm.ctx.native_proc_waits.push(wait);
            vm.ctx.input.use_current();
            vm.ctx.script_input.use_current();
        }
        Ok(())
    }
}
