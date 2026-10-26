// SPDX-License-Identifier: MPL-2.0
use crate::runtime::CommandContext;
const EXCALL_LOCAL_NS_XOR: u32 = 0x4000;
pub(crate) fn is_stage_form_id(ctx: &CommandContext, form_id: i32) -> bool {
    let primary = ctx.ids.form_global_stage as i32;
    let canonical = crate::runtime::forms::codes::FORM_GLOBAL_STAGE as i32;
    let primary_local = if primary != 0 { primary ^ 0x4000 } else { 0 };
    let canonical_local = canonical ^ 0x4000;
    (primary != 0 && form_id == primary)
        || form_id == canonical
        || (primary != 0 && form_id == primary_local)
        || form_id == canonical_local
        || form_id == crate::runtime::constants::global_form::STAGE_DEFAULT as i32
        || form_id == crate::runtime::constants::global_form::BACK as i32
        || form_id == crate::runtime::constants::global_form::FRONT as i32
        || form_id == crate::runtime::constants::global_form::NEXT as i32
        || form_id == crate::runtime::constants::global_form::STAGE_ALIAS_37 as i32
        || form_id == crate::runtime::constants::global_form::STAGE_ALIAS_38 as i32
}

fn normal_stage_form_id(ctx: &CommandContext) -> u32 {
    if ctx.ids.form_global_stage != 0 {
        ctx.ids.form_global_stage
    } else {
        crate::runtime::forms::codes::FORM_GLOBAL_STAGE
    }
}

pub(crate) fn stage_storage_form_id(ctx: &CommandContext, raw_form_id: i32) -> u32 {
    let normal = normal_stage_form_id(ctx);
    let local = normal ^ EXCALL_LOCAL_NS_XOR;
    let canonical_local = crate::runtime::forms::codes::FORM_GLOBAL_STAGE ^ EXCALL_LOCAL_NS_XOR;
    if raw_form_id == local as i32 || raw_form_id == canonical_local as i32 {
        local
    } else {
        // BACK/FRONT/NEXT and old aliases all address the ordinary global
        // stage container, just as in C++ C_elm_excall only gets its own
        // container through the EXCALL child forwarding path.
        normal
    }
}

pub(crate) fn current_stage_form_id(ctx: &CommandContext) -> u32 {
    ctx.vm_call
        .as_ref()
        .and_then(|meta| meta.element.first().copied())
        .map(|raw| stage_storage_form_id(ctx, raw))
        .unwrap_or_else(|| normal_stage_form_id(ctx))
}

