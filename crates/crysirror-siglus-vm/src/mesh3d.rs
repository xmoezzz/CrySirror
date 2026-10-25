// SPDX-License-Identifier: MPL-2.0
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MeshAnimationState {
    pub clip_name: Option<String>,
    pub clip_index: Option<usize>,
    pub blend_clip_name: Option<String>,
    pub blend_clip_index: Option<usize>,
    pub blend_weight: f32,
    /// Object-local animation controller time, advanced by runtime tick.
    pub time_sec: f32,
    pub rate: f32,
    pub time_offset_sec: f32,
    /// Frozen controller sample time used while paused.
    pub hold_time_sec: f32,
    pub paused: bool,
    pub looped: bool,
    /// tona3-like controller state.
    pub anim_track_no: u32,
    pub anim_shift_time_sec: f32,
    pub is_anim: bool,
    pub prev_clip_name: Option<String>,
    pub prev_clip_index: Option<usize>,
    pub prev_time_sec: f32,
    pub prev_time_offset_sec: f32,
    pub prev_rate: f32,
    pub transition_elapsed_sec: f32,
}

pub const MESH_ANIM_CONTROLLER_STEP_SEC: f32 = 60.0 / 4800.0;

impl Default for MeshAnimationState {
    fn default() -> Self {
        Self {
            clip_name: None,
            clip_index: None,
            blend_clip_name: None,
            blend_clip_index: None,
            blend_weight: 0.0,
            time_sec: 0.0,
            rate: 1.0,
            time_offset_sec: 0.0,
            hold_time_sec: 0.0,
            paused: false,
            looped: true,
            anim_track_no: 0,
            anim_shift_time_sec: 0.0,
            is_anim: true,
            prev_clip_name: None,
            prev_clip_index: None,
            prev_time_sec: 0.0,
            prev_time_offset_sec: 0.0,
            prev_rate: 1.0,
            transition_elapsed_sec: 0.0,
        }
    }
}

impl MeshAnimationState {
    pub fn sanitized(&self) -> Self {
        let mut out = self.clone();
        out.blend_weight = out.blend_weight.clamp(0.0, 1.0);
        out.time_sec = out.time_sec.max(0.0);
        out.rate = out.rate.max(0.0);
        out.time_offset_sec = out.time_offset_sec.max(0.0);
        out.hold_time_sec = out.hold_time_sec.max(0.0);
        out.anim_shift_time_sec = out.anim_shift_time_sec.max(0.0);
        out.prev_time_sec = out.prev_time_sec.max(0.0);
        out.prev_time_offset_sec = out.prev_time_offset_sec.max(0.0);
        out.prev_rate = out.prev_rate.max(0.0);
        out.transition_elapsed_sec = out.transition_elapsed_sec.max(0.0);
        if out.anim_shift_time_sec <= 0.0 {
            out.prev_clip_name = None;
            out.prev_clip_index = None;
            out.prev_time_sec = 0.0;
            out.prev_time_offset_sec = 0.0;
            out.prev_rate = 1.0;
            out.transition_elapsed_sec = 0.0;
        }
        out
    }

    pub fn current_sample_base_sec(&self) -> f32 {
        if self.paused || !self.is_anim {
            self.hold_time_sec.max(0.0)
        } else {
            self.time_sec.max(0.0) * self.rate.max(0.0)
        }
    }

    pub fn current_sample_time_sec(&self) -> f32 {
        self.current_sample_base_sec() + self.time_offset_sec.max(0.0)
    }

    pub fn previous_track_sample_time_sec(&self) -> f32 {
        (self.prev_time_sec.max(0.0) * self.prev_rate.max(0.0)) + self.prev_time_offset_sec.max(0.0)
    }

    pub fn previous_track_weight(&self) -> f32 {
        if (self.prev_clip_name.is_none() && self.prev_clip_index.is_none())
            || self.anim_shift_time_sec <= 0.0
        {
            0.0
        } else {
            (1.0 - (self.transition_elapsed_sec / self.anim_shift_time_sec)).clamp(0.0, 1.0)
        }
    }

    pub fn current_track_weight(&self) -> f32 {
        (1.0 - self.previous_track_weight()).clamp(0.0, 1.0)
    }

