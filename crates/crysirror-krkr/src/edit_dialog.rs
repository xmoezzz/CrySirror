// SPDX-License-Identifier: MPL-2.0
//! The single-edit WIN32Dialog template over engine-owned modal widgets.
//! The script handles native command IDs and constructs its own result.
use crate::natives::{Rt, this_of};
use crysirror_core::{
    service::{EngineQuery, HostServices},
    state::ClientId,
};
use crysirror_scene::ui::{UiEvent, WidgetSpec, WidgetTextAction, WidgetTextActionKind};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::json;

const FIELD: u32 = 3;
const CLEAR: u32 = 4;
const CANCEL: u32 = 5;
const OK: u32 = 6;

pub(crate) struct Prompt {
    root: u32,
    pub owner: ObjectHandle,
    extra_button: Option<i64>,
    phase: Phase,
    closed: Option<i64>,
    last_key_serial: u32,
    selection: Option<(i64, i64)>,
}

enum Phase {
    Init,
    Waiting,
    Callback,
}

pub(crate) fn store_part(
    rt: &mut Rt,
    this: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    let part = this_of(rt, this, "dialog template part")?;
    let data = args[0]
        .object_handle()
        .ok_or_else(|| TjsError::runtime("dialog template dictionary required"))?;
    let copy = rt.alloc_dictionary_object();
    for name in ["windowClass", "id", "title", "style"] {
        rt.set_object_member(copy, name, rt.object_member(data, name));
    }
    rt.set_object_member(part, "__templatePart", Variant::Object(copy));
    Ok(Variant::Void)
}

pub(crate) fn make_template(
    rt: &mut Rt,
    this: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    let owner = this_of(rt, this, "WIN32Dialog.makeTemplate")?;
    let parts = args
        .iter()
        .map(|arg| {
            arg.object_handle()
                .map(|h| rt.object_member(h, "__templatePart"))
                .unwrap_or_default()
        })
        .collect();
    let parts = rt.alloc_array_object(parts);
    rt.set_object_member(owner, "__hasDialogTemplate", Variant::Object(parts));
    Ok(Variant::Void)
}

fn item_key(id: i64) -> String {
    format!("__editText{id}")
}

pub(crate) fn set_item_text(
    rt: &mut Rt,
    this: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    let owner = this_of(rt, this, "WIN32Dialog.setItemText")?;
    let id = args[0].to_integer()?;
    if rt.object_member(owner, "__editControl") == Variant::Integer(id) {
        if let Some(prompt) = rt.host_mut().input_prompt.as_mut().filter(|p|p.owner == owner) {
            prompt.selection = None;
        }
    }
    rt.set_object_member(
        owner,
        &item_key(id),
        Variant::String(args[1].to_tjs_string()?),
    );
    Ok(Variant::Void)
}

pub(crate) fn send_item_message(rt: &mut Rt, this: Option<ObjectHandle>, args: Vec<Variant>) -> Result<Variant> {
    const EM_SETSEL: i64 = 0x00b1;
    if args[1].to_integer()? == EM_SETSEL {
        let owner = this_of(rt, this, "WIN32Dialog.sendItemMessage")?;
        if rt.object_member(owner, "__editControl") == Variant::Integer(args[0].to_integer()?) {
            let start = args.get(2).map(Variant::to_integer).transpose()?.unwrap_or(0);
            let end = args.get(3).map(Variant::to_integer).transpose()?.unwrap_or(0);
            if let Some(prompt) = rt.host_mut().input_prompt.as_mut().filter(|p|p.owner == owner) {
                prompt.selection = Some((start, end));
            }
        }
    }
    Ok(Variant::Integer(0))
}

pub(crate) fn get_item_text(
    rt: &mut Rt,
    this: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    let owner = this_of(rt, this, "WIN32Dialog.getItemText")?;
    Ok(Variant::String(
        rt.object_member(owner, &item_key(args[0].to_integer()?))
            .to_tjs_string()?,
    ))
}

pub(crate) fn close_template(
    rt: &mut Rt,
    this: Option<ObjectHandle>,
    args: Vec<Variant>,
) -> Result<Variant> {
    let owner = this_of(rt, this, "WIN32Dialog.close")?;
    if let Some(p) = rt
        .host_mut()
        .input_prompt
        .as_mut()
        .filter(|p| p.owner == owner)
    {
        p.closed = Some(args[0].to_integer()?);
    }
    Ok(Variant::Void)
}

struct Labels {
    title: String,
    prompt: String,
    initial: String,
    ok: String,
    cancel: String,
    clear: String,
}

