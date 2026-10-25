// SPDX-License-Identifier: MPL-2.0
use std::path::PathBuf;
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TwitterState {
    pub request_token: String,
    pub request_token_secret: String,
    pub access_token: String,
    pub access_token_secret: String,
    pub user_id: String,
    pub user_name: String,
    pub screen_name: String,
    pub loaded: bool,
}

impl TwitterState {
    pub fn is_authorized(&self) -> bool {
        !self.access_token.is_empty()
            && !self.access_token_secret.is_empty()
            && !self.user_id.is_empty()
            && !self.user_name.is_empty()
            && !self.screen_name.is_empty()
    }

    pub fn clear_authorization(&mut self) {
        self.request_token.clear();
        self.request_token_secret.clear();
        self.access_token.clear();
        self.access_token_secret.clear();
        self.user_id.clear();
        self.user_name.clear();
        self.screen_name.clear();
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TwitterDialogRequest {
    pub image_path: PathBuf,
    pub image_rgba: Vec<u8>,
    pub image_width: u32,
    pub image_height: u32,
    pub initial_text: String,
}
