// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native window preferences -> shared retained UI requests, no OS calls.
use super::*;
#[derive(Default)]
pub(super) struct Presentation {
    fullscreen: Option<bool>,
}
impl SiglusVm {
    pub(super) fn sync_presentation(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let Some(vm) = self.state.vm.as_ref() else {
            return Ok(());
        };
        let value = vm
            .ctx
            .globals
            .syscom
            .config_int
            .get(&crate::runtime::forms::codes::syscom_op::GET_WINDOW_MODE)
            .copied()
            .unwrap_or(0);
        // The native presenter uses mode==0 for windowed, any other raw value
        // for fullscreen; only the default/reset path clamps authored config.
        let fullscreen = value != 0;
        if self.presentation.fullscreen != Some(fullscreen) {
            let batch = crysirror_scene::ServiceBatch {
                ui: vec![crysirror_scene::ui::UiCommand::SetFullscreen {
                    enabled: fullscreen,
                }],
                ..Default::default()
            };
            host.submit_engine(ClientId(self.state.client), 1, &serde_json::to_vec(&batch)?)
                .map_err(err)?;
            self.presentation.fullscreen = Some(fullscreen);
        }
        Ok(())
    }
}