pub(crate) fn open_template(rt: &mut Rt, owner: ObjectHandle) -> Result<Variant> {
    if !rt.host().can_suspend || rt.host().input_prompt.is_some() {
        return Err(TjsError::runtime(
            "text entry requires a free modal input scope",
        ));
    }
    let parts = rt
        .object_member(owner, "__hasDialogTemplate")
        .object_handle()
        .and_then(|h| rt.array_elements(h))
        .unwrap_or_default();
    let part = |v: &Variant| {
        v.object_handle()
            .ok_or_else(|| TjsError::runtime("missing dialog template part"))
    };
    let header = parts
        .first()
        .ok_or_else(|| TjsError::runtime("missing dialog header"))?;
    let mut labels = Labels {
        title: rt.object_member(part(header)?, "title").to_tjs_string()?,
        prompt: String::new(),
        initial: String::new(),
        ok: String::new(),
        cancel: String::new(),
        clear: String::new(),
    };
    let mut editor = None;
    let mut extra = None;
    for v in &parts[1..] {
        let h = part(v)?;
        let class = rt
            .object_member(h, "windowClass")
            .to_tjs_string()?
            .to_ascii_lowercase();
        let id = rt.object_member(h, "id").to_integer()?;
        let title = rt.object_member(h, "title").to_tjs_string()?;
        match class.as_str() {
            "130" | "static" => labels.prompt = title,
            "129" | "edit" if editor.is_none() => {
                let style = rt.object_member(h, "style").to_integer()?;
                if style & (0x20 | 0x800) != 0 {
                    return Err(TjsError::runtime(
                        "password and read-only edit templates are unsupported",
                    ));
                }
                editor = Some((id, style & 4 != 0));
                labels.initial = title;
            }
            "128" | "button" if rt.object_member(h, "style").to_integer()? & 0xf > 1 => {
                return Err(TjsError::runtime("edit dialog requires push buttons"));
            }
            "128" | "button" if id == 1 => labels.ok = title,
            "128" | "button" if id == 2 => labels.cancel = title,
            "128" | "button" if extra.is_none() => {
                extra = Some(id);
                labels.clear = title;
            }
            _ => {
                return Err(TjsError::runtime(format!(
                    "unsupported edit dialog control {class}/{id}"
                )));
            }
        }
    }
    let (id, multiline) = editor
        .ok_or_else(|| TjsError::runtime("unsupported dialog template without an edit control"))?;
    if labels.ok.is_empty() || labels.cancel.is_empty() {
        return Err(TjsError::runtime("edit dialog needs OK and Cancel"));
    }
    rt.set_object_member(owner, "__editControl", Variant::Integer(id));
    rt.set_object_member(
        owner,
        &item_key(id),
        Variant::String(labels.initial.clone()),
    );
    present(rt, labels, owner, extra, multiline)
}

fn present(
    rt: &mut Rt,
    labels: Labels,
    owner: ObjectHandle,
    extra_button: Option<i64>,
    multiline: bool,
) -> Result<Variant> {
    let host = rt.host_mut();
    host.dialog_serial = host.dialog_serial % 65535 + 1;
    let root = 0x6e00_0000 + host.dialog_serial * 8;
    let [cw, ch] = host.canvas.map(|v| v as f32);
    let scale = (cw / 680.).min(ch / 240.).min((cw / 1280.).clamp(1., 1.5));
    let (w, h) = (640. * scale, 204. * scale);
    let (x, y) = ((cw - w) * 0.5, (ch - h) * 0.5);
    let font = host.font.clone();
    let size = (18. * scale).round().clamp(8., 32.) as u16;
    host.ui_op(json!({"op":"put","widget":{"id":root,"rect":[0,0,cw,ch],
        "color":[0,0,0,115],"modal":true,"z":20000,"keyboard_focusable":false}}));
    host.ui_op(json!({"op":"put","widget":{"id":root+1,"parent":root,
        "rect":[x,y,w,h],"color":[246,247,250,255],"keyboard_focusable":false}}));
    let mut widget = |offset, kind, rect: [f32; 4], text: &str, color| {
        host.ui_op(
            json!({"op":"put","widget":{"id":root+offset,"parent":root+1,
            "kind":kind,"rect":rect.map(|v|v*scale),"text":text,"font":font,
            "text_color":[24,31,41,255],"text_size":size,"text_inset":[6,3],"color":color,
            "multiline":kind=="text_field" && multiline,"max_length":65536,
            "keyboard_focusable":kind!="label","keyboard_activation":kind=="button"}}),
        );
    };
    widget(
        2,
        "label",
        [18., 10., 604., 32.],
        &labels.title,
        [246, 247, 250, 255],
    );
    widget(
        7,
        "label",
        [18., 48., 604., 30.],
        &labels.prompt,
        [246, 247, 250, 255],
    );
    widget(
        FIELD,
        "text_field",
        [18., 88., 604., 48.],
        &labels.initial,
        [255, 255, 255, 255],
    );
    widget(
        CLEAR,
        "button",
        [18., 148., 110., 38.],
        if labels.clear.is_empty() {
            "Clear"
        } else {
            &labels.clear
        },
        [226, 231, 238, 255],
    );
    widget(
        CANCEL,
        "button",
        [384., 148., 110., 38.],
        &labels.cancel,
        [226, 231, 238, 255],
    );
    widget(
        OK,
        "button",
        [508., 148., 114., 38.],
        &labels.ok,
        [188, 217, 250, 255],
    );
    host.ui_op(json!({"op":"focus","id":root+FIELD}));
    host.input_prompt = Some(Prompt {
        root,
        owner,
        extra_button,
        phase: Phase::Init,
        closed: None,
        last_key_serial: 0,
        selection: None,
    });
    rt.request_suspend();
    Ok(Variant::Void)
}

