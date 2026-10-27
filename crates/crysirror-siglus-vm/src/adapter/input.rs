// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Original VK/DECIDE semantic mapping over an engine-owned logical inbox.
use super::*;
impl SiglusVm {
    pub(super) fn apply_queued_input(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let owner = self.state.client;
        let batch = self.state.inputs.0.read(owner, 1)?;
        let vm = self.state.vm.as_mut().unwrap();
        vm.ctx.input.next_frame();
        if let Some(sample) = batch.samples.first() {
            let input = sample.frame;
            for (bit, key) in [
                (1, crysirror_scene::input::VmKey::Enter),
                (2, crysirror_scene::input::VmKey::Space),
                (4, crysirror_scene::input::VmKey::ArrowLeft),
                (8, crysirror_scene::input::VmKey::ArrowRight),
                (16, crysirror_scene::input::VmKey::ArrowUp),
                (32, crysirror_scene::input::VmKey::ArrowDown),
                (64, crysirror_scene::input::VmKey::Tab),
            ] {
                if input.buttons_pressed & bit != 0 {
                    vm.ctx.input.on_key_down(key);
                }
                if sample.released & bit != 0 {
                    vm.ctx.input.on_key_up(key);
                    // Original on_key_up only releases wait-key procs after a
                    // completed DECIDE down-up, not an arbitrary key-down.
                    if matches!(
                        key,
                        crysirror_scene::input::VmKey::Enter | crysirror_scene::input::VmKey::Space
                    ) {
                        let vk = crysirror_scene::input::vmkey_to_vk_code(key).unwrap();
                        if vm.ctx.input.vk_down_up_stock(vk) {
                            if crate::message::complete_on_decide(
                                &mut vm.ctx,
                                ClientId(self.state.client),
                                host,
                            )? {
                                continue;
                            }
                            let skipped = vm.ctx.wait.notify_movie_down_up(
                                &mut vm.ctx.globals,
                                &vm.ctx.ids,
                                1,
                            );
                            if !skipped {
                                vm.ctx.wait.notify_key(&mut vm.ctx.globals, &vm.ctx.ids);
                            }
                        }
                    }
                }
            }
            vm.ctx.input.on_mouse_move(input.pointer_x, input.pointer_y);
            let bit = 1 << 16;
            let mouse = crate::runtime::input::VmMouseButton::Left;
            if input.buttons_pressed & bit != 0 {
                vm.ctx.input.on_mouse_down(mouse);
            }
            if sample.released & bit != 0 {
                vm.ctx.input.on_mouse_up(mouse);
                if vm.ctx.input.vk_down_up_stock(0x01) {
                    let revealed = crate::message::complete_on_decide(
                        &mut vm.ctx,
                        ClientId(self.state.client),
                        host,
                    )?;
                    let skipped = !revealed
                        && vm
                            .ctx
                            .wait
                            .notify_movie_down_up(&mut vm.ctx.globals, &vm.ctx.ids, 1);
                    if !skipped && !revealed {
                        vm.ctx.wait.notify_key(&mut vm.ctx.globals, &vm.ctx.ids);
                    }
                }
            }
        }
        vm.ctx.sync_script_input_from_runtime();
        if let Some(receipt) = batch.receipt {
            self.state.inputs.0.acknowledge(owner, receipt)?;
        }
        Ok(())
    }
    pub(super) fn discard_queued_input(&mut self) -> Result<()> {
        self.state.inputs.0.discard(self.state.client)?;
        if let Some(f) = self.state.inputs.0.observed() {
            if let Some(vm) = &mut self.state.vm {
                vm.ctx.input.on_mouse_move(f.pointer_x, f.pointer_y);
            }
        }
        Ok(())
    }
}
