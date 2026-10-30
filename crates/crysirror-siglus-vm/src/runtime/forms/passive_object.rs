// SPDX-License-Identifier: MPL-2.0
//! Bounded native object metadata operations, without a renderer/hit tester.
//! Image/rectangle payloads are native state only; CrySirror owns their layers.
//! Callback-bearing destruction remains explicit until native finalizers are mapped.
use crate::runtime::{
    CommandContext, Value, constants::RuntimeConstants, forms::codes, globals::ObjectState,
};
use anyhow::{Context, Result, bail, ensure};
pub(super) fn count(ctx: &CommandContext) -> Result<usize> {
    let mut todo = Vec::new();
    for stage in ctx.globals.stage_forms.values() {
        todo.extend(stage.object_lists.values().flatten());
        for w in stage.mwnd_lists.values().flatten() {
            todo.extend(
                w.object_list
                    .iter()
                    .chain(&w.face_list)
                    .chain(&w.button_list),
            );
        }
    }
    let mut count = 0;
    while let Some(object) = todo.pop() {
        count += 1;
        ensure!(count + todo.len() <= 4096, "native object tree budget");
        todo.extend(&object.runtime.child_objects);
    }
    Ok(count)
}
fn passive(root: &ObjectState) -> bool {
    let mut todo = vec![root];
    let mut count = 0;
    while let Some(object) = todo.pop() {
        count += 1;
        if count > 4096
            || !matches!(object.object_type, 0 | 1 | 2 | 9)
            || object.button.enabled
            || !object.frame_action.cmd_name.is_empty()
            || object
                .frame_action_ch
                .iter()
                .any(|f| !f.cmd_name.is_empty())
        {
            return false;
        }
        todo.extend(&object.runtime.child_objects);
    }
    true
}
pub(super) fn dispatch(
    object: &mut ObjectState,
    ids: &RuntimeConstants,
    images: &mut crate::image_manager::ImageManager,
    media_revision: &mut u64,
    tail: &[i32],
    args: &[Value],
    overload: i64,
    available: usize,
    depth: usize,
) -> Result<Option<Value>> {
    ensure!(depth <= 16, "native object child depth limit");
    let op = *tail.first().context("native object operation")?;
    if op == codes::ELM_OBJECT_CHILD {
        if tail.len() == 2 {
            match tail[1] {
                codes::ELM_OBJECTLIST_GET_SIZE => {
                    ensure!(
                        args.is_empty() && overload == 0,
                        "native child-list size arguments"
                    );
                    return Ok(Some(Value::Int(object.runtime.child_objects.len() as i64)));
                }
                codes::ELM_OBJECTLIST_RESIZE => {
                    ensure!(
                        args.len() == 1 && overload == 0,
                        "native child-list resize arguments"
                    );
                    let count = args[0].as_i64().context("native child count")?.max(0) as usize;
                    ensure!(
                        count <= 256
                            && count.saturating_sub(object.runtime.child_objects.len())
                                <= available,
                        "native child count/budget"
                    );
                    ensure!(
                        object.runtime.child_objects.iter().skip(count).all(passive),
                        "native child resize needs resource/callback destruction mapping"
                    );
                    object
                        .runtime
                        .child_objects
                        .resize_with(count, Default::default);
                    return Ok(None);
                }
                _ => {}
            }
        }
        ensure!(
            tail.len() >= 4 && tail[1] == ids.elm_array,
            "unsupported native child chain {tail:?}"
        );
        let index = usize::try_from(tail[2]).context("negative native child index")?;
        let child = object
            .runtime
            .child_objects
            .get_mut(index)
            .context("native child slot outside allocated list")?;
        return dispatch(
            child,
            ids,
            images,
            media_revision,
            &tail[3..],
            args,
            overload,
            available,
            depth + 1,
        );
    }
    if op == codes::ELM_OBJECT_FRAME_ACTION && tail.get(1) == Some(&codes::ELM_FRAMEACTION_COUNTER)
    {
        let counter = &mut object.frame_action.counter;
        if tail.len() == 2 {
            if overload == 1 {
                ensure!(args.len() == 1, "native object counter setter arguments");
                counter.set_count(args[0].as_i64().context("native object counter value")?);
                return Ok(None);
            }
            ensure!(
                overload == 0 && args.is_empty(),
                "native object counter getter arguments"
            );
            return Ok(Some(Value::Int(counter.get_count())));
        }
        ensure!(
            tail.len() == 3 && overload == 0,
            "native object counter operation chain"
        );
        let command = tail[2];
        let integer = |i: usize| {
            args.get(i)
                .and_then(Value::as_i64)
                .context("native object counter integer")
        };
        if matches!(
            command,
            codes::ELM_COUNTER_START_FRAME
                | codes::ELM_COUNTER_START_FRAME_REAL
                | codes::ELM_COUNTER_START_FRAME_LOOP
                | codes::ELM_COUNTER_START_FRAME_LOOP_REAL
        ) {
            ensure!(args.len() == 3, "native object range counter arguments");
            let (from, to, time) = (integer(0)?, integer(1)?, integer(2)?);
            match command {
                codes::ELM_COUNTER_START_FRAME => counter.start_frame(from, to, time),
                codes::ELM_COUNTER_START_FRAME_REAL => counter.start_frame_real(from, to, time),
                codes::ELM_COUNTER_START_FRAME_LOOP => counter.start_frame_loop(from, to, time),
                _ => counter.start_frame_loop_real(from, to, time),
            }
            return Ok(None);
        }
        ensure!(
            args.len()
                == usize::from(matches!(
                    command,
                    codes::ELM_COUNTER_SET | codes::ELM_COUNTER_CHECK_VALUE
                )),
            "native object counter arguments"
        );
        match command {
            codes::ELM_COUNTER_SET => counter.set_count(integer(0)?),
            codes::ELM_COUNTER_GET => return Ok(Some(Value::Int(counter.get_count()))),
            codes::ELM_COUNTER_RESET => counter.reset(),
            codes::ELM_COUNTER_START => counter.start(),
            codes::ELM_COUNTER_START_REAL => counter.start_real(),
            codes::ELM_COUNTER_STOP => counter.stop(),
            codes::ELM_COUNTER_RESUME => counter.resume(),
            codes::ELM_COUNTER_CHECK_ACTIVE => {
                return Ok(Some(Value::Int(i64::from(counter.is_running()))));
            }
            codes::ELM_COUNTER_CHECK_VALUE => {
                return Ok(Some(Value::Int(i64::from(
                    counter.get_count() >= integer(0)?,
                ))));
            }
            _ => bail!("unsupported native object counter operation {command}"),
        }
        return Ok(None);
    }
    if object.int_event_by_op(ids, op).is_some() {
        return super::object_event::dispatch(object, ids, tail, args, overload);
    }
    if object.int_event_list_by_op(ids, op).is_some() {
        ensure!(
            tail.len() == 4 && tail[1] == ids.elm_array && overload == 0,
            "native offset event list chain/overload"
        );
        let index = usize::try_from(tail[2]).context("negative native offset event index")?;
        ensure!(index < 256, "native offset event list budget");
        let default = if op == ids.obj_tr_rep_eve { 255 } else { 0 };
        let list = object.int_event_list_by_op_mut(ids, op).unwrap();
        list.resize_with(list.len().max(index + 1), || {
            crysirror_scene::animation::IntEvent::new(default)
        });
        return super::object_event::run(&mut list[index], tail[3], args);
    }
    if object.rep_int_event_list_by_rep_op(ids, op).is_some() {
        let default = if op == ids.obj_tr_rep { 255 } else { 0 };
        let list = object.rep_int_event_list_by_rep_op_mut(ids, op).unwrap();
        if tail.len() == 2 && tail[1] == codes::ELM_INTLIST_RESIZE {
            ensure!(
                overload == 0 && args.len() == 1,
                "native offset-list resize arguments"
            );
            let size =
                usize::try_from(args[0].as_i64().context("native offset-list size")?.max(0))?;
            ensure!(size <= 256, "native offset-list budget");
            ensure!(
                list.iter().skip(size).all(|v| !v.check_event()),
                "active native offset-list destruction profile"
            );
            list.resize_with(size, || crysirror_scene::animation::IntEvent::new(default));
            return Ok(None);
        }
        if tail.len() == 2 && tail[1] == codes::ELM_INTLIST_GET_SIZE {
            ensure!(
                overload == 0 && args.is_empty(),
                "native offset-list size arguments"
            );
            return Ok(Some(Value::Int(list.len() as i64)));
        }
        let index = if tail.len() == 3 && tail[1] == ids.elm_array {
            Some(tail[2])
        } else {
            None
        };
        if let Some(index) = index {
            let index = usize::try_from(index).context("negative native offset index")?;
            ensure!(index < 256, "native offset-list index budget");
            list.resize_with(list.len().max(index + 1), || {
                crysirror_scene::animation::IntEvent::new(default)
            });
            if overload == 1 {
                ensure!(args.len() == 1, "native offset setter arguments");
                let value = i32::try_from(args[0].as_i64().context("native offset value")?)?;
                list[index].set_value(value);
                return Ok(None);
            }
            ensure!(
                overload == 0 && args.is_empty(),
                "native offset getter arguments"
            );
            return Ok(Some(Value::Int(i64::from(list[index].get_value()))));
        }
        bail!("unsupported native offset-list operation {tail:?}");
    }
    ensure!(tail.len() == 1, "unsupported native object chain {tail:?}");
    if op == codes::ELM_OBJECT_FREE {
        ensure!(
            args.is_empty() && overload == 0,
            "native object FREE arguments"
        );
        ensure!(
            passive(object),
            "native object FREE requires callback finalization mapping"
        );
        // C_elm_object::free() resets only the type-owned resource. Children,
        // transform parameters and original events retain their identity.
        object.init_type_like();
        object.used = false;
        return Ok(None);
    }
    if op == codes::ELM_OBJECT_INIT {
        ensure!(
            args.is_empty() && overload == 0,
            "native object INIT arguments"
        );
        ensure!(
            passive(object),
            "native INIT needs resource/callback finalization mapping"
        );
        *object = Default::default();
        object.used = true;
        return Ok(None);
    }
    if matches!(
        op,
        codes::ELM_OBJECT_CREATE_MOVIE | codes::ELM_OBJECT_CREATE_MOVIE_LOOP
    ) {
        ensure!(
            passive(object),
            "native Movie replacement requires callback finalization mapping"
        );
        let positional = args
            .iter()
            .filter(|v| !matches!(v, Value::NamedArg { .. }))
            .collect::<Vec<_>>();
        ensure!(
            (0..=2).contains(&overload) && positional.len() == [1, 2, 4][overload as usize],
            "native object movie create overload/arguments"
        );
        let file = positional[0]
            .as_str()
            .context("native object movie filename")?;
        ensure!(
            !file.is_empty() && file.len() <= 4096 && !file.chars().any(char::is_control),
            "native object movie filename bounds"
        );
        let (mut auto_free, mut real, mut ready) = (true, true, false);
        for arg in args {
            if let Value::NamedArg { id, value } = arg {
                let flag = value
                    .as_i64()
                    .context("native object movie named boolean")?
                    != 0;
                match id {
                    0 => auto_free = flag,
                    1 => real = flag,
                    2 => ready = flag,
                    _ => bail!("unknown native object movie named argument {id}"),
                }
            }
        }
        let revision = media_revision
            .checked_add(1)
            .context("native movie declaration revision exhausted")?;
        object.init_type_like();
        object.init_param_like();
        object.used = true;
        object.object_type = 9;
        object.file_name = Some(file.into());
        object.movie.start(
            None,
            op == codes::ELM_OBJECT_CREATE_MOVIE_LOOP,
            auto_free,
            real,
            ready,
        );
        *media_revision = revision;
        object.movie.engine_revision = revision;
        object.base.disp = i64::from(
            overload >= 1
                && positional[1]
                    .as_i64()
                    .context("native movie display flag")?
                    != 0,
        );
        if overload >= 2 {
            object.set_int_prop(
                ids,
                ids.obj_x,
                positional[2].as_i64().context("native movie X")?,
            );
            object.set_int_prop(
                ids,
                ids.obj_y,
                positional[3].as_i64().context("native movie Y")?,
            );
        }
        return Ok(None);
    }
    if matches!(
        op,
        codes::ELM_OBJECT_GET_SIZE_X
            | codes::ELM_OBJECT_GET_SIZE_Y
            | codes::ELM_OBJECT_GET_FILE_NAME
            | codes::ELM_OBJECT_CHECK_MOVIE
            | codes::ELM_OBJECT_PAUSE_MOVIE
            | codes::ELM_OBJECT_RESUME_MOVIE
            | codes::ELM_OBJECT_SET_MOVIE_AUTO_FREE
    ) {
        ensure!(
            tail.len() == 1 && overload == 0,
            "native object query/control chain"
        );
        if op == codes::ELM_OBJECT_SET_MOVIE_AUTO_FREE {
            ensure!(args.len() == 1, "native movie auto-free arguments");
            object.movie.auto_free_flag =
                args[0].as_i64().context("native movie auto-free value")? != 0;
            return Ok(None);
        }
        ensure!(args.is_empty(), "native object query/control arguments");
        if op == codes::ELM_OBJECT_GET_FILE_NAME {
            return Ok(Some(Value::Str(
                object.file_name.clone().unwrap_or_default(),
            )));
        }
        if op == codes::ELM_OBJECT_CHECK_MOVIE {
            return Ok(Some(Value::Int(i64::from(
                object.object_type == 9 && object.movie.playing,
            ))));
        }
        if matches!(
            op,
            codes::ELM_OBJECT_PAUSE_MOVIE | codes::ELM_OBJECT_RESUME_MOVIE
        ) {
            ensure!(object.object_type == 9, "native movie control target");
            object.movie.pause_flag = op == codes::ELM_OBJECT_PAUSE_MOVIE;
            if !object.movie.pause_flag {
                object.movie.playing = true;
            }
            return Ok(None);
        }
        let size = match object.object_type {
            0 => [0, 0],
            1 => [
                object
                    .rect_param
                    .left
                    .abs_diff(object.rect_param.right)
                    .clamp(1, 32767) as u32,
                object
                    .rect_param
                    .top
                    .abs_diff(object.rect_param.bottom)
                    .clamp(1, 32767) as u32,
            ],
            2 => {
                let image = images.load_g00(
                    object
                        .file_name
                        .as_deref()
                        .context("native PCT query filename")?,
                    usize::try_from(object.get_int_prop(ids, ids.obj_patno))?,
                )?;
                let image = &images.images[image];
                [image.width, image.height]
            }
            9 => {
                ensure!(
                    object.movie.width > 0 && object.movie.height > 0,
                    "native object movie metadata pending (query inside a callback needs an engine resource continuation)"
                );
                [object.movie.width, object.movie.height]
            }
            other => bail!("native size query unsupported object type {other}"),
        };
        return Ok(Some(Value::Int(i64::from(
            size[usize::from(op == codes::ELM_OBJECT_GET_SIZE_Y)],
        ))));
    }
    if op == codes::ELM_OBJECT_CREATE || op == codes::ELM_OBJECT_CREATE_RECT {
        ensure!(
            passive(object),
            "native resource replacement needs callback finalization mapping"
        );
        let integer = |i: usize| {
            args.get(i)
                .and_then(Value::as_i64)
                .context("native object create integer")
        };
        if op == codes::ELM_OBJECT_CREATE {
            ensure!(
                (0..=3).contains(&overload) && [1, 2, 4, 5][overload as usize] == args.len(),
                "native PCT create overload/arguments"
            );
            let file = args[0].as_str().context("native PCT filename")?;
            ensure!(
                !file.contains('?') && file.len() <= 4096,
                "native PCT tonecurve/name profile"
            );
            let patno = if overload >= 3 { integer(4)? } else { 0 };
            if !file.is_empty() {
                images.load_g00(
                    file,
                    usize::try_from(patno).context("negative native PATNO")?,
                )?;
            }
            object.init_type_like();
            object.init_param_like();
            object.used = true;
            if !file.is_empty() {
                object.object_type = 2;
                object.file_name = Some(file.into());
                object.base.disp = i64::from(overload >= 1 && integer(1)? != 0);
                if overload >= 2 {
                    object.set_int_prop(ids, ids.obj_x, integer(2)?);
                    object.set_int_prop(ids, ids.obj_y, integer(3)?);
                }
                object.set_int_prop(ids, ids.obj_patno, patno);
            }
        } else {
            ensure!(
                (0..=2).contains(&overload) && [8, 9, 11][overload as usize] == args.len(),
                "native RECT create overload/arguments"
            );
            let bounds = [integer(0)?, integer(1)?, integer(2)?, integer(3)?];
            let c = [integer(4)?, integer(5)?, integer(6)?, integer(7)?].map(|v| v.clamp(0, 255));
            object.init_type_like();
            object.init_param_like();
            object.used = true;
            object.object_type = 1;
            let p = &mut object.rect_param;
            p.left = bounds[0];
            p.top = bounds[1];
            p.right = bounds[2];
            p.bottom = bounds[3];
            p.color_argb = (c[3] << 24) | (c[0] << 16) | (c[1] << 8) | c[2];
            object.base.disp = i64::from(overload >= 1 && integer(8)? != 0);
            if overload >= 2 {
                object.set_int_prop(ids, ids.obj_x, integer(9)?);
                object.set_int_prop(ids, ids.obj_y, integer(10)?);
            }
        }
        return Ok(None);
    }
    if ids.object_op_tables().fixed_int_prop(op).is_some() {
        if overload == 1 {
            ensure!(args.len() == 1, "native object property setter arguments");
            object.set_int_prop(
                ids,
                op,
                args[0].as_i64().context("native object property value")?,
            );
            return Ok(None);
        }
        ensure!(
            overload == 0 && args.is_empty(),
            "native object property getter arguments"
        );
        return Ok(Some(Value::Int(object.get_int_prop(ids, op))));
    }
    bail!("unsupported native embedded object operation {op}")
}