pub(crate) fn poll(
    rt: &mut Rt,
    host: &mut dyn HostServices,
    client: ClientId,
) -> anyhow::Result<bool> {
    let Some(prompt) = &rt.host().input_prompt else {
        return Ok(true);
    };
    match prompt.phase {
        Phase::Init => return dispatch(rt, prompt.owner, "onInit", 0x110, 0),
        Phase::Callback if rt.is_suspended() => return Ok(true),
        Phase::Callback => return finish_command(rt),
        Phase::Waiting => {}
    }
    let (root, owner, extra, prior_serial) = (
        prompt.root,
        prompt.owner,
        prompt.extra_button,
        prompt.last_key_serial,
    );
    let events: Vec<UiEvent> =
        serde_json::from_slice(&host.query_engine(client, EngineQuery::UiSubtreeEvents(root))?)?;
    let key: Option<WidgetTextAction> = serde_json::from_slice(
        &host.query_engine(client, EngineQuery::UiTextAction(root + FIELD))?,
    )?;
    let key = key.filter(|k| k.serial != prior_serial);
    if let Some(action) = key {
        rt.host_mut().input_prompt.as_mut().unwrap().last_key_serial = action.serial;
    }
    let clicked = events
        .iter()
        .find(|e| e.kind == "activate")
        .map(|e| e.widget.id - root);
    let closing = std::mem::take(&mut rt.host_mut().user_close_pending);
    let button = if closing
        || clicked == Some(CANCEL)
        || matches!(key, Some(k) if k.kind == WidgetTextActionKind::Cancel)
    {
        2
    } else if clicked == Some(OK)
        || matches!(key, Some(k) if k.kind == WidgetTextActionKind::Submit)
    {
        1
    } else if let (Some(CLEAR), Some(id)) = (clicked, extra) {
        id
    } else {
        if clicked == Some(CLEAR) {
            rt.host_mut()
                .ui_op(json!({"op":"set_text","id":root+FIELD,"text":""}));
            rt.host_mut().ui_op(json!({"op":"focus","id":root+FIELD}));
        }
        return Ok(false);
    };
    let field: WidgetSpec =
        serde_json::from_slice(&host.query_engine(client, EngineQuery::UiWidget(root + FIELD))?)?;
    let id = rt.object_member(owner, "__editControl").to_integer()?;
    rt.set_object_member(owner, &item_key(id), Variant::String(field.text));
    dispatch(rt, owner, "onCommand", 0x111, button)
}

fn dispatch(
    rt: &mut Rt,
    owner: ObjectHandle,
    method: &str,
    message: i64,
    wparam: i64,
) -> anyhow::Result<bool> {
    // A script handler may itself call a suspending native. Keep the original
    // open() stack parked until that handler has finished and chosen a result.
    rt.park_suspended_call()?;
    rt.host_mut().input_prompt.as_mut().unwrap().phase = Phase::Callback;
    rt.call_object_method(
        owner,
        method,
        vec![
            Variant::Integer(message),
            Variant::Integer(wparam),
            Variant::Integer(0),
        ],
    )?;
    if rt.is_suspended() {
        return Ok(true);
    }
    finish_command(rt)
}

fn finish_command(rt: &mut Rt) -> anyhow::Result<bool> {
    let prompt = rt.host().input_prompt.as_ref().unwrap();
    let (root, owner, closed) = (prompt.root, prompt.owner, prompt.closed);
    if let Some(result) = closed {
        // Shared UI may have consumed the key-down before VM input routing.
        // A still-held Escape must not also back out of the save/load page.
        rt.host_mut().prompt_release_mask |= crysirror_core::service::buttons::ESCAPE
            | crysirror_core::service::buttons::ENTER | crysirror_core::service::buttons::SPACE;
        rt.host_mut().ui_op(json!({"op":"remove","id":root}));
        rt.host_mut().input_prompt = None;
        rt.restore_parked_call(Variant::Integer(result))?;
        Ok(true)
    } else {
        let id = rt.object_member(owner, "__editControl").to_integer()?;
        let text = rt.object_member(owner, &item_key(id)).to_tjs_string()?;
        let (start,end) = rt.host_mut().input_prompt.as_mut().unwrap().selection.take()
            .unwrap_or((i64::MAX, i64::MAX));
        rt.host_mut()
            .ui_op(json!({"op":"set_text","id":root+FIELD,"text":text}));
        rt.host_mut().ui_op(json!({"op":"set_text_selection","id":root+FIELD,"start":start,"end":end}));
        rt.host_mut().ui_op(json!({"op":"focus","id":root+FIELD}));
        rt.host_mut().input_prompt.as_mut().unwrap().phase = Phase::Waiting;
        rt.restore_parked_call(Variant::Void)?;
        Ok(false)
    }
}
