// SPDX-License-Identifier: MPL-2.0
use crate::runtime::CommandContext;
pub const CAPTURE_PRIOR_SAVE: i32 = 0;
pub fn prepare_runtime_save_thumb_capture(ctx: &mut CommandContext) {
    ctx.unknown
        .record_note("native save thumbnail capture unsupported");
}
pub fn free_runtime_save_thumb_capture(_: &mut CommandContext, _: i32) {}
pub fn write_runtime_slot_thumb(ctx: &mut CommandContext, _: usize) {
    ctx.unknown
        .record_note("native slot thumbnail write unsupported");
}
pub fn write_global_save(ctx: &CommandContext) {
    ctx.backend_faults
        .borrow_mut()
        .push("native global/config save backend unavailable".into());
}
pub fn read_save_flag_values(
    ctx: &mut CommandContext,
    quick: bool,
    idx: i64,
    count: usize,
) -> Option<Vec<i64>> {
    if idx < 0 {
        return None;
    }
    let slots = if quick {
        &ctx.globals.syscom.quick_save_slots
    } else {
        &ctx.globals.syscom.save_slots
    };
    let slot = slots.get(idx as usize).filter(|s| s.exist)?;
    Some(
        (0..count.min(256))
            .map(|i| slot.values.get(&(i as i32)).copied().unwrap_or(0))
            .collect(),
    )
}
pub fn write_save_flag_values(ctx: &mut CommandContext, quick: bool, idx: i64, v: &[i64]) -> bool {
    if idx < 0 {
        return false;
    }
    let slots = if quick {
        &mut ctx.globals.syscom.quick_save_slots
    } else {
        &mut ctx.globals.syscom.save_slots
    };
    let Some(slot) = slots.get_mut(idx as usize).filter(|s| s.exist) else {
        return false;
    };
    for (i, v) in v.iter().take(256).enumerate() {
        slot.values.insert(i as i32, *v);
    }
    true
}
pub fn append_current_save_message(ctx: &mut CommandContext, msg: &str) {
    ctx.globals.syscom.current_save_message.push_str(msg);
    ctx.globals.syscom.current_save_message = ctx
        .globals
        .syscom
        .current_save_message
        .chars()
        .take(256)
        .collect();
    ctx.globals.syscom.current_save_full_message.push_str(msg);
    ctx.globals.syscom.current_save_full_message = ctx
        .globals
        .syscom
        .current_save_full_message
        .chars()
        .take(256)
        .collect();
    if let Some(s) = &mut ctx.local_save_snapshot {
        s.save_msg = ctx.globals.syscom.current_save_message.clone();
        s.save_full_msg = ctx.globals.syscom.current_save_full_message.clone();
    }
}
