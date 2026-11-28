// SPDX-License-Identifier: MPL-2.0
//! Supporting-character selector shared by the two COM3D2 1.0.4 managers.
use super::*;

pub(super) const PREFIX: &str = "UI Root/SubCharacterSelectPanel/";
const VISIBLE: usize = 9;

#[derive(Default)]
pub(super) struct Picker {
    count: usize,
    roster: Vec<(usize, Vec<Widget>)>,
    selected: Vec<usize>,
    offset: usize,
}

impl Picker {
    fn scroll(&mut self, back: bool) {
        self.offset = if back {
            self.offset.saturating_sub(1)
        } else {
            (self.offset + 1).min(self.roster.len().saturating_sub(VISIBLE))
        };
    }
}

impl Game {
    pub(super) fn open_sub_cast(
        &mut self,
        destination: Destination,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
    ) -> Result<String> {
        let template = self.assets.scene(op, host, client, "PickerPlate")?;
        let mut vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let state = &mut vm.runtime.host_mut().state;
        let (count, ids) = crate::tjs::retail::sub_cast(state, destination == OldYotogi)?;
        ensure!(count < 18, "Yotogi supporting character slot limit");
        let mut roster = Vec::with_capacity(ids.len());
        for id in ids {
            let mut row = Vec::new();
            for mut w in template.iter().cloned() {
                if business::prepare_plate(&mut w, id, true, state, &mut self.assets, host, client)?
                {
                    row.push(w);
                }
            }
            roster.push((id, row));
        }
        let backdrop = if state.stage_prefab.is_empty() {
            state.bg_name.clone()
        } else {
            state.stage_prefab.clone()
        };
        drop(vm);
        self.leisure.native_menu.sub_cast = Picker {
            count,
            roster,
            ..Default::default()
        };
        self.leisure.native_menu.page = Page::SubCast;
        self.draw_native_menu(destination)?;
        Ok(backdrop)
    }

    pub(super) fn draw_sub_cast(&mut self) -> Result<()> {
        let picker = &self.leisure.native_menu.sub_cast;
        let cancel = self
            .script
            .tag_backup
            .get("cancel_label")
            .is_some_and(|label| !label.is_empty());
        self.widgets
            .retain(|w| cancel || !w.path.starts_with(&format!("{PREFIX}Cancel")));
        if picker.count == 0 || picker.selected.len() != picker.count {
            for w in self
                .widgets
                .iter_mut()
                .filter(|w| w.path == format!("{PREFIX}Ok"))
            {
                w.collider = None;
                w.tint[3] = w.tint[3].min(100);
            }
        }
        for (i, (id, row)) in picker
            .roster
            .iter()
            .skip(picker.offset)
            .take(VISIBLE)
            .enumerate()
        {
            for base in row {
                if base.path.contains("/Select") && !picker.selected.contains(id) {
                    continue;
                }
                let mut w = base.clone();
                business::move_by(&mut w, -700., -354. + i as f32 * 92.);
                w.depth += 200;
                w.path = format!("sub-cast/{id}/{}", base.path);
                self.widgets.push(w);
            }
        }
        Ok(())
    }

    pub(super) fn sub_cast_scroll(
        &mut self,
        destination: Destination,
        input: InputFrame,
    ) -> Result<()> {
        let wheel = input.buttons_pressed & (buttons::WHEEL_UP | buttons::WHEEL_DOWN);
        if wheel != 0 && self.leisure.pressed.is_none() {
            self.leisure
                .native_menu
                .sub_cast
                .scroll(wheel & buttons::WHEEL_UP != 0);
            self.draw_native_menu(destination)?;
        }
        Ok(())
    }

    pub(super) fn sub_cast_click(
        &mut self,
        destination: Destination,
        path: &str,
        op: &mut Opener,
        host: &mut dyn HostServices,
        client: ClientId,
        now: u64,
    ) -> Result<()> {
        let picker = &mut self.leisure.native_menu.sub_cast;
        if let Some((id, _)) = path
            .strip_prefix("sub-cast/")
            .and_then(|p| p.split_once('/'))
        {
            let id = id.parse::<usize>()?;
            ensure!(
                picker.roster.iter().any(|(entry, _)| *entry == id),
                "unknown supporting character"
            );
            if picker.count == 1 {
                picker.selected.clear();
                picker.selected.push(id);
            } else if let Some(index) = picker.selected.iter().position(|entry| *entry == id) {
                picker.selected.remove(index);
            } else if picker.selected.len() < picker.count {
                picker.selected.push(id);
            }
        } else if let Some(control) = path.strip_prefix(PREFIX) {
            match control {
                "Cancel" => return self.resume_at("cancel_label", op, host, client, now),
                "Ok" if picker.count != 0 && picker.selected.len() == picker.count => {
                    ensure!(
                        self.script.tag_backup.contains_key("label"),
                        "missing supporting-character return label"
                    );
                    let ids = picker.selected.clone();
                    {
                        let mut vm = self
                            .script
                            .tjs
                            .lock()
                            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                        let s = &mut vm.runtime.host_mut().state;
                        ensure!(
                            ids.iter().all(|id| s.retail.stock_models.contains_key(id)),
                            "missing supporting character model"
                        );
                        s.active_maids
                            .resize(s.active_maids.len().max(ids.len() + 1), None);
                        for (i, id) in ids.iter().enumerate() {
                            s.active_maids[i + 1] = Some(*id);
                        }
                    }
                    for (i, id) in ids.into_iter().enumerate() {
                        let slot = i + 1;
                        self.actors
                            .activate_stock(&self.script, id, slot, Some(true))?;
                        self.actors.apply(
                            crate::kag::AdvUi::AllProcProp { man: false, slot },
                            &self.script,
                            &self.files,
                            &mut self.assets,
                            op,
                            now,
                        )?;
                    }
                    return self.resume_at("label", op, host, client, now);
                }
                _ if control.ends_with("/Up") || control.ends_with("/Down") => {
                    picker.scroll(control.ends_with("/Up"));
                }
                _ => {}
            }
        }
        self.draw_native_menu(destination)
    }
}