    pub fn previous_track_speed(&self) -> f32 {
        if (self.prev_clip_name.is_none() && self.prev_clip_index.is_none())
            || self.anim_shift_time_sec <= 0.0
        {
            0.0
        } else {
            (1.0 - (self.transition_elapsed_sec / self.anim_shift_time_sec)).clamp(0.0, 1.0)
        }
    }

    pub fn current_track_speed(&self) -> f32 {
        if (self.prev_clip_name.is_none() && self.prev_clip_index.is_none())
            || self.anim_shift_time_sec <= 0.0
        {
            1.0
        } else {
            (self.transition_elapsed_sec / self.anim_shift_time_sec).clamp(0.0, 1.0)
        }
    }

    pub fn set_anim_shift_time_sec(&mut self, shift_sec: f32) {
        self.anim_shift_time_sec = shift_sec.max(0.0);
        *self = self.sanitized();
    }

    pub fn change_animation_clip(
        &mut self,
        next_clip_name: Option<String>,
        next_clip_index: Option<usize>,
    ) {
        if self.clip_name == next_clip_name && self.clip_index == next_clip_index {
            return;
        }
        self.prev_clip_name = self.clip_name.clone();
        self.prev_clip_index = self.clip_index;
        self.prev_time_sec = self.time_sec.max(0.0);
        self.prev_time_offset_sec = self.time_offset_sec.max(0.0);
        self.prev_rate = self.rate.max(0.0);
        self.clip_name = next_clip_name;
        self.clip_index = next_clip_index;
        self.time_sec = 0.0;
        self.hold_time_sec = 0.0;
        self.transition_elapsed_sec = 0.0;
        self.anim_track_no = if self.anim_track_no == 0 { 1 } else { 0 };
        if self.anim_shift_time_sec <= 0.0 {
            self.prev_clip_name = None;
            self.prev_clip_index = None;
            self.prev_time_sec = 0.0;
            self.prev_time_offset_sec = 0.0;
            self.prev_rate = 1.0;
        }
        *self = self.sanitized();
    }

    pub fn advance_controller_frames(&mut self, delta_frames: i32) {
        let delta_sec = (delta_frames.max(0) as f32) * MESH_ANIM_CONTROLLER_STEP_SEC;
        if !self.paused && self.is_anim {
            let has_prev = self.prev_clip_name.is_some() || self.prev_clip_index.is_some();
            if has_prev && self.anim_shift_time_sec > 0.0 {
                let start = self.transition_elapsed_sec.max(0.0);
                let end = (start + delta_sec).min(self.anim_shift_time_sec);
                let active_delta = (end - start).max(0.0);
                if active_delta > 0.0 {
                    let shift = self.anim_shift_time_sec.max(0.000_001);
                    let cur_advance = ((end * end) - (start * start)) / (2.0 * shift);
                    let prev_advance = active_delta - cur_advance;
                    self.time_sec = (self.time_sec.max(0.0) + cur_advance.max(0.0)).max(0.0);
                    self.prev_time_sec =
                        (self.prev_time_sec.max(0.0) + prev_advance.max(0.0)).max(0.0);
                    self.transition_elapsed_sec = end.max(0.0);
                }
                if delta_sec > active_delta {
                    self.time_sec = (self.time_sec.max(0.0) + (delta_sec - active_delta)).max(0.0);
                }
                if self.transition_elapsed_sec >= self.anim_shift_time_sec {
                    self.prev_clip_name = None;
                    self.prev_clip_index = None;
                    self.prev_time_sec = 0.0;
                    self.prev_time_offset_sec = 0.0;
                    self.prev_rate = 1.0;
                    self.transition_elapsed_sec = 0.0;
                }
            } else {
                self.time_sec = (self.time_sec.max(0.0) + delta_sec).max(0.0);
                if has_prev {
                    self.prev_clip_name = None;
                    self.prev_clip_index = None;
                    self.prev_time_sec = 0.0;
                    self.prev_time_offset_sec = 0.0;
                    self.prev_rate = 1.0;
                    self.transition_elapsed_sec = 0.0;
                }
            }
        }
        *self = self.sanitized();
    }
}