pub fn cd_text_current_mwnd(
    ctx: &mut CommandContext,
    text: &str,
    read_flag: i64,
) -> anyhow::Result<bool> {
    crate::message::append(ctx, text, read_flag)
}
pub fn cd_name_current_mwnd(ctx: &mut CommandContext, name: &str) -> anyhow::Result<bool> {
    crate::message::name(ctx, name)
}
pub fn restore_stage_form_backends_after_load(
    _: &mut CommandContext,
    st: &mut crate::runtime::globals::StageFormState,
) {
    st.rect_layers.clear();
}
/// Native selector parsing plus CPU object state. The original interpreter still
/// owns decoded values/argument overloads; shared layers are extracted afterward.
pub fn dispatch(ctx: &mut CommandContext, args: &[crate::runtime::Value]) -> anyhow::Result<bool> {
    use crate::runtime::{Value, forms::codes};
    use anyhow::{Context, bail, ensure};
    let call = ctx
        .vm_call
        .clone()
        .context("native stage missing call metadata")?;
    let chain = &call.element;
    let root = *chain.first().context("empty stage chain")?;
    let (stage_idx, mut p) = if root == codes::ELM_GLOBAL_BACK {
        (0, 1)
    } else if root == codes::ELM_GLOBAL_FRONT {
        (1, 1)
    } else if root == codes::ELM_GLOBAL_NEXT {
        (2, 1)
    } else {
        ensure!(
            chain.get(1) == Some(&ctx.ids.elm_array),
            "native stage requires array selector"
        );
        (i64::from(*chain.get(2).context("native stage index")?), 3)
    };
    ensure!(
        (0..=2).contains(&stage_idx),
        "native stage index unsupported"
    );
    let family = stage_storage_form_id(ctx, root);
    let op = *chain.get(p).context("native stage operation missing")?;
    if op == codes::ELM_STAGE_CREATE_OBJECT {
        let count = args
            .first()
            .and_then(Value::as_i64)
            .context("CREATE_OBJECT count")?;
        ensure!((0..=4096).contains(&count), "native object count");
        ctx.globals
            .stage_forms
            .entry(family)
            .or_default()
            .ensure_object_list(stage_idx, count as usize);
        if call.ret_form != 0 {
            ctx.push(Value::Int(0));
        }
        return Ok(true);
    }
    ensure!(
        op == codes::ELM_STAGE_OBJECT,
        "unsupported native stage member {op}"
    );
    p += 1;
    ensure!(
        chain.get(p) == Some(&ctx.ids.elm_array),
        "native object selector"
    );
    let index = *chain.get(p + 1).context("native object index")?;
    ensure!(index >= 0, "negative native object index");
    p += 2;
    let op = *chain.get(p).context("native object operation")?;
    ensure!(
        p + 1 == chain.len(),
        "unsupported native nested object chain {chain:?}"
    );
    let stage = ctx
        .globals
        .stage_forms
        .get_mut(&family)
        .context("native stage not created")?;
    let obj = stage
        .object_lists
        .get_mut(&stage_idx)
        .and_then(|v| v.get_mut(index as usize))
        .context("native object slot missing")?;
    let num = |i: usize| args.get(i).and_then(Value::as_i64).unwrap_or(0);
    if op == codes::ELM_OBJECT_CREATE_RECT {
        ensure!(args.len() >= 8, "CREATE_RECT requires bounds and RGBA");
        obj.init_type_like();
        obj.init_param_like();
        obj.used = true;
        obj.object_type = 1;
        obj.rect_param.left = num(0);
        obj.rect_param.top = num(1);
        obj.rect_param.right = num(2);
        obj.rect_param.bottom = num(3);
        let c = [num(4), num(5), num(6), num(7)].map(|v| v.clamp(0, 255));
        obj.rect_param.color_argb = (c[3] << 24) | (c[0] << 16) | (c[1] << 8) | c[2];
        obj.set_int_prop(
            &ctx.ids,
            ctx.ids.obj_disp,
            i64::from(args.len() >= 9 && num(8) != 0),
        );
        if args.len() >= 11 {
            obj.set_int_prop(&ctx.ids, ctx.ids.obj_x, num(9));
            obj.set_int_prop(&ctx.ids, ctx.ids.obj_y, num(10));
        }
    } else if op == codes::ELM_OBJECT_CREATE {
        let file = args
            .first()
            .and_then(Value::as_str)
            .context("OBJECT.CREATE filename")?;
        obj.init_type_like();
        obj.init_param_like();
        if !file.is_empty() {
            ensure!(
                !file.contains('?'),
                "native tonecurve create requires effect support"
            );
            obj.used = true;
            obj.object_type = 2;
            obj.file_name = Some(file.into());
            obj.set_int_prop(
                &ctx.ids,
                ctx.ids.obj_disp,
                i64::from(args.len() >= 2 && num(1) != 0),
            );
            if args.len() >= 4 {
                obj.set_int_prop(&ctx.ids, ctx.ids.obj_x, num(2));
                obj.set_int_prop(&ctx.ids, ctx.ids.obj_y, num(3));
            }
            if args.len() >= 5 {
                obj.set_int_prop(&ctx.ids, ctx.ids.obj_patno, num(4));
            }
            ctx.images.load_g00(
                file,
                usize::try_from(obj.base.patno).context("negative PATNO")?,
            )?;
        }
    } else if ctx.ids.object_op_tables().fixed_int_prop(op).is_some() {
        if call.al_id == 1 {
            obj.set_int_prop(&ctx.ids, op, num(0));
        } else {
            let value = obj.get_int_prop(&ctx.ids, op);
            ctx.push(Value::Int(value));
            return Ok(true);
        }
    } else {
        bail!("unsupported Siglus object operation {op} chain {chain:?}")
    }
    if call.ret_form != 0 {
        ctx.push(Value::Int(0));
    }
    Ok(true)
}
