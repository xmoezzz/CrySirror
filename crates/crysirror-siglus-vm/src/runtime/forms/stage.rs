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
    use anyhow::{Context, ensure};
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
    if op == codes::ELM_STAGE_MWND {
        ensure!(stage_idx == 1, "native MWND bridge supports FRONT only");
        if chain.get(p + 3) == Some(&codes::ELM_MWND_BUTTON) {
            ensure!(
                chain.len() >= p + 7
                    && chain[p + 1] == ctx.ids.elm_array
                    && chain[p + 4] == ctx.ids.elm_array,
                "native MWND button chain"
            );
            let window = usize::try_from(chain[p + 2]).context("native MWND button window")?;
            let index = usize::try_from(chain[p + 5]).context("native MWND button index")?;
            ensure!(
                window == 0 && index < 256,
                "native WAKU button profile/budget"
            );
            let op = chain[p + 6];
            if chain.len() == p + 7
                && matches!(
                    op,
                    codes::ELM_OBJECT_GET_BUTTON_HIT_STATE
                        | codes::ELM_OBJECT_GET_BUTTON_STATE
                        | codes::ELM_OBJECT_GET_BUTTON_REAL_STATE
                        | codes::ELM_OBJECT_GET_BUTTON_NO
                        | codes::ELM_OBJECT_GET_BUTTON_PUSHKEEP
                        | codes::ELM_OBJECT_GET_BUTTON_ALPHA_TEST
                )
            {
                ensure!(
                    call.al_id == 0 && args.is_empty(),
                    "native button getter arguments"
                );
                let w = crate::message::buttons::native(ctx);
                let object = w.and_then(|w| w.button_list.get(index));
                let value = match op {
                    codes::ELM_OBJECT_GET_BUTTON_HIT_STATE => {
                        i64::from(w.is_some_and(|w| w.open) && object.is_some_and(|o| o.button.hit))
                    }
                    codes::ELM_OBJECT_GET_BUTTON_STATE => object.map_or(0, |o| o.button.state),
                    codes::ELM_OBJECT_GET_BUTTON_REAL_STATE => {
                        object.map_or(0, |o| crate::message::buttons::real_state(ctx, index, o))
                    }
                    codes::ELM_OBJECT_GET_BUTTON_NO => object.map_or(0, |o| o.button.button_no),
                    codes::ELM_OBJECT_GET_BUTTON_PUSHKEEP => {
                        i64::from(object.is_some_and(|o| o.button.push_keep))
                    }
                    _ => i64::from(object.is_some_and(|o| o.button.alpha_test)),
                };
                ctx.push(Value::Int(value));
                return Ok(true);
            }
            crate::message::initialize_profile(ctx)?;
            let o = ctx
                .globals
                .stage_forms
                .get_mut(&family)
                .and_then(|stage| stage.mwnd_lists.get_mut(&stage_idx))
                .and_then(|list| list.get_mut(window))
                .and_then(|w| w.button_list.get_mut(index))
                .context("native WAKU button missing")?;
            let value = super::passive_object::dispatch(
                o,
                &ctx.ids,
                &mut ctx.images,
                &mut ctx.native_media_revision,
                &chain[p + 6..],
                args,
                call.al_id,
                0,
                0,
            )?;
            if let Some(value) = value {
                ctx.push(value);
            } else if call.ret_form != 0 {
                ctx.push(Value::Int(0));
            }
            return Ok(true);
        }
        if chain.get(p + 3) == Some(&codes::ELM_MWND_OBJECT) {
            ensure!(
                chain.len() >= p + 7
                    && chain.get(p + 1) == Some(&ctx.ids.elm_array)
                    && chain.get(p + 4) == Some(&ctx.ids.elm_array),
                "native MWND embedded object chain {chain:?}"
            );
            let window = usize::try_from(chain[p + 2]).context("native MWND index")?;
            let child = usize::try_from(chain[p + 5]).context("native MWND object index")?;
            ensure!(window < 16 && child < 256, "native MWND object count limit");
            let total = super::passive_object::count(ctx)?;
            let old = ctx
                .globals
                .stage_forms
                .get(&family)
                .and_then(|s| s.mwnd_lists.get(&stage_idx))
                .and_then(|w| w.get(window))
                .map_or(0, |w| w.object_list.len());
            let growth = (child + 1).saturating_sub(old);
            ensure!(total + growth <= 4096, "native MWND object tree budget");
            let windows = ctx
                .globals
                .stage_forms
                .entry(family)
                .or_default()
                .mwnd_lists
                .entry(stage_idx)
                .or_default();
            windows.resize_with(windows.len().max(window + 1), Default::default);
            let objects = &mut windows[window].object_list;
            objects.resize_with(objects.len().max(child + 1), Default::default);
            let value = super::passive_object::dispatch(
                &mut objects[child],
                &ctx.ids,
                &mut ctx.images,
                &mut ctx.native_media_revision,
                &chain[p + 6..],
                args,
                call.al_id,
                4096 - total - growth,
                0,
            )?;
            if let Some(value) = value {
                ctx.push(value);
            } else if call.ret_form != 0 {
                ctx.push(Value::Int(0));
            }
            return Ok(true);
        }
        if chain.len() == p + 4 && chain[p + 1] == ctx.ids.elm_array && call.al_id == 0 {
            let window = usize::try_from(chain[p + 2]).context("native MWND query window")?;
            ensure!(window < 16, "native MWND query window budget");
            if crate::message::query_metadata(ctx, window, chain[p + 3], args)? {
                return Ok(true);
            }
        }
        ensure!(
            chain.get(p + 1) == Some(&ctx.ids.elm_array)
                && chain.get(p + 2) == Some(&0)
                && chain.len() == p + 4,
            "native MWND bridge supports MWND[0] only"
        );
        return crate::message::dispatch(ctx, chain[p + 3], args);
    }
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
    ensure!(stage_idx == 1, "native object bridge supports FRONT only");
    let index = index as usize;
    ensure!(index < 4096, "native object slot budget");
    let total = super::passive_object::count(ctx)?;
    let old = ctx
        .globals
        .stage_forms
        .get(&family)
        .and_then(|s| s.object_lists.get(&stage_idx))
        .map_or(0, Vec::len);
    let growth = (index + 1).saturating_sub(old);
    ensure!(total + growth <= 4096, "native object tree budget");
    let list = ctx
        .globals
        .stage_forms
        .entry(family)
        .or_default()
        .object_lists
        .entry(stage_idx)
        .or_default();
    list.resize_with(list.len().max(index + 1), Default::default);
    let value = super::passive_object::dispatch(
        &mut list[index],
        &ctx.ids,
        &mut ctx.images,
        &mut ctx.native_media_revision,
        &chain[p..],
        args,
        call.al_id,
        4096 - total - growth,
        0,
    )?;
    if let Some(value) = value {
        ctx.push(value);
    } else if call.ret_form != 0 {
        ctx.push(Value::Int(0));
    }
    Ok(true)
}
