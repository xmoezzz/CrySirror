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

/// Native current-window/scene-title references are VM/save policy; actual
/// message rendering and window presentation remain shared service operations.
pub fn dispatch_metadata(
    ctx: &mut CommandContext,
    form: u32,
    args: &[crate::runtime::Value],
) -> anyhow::Result<bool> {
    use crate::runtime::{Value, forms::codes};
    use anyhow::{Context, ensure};
    let op = form as i32;
    if !matches!(
        op,
        codes::ELM_GLOBAL_GET_MWND
            | codes::ELM_GLOBAL_SET_MWND
            | codes::ELM_GLOBAL_GET_SEL_MWND
            | codes::ELM_GLOBAL_SET_SEL_MWND
            | codes::ELM_GLOBAL_GET_TITLE
            | codes::ELM_GLOBAL_SET_TITLE
    ) {
        return Ok(false);
    }
    let call = ctx.vm_call.as_ref().context("native global metadata")?;
    ensure!(
        call.element.as_slice() == [op],
        "native global metadata chain"
    );
    match op {
        codes::ELM_GLOBAL_GET_MWND | codes::ELM_GLOBAL_GET_SEL_MWND => {
            ensure!(
                args.is_empty() && call.al_id == 0,
                "native current MWND getter arguments"
            );
            let no = if op == codes::ELM_GLOBAL_GET_MWND {
                ctx.globals.current_mwnd_no
            } else {
                ctx.globals.current_sel_mwnd_no
            };
            ctx.push(Value::Int(no.map_or(-1, |n| n as i64)));
        }
        codes::ELM_GLOBAL_SET_MWND | codes::ELM_GLOBAL_SET_SEL_MWND => {
            ensure!(args.len() == 1, "native current MWND setter arguments");
            let element = match (call.al_id, args[0].unwrap_named()) {
                (0, Value::Element(chain)) => chain.clone(),
                (1, Value::Int(no)) if (0..16).contains(no) => vec![
                    codes::ELM_GLOBAL_FRONT,
                    codes::ELM_STAGE_MWND,
                    codes::ELM_ARRAY,
                    *no as i32,
                ],
                _ => anyhow::bail!("native current MWND setter overload/value"),
            };
            let reference = match element.as_slice() {
                [root, codes::ELM_STAGE_MWND, codes::ELM_ARRAY, no] => {
                    let stage = match *root {
                        codes::ELM_GLOBAL_BACK => 0,
                        codes::ELM_GLOBAL_FRONT => 1,
                        codes::ELM_GLOBAL_NEXT => 2,
                        _ => anyhow::bail!("native MWND stage alias"),
                    };
                    (stage, *no)
                }
                [
                    codes::ELM_GLOBAL_STAGE,
                    codes::ELM_ARRAY,
                    stage,
                    codes::ELM_STAGE_MWND,
                    codes::ELM_ARRAY,
                    no,
                ] => (i64::from(*stage), *no),
                _ => anyhow::bail!("native current MWND element profile"),
            };
            ensure!(
                (0..=2).contains(&reference.0) && (0..16).contains(&reference.1),
                "native current MWND bounds"
            );
            if op == codes::ELM_GLOBAL_SET_MWND {
                ctx.globals.current_mwnd_element = element;
                ctx.globals.current_mwnd_stage_idx = reference.0;
                ctx.globals.current_mwnd_no = Some(reference.1 as usize);
            } else {
                ctx.globals.current_sel_mwnd_element = element;
                ctx.globals.current_sel_mwnd_stage_idx = reference.0;
                ctx.globals.current_sel_mwnd_no = Some(reference.1 as usize);
            }
        }
        codes::ELM_GLOBAL_GET_TITLE => {
            ensure!(
                args.is_empty() && call.al_id == 0,
                "native scene title getter arguments"
            );
            ctx.push(Value::Str(
                ctx.globals.syscom.current_save_scene_title.clone(),
            ));
        }
        codes::ELM_GLOBAL_SET_TITLE => {
            ensure!(
                args.len() == 1 && call.al_id == 0,
                "native scene title setter arguments"
            );
            let title = args[0].as_str().context("native scene title")?;
            ensure!(title.len() <= 65536, "native scene title budget");
            ctx.globals.syscom.current_save_scene_title = title.into();
        }
        _ => unreachable!(),
    }
    Ok(true)
}
