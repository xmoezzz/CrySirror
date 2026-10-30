// SPDX-License-Identifier: MPL-2.0
pub mod codes;
pub mod counter;
pub mod excall;
pub mod frame_action;
pub mod frame_action_ch;
pub mod global;
pub mod input;
pub mod int_event;
pub mod int_list;
pub mod key;
pub mod math;
pub mod mouse;
pub mod mov;
mod object_event;
mod passive_object;
pub mod pcmch;
pub mod prop_access;
pub mod script;
pub mod stage;
pub mod str_list;
pub mod syscom;
mod syscom_flags;
mod syscom_preferences;
pub mod system;
pub mod timewait;
pub fn dispatch_form(
    ctx: &mut super::CommandContext,
    id: u32,
    args: &[super::Value],
) -> anyhow::Result<bool> {
    super::dispatch_form_code(ctx, id, args)
}
