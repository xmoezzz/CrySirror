// SPDX-License-Identifier: MPL-2.0
use crate::runtime::{CommandContext, globals::BtnSelectRuntimeState};
pub fn prepare_stage_btnselitems(ctx: &mut CommandContext) {
    if !ctx.globals.selbtn.choices.is_empty() {
        ctx.unknown
            .record_note("native selection layout unsupported");
    }
}
pub fn prepare_saved_stage_btnselitems(
    ctx: &mut CommandContext,
    _: i64,
    state: BtnSelectRuntimeState,
) -> BtnSelectRuntimeState {
    if !state.choices.is_empty() {
        ctx.unknown
            .record_note("native saved selection layout unsupported");
    }
    state
}
