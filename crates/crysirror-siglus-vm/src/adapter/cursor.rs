// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native Gameexe cursor policy on client-scoped engine UI definitions.
use super::*;
use crysirror_scene::{
    ServiceBatch,
    ui::{
        UiCommand,
        cursor::{CursorFrameSpec, CursorSelect, CursorSpec, StandardCursor},
    },
};
#[derive(Default)]
pub(super) struct Cursors {
    defined: BTreeSet<i64>,
    selected: Option<i64>,
    visible: Option<bool>,
}
pub(super) fn default_no(ctx: &CommandContext) -> Result<i64> {
    let Some(cfg) = &ctx.tables.gameexe else {
        return Ok(-1);
    };
    let count = cfg
        .get_unquoted("MOUSE_CURSOR.CNT")
        .map(str::parse::<i64>)
        .transpose()?
        .unwrap_or(16)
        .clamp(0, 256);
    let no = cfg
        .get_unquoted("MOUSE_CURSOR.DEFAULT")
        .map(str::parse::<i64>)
        .transpose()?
        .unwrap_or(-1);
    Ok(if no >= 0 && no < count { no } else { -1 })
}
impl Cursors {
    fn sync(
        &mut self,
        ctx: &mut CommandContext,
        client: ClientId,
        host: &mut dyn HostServices,
    ) -> Result<()> {
        let no = ctx.globals.script.cursor_no;
        let visible =
            ctx.globals.script.cursor_runtime_visible && !ctx.globals.script.cursor_disp_off;
        let mut batch = ServiceBatch::default();
        if self.selected != Some(no) {
            let selection = if no < 0 {
                CursorSelect::Standard {
                    shape: StandardCursor::Default,
                }
            } else {
                let cfg = ctx
                    .tables
                    .gameexe
                    .as_ref()
                    .context("native custom cursor requires Gameexe")?;
                let count = cfg
                    .get_unquoted("MOUSE_CURSOR.CNT")
                    .map(str::parse::<i64>)
                    .transpose()?
                    .unwrap_or(16)
                    .clamp(0, 256);
                ensure!(no < count, "native cursor index outside configured table");
                if !self.defined.contains(&no) {
                    let file = cfg
                        .get_indexed_field_unquoted("MOUSE_CURSOR", no as usize, "FILE")
                        .filter(|s| !s.is_empty())
                        .context("native cursor file missing")?;
                    let speed = cfg
                        .get_indexed_field_unquoted("MOUSE_CURSOR", no as usize, "SPEED")
                        .map(str::parse::<i64>)
                        .transpose()?
                        .unwrap_or(100);
                    let frames = ctx.images.album_frames(file)?;
                    ensure!(frames.len() <= 256, "native cursor frame limit");
                    let frames=frames.into_iter().map(|id|->Result<CursorFrameSpec>{
                        let image=&ctx.images.images[id];
                        ensure!(image.width==32 && image.height==32,"native Siglus cursor must be 32x32");
                        ensure!((0..32).contains(&image.center_x) && (0..32).contains(&image.center_y),"native cursor hotspot outside image requires a padded cursor profile");
                        Ok(CursorFrameSpec{width:32,height:32,hotspot:[image.center_x as u32,image.center_y as u32],rgba:image.rgba.clone()})
                    }).collect::<Result<Vec<_>>>()?;
                    let steps = if speed <= 0 {
                        vec![[0, 0]]
                    } else {
                        let ms = u32::try_from(speed).context("native cursor step duration")?;
                        (0..frames.len()).map(|i| [i as u32, ms]).collect()
                    };
                    let cursor = CursorSpec { frames, steps };
                    cursor.validate()?;
                    batch.ui.push(UiCommand::DefineCursor {
                        id: no as u32,
                        cursor,
                    });
                }
                CursorSelect::Custom { id: no as u32 }
            };
            batch.ui.push(UiCommand::SetCursor { cursor: selection });
        }
        if self.visible != Some(visible) {
            batch.ui.push(UiCommand::SetCursorVisible { visible });
        }
        if !batch.ui.is_empty() {
            host.submit_engine(client, 1, &serde_json::to_vec(&batch)?)?;
        }
        if no >= 0 {
            self.defined.insert(no);
        }
        self.selected = Some(no);
        self.visible = Some(visible);
        Ok(())
    }
}
impl SiglusVm {
    pub(super) fn sync_cursor(&mut self, host: &mut dyn HostServices) -> Result<()> {
        for _ in 0..4 {
            self.state.io.missing.clear();
            let result = io_fs::scoped(&mut self.state.io, || {
                self.cursors.sync(
                    &mut self.state.vm.as_mut().unwrap().ctx,
                    ClientId(self.state.client),
                    host,
                )
            });
            if self.state.io.missing.is_empty() {
                return result;
            }
            for key in std::mem::take(&mut self.state.io.missing) {
                self.fetch(host, &key)?;
            }
        }
        anyhow::bail!("native cursor resource admission retry limit")
    }
}
