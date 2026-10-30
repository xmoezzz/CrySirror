// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native SYSTEM policy over capability storage and per-VM logical state.
//! ABI reference: siglus_rs 5f358ef, runtime/forms/system.rs and compiler table.
use crate::runtime::{CommandContext, Value};
use anyhow::{Context, Result, bail, ensure};

pub fn dispatch(ctx: &mut CommandContext, args: &[Value]) -> Result<bool> {
    let call = ctx.vm_call.as_ref().context("native SYSTEM metadata")?;
    ensure!(
        call.element.len() == 2 && call.al_id == 0,
        "unsupported native SYSTEM chain/overload"
    );
    let op = call.element[1];
    match op {
        0 => {
            ensure!(args.is_empty(), "SYSTEM.CHECK_ACTIVE arguments");
            ctx.push(Value::Int(i64::from(ctx.globals.system.active_flag)));
        }
        6 | 12 => {
            ensure!(
                args.len() == 1,
                "SYSTEM.CHECK_FILE_EXIST requires one filename"
            );
            let name = args[0].as_str().context("native file-existence filename")?;
            let path = std::path::PathBuf::from(if op == 6 { "asset" } else { "save" })
                .join(name.replace('\\', "/"));
            // probe_file validates the capability path before queuing a query.
            let present = crate::io_fs::probe_file(path)?;
            ctx.push(Value::Int(i64::from(present)));
        }
        13 => {
            ensure!(args.is_empty(), "SYSTEM.CHECK_DEBUG_FLAG arguments");
            ctx.push(Value::Int(i64::from(ctx.globals.system.debug_flag)));
        }
        16 => {
            ensure!(args.is_empty(), "SYSTEM.GET_LANGUAGE arguments");
            ctx.push(Value::Str(ctx.globals.system.language_code.clone()));
        }
        21 => {
            ensure!(args.is_empty(), "SYSTEM.CLEAR_DUMMY_FILE arguments");
            ctx.globals.system.dummy_checks.clear();
        }
        2 => {
            ensure!(
                args.len() == 3
                    && args[0].as_str().is_some()
                    && args[1].as_i64().is_some()
                    && args[2].as_str().is_some(),
                "SYSTEM.CHECK_DUMMY_FILE_ONCE requires filename, key and code"
            );
            let name = args[0].as_str().unwrap();
            let key = args[1].as_i64().unwrap();
            let code = args[2].as_str().unwrap();
            ensure!(
                !name.is_empty()
                    && name.len() <= 4096
                    && code.len() <= 4096
                    && i32::try_from(key).is_ok(),
                "native optical-disc check parameter bounds"
            );
            // Owner-selected CrySirror policy: the original Windows optical
            // host check is inapplicable here. Consume its void ABI and retain
            // once identity, but never report successful disc authentication.
            ctx.globals
                .system
                .dummy_checks
                .insert(format!("{name}:{key}:{code}"));
        }
        _ => bail!("unsupported Siglus SYSTEM operation {op}"),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(op: i32) -> CommandContext {
        let mut ctx = CommandContext::new("asset".into());
        ctx.vm_call = Some(crate::runtime::VmCallMeta {
            element: vec![92, op],
            ..Default::default()
        });
        ctx
    }
    #[test]
    fn file_presence_is_pending_until_capability_observation_and_rejects_escape() {
        let mut state = crate::io_fs::State::default();
        let mut context = ctx(6);
        crate::io_fs::scoped(&mut state, || {
            assert!(dispatch(&mut context, &[Value::Str("g00\\picture.g00".into())]).is_err());
            assert!(context.stack.is_empty());
        });
        assert!(state.pending_probes.contains("asset/g00/picture.g00"));
        state.probes.insert("asset/g00/picture.g00".into(), true);
        crate::io_fs::scoped(&mut state, || {
            dispatch(&mut context, &[Value::Str("g00/picture.g00".into())])
        })
        .unwrap();
        assert_eq!(context.pop().and_then(|v| v.as_i64()), Some(1));
        for name in ["../other/file", "/etc/passwd", "C:\\secret"] {
            crate::io_fs::scoped(&mut state, || {
                assert!(dispatch(&mut context, &[Value::Str(name.into())]).is_err())
            });
        }
        context.vm_call.as_mut().unwrap().element[1] = 12;
        state.probes.insert("save/slot.sav".into(), false);
        crate::io_fs::scoped(&mut state, || {
            dispatch(&mut context, &[Value::Str("slot.sav".into())])
        })
        .unwrap();
        assert_eq!(context.pop().and_then(|v| v.as_i64()), Some(0));
    }
    #[test]
    fn native_system_queries_and_clear_preserve_client_state_without_fake_platform_ops() {
        let mut context = ctx(13);
        context.globals.system.debug_flag = true;
        dispatch(&mut context, &[]).unwrap();
        assert_eq!(context.pop().and_then(|v| v.as_i64()), Some(1));
        context.vm_call.as_mut().unwrap().element[1] = 16;
        context.globals.system.language_code = "JP".into();
        dispatch(&mut context, &[]).unwrap();
        assert_eq!(
            context.pop().and_then(|v| v.as_str().map(str::to_owned)),
            Some("JP".into())
        );
        context.vm_call.as_mut().unwrap().element[1] = 21;
        context
            .globals
            .system
            .dummy_checks
            .insert("previous verified observation".into());
        dispatch(&mut context, &[]).unwrap();
        assert!(context.globals.system.dummy_checks.is_empty());
        context.vm_call.as_mut().unwrap().element[1] = 0;
        context.globals.system.active_flag = false;
        dispatch(&mut context, &[]).unwrap();
        assert_eq!(
            context.pop().and_then(|v| v.as_i64()),
            Some(0),
            "inactive host must not be reported active"
        );
    }
}
