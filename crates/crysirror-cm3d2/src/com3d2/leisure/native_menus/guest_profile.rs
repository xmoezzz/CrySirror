// SPDX-License-Identifier: MPL-2.0
//! Level 60 MaidMiniStatusCtrl: native status labels and the free comment.
use super::*;
use crysirror_core::service::{TextInputSessionHandle, TextInputSessionSpec};
use unicode_segmentation::UnicodeSegmentation;

const STATUS: &str = "/MaidMiniStatusPanel/";
pub(super) const COMMENT: &str = "UI Root/MaidMiniManagement/MaidMiniStatusPanel/FreeCommentWindow/FreeComment";

pub(super) fn paint(widget: &mut Widget, overview: &guest::MaidOverview, editable: bool) {
    if widget.path == COMMENT && !editable {
        widget.collider = None;
    }
    let Some(label) = &mut widget.label else { return };
    let status = &overview.original;
    let text = match widget.path.split_once(STATUS).map(|(_, path)| path) {
        Some("FirstName/Value") => status.first_name.clone(),
        Some("LastName/Value") => status.last_name.clone(),
        Some("Column1/Character/Value") => overview.personality.clone(),
        Some("Column1/SexExperience/Value") => crate::params::enum_text(crate::params::SEIKEIKEN_TEXT, status.seikeiken),
        Some("Column1/BodySize/Parent/Height/Value") => status.body.height.to_string(),
        Some("Column1/BodySize/Parent/Weight/Value") => status.body.weight.to_string(),
        Some("Column1/BodySize/Parent/Bust/Value") => status.body.bust.to_string(),
        Some("Column1/BodySize/Parent/Bust/Cup/Value") => status.body.cup.clone(),
        Some("Column1/BodySize/Parent/Waist/Value") => status.body.waist.to_string(),
        Some("Column1/BodySize/Parent/Hip/Value") => status.body.hip.to_string(),
        Some("WorkCount/Value") => overview.work_count.to_string(),
        Some("FreeCommentWindow/FreeComment/TextBox") => overview.comment.clone(),
        _ => return,
    };
    label.set_text(&text);
}

pub(super) struct Comment {
    session: TextInputSessionHandle,
    selected: bool,
    maid: usize,
    text: String,
}

impl Comment {
    pub(super) fn open(maid: usize, text: String, rect: [f32; 4], host: &mut dyn HostServices, client: ClientId) -> Result<Self> {
        let session = host.open_text_input_session(client, TextInputSessionSpec {
            capture: true,
            cursor_area: Some(rect.map(|v| v as i32)),
            ..Default::default()
        })?;
        Ok(Self { session, selected: true, maid, text })
    }

    pub(super) fn close(self, host: &mut dyn HostServices, client: ClientId) -> Result<()> {
        Ok(host.release_text_input_session(client, self.session)?)
    }

    pub(super) fn update(
        &mut self, host: &mut dyn HostServices, client: ClientId, input: InputFrame, limit: usize,
    ) -> Result<Option<(usize, String)>> {
        let committed = host.take_text_input_committed(client, self.session)?;
        let backspace = input.buttons_pressed & buttons::BACKSPACE != 0;
        if committed.is_empty() && !backspace {
            return Ok(None);
        }
        if std::mem::take(&mut self.selected) {
            self.text.clear();
        } else if backspace && let Some((at, _)) = self.text.grapheme_indices(true).last() {
            self.text.truncate(at);
        }
        let mut remaining = limit.saturating_sub(self.text.encode_utf16().count());
        for ch in committed.chars() {
            if ch.len_utf16() > remaining {
                break;
            }
            self.text.push(ch);
            remaining -= ch.len_utf16();
        }
        Ok(Some((self.maid, self.text.clone())))
    }
}
