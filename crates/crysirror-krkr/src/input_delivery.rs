//! Retained native input delivery over shared asynchronous image preparation.
//! An Enter/Leave/Click callback may replace a bitmap. Rechecks wait for that
//! exact composition before proceeding; already delivered callbacks never replay.
use super::*;
use std::collections::VecDeque;
type Hit = Option<(usize, i64, i64)>;

pub(super) struct Delivery {
    posted: bool,
    frame: InputFrame,
    receipt: Option<crysirror_core::input_buffer::InputReceipt>,
    window: ObjectHandle,
    flags: i64,
    previous_hover: Option<usize>,
    pointer_active: bool,
    hit: Hit,
    actions: VecDeque<Action>,
    hit_candidates: Option<VecDeque<(usize, i64, i64)>>,
}
#[derive(Clone)]
enum Action {
    Call(ObjectHandle, &'static str, Vec<Variant>),
    Hover {
        stage: u8,
        selected: Hit,
        moved: bool,
    },
    HoverDone(Hit, bool),
    ResolveOnly,
    PointerFeedback,
    Down(i64),
    DownTarget(i64),
    CaptureAfter(usize),
    ReleaseBit(u64),
    ClickWindow,
    ClickTarget(i64, i64),
    UpTarget(i64),
    ReleaseCapture,
    LayerKey(&'static str, i64),
    LayerWheel(i64),
    Finish,
}
impl Delivery {
    pub(super) fn is_posted(&self) -> bool {
        self.posted
    }
    pub(super) fn pad_close(frame: InputFrame, owner: ObjectHandle, notify: bool) -> Self {
        let mut value = Self::posted(frame, owner, "__crysirror_pad_close", 0, 0);
        value.actions = VecDeque::new();
        if notify {
            value
                .actions
                .push_back(Action::Call(owner, "onClose", vec![]));
        }
        value
            .actions
            .push_back(Action::Call(owner, "__crysirror_pad_close", vec![]));
        value.actions.push_back(Action::Finish);
        value
    }
    pub(super) fn posted(
        frame: InputFrame,
        window: ObjectHandle,
        method: &'static str,
        key: i64,
        flags: i64,
    ) -> Self {
        let args = if method == "onKeyPress" {
            vec![Variant::String(
                char::from_u32(key as u32).unwrap_or('\u{fffd}').to_string(),
            )]
        } else {
            vec![Variant::Integer(key), Variant::Integer(flags)]
        };
        Self {
            posted: true,
            frame,
            receipt: None,
            window,
            flags,
            previous_hover: None,
            pointer_active: false,
            hit: None,
            hit_candidates: None,
            actions: VecDeque::from([
                Action::Call(window, method, args),
                Action::LayerKey(method, key),
                Action::Finish,
            ]),
        }
    }
    pub(super) fn roots(&self, out: &mut Vec<Variant>) {
        out.push(Variant::Object(self.window));
        if let Some(candidates) = &self.hit_candidates {
            out.extend(candidates.iter().map(|(id, _, _)| Variant::Object(ObjectHandle(*id))));
        }
        for action in &self.actions {
            match action {
                Action::Call(h, _, args) => {
                    out.push(Variant::Object(*h));
                    out.extend(args.iter().cloned());
                }
                Action::Hover {
                    selected: Some((id, _, _)),
                    ..
                }
                | Action::HoverDone(Some((id, _, _)), _)
                | Action::CaptureAfter(id) => out.push(Variant::Object(ObjectHandle(*id))),
                _ => {}
            }
        }
        if let Some((id, _, _)) = self.hit {
            out.push(Variant::Object(ObjectHandle(id)));
        }
    }
    fn call(&mut self, h: ObjectHandle, method: &'static str, args: Vec<Variant>) {
        self.actions.push_front(Action::Call(h, method, args));
    }
    fn mouse_args(&self, x: i64, y: i64, button: i64) -> Vec<Variant> {
        [x, y, button, self.flags].map(Variant::Integer).to_vec()
    }
}
impl KrkrVm {
    /// Returns None only while a shared image dependency is pending. The cursor
    /// and input receipt remain owned here until the entire delivery completes.
    pub(super) fn deliver_input(
        &mut self,
        host: &mut dyn HostServices,
        frame: InputFrame,
        receipt: Option<crysirror_core::input_buffer::InputReceipt>,
    ) -> Result<Option<Option<crysirror_core::input_buffer::InputReceipt>>> {
        if self.input_delivery.is_none() {
            let rt = self.rt.as_mut().context("KiriKiri runtime absent")?;
            rt.host_mut().input = frame;
            rt.host_mut().input.buttons_down |= frame.buttons_pressed;
            let window = rt
                .host()
                .modal_windows
                .last()
                .map(|m| ObjectHandle(m.window))
                .or_else(|| {
                    rt.host()
                        .windows
                        .iter()
                        .find(|(_, w)| w.visible && w.output)
                        .map(|(id, _)| ObjectHandle(*id))
                });
            if rt.host().event_disabled || window.is_none() {
                self.last_down = frame.buttons_down;
                self.last_pointer = [frame.pointer_x, frame.pointer_y];
                return Ok(Some(receipt));
            }
            let window = window.unwrap();
            let bits = frame.buttons_down | frame.buttons_pressed;
            let flags = i64::from(bits & buttons::SHIFT != 0)
                | (i64::from(bits & buttons::CTRL != 0) << 2)
                | (i64::from(bits & buttons::MOUSE_LEFT != 0) << 3)
                | (i64::from(bits & buttons::MOUSE_RIGHT != 0) << 4);
            let mask = buttons::MOUSE_LEFT | buttons::MOUSE_RIGHT;
            let moved = [frame.pointer_x, frame.pointer_y] != self.last_pointer;
            let active = moved
                || ((frame.buttons_down ^ self.last_down) | frame.buttons_pressed) & mask != 0;
            let recheck = rt.host().now.saturating_sub(rt.host().mouse.last_recheck) > 1000;
            if recheck {
                let now = rt.host().now;
                rt.host_mut().mouse.last_recheck = now;
            }
            let mut actions = VecDeque::new();
            let (x, y) = (i64::from(frame.pointer_x), i64::from(frame.pointer_y));
            if moved {
                rt.set_object_member(window, "__temporary_cursor_hide", Variant::Integer(0));
                self.last_pointer = [frame.pointer_x, frame.pointer_y];
                actions.push_back(Action::Call(
                    window,
                    "onMouseMove",
                    [x, y, flags].map(Variant::Integer).to_vec(),
                ));
            }
            if moved || recheck {
                actions.push_back(Action::Hover {
                    stage: 0,
                    selected: None,
                    moved,
                });
            } else if active {
                actions.push_back(Action::ResolveOnly);
            }
            actions.push_back(Action::PointerFeedback);
            for (bit, button) in [(buttons::MOUSE_LEFT, 0), (buttons::MOUSE_RIGHT, 1)] {
                let down = frame.buttons_down & bit != 0;
                let was = self.last_down & bit != 0;
                if (down || frame.buttons_pressed & bit != 0) && !was {
                    actions.push_back(Action::Down(button));
                }
                if !down && (was || frame.buttons_pressed & bit != 0) {
                    actions.push_back(Action::ReleaseBit(bit));
                    if button == 0 {
                        actions.push_back(Action::ClickWindow);
                    }
                    actions.push_back(Action::Call(
                        window,
                        "onMouseUp",
                        [x, y, button, flags].map(Variant::Integer).to_vec(),
                    ));
                    actions.push_back(Action::UpTarget(button));
                }
            }
            for &(bit, key) in crate::pointer::KEYS {
                let args = vec![Variant::Integer(key), Variant::Integer(flags)];
                if frame.buttons_pressed & bit != 0 {
                    actions.push_back(Action::Call(window, "onKeyDown", args.clone()));
                    actions.push_back(Action::LayerKey("onKeyDown", key));
                }
                if frame.buttons_down & bit == 0
                    && (self.last_down | frame.buttons_pressed) & bit != 0
                {
                    actions.push_back(Action::ReleaseBit(bit));
                    actions.push_back(Action::Call(window, "onKeyUp", args));
                    actions.push_back(Action::LayerKey("onKeyUp", key));
                }
            }
            for (bit, delta) in [(buttons::WHEEL_UP, 120), (buttons::WHEEL_DOWN, -120)] {
                if frame.buttons_pressed & bit != 0 {
                    actions.push_back(Action::Call(
                        window,
                        "onMouseWheel",
                        [flags, delta, x, y].map(Variant::Integer).to_vec(),
                    ));
                    actions.push_back(Action::LayerWheel(delta));
                }
            }
            actions.push_back(Action::Finish);
            self.input_delivery = Some(Delivery {
                posted: false,
                frame,
                receipt,
                window,
                flags,
                previous_hover: rt.host().mouse.hover,
                pointer_active: active,
                hit: None,
                hit_candidates: None,
                actions,
            });
        }
        for _ in 0..512 {
            // Bitmap writes queued by the preceding callback must complete
            // before its next native recheck observes the image generation.
            if !self.drive_sound(host)? {
                return Ok(None);
            }
            let mut delivery = self.input_delivery.take().unwrap();
            let Some(action) = delivery.actions.pop_front() else {
                return Ok(Some(delivery.receipt));
            };
            let (x, y) = (
                i64::from(delivery.frame.pointer_x),
                i64::from(delivery.frame.pointer_y),
            );
            let mut pending = false;
            match action.clone() {
                Action::Call(h, m, args) => {
                    self.input_delivery = Some(delivery);
                    let old = self.rt.as_ref().unwrap().host().can_suspend;
                    // The action was dequeued before dispatch, so a native
                    // image/clock wait can resume its TJS stack without replay.
                    self.rt.as_mut().unwrap().host_mut().can_suspend = true;
                    let result = self.call(h, m, args);
                    self.rt.as_mut().unwrap().host_mut().can_suspend = old;
                    result?;
                    continue;
                }
                Action::Hover {
                    stage,
                    selected,
                    moved,
                } => {
                    if let Some(hit) = self.prepared_mouse_target(&mut delivery, host, x, y, true)? {
                        match stage {
                            0 => {
                                let previous = self.rt.as_ref().unwrap().host().mouse.hover;
                                if previous != hit.map(|v| v.0) {
                                    delivery.actions.push_front(Action::Hover {
                                        stage: 1,
                                        selected: None,
                                        moved,
                                    });
                                    if let Some(old) = previous {
                                        delivery.call(ObjectHandle(old), "onMouseLeave", vec![]);
                                    }
                                } else {
                                    delivery.actions.push_front(Action::HoverDone(hit, moved));
                                }
                            }
                            1 => {
                                if let Some((id, _, _)) = hit {
                                    delivery.actions.push_front(Action::Hover {
                                        stage: 2,
                                        selected: hit,
                                        moved,
                                    });
                                    delivery.call(ObjectHandle(id), "onMouseEnter", vec![]);
                                } else {
                                    delivery.actions.push_front(Action::HoverDone(None, moved));
                                }
                            }
                            _ => {
                                delivery.actions.push_front(Action::HoverDone(hit, moved));
                                if selected.map(|v| v.0) != hit.map(|v| v.0) {
                                    if let Some((id, _, _)) = hit {
                                        delivery.call(ObjectHandle(id), "onMouseEnter", vec![]);
                                    }
                                    if let Some((id, _, _)) = selected {
                                        delivery.call(ObjectHandle(id), "onMouseLeave", vec![]);
                                    }
                                }
                            }
                        }
                    } else {
                        pending = true;
                    }
                }
                Action::HoverDone(hit, moved) => {
                    self.rt.as_mut().unwrap().host_mut().mouse.hover = hit.map(|v| v.0);
                    delivery.hit = hit;
                    if moved {
                        if let Some((id, lx, ly)) = hit {
                            delivery.call(
                                ObjectHandle(id),
                                "onMouseMove",
                                [lx, ly, delivery.flags].map(Variant::Integer).to_vec(),
                            );
                        }
                    }
                }
                Action::ResolveOnly => {
                    if let Some(hit) = self.prepared_mouse_target(&mut delivery, host, x, y, true)? {
                        delivery.hit = hit;
                    } else {
                        pending = true;
                    }
                }
                Action::PointerFeedback => {
                    if delivery.pointer_active
                        || delivery.previous_hover != self.rt.as_ref().unwrap().host().mouse.hover
                    {
                        crate::tooltip::pointer(
                            self.rt.as_mut().unwrap(),
                            delivery.hit.map(|v| v.0),
                            [x as f32, y as f32],
                            (delivery.frame.buttons_down | delivery.frame.buttons_pressed)
                                & (buttons::MOUSE_LEFT | buttons::MOUSE_RIGHT)
                                != 0,
                        )
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                        crate::pointer::apply(
                            self.rt.as_mut().unwrap(),
                            delivery.hit.map_or(delivery.window, |v| ObjectHandle(v.0)),
                        );
                    }
                }
                Action::Down(button) => {
                    self.rt.as_mut().unwrap().host_mut().mouse.down_point = Some([x, y]);
                    delivery.actions.push_front(Action::DownTarget(button));
                    delivery.actions.push_front(Action::Hover {
                        stage: 0,
                        selected: None,
                        moved: false,
                    });
                    let args = delivery.mouse_args(x, y, button);
                    delivery.call(delivery.window, "onMouseDown", args);
                }
                Action::DownTarget(button) => {
                    if let Some(hit) = self.prepared_mouse_target(&mut delivery, host, x, y, true)? {
                        if let Some((id, lx, ly)) = hit {
                            self.rt.as_mut().unwrap().host_mut().mouse.release_called = false;
                            delivery.actions.push_front(Action::CaptureAfter(id));
                            let args = delivery.mouse_args(lx, ly, button);
                            delivery.call(ObjectHandle(id), "onMouseDown", args);
                        } else {
                            crate::native_mouse::release(self.rt.as_mut().unwrap());
                        }
                    } else {
                        pending = true;
                    }
                }
                Action::CaptureAfter(id) => {
                    let mouse = &mut self.rt.as_mut().unwrap().host_mut().mouse;
                    if mouse.capture != Some(id) {
                        let no_capture = mouse.release_called;
                        mouse.capture = None;
                        mouse.release_called = true;
                        if !no_capture {
                            mouse.capture = Some(id);
                        }
                    }
                }
                Action::ReleaseBit(bit) => {
                    self.rt.as_mut().unwrap().host_mut().input.buttons_down &= !bit
                }
                Action::ClickWindow => {
                    let [cx, cy] = self
                        .rt
                        .as_ref()
                        .unwrap()
                        .host()
                        .mouse
                        .down_point
                        .unwrap_or([x, y]);
                    delivery.actions.push_front(Action::ClickTarget(cx, cy));
                    delivery.call(
                        delivery.window,
                        "onClick",
                        [cx, cy].map(Variant::Integer).to_vec(),
                    );
                }
                Action::ClickTarget(cx, cy) => {
                    if let Some(hit) = self.prepared_mouse_target(&mut delivery, host, cx, cy, false)? {
                        if let Some((id, lx, ly)) = hit {
                            if self.rt.as_ref().unwrap().host().mouse.capture == Some(id) {
                                delivery.call(
                                    ObjectHandle(id),
                                    "onClick",
                                    [lx, ly].map(Variant::Integer).to_vec(),
                                );
                            }
                        }
                    } else {
                        pending = true;
                    }
                }
                Action::UpTarget(button) => {
                    if let Some(hit) = self.prepared_mouse_target(&mut delivery, host, x, y, true)? {
                        if let Some((id, lx, ly)) = hit {
                            if delivery.frame.buttons_down
                                & (buttons::MOUSE_LEFT | buttons::MOUSE_RIGHT)
                                == 0
                            {
                                delivery.actions.push_front(Action::Hover {
                                    stage: 0,
                                    selected: None,
                                    moved: false,
                                });
                                delivery.actions.push_front(Action::ReleaseCapture);
                            }
                            let args = delivery.mouse_args(lx, ly, button);
                            delivery.call(ObjectHandle(id), "onMouseUp", args);
                        }
                    } else {
                        pending = true;
                    }
                }
                Action::ReleaseCapture => crate::native_mouse::release(self.rt.as_mut().unwrap()),
                Action::LayerKey(event, key) => {
                    self.input_delivery = Some(delivery);
                    self.layer_key(
                        self.input_delivery.as_ref().unwrap().window,
                        event,
                        key,
                        self.input_delivery.as_ref().unwrap().flags,
                    )?;
                    continue;
                }
                Action::LayerWheel(delta) => {
                    if self.rt.as_ref().unwrap().object_valid(delivery.window) {
                        let window = delivery.window;
                        let flags = delivery.flags;
                        self.input_delivery = Some(delivery);
                        crate::native_members::primary_wheel(
                            self.rt.as_mut().unwrap(),
                            window,
                            flags,
                            delta,
                            x,
                            y,
                        )
                        .map_err(|e| anyhow::anyhow!("TJS event onMouseWheel: {e}"))?;
                        continue;
                    }
                }
                Action::Finish => {
                    self.rt.as_mut().unwrap().host_mut().input = delivery.frame;
                    self.last_down = delivery.frame.buttons_down;
                    return Ok(Some(delivery.receipt));
                }
            }
            if pending {
                delivery.actions.push_front(action);
            }
            self.input_delivery = Some(delivery);
            if pending {
                return Ok(None);
            }
        }
        anyhow::bail!("native input callback budget")
    }
    fn prepared_mouse_target(
        &mut self, delivery: &mut Delivery, host: &mut dyn HostServices,
        x: i64, y: i64, capture: bool,
    ) -> Result<Option<Hit>> {
        if capture && let Some(hit) = self.captured_mouse_target(x, y)? {
            delivery.hit_candidates = None;
            return Ok(Some(Some(hit)));
        }
        let candidates = delivery.hit_candidates.get_or_insert_with(|| {
            self.rt.as_ref().unwrap().host().hits(x, y).into()
        });
        while let Some(&(id, lx, ly)) = candidates.front() {
            let rt = self.rt.as_mut().unwrap();
            if !rt.object_valid(ObjectHandle(id)) {
                candidates.pop_front();
                continue;
            }
            if !crate::image_api::prepare_hit_image(rt, host, self.client, id, lx, ly)? {
                return Ok(None);
            }
            candidates.pop_front();
            let old = std::mem::replace(&mut rt.host_mut().can_suspend, false);
            let result = self.hit_candidate(id, lx, ly);
            self.rt.as_mut().unwrap().host_mut().can_suspend = old;
            if let Some(enabled) = result? {
                delivery.hit_candidates = None;
                return Ok(Some(enabled.then_some((id, lx, ly))));
            }
        }
        delivery.hit_candidates = None;
        Ok(Some(None))
    }
}
