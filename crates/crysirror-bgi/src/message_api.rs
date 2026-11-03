//! Original native message policy; shared text state supplies parsing and clocks.
use super::*;
use crate::window_api::NativeWindow;
use crysirror_bgi_vm::native_call::NativeProcedureStart;
use crysirror_bgi_vm::{MessageSignal, NativeMessageProcedureClass, NativeMessageProcedureConfig};
use crysirror_text::message::{
    ParsedMessageMarkup, TextRuntime,
    layout::{
        MessageLayoutState, append_message_markup, place_message_glyphs, wrap_parsed_message,
    },
    parse_message_markup_with,
};
#[derive(Default)]
pub(crate) struct MessageState {
    pub runtime: TextRuntime,
    pub active: Option<i32>,
    last_tick: Option<u64>,
    pub animation: crate::text_policy::TextAnimationDefaults,
    delay_enabled: bool,
    delay_ms: i32,
    input_scope_mode: i32,
    input_scope_value: i32,
    input_filter: bool,
    input_forces_completion: bool,
    shadow_enabled: bool,
    shadow: [i32; 3],
    caret_frames: Vec<Option<i32>>,
    caret_delay: i32,
    caret_position: [i32; 3],
    caret_tick: u32,
    caret_index: usize,
    caret_window: Option<i32>,
}
pub(crate) fn handles(group: u8, id: u16) -> bool {
    matches!((group,id),(0x90,0x90..=0x92|0x94..=0x9d|0x9f)|(0x91,0x90..=0x93)|(0x92,0x90|0x91|0x9f))
}
fn rgb(value: u32) -> [u8; 4] {
    [(value >> 16) as u8, (value >> 8) as u8, value as u8, 255]
}
impl BgiVm {
    pub(crate) fn message_window_busy(&self, window: i32) -> bool {
        self.messages.active == Some(window)
    }
    pub(crate) fn finish_message_for_window(&mut self, window: i32) {
        if self.messages.active == Some(window) {
            self.messages.active = None;
        }
    }
    fn message_auto_delay(&self) -> Option<i32> {
        self.messages
            .animation
            .auto_advance_enabled
            .then_some(self.messages.animation.auto_advance_delay_ms.max(0))
    }
    pub(crate) fn tick_messages(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
    ) -> Result<()> {
        let now = host.now_ms();
        let elapsed = self
            .messages
            .last_tick
            .replace(now)
            .map_or(0, |last| now.saturating_sub(last));
        let mut changed = false;
        for signal in vm.take_message_signals() {
            match signal {
                MessageSignal::Reveal => {
                    changed |= self.messages.runtime.reveal_all().is_some();
                }
                MessageSignal::Finish => {
                    self.messages.active = None;
                    self.clear_message_caret(host)?;
                }
                MessageSignal::Caret { now_tick } => self.tick_message_caret(host, now_tick)?,
            }
        }
        changed |= self.messages.runtime.tick(elapsed).is_some();
        for event in self.messages.runtime.take_fired_events() {
            vm.post_message_event(event)?;
        }
        if changed {
            if let Some(window) = self.messages.runtime.target_node {
                if self.windows.has_window(window) {
                    self.publish_window_message(host, window, self.messages.runtime.visible_chars)?;
                }
            }
        }
        vm.update_message_state(
            self.messages.runtime.is_animating(),
            self.message_auto_delay(),
        );
        Ok(())
    }
    fn message_config(
        &self,
        window: i32,
        class: NativeMessageProcedureClass,
    ) -> NativeMessageProcedureConfig {
        NativeMessageProcedureConfig {
            class,
            initial_delay_enabled: self.messages.delay_enabled,
            initial_delay_ms: if self.messages.delay_enabled {
                self.messages.delay_ms.max(0)
            } else {
                0
            },
            reveal_duration_ms: self.messages.runtime.duration_ms(),
            reveal_steps: self.messages.animation.reveal_steps,
            reveal_step_delay_ms: self.messages.animation.reveal_step_delay_ms,
            settle_steps: self.messages.animation.settle_steps,
            settle_step_delay_ms: self.messages.animation.settle_step_delay_ms,
            auto_advance_delay_ms: self.message_auto_delay(),
            input_scope: match self.messages.input_scope_mode {
                1 => self.messages.input_scope_value.wrapping_shl(16) | 0xffff,
                2 => window.wrapping_shl(16) | 0xffff,
                _ => 2,
            },
            completion_control: 0,
            end_wait_policy: 0,
            allow_high_bit_input: false,
            allow_auxiliary_input: true,
            auxiliary_input_mask: 0,
            input_forces_completion: self.messages.input_forces_completion,
        }
    }
    fn message_shadow(
        &self,
        window: &NativeWindow,
    ) -> Option<crysirror_core::image_processing::BitmapTextShadow> {
        if window.shadow_configured {
            return window.shadow;
        }
        window.shadow.or_else(|| {
            self.messages.shadow_enabled.then(|| {
                crysirror_core::image_processing::BitmapTextShadow {
                    mode: 1,
                    offset: [
                        (window.font_size * self.messages.shadow[0] / 100).max(0) as u16,
                        (window.font_size * self.messages.shadow[1] / 100).max(0) as u16,
                    ],
                    color: rgb(0)[..3].try_into().unwrap(),
                    concentration: self.messages.shadow[2].clamp(0, 256) as u16,
                }
            })
        })
    }
    fn style_window_message(&mut self, window: i32, color: u32, style: [i32; 5]) -> Result<()> {
        self.text.output_color = color as i32;
        let w = self.windows.get_mut(window)?;
        let [mode, x, y, rgb, density] = style;
        w.color = color & 0xffffff;
        w.shadow_configured = true;
        w.shadow = (matches!(mode, 1 | 2)
            && (0..=100).contains(&x)
            && (0..=100).contains(&y)
            && (0..=256).contains(&density))
        .then(|| crysirror_core::image_processing::BitmapTextShadow {
            mode: mode as u8,
            offset: [
                (w.font_size * x / 100).max(1) as u16,
                (w.font_size * y / 100).max(1) as u16,
            ],
            color: [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8],
            concentration: density as u16,
        });
        Ok(())
    }
    fn prepare_message_page(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
        text: &str,
    ) -> Result<(ParsedMessageMarkup, usize)> {
        let text = if let Some(at) = text.rfind('\x0c') {
            self.reset_window_text(host, window)?;
            &text[at + 1..]
        } else {
            text
        };
        ensure!(text.len() <= 1024 * 1024, "BGI message byte budget");
        let w = self.windows.get(window)?.clone();
        let alias = self.font_alias(host, w.font)?;
        let library = host.font_library(self.client)?;
        let next = parse_message_markup_with(
            text,
            &mut self.text.ruby,
            (self.text.override_color != -1).then_some(self.text.override_color as u32 & 0xffffff),
        );
        let page_chars = w.page.text.chars().count();
        let markup = append_message_markup(&w.page, next);
        ensure!(
            markup.text.chars().count() <= 65536,
            "BGI message glyph budget"
        );
        let origin = if w.page.text.is_empty() {
            let mut point = w.cursor;
            if w.variant == 1 {
                if point[1] == w.valid[1] {
                    point[1] = point[1].saturating_add(self.text.layout.boundary_offset);
                }
            } else if point[0] == w.valid[0] {
                point[0] = point[0].saturating_add(self.text.layout.boundary_offset);
            }
            point
        } else {
            w.page_origin
        };
        let size = if w.font_size > 0 {
            w.font_size as f32
        } else {
            26.
        };
        let pitch = crysirror_text::bitmap_layout::line_pitch(size, w.line_spacing);
        let mut advances = BTreeMap::new();
        for ch in markup.text.chars() {
            if !ch.is_control() && !advances.contains_key(&ch) {
                advances.insert(
                    ch,
                    library.metrics(&alias, size, ch)?.advance.max(size * 0.5)
                        * w.width_percent as f32
                        / 100.,
                );
            }
        }
        let advance = |ch| advances.get(&ch).copied().unwrap_or(0.);
        let state = MessageLayoutState {
            x: origin[0] as f32,
            y: origin[1] as f32,
            width: if w.variant == 1 {
                (w.valid[3] + 1 - origin[1]).max(1) as f32
            } else {
                (w.valid[2] + 1 - origin[0]).max(1) as f32
            },
            height: if w.variant == 1 {
                (origin[0] - w.valid[0] + 1).max(1) as f32
            } else {
                (w.valid[3] + 1 - origin[1]).max(1) as f32
            },
            font_size: size,
            line_height: pitch,
        };
        let (parsed, map) = wrap_parsed_message(markup.clone(), &state, &advance);
        let committed = map.get(page_chars).copied().unwrap_or(0);
        let (_, cursor) = place_message_glyphs(
            &parsed.text,
            [state.x, state.y],
            pitch,
            w.variant == 1,
            &advance,
        );
        let (fragment_positions, _) = place_message_glyphs(
            &parsed.text,
            [state.x, state.y],
            pitch,
            false,
            &|ch: char| if ch.is_ascii() { size * 0.5 } else { size },
        );
        self.text.fragments = parsed
            .link_spans
            .iter()
            .filter_map(|link| {
                let g = fragment_positions
                    .iter()
                    .find(|g| g.index >= link.start_char)?;
                Some(crysirror_bgi_vm::System92TextFragmentRecord {
                    text: link.text.clone(),
                    x: g.position[0].round() as i32,
                    y: g.position[1].round() as i32,
                })
            })
            .collect();
        let w = self.windows.get_mut(window)?;
        w.page = markup;
        w.page_origin = origin;
        w.cursor = cursor.map(|v| v.round() as i32);
        Ok((parsed, committed))
    }
    fn publish_window_message(
        &mut self,
        host: &mut dyn HostServices,
        window: i32,
        visible: usize,
    ) -> Result<()> {
        let w = self.windows.get(window)?.clone();
        let parsed = if self.messages.runtime.target_node == Some(window) {
            ParsedMessageMarkup {
                text: self.messages.runtime.full_text.clone(),
                ruby_spans: self.messages.runtime.ruby_spans.clone(),
                style_spans: self.messages.runtime.style_spans.clone(),
                link_spans: self.messages.runtime.link_spans.clone(),
                timeline: Vec::new(),
            }
        } else {
            w.page.clone()
        };
        let alias = self.font_alias(host, w.font)?;
        let ruby_alias = if let Some(name) = self.text.style.font_name.as_deref() {
            self.named_font_alias(host, name)?
        } else {
            alias.clone()
        };
        let library = host.font_library(self.client)?;
        let size = if w.font_size > 0 {
            w.font_size as f32
        } else {
            26.
        };
        let pitch = crysirror_text::bitmap_layout::line_pitch(size, w.line_spacing);
        let ascent = library.metrics(&alias, size, '字')?.ascent;
        let ruby_height = if self.text.style.ruby_height > 0 {
            self.text.style.ruby_height
        } else {
            size as i32 * self.text.layout.font_percent / 100
        }
        .max(4) as f32;
        let body_offset = ruby_height;
        let mut advances = BTreeMap::new();
        for ch in parsed.text.chars() {
            if !ch.is_control() && !advances.contains_key(&ch) {
                advances.insert(
                    ch,
                    library.metrics(&alias, size, ch)?.advance.max(size * 0.5)
                        * w.width_percent as f32
                        / 100.,
                );
            }
        }
        let advance = |ch| advances.get(&ch).copied().unwrap_or(0.);
        let (placed, _) = place_message_glyphs(
            &parsed.text,
            w.page_origin.map(|n| n as f32),
            pitch,
            w.variant == 1,
            &advance,
        );
        let mut glyphs = Vec::new();
        let mut italic = Vec::new();
        for g in &placed {
            if g.index >= visible {
                continue;
            }
            let style = parsed
                .style_spans
                .iter()
                .find(|s| s.start_char <= g.index && g.index < s.end_char)
                .map(|s| s.style)
                .unwrap_or_default();
            let color = rgb(style.packed_rgb.unwrap_or(w.color));
            let baseline = [g.position[0], g.position[1] + body_offset + ascent];
            let glyph = json!({"character":g.character,"baseline":baseline,"color":color,"bold":style.bold||w.font_style!=0});
            if style.italic {
                italic.push(glyph);
            } else {
                glyphs.push(glyph);
            }
        }
        let mut ruby_glyphs = Vec::new();
        // Ruby spans appear only after all their base glyphs are committed.
        for span in parsed.ruby_spans.iter().filter(|s| s.end_char <= visible) {
            let base = placed
                .iter()
                .filter(|g| g.index >= span.start_char && g.index < span.end_char)
                .collect::<Vec<_>>();
            if base.is_empty() {
                continue;
            }
            let width = base.iter().map(|g| advance(g.character)).sum::<f32>();
            let mut reading_advances = Vec::new();
            for ch in span.reading.chars() {
                reading_advances.push((
                    ch,
                    library
                        .metrics(&ruby_alias, ruby_height, ch)?
                        .advance
                        .max(ruby_height * 0.5),
                ));
            }
            let total = reading_advances.iter().map(|p| p.1).sum::<f32>();
            let mut x = base[0].position[0]
                + (width - total) * 0.5
                + size * 0.125
                + self.text.style.ruby_x_offset as f32;
            let y = base[0].position[1]
                + self.text.style.ruby_y_offset as f32
                + library.metrics(&ruby_alias, ruby_height, '字')?.ascent;
            for (ch, step) in reading_advances {
                ruby_glyphs.push(json!({"character":ch,"baseline":[x,y],"size":ruby_height,"color":rgb(w.color)}));
                x += step;
            }
        }
        let canvas = NativeWindow::canvas(window, 1);
        let mut commands = vec![
            json!({"op":"clear","id":canvas,"color":[0,0,0,0]}),
            json!({"op":"copy","source":NativeWindow::canvas(window,4),"destination":canvas,"replace":true,"rect":[0,0,w.size[0],w.size[1]],"position":[0,0],"opacity":255}),
        ];
        let shadow = self.message_shadow(&w);
        let shadow_offset = shadow
            .filter(|s| s.mode == 2)
            .map_or([0, 0], |s| s.offset.map(i32::from));
        let scale = w.width_percent as f32 / 100.;
        let origin = w.page_origin[0] as f32;
        let layer = |id: u32,
                     font: &str,
                     mut glyphs: Vec<Value>,
                     position: [i32; 2],
                     blur: Value| {
            for glyph in &mut glyphs {
                if let Some(x) = glyph["baseline"][0].as_f64() {
                    glyph["baseline"][0] = json!(origin + (x as f32 - origin) / scale);
                }
            }
            json!({"id":id,"order":[if matches!(id,0|1|4){0}else{1},id as i32],"rect":[position[0],position[1],w.size[0],w.size[1]],"transform":{"scale_x":scale,"pivot_x":origin},"paint":{"kind":"glyph_run","font":font,"size":size,"color":[255,255,255,255],"glyphs":glyphs,"blur":blur}})
        };
        if let Some(shadow) = shadow {
            let dx = u8::try_from(shadow.offset[0])?;
            let dy = u8::try_from(shadow.offset[1])?;
            for (id, font, source, oblique) in [
                (0, alias.as_str(), glyphs.clone(), false),
                (1, alias.as_str(), italic.clone(), true),
                (4, ruby_alias.as_str(), ruby_glyphs.clone(), false),
            ] {
                if source.is_empty() {
                    continue;
                }
                let mut source = source;
                for glyph in &mut source {
                    glyph["color"] = json!([
                        shadow.color[0],
                        shadow.color[1],
                        shadow.color[2],
                        ((u32::from(shadow.concentration) * 255 + 128) / 256) as u8
                    ]);
                }
                let (rx, ry) = if shadow.mode == 2 { (dx, dy) } else { (0, 0) };
                let blur = if oblique {
                    json!({"radius":0,"level":256,"kernel":{"oblique_box_sum":{"numerator":1,"denominator":5,"radius_x":rx,"radius_y":ry}}})
                } else if shadow.mode == 2 {
                    json!({"radius":0,"level":256,"kernel":{"box_sum_rect":{"radius_x":rx,"radius_y":ry}}})
                } else {
                    Value::Null
                };
                commands.push(json!({"op":"draw","id":canvas,"layer":layer(id,font,source,[i32::from(dx),i32::from(dy)],blur)}));
            }
        }
        if !glyphs.is_empty() {
            commands.push(
                json!({"op":"draw","id":canvas,"layer":layer(2,&alias,glyphs,shadow_offset,Value::Null)}),
            );
        }
        if !italic.is_empty() {
            let blur = json!({"radius":0,"level":256,"kernel":{"oblique_box_sum":{"numerator":1,"denominator":5,"radius_x":0,"radius_y":0}}});
            commands.push(
                json!({"op":"draw","id":canvas,"layer":layer(3,&alias,italic,shadow_offset,blur)}),
            );
        }
        if !ruby_glyphs.is_empty() {
            commands.push(json!({"op":"draw","id":canvas,"layer":layer(5,&ruby_alias,ruby_glyphs,shadow_offset,Value::Null)}));
        }
        self.submit(host, json!({"canvases":commands}))?;
        self.windows.get_mut(window)?.dirty = true;
        self.publish_windows(host)?;
        Ok(())
    }
    pub(crate) fn message_call(
        &mut self,
        host: &mut dyn HostServices,
        vm: &mut Machine,
        call: &NativeCall,
    ) -> Result<bool> {
        if !handles(call.group, call.id) {
            return Ok(false);
        }
        let a = &call.args;
        match (call.group, call.id) {
            (0x90, 0x91) => {
                ensure!((0..=2).contains(&a[0]), "BGI message scope mode");
                if a[0] == 1 {
                    target_checks::priority(a[1])?;
                }
                self.messages.input_scope_mode = a[0];
                if a[0] == 1 {
                    self.messages.input_scope_value = a[1];
                }
            }
            (0x90, 0x92) => {
                self.messages.input_filter = a[0] != 0;
                vm.set_message_input_filter(a[0] != 0);
            }
            (0x90, 0x94) => {
                self.messages.animation.set_glyph_delay(a[0]);
                self.messages.runtime.set_glyph_delay_ms(a[0]);
            }
            (0x90, 0x95) => {
                target_checks::division_count(a[0])?;
                self.messages.animation.set_reveal(a[0], a[1]);
            }
            (0x90, 0x96) => {
                target_checks::division_count(a[0])?;
                self.messages.animation.set_settle(a[0], a[1]);
            }
            (0x90, 0x97) => self.messages.animation.set_auto_advance(a[0], a[1]),
            (0x90, 0x99) => self.messages.caret_delay = a[0],
            (0x90, 0x9a) => self.messages.caret_position = [a[0], a[1], a[2]],
            (0x90, 0x9b) => {
                self.messages.delay_enabled = a[0] != 0;
                self.messages.delay_ms = a[1];
            }
            (0x90, 0x9c) => self.messages.shadow_enabled = a[0] != 0,
            (0x90, 0x9d) => {
                ensure!(a[0] >= 0 && a[1] >= 0, "BGI shadow offset");
                target_checks::effect_level(a[2])?;
                if a[0] <= 100 && a[1] <= 100 {
                    self.messages.shadow = [a[0], a[1], a[2]];
                }
            }
            (0x90, 0x9f) => self.messages.input_forces_completion = a[0] != 0,
            (0x92, 0x9f) => self.text.override_color = a[0],
            (0x90, 0x98) => {
                self.clear_message_caret(host)?;
                self.messages.caret_frames.clear();
                if a[0] > 1 {
                    let count = usize::try_from(a[0])?;
                    ensure!(count <= 65536, "BGI caret table entry budget");
                    let bytes = vm.memory(
                        a[1] as u32,
                        count
                            .checked_mul(4)
                            .context("BGI caret table length overflow")?,
                    )?;
                    for word in bytes.chunks_exact(4) {
                        let bitmap = i32::from_le_bytes(word.try_into().unwrap());
                        if bitmap == -1 {
                            self.messages.caret_frames.push(None);
                        } else {
                            self.window_image_source(bitmap)?;
                            self.messages.caret_frames.push(Some(bitmap));
                        }
                    }
                }
            }
            (0x90, 0x90) | (0x91, 0x90..=0x93) | (0x92, 0x90 | 0x91) => {
                let window = a[0];
                let text = vm.string(a[1])?;
                let w = self.windows.get(window)?;
                if call.group != 0x90 {
                    ensure!(
                        w.font_size != 0,
                        "指定されたウィンドウにはフォントが設定されていません"
                    );
                }
                if call.group == 0x92 {
                    let next_color = a[2] as u32 & 0xffffff;
                    if !w.page.text.is_empty() && w.color != next_color {
                        self.archive_window_text(host, window)?;
                    }
                    self.style_window_message(
                        window,
                        a[2] as u32,
                        [a[6], a[7], a[8], a[9], a[10]],
                    )?;
                }
                let (parsed, committed) = self.prepare_message_page(host, window, &text)?;
                self.messages
                    .runtime
                    .set_glyph_delay_ms(self.messages.animation.glyph_delay_ms);
                self.messages
                    .runtime
                    .start_continued_message(parsed, window, committed);
                self.messages.last_tick = Some(host.now_ms());
                let instant = (call.group == 0x92 && call.id == 0x91)
                    || (call.group == 0x91 && matches!(call.id, 0x91 | 0x93));
                if instant {
                    self.messages.runtime.reveal_all();
                    self.publish_window_message(host, window, self.messages.runtime.visible_chars)?;
                    vm.commit_call(&[])?;
                } else {
                    self.clear_message_caret(host)?;
                    self.messages.active = Some(window);
                    let class = if call.group == 0x90 {
                        NativeMessageProcedureClass::DspMsg
                    } else if call.group == 0x92 && self.windows.get(window)?.variant == 1 {
                        NativeMessageProcedureClass::DspMsgExVE
                    } else {
                        NativeMessageProcedureClass::DspMsgEx
                    };
                    let mut config = self.message_config(window, class);
                    config.auxiliary_input_mask = vm.message_auxiliary_input_mask();
                    let reversed = |i: usize| a[a.len() - 1 - i];
                    if call.group == 0x90 {
                        config.allow_high_bit_input = reversed(0) != 0;
                        config.end_wait_policy = reversed(1);
                        config.completion_control = reversed(2);
                        config.allow_auxiliary_input = true;
                    } else if call.group == 0x91 {
                        config.allow_high_bit_input = reversed(4) != 0;
                        config.end_wait_policy = reversed(5);
                        config.completion_control = reversed(6);
                        config.allow_auxiliary_input = reversed(3) != 0;
                    } else {
                        config.allow_high_bit_input = reversed(1) != 0;
                        config.end_wait_policy = reversed(2);
                        config.completion_control = reversed(3);
                        config.allow_auxiliary_input = reversed(0) != 0;
                    }
                    self.publish_window_message(host, window, self.messages.runtime.visible_chars)?;
                    vm.update_message_state(
                        self.messages.runtime.is_animating(),
                        self.message_auto_delay(),
                    );
                    vm.commit_call_with_start(&[], NativeProcedureStart::Message(config))?;
                }
                self.calls += 1;
                return Ok(true);
            }
            _ => unreachable!(),
        }
        vm.update_message_state(
            self.messages.runtime.is_animating(),
            self.message_auto_delay(),
        );
        vm.commit_call(&[])?;
        self.calls += 1;
        Ok(true)
    }
    fn clear_message_caret(&mut self, host: &mut dyn HostServices) -> Result<()> {
        if let Some(window) = self.messages.caret_window.take() {
            if self.windows.has_window(window) {
                self.windows.get_mut(window)?.icons.remove(&-1);
                self.rebuild_window_icons(host, window)?;
                self.publish_windows(host)?;
            }
        }
        self.messages.caret_tick = 0;
        self.messages.caret_index = 0;
        Ok(())
    }
    fn tick_message_caret(&mut self, host: &mut dyn HostServices, now: u32) -> Result<()> {
        let Some(window) = self.messages.active else {
            return self.clear_message_caret(host);
        };
        if self.messages.caret_frames.is_empty() {
            return self.clear_message_caret(host);
        }
        let delay = self.messages.caret_delay.max(0) as u32;
        if self.messages.caret_window == Some(window) {
            if now.wrapping_sub(self.messages.caret_tick) < delay {
                return Ok(());
            }
            self.messages.caret_index =
                (self.messages.caret_index + 1) % self.messages.caret_frames.len();
        } else {
            self.clear_message_caret(host)?;
            self.messages.caret_index = 0;
        }
        self.messages.caret_tick = now;
        self.messages.caret_window = Some(window);
        let frame = self.messages.caret_frames[self.messages.caret_index];
        let w = self.windows.get(window)?;
        let offset = [
            self.messages.caret_position[1],
            self.messages.caret_position[2],
        ];
        let point = if self.messages.caret_position[0] == 1 {
            offset
        } else {
            let mut cursor = w.cursor;
            let size = if w.font_size > 0 { w.font_size } else { 26 };
            let ruby = if self.text.style.ruby_height > 0 {
                self.text.style.ruby_height
            } else {
                size * self.text.layout.font_percent / 100
            }
            .max(4);
            cursor[1] = cursor[1].wrapping_add(ruby);
            let widest = self
                .messages
                .caret_frames
                .iter()
                .flatten()
                .filter_map(|&bitmap| self.window_image_source(bitmap).ok())
                .map(|(_, size)| size[0])
                .max()
                .unwrap_or(0);
            if i64::from(cursor[0]) + i64::from(widest) > i64::from(w.valid[2]) + 1 {
                cursor[0] = w.page_origin[0];
                cursor[1] = cursor[1].wrapping_add(
                    crysirror_text::bitmap_layout::line_pitch(size as f32, w.line_spacing).round()
                        as i32,
                );
            }
            [
                cursor[0].wrapping_add(offset[0]),
                cursor[1].wrapping_add(offset[1]),
            ]
        };
        let w = self.windows.get_mut(window)?;
        if let Some(bitmap) = frame {
            w.icons.insert(
                -1,
                crate::window_api::NativeIcon {
                    bitmap,
                    position: point,
                },
            );
        } else {
            w.icons.remove(&-1);
        }
        self.rebuild_window_icons(host, window)?;
        self.publish_windows(host)?;
        Ok(())
    }
}
