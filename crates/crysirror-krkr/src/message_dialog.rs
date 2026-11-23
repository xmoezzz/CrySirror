// SPDX-License-Identifier: MPL-2.0
//! KiriKiri message calls wait on the shared UI dialog and resume with the
//! chosen button value. WIN32Dialog and System use their own result numbers.
use crate::natives::{Rt, unsupported};
use crysirror_core::{
    service::{EngineQuery, HostServices},
    state::ClientId,
};
use crysirror_scene::ui::{
    UiEvent,
    dialog::{DialogButton, DialogSpec},
};
use crysirror_tjs2::{
    Result, TjsError,
    runtime::{ObjectHandle, Variant},
};
use serde_json::json;

pub(crate) fn confirm(rt: &mut Rt, _: Option<ObjectHandle>, args: Vec<Variant>) -> Result<Variant> {
    let text = |index: usize| match args.get(index) {
        None | Some(Variant::Void) => Ok(String::new()),
        Some(value) => value.to_tjs_string(),
    };
    let dialog = DialogSpec {
        id: 0,
        title: text(1)?,
        message: text(0)?,
        buttons: vec![
            DialogButton {
                id: 1,
                label: "Yes".into(),
            },
            DialogButton {
                id: 0,
                label: "No".into(),
            },
        ],
        default: 0,
        cancel: Some(0),
        font: rt.host().font.clone(),
        text_size: 18,
    };
    open(rt, "System.confirm", dialog)
}

pub(crate) fn open(rt: &mut Rt, method: &str, mut dialog: DialogSpec) -> Result<Variant> {
    let host = rt.host_mut();
    if !host.can_suspend {
        return Err(unsupported(method, "a resumable TJS boundary"));
    }
    if host.dialog_pending.is_some() {
        return Err(TjsError::runtime(format!(
            "{method}: a message box is already open"
        )));
    }
    dialog.id = host.dialog_serial % 65535 + 1;
    dialog
        .validate()
        .map_err(|e| TjsError::runtime(format!("{method}: {e}")))?;
    host.ui_op(json!({"op": "open_dialog", "dialog": dialog}));
    host.dialog_serial = dialog.id;
    host.dialog_pending = Some(dialog.id);
    rt.request_suspend();
    Ok(Variant::Integer(0))
}

pub(crate) fn poll(
    rt: &mut Rt,
    host: &mut dyn HostServices,
    client: ClientId,
) -> anyhow::Result<bool> {
    let Some(id) = rt.host().dialog_pending else {
        return Ok(true);
    };
    let bytes = host
        .query_engine(client, EngineQuery::DialogEvents)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let events: Vec<UiEvent> = serde_json::from_slice(&bytes)?;
    let Some(event) = events
        .iter()
        .find(|e| e.widget.owner == client.0 && e.widget.id == id)
    else {
        return Ok(false);
    };
    rt.host_mut().dialog_pending = None;
    rt.set_resume_value(Variant::Integer(event.value as i64));
    Ok(true)
}
