// SPDX-License-Identifier: MPL-2.0
pub fn finalize_pending_free(ctx: &mut crate::runtime::CommandContext) {
    if ctx.excall_state.pending_free {
        ctx.unknown.record_note("EXCALL finalization not supported");
    }
}
