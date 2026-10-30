// SPDX-License-Identifier: MPL-2.0
//! Original global MOV opcode/argument/wait policy over shared engine Media.
//! Reference: siglus_rs 5f358ef runtime/forms/mov.rs; no decoder or mixer here.
use super::{codes::mov_op, prop_access};
use crate::runtime::{CommandContext, ProcKind, Value};
use anyhow::{Context, Result, bail, ensure};
fn integer(args: &[Value], index: usize, default: i64) -> Result<i64> {
    match args.get(index) {
        Some(v) => v.as_i64().context("MOV expects integer argument"),
        None => Ok(default),
    }
}
pub fn dispatch(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let op = prop_access::current_op_from_ctx_or_args(ctx, args)
        .context("MOV expects element opcode")?;
    let args = prop_access::params_without_op(ctx, args);
    match op {
        mov_op::PLAY | mov_op::PLAY_WAIT | mov_op::PLAY_WAIT_KEY => {
            ensure!(args.len() <= 5, "MOV argument count");
            let name = args
                .first()
                .and_then(Value::as_str)
                .context("MOV expects movie name")?
                .to_owned();
            let x = i32::try_from(integer(args, 1, 0)?)?;
            let y = i32::try_from(integer(args, 2, 0)?)?;
            let raw_w = integer(args, 3, i64::from(ctx.screen_w))?;
            let raw_h = integer(args, 4, i64::from(ctx.screen_h))?;
            let width = if raw_w <= 0 {
                ctx.screen_w
            } else {
                u32::try_from(raw_w)?
            };
            let height = if raw_h <= 0 {
                ctx.screen_h
            } else {
                u32::try_from(raw_h)?
            };
            ensure!(
                (1..=32767).contains(&width) && (1..=32767).contains(&height),
                "MOV canvas limit"
            );
            ctx.movie.play(&name)?;
            let key_skip = op == mov_op::PLAY_WAIT_KEY;
            ctx.globals
                .mov
                .start(name, x, y, width, height, None, key_skip);
            if op != mov_op::PLAY {
                let ret = prop_access::current_vm_meta(ctx).1.unwrap_or(0);
                ctx.wait
                    .wait_global_movie(key_skip, ret != super::codes::FM_VOID as i64);
                ctx.request_wait_proc_boundary(ProcKind::MovieWait);
            }
            Ok(true)
        }
        mov_op::STOP => {
            ensure!(args.is_empty(), "MOV.STOP argument count");
            ctx.globals.mov.stop();
            ctx.movie.stop();
            Ok(true)
        }
        _ => bail!("unsupported Siglus MOV opcode {op}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{VmCallMeta, forms::codes};
    fn start(op: i32) -> CommandContext {
        let mut ctx = CommandContext::new("asset".into());
        ctx.vm_call = Some(VmCallMeta {
            element: vec![codes::FORM_GLOBAL_MOV as i32, op],
            ret_form: codes::FM_INT as i64,
            ..Default::default()
        });
        dispatch(
            &mut ctx,
            &[
                Value::Str("opening".into()),
                Value::Int(12),
                Value::Int(24),
                Value::Int(320),
                Value::Int(180),
            ],
        )
        .unwrap();
        ctx
    }
    #[test]
    fn global_movie_end_and_decide_cancel_preserve_original_returns() {
        let mut end = start(mov_op::PLAY_WAIT);
        assert!(end.wait_poll());
        assert_eq!(
            (
                end.globals.mov.x,
                end.globals.mov.y,
                end.globals.mov.width,
                end.globals.mov.height
            ),
            (12, 24, 320, 180)
        );
        end.globals.mov.playing = false;
        assert!(!end.wait_poll());
        assert_eq!(end.pop().and_then(|v| v.as_i64()), Some(0));
        for result in [1, 2] {
            let mut key = start(mov_op::PLAY_WAIT_KEY);
            assert!(
                key.wait
                    .notify_movie_down_up(&mut key.globals, &key.ids, result)
            );
            assert!(!key.globals.mov.playing);
            assert!(!key.wait_poll());
            assert_eq!(key.pop().and_then(|v| v.as_i64()), Some(result));
        }
        let mut unskippable = start(mov_op::PLAY_WAIT);
        assert!(!unskippable.wait.notify_movie_down_up(
            &mut unskippable.globals,
            &unskippable.ids,
            1
        ));
        assert!(unskippable.globals.mov.playing);
    }
    #[test]
    fn unknown_opcode_and_invalid_dimensions_are_visible_errors() {
        let mut ctx = CommandContext::new("asset".into());
        ctx.vm_call = Some(VmCallMeta {
            element: vec![codes::FORM_GLOBAL_MOV as i32, 99],
            ..Default::default()
        });
        assert!(
            dispatch(&mut ctx, &[])
                .unwrap_err()
                .to_string()
                .contains("opcode 99")
        );
        ctx.vm_call.as_mut().unwrap().element[1] = mov_op::PLAY;
        assert!(
            dispatch(
                &mut ctx,
                &[Value::Str("opening".into()), Value::Int(i64::MAX)]
            )
            .is_err()
        );
        assert!(!ctx.movie.live);
    }
}
