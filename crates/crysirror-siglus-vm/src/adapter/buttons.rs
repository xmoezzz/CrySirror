// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! UI observation -> owned native intent -> original VM callback admission.
//! Hit-testing, focus, gesture completion and activation identity belong to UI.
use super::*;
use crate::message::buttons as native;
use crysirror_scene::ui::WidgetInteraction;
#[derive(Default)]
pub(super) struct Buttons {
    pub published: BTreeSet<u32>,
    observations: BTreeMap<u32, WidgetInteraction>,
    consumed: BTreeMap<u32, u32>,
    pending: Vec<Intent>,
    pointer: Option<[i32; 2]>,
}
struct Intent {
    index: usize,
    file: Option<String>,
    action: Option<crate::runtime::globals::PendingButtonAction>,
    sound: Option<&'static str>,
}
impl Buttons {
    pub(super) fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub(super) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub(super) fn reset_input(&mut self) {
        self.observations.clear();
        self.consumed.clear();
        self.pending.clear();
        self.pointer = None;
    }
}
impl SiglusVm {
    pub(super) fn observe_buttons(
        &mut self,
        host: &mut dyn HostServices,
        input: InputFrame,
    ) -> Result<()> {
        let moved = self
            .buttons
            .pointer
            .is_some_and(|old| old != [input.pointer_x, input.pointer_y]);
        self.buttons.pointer = Some([input.pointer_x, input.pointer_y]);
        for id in self.buttons.published.iter().copied() {
            let bytes = host
                .query_engine(
                    ClientId(self.state.client),
                    crysirror_core::service::EngineQuery::UiInteraction(id),
                )
                .map_err(err)?;
            let state: Option<WidgetInteraction> = serde_json::from_slice(&bytes)?;
            let state = state.context("native WAKU engine widget removed outside adapter")?;
            let index = (id - native::WIDGET_BASE) as usize;
            let Some(object) = self
                .state
                .vm
                .as_ref()
                .and_then(|v| native::native(&v.ctx))
                .and_then(|w| w.button_list.get(index))
            else {
                continue;
            };
            let before = self.buttons.observations.insert(id, state);
            let hover = moved && state.hovered && !before.is_some_and(|s| s.hovered);
            let push = state.pressed && !before.is_some_and(|s| s.pressed);
            if hover || push {
                self.buttons.pending.push(Intent {
                    index,
                    file: object.file_name.clone(),
                    action: None,
                    sound: Some(if push { "PUSH" } else { "HIT" }),
                });
            }
            if state.activated {
                ensure!(
                    state.activation_serial != 0,
                    "native UI host lacks activation identity"
                );
                if self.buttons.consumed.get(&id) != Some(&state.activation_serial) {
                    self.buttons.consumed.insert(id, state.activation_serial);
                    self.buttons.pending.push(Intent {
                        index,
                        file: object.file_name.clone(),
                        action: native::decided(object),
                        sound: Some("DECIDE"),
                    });
                }
            }
            ensure!(
                self.buttons.pending.len() <= 256,
                "native UI intent queue budget"
            );
        }
        Ok(())
    }
    pub(super) fn commit_button_inputs(&mut self) -> Result<()> {
        let Some(vm) = self.state.vm.as_mut() else {
            return Ok(());
        };
        fn flags(ctx: &mut CommandContext, observations: &BTreeMap<u32, WidgetInteraction>) {
            if let Some(window) = native::native_mut(ctx) {
                for (index, object) in window.button_list.iter_mut().enumerate() {
                    let state = observations.get(&(native::WIDGET_BASE + index as u32));
                    object.button.last_hit = object.button.hit;
                    object.button.last_pushed = object.button.pushed;
                    object.button.hit = window.open && state.is_some_and(|s| s.hovered);
                    object.button.pushed = window.open && state.is_some_and(|s| s.pressed);
                }
            }
        }
        if self.buttons.pending.is_empty() {
            flags(&mut vm.ctx, &self.buttons.observations);
            return Ok(());
        }
        // Admission is atomic: validation or preparation errors preserve the
        // observed intents and the VM's prior state instead of losing clicks.
        let mut next = vm.ctx.clone();
        flags(&mut next, &self.buttons.observations);
        for intent in &self.buttons.pending {
            let object = native::native(&next)
                .and_then(|w| w.button_list.get(intent.index))
                .context("native UI activation target vanished")?;
            ensure!(
                object.file_name == intent.file,
                "native UI activation target changed while preparing"
            );
            if intent.action.is_some() {
                ensure!(
                    serde_json::to_value(native::decided(object))?
                        == serde_json::to_value(&intent.action)?,
                    "native UI callback declaration changed while preparing"
                );
            }
            let se = object.button.se_no;
            if let Some(sound) = intent.sound {
                crate::sfx_bridge::button_sound(&mut next, se, sound)?;
            }
            if let Some(action) = intent.action.clone() {
                next.globals.pending_button_actions.push(action);
            }
        }
        vm.ctx = next;
        self.buttons.pending.clear();
        Ok(())
    }
}
