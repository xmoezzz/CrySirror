// SPDX-License-Identifier: MPL-2.0
use crate::gameexe::GameexeConfig;
const INIDEF_MWND_WAKU_BTN_CNT: usize = 8;
const INIDEF_MWND_WAKU_FACE_CNT: usize = 1;
const INIDEF_MWND_WAKU_OBJECT_CNT: usize = 1;
#[derive(Default, serde::Serialize, serde::Deserialize, Clone)]
pub struct AssetTables {
    pub gameexe: Option<GameexeConfig>,
    pub mwnd_render: MwndRenderTemplate,
    pub waku_templates: Vec<WakuTemplate>,
    pub sel_btn_templates: Vec<SelBtnTemplate>,
}
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct ShakeStep {
    pub x: i32,
    pub y: i32,
    pub time_ms: i32,
}
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MwndRenderTemplate {
    /// #TATEYOKO_MODE: 0 = horizontal, 1 = vertical.
    pub vertical_writing: bool,
    pub default_mwnd_no: i64,
    pub default_sel_mwnd_no: i64,
    pub order: i64,
    pub filter_layer_rep: i64,
    pub waku_layer_rep: i64,
    pub face_layer_rep: i64,
    pub shadow_layer_rep: i64,
    pub fuchi_layer_rep: i64,
    pub moji_layer_rep: i64,
    pub shadow_color: i64,
    pub fuchi_color: i64,
    pub moji_color: i64,
}

impl Default for MwndRenderTemplate {
    fn default() -> Self {
        Self {
            vertical_writing: false,
            default_mwnd_no: 0,
            default_sel_mwnd_no: 1,
            order: 1,
            filter_layer_rep: 0,
            waku_layer_rep: 1,
            face_layer_rep: 2,
            shadow_layer_rep: 3,
            fuchi_layer_rep: 4,
            moji_layer_rep: 5,
            shadow_color: 1,
            fuchi_color: 1,
            moji_color: 0,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct WakuButtonTemplate {
    pub file_name: String,
    pub cut_no: i64,
    pub pos_base: i64,
    pub pos: (i64, i64),
    pub action_no: i64,
    pub se_no: i64,
    pub sys_type: i64,
    pub sys_type_opt: i64,
    pub btn_mode: i64,
    pub scn_name: String,
    pub cmd_name: String,
    pub z_no: i64,
    pub frame_action_scn_name: String,
    pub frame_action_cmd_name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WakuTemplate {
    pub extend_type: i64,
    pub waku_file: String,
    pub filter_file: String,
    pub filter_margin: (i64, i64, i64, i64),
    pub filter_color: (u8, u8, u8, u8),
    pub filter_config_color: bool,
    pub filter_config_tr: bool,
    pub icon_no: i64,
    pub page_icon_no: i64,
    pub icon_pos_type: i64,
    pub icon_pos_base: i64,
    pub icon_pos: (i64, i64, i64),
    pub buttons: Vec<WakuButtonTemplate>,
    pub face_pos: Vec<(i64, i64)>,
    pub object_cnt: usize,
}

impl Default for WakuTemplate {
    fn default() -> Self {
        Self {
            extend_type: 0,
            waku_file: String::new(),
            filter_file: String::new(),
            filter_margin: (0, 0, 0, 0),
            filter_color: (0, 0, 255, 128),
            filter_config_color: true,
            filter_config_tr: true,
            icon_no: -1,
            page_icon_no: -1,
            icon_pos_type: 0,
            icon_pos_base: 0,
            icon_pos: (0, 0, 0),
            buttons: vec![WakuButtonTemplate::default(); INIDEF_MWND_WAKU_BTN_CNT],
            face_pos: vec![(0, 0); INIDEF_MWND_WAKU_FACE_CNT],
            object_cnt: INIDEF_MWND_WAKU_OBJECT_CNT,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SelBtnTemplate {
    pub base_file: String,
    pub filter_file: String,
    pub base_pos: (i64, i64),
    pub rep_pos: (i64, i64),
    pub x_align: i64,
    pub y_align: i64,
    pub max_y_cnt: i64,
    pub line_width: i64,
    pub moji_cnt: i64,
    pub moji_pos: (i64, i64),
    pub moji_size: i64,
    pub moji_space: (i64, i64),
    pub moji_x_align: i64,
    pub moji_y_align: i64,
    pub moji_color: i64,
    pub moji_hit_color: i64,
    pub btn_action_no: i64,
    pub open_anime_type: i64,
    pub open_anime_time: i64,
    pub close_anime_type: i64,
    pub close_anime_time: i64,
    pub decide_anime_type: i64,
    pub decide_anime_time: i64,
}

impl Default for SelBtnTemplate {
    fn default() -> Self {
        Self {
            base_file: String::new(),
            filter_file: String::new(),
            base_pos: (0, 0),
            rep_pos: (0, 0),
            x_align: 0,
            y_align: 0,
            max_y_cnt: 0,
            line_width: 100,
            moji_cnt: 0,
            moji_pos: (0, 0),
            moji_size: 25,
            moji_space: (0, 0),
            moji_x_align: 0,
            moji_y_align: 0,
            moji_color: 0,
            moji_hit_color: 5,
            btn_action_no: 0,
            open_anime_type: 1,
            open_anime_time: 500,
            close_anime_type: 1,
            close_anime_time: 500,
            decide_anime_type: 1,
            decide_anime_time: 500,
        }
    }
}
