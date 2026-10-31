// SPDX-License-Identifier: MPL-2.0
//! CPU-owned input receivers with stable priority, coverage and leased delivery.
//! Registry state is persistent; physical input, captures and receipts are live
//! session state and never replay after deserialization. No VM callbacks run here.
use crate::{input_buffer::InputInbox, ui::WidgetHitMask};
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    input_buffer::{InputBuffer, InputReceipt},
    service::{buttons, InputFrame},
    state::ClientId,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
};
const MAX_RECEIVERS: usize = 4096;
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);
fn epoch() -> Result<u64> {
    NEXT_EPOCH
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| anyhow::anyhow!("input routing epoch exhausted"))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRouteSource {
    Keyboard,
    Pointer,
    Controller,
}
impl InputRouteSource {
    fn mask(self) -> u8 {
        match self {
            Self::Keyboard => 1,
            Self::Pointer => 2,
            Self::Controller => 4,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRoutePhase {
    Press,
    Release,
    Repeat,
    /// Explicit backend drag; each eligible receiver gets its own ACK lease.
    Drag,
    Move,
    HoverEnter,
    HoverLeave,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRouteEvent {
    pub source: InputRouteSource,
    pub code: u32,
    pub phase: InputRoutePhase,
    pub point: [i32; 2],
    /// Required only for Drag: eligibility uses this origin, not current point.
    #[serde(default)]
    pub origin: Option<[i32; 2]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputReceiverSpec {
    pub id: u32,
    pub priority: i32,
    /// Keyboard=1, pointer=2, controller=4. Zero and unknown bits are invalid.
    pub sources: u8,
    /// None is global for pointer press; it never participates in hover.
    pub rect: Option<[i64; 4]>,
    #[serde(default)]
    pub coverage: Option<WidgetHitMask>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "yes")]
    pub hover_enabled: bool,
    /// Accept explicit pointer drags. Automatic hardware drag detection is a
    /// separate backend operation; this receiver flag does not invent events.
    #[serde(default = "yes")]
    pub drag_enabled: bool,
}
fn yes() -> bool {
    true
}
impl InputReceiverSpec {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.sources > 0 && self.sources & !7 == 0,
            "input receiver source mask"
        );
        if let Some(rect) = self.rect {
            ensure!(
                rect.iter().all(|v| v.unsigned_abs() <= 1_000_000) && rect[2] >= 0 && rect[3] >= 0,
                "input receiver rectangle"
            );
            if let Some(mask) = &self.coverage {
                mask.validate(rect.map(|v| v as f32))?;
            }
        } else {
            ensure!(
                self.coverage.is_none(),
                "global input receiver cannot have pixel coverage"
            );
        }
        Ok(())
    }
    fn hit(&self, point: [i32; 2], hover: bool) -> bool {
        match self.rect {
            None => !hover,
            Some(rect) => self.coverage.as_ref().map_or_else(
                || crysirror_core::region_map::contains_rect(rect, point.map(i64::from)),
                |mask| mask.hit(point[0], point[1], rect.map(|v| v as f32)),
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDeliveryReceipt {
    pub client: u32,
    #[serde(with = "crate::input_buffer::exact_u64")]
    pub epoch: u64,
    #[serde(with = "crate::input_buffer::exact_u64")]
    pub event: u64,
    #[serde(with = "crate::input_buffer::exact_u64")]
    pub delivery: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDelivery {
    pub receiver: u32,
    pub event: InputRouteEvent,
    pub receipt: InputDeliveryReceipt,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputRouteCommand {
    Put {
        receiver: InputReceiverSpec,
    },
    Remove {
        id: u32,
    },
    Clear,
    SetEnabled {
        id: u32,
        enabled: bool,
    },
    SetPriority {
        id: u32,
        priority: i32,
    },
    Acknowledge {
        receipt: InputDeliveryReceipt,
        handled: bool,
    },
    Cancel,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputRouteStats {
    pub receivers: usize,
    pub queued_samples: usize,
    pub queued_events: usize,
    pub leased: bool,
    pub captures: usize,
    pub hover: Option<u32>,
    #[serde(with = "crate::input_buffer::exact_u64")]
    pub epoch: u64,
}
#[derive(Clone, Debug)]
struct Plan {
    event: InputRouteEvent,
    receivers: Vec<u32>,
    cursor: usize,
    serial: u64,
}
#[derive(Clone, Debug, Default)]
struct Live {
    epoch: u64,
    next_event: u64,
    next_delivery: u64,
    journal: Option<InputInbox>,
    sample: Option<InputReceipt>,
    events: VecDeque<InputRouteEvent>,
    plan: Option<Plan>,
    leased: Option<InputDelivery>,
    captures: BTreeMap<u32, u32>,
    hover: Option<u32>,
    hover_marked: BTreeSet<u32>,
    hover_dirty: bool,
    point: Option<[i32; 2]>,
}
impl Live {
    fn init(&mut self, client: u32) -> Result<()> {
        if self.epoch == 0 {
            self.epoch = epoch()?;
            self.journal = Some(InputInbox(InputBuffer::new(client, self.epoch, 4096)?));
        }
        Ok(())
    }
    fn finish(&mut self) {
        if let Some(plan) = self.plan.take() {
            if plan.event.phase == InputRoutePhase::Release {
                self.captures.remove(&plan.event.code);
            }
        }
        self.leased = None;
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Domain {
    receivers: Vec<InputReceiverSpec>,
    #[serde(skip)]
    live: Live,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRouting {
    clients: BTreeMap<u32, Domain>,
    #[serde(default)]
    clock_ms: u64,
}
impl InputRouting {
    pub fn client_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.clients.keys().copied()
    }
    pub fn active(&self) -> bool {
        !self.clients.is_empty()
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.clients.len() <= 64, "input routing owner count");
        let mut total = 0usize;
        let mut mask_bytes = 0usize;
        for (client, d) in &self.clients {
            ensure!(
                *client > 0 && !d.receivers.is_empty() && d.receivers.len() <= MAX_RECEIVERS,
                "input routing receiver count/owner"
            );
            let mut ids = std::collections::BTreeSet::new();
            let mut last = i32::MAX;
            for r in &d.receivers {
                r.validate()?;
                ensure!(
                    ids.insert(r.id) && r.priority <= last,
                    "input receiver identity/order"
                );
                last = r.priority;
                mask_bytes = mask_bytes
                    .checked_add(r.coverage.as_ref().map_or(0, |m| m.coverage.len()))
                    .context("input coverage budget overflow")?;
            }
            total += d.receivers.len();
            ensure!(
                d.live.hover_marked.iter().all(|id| ids.contains(id)),
                "input hover marker owner missing"
            );
            if let Some(j) = &d.live.journal {
                ensure!(
                    j.0.client() == *client && j.0.epoch() == d.live.epoch,
                    "input routing journal ownership"
                );
                j.0.snapshot().validate()?;
            }
            if let Some(l) = &d.live.leased {
                ensure!(
                    l.receipt.client == *client
                        && l.receipt.epoch == d.live.epoch
                        && d.live.plan.is_some(),
                    "input delivery ownership"
                );
            }
        }
        ensure!(
            total <= 16384 && mask_bytes <= 128 * 1024 * 1024,
            "input routing total resources"
        );
        Ok(())
    }
    pub fn receiver(&self, client: ClientId, id: u32) -> Option<&InputReceiverSpec> {
        self.clients
            .get(&client.0)?
            .receivers
            .iter()
            .find(|r| r.id == id)
    }
    pub fn stats(&self, client: ClientId) -> InputRouteStats {
        let d = self.clients.get(&client.0);
        let l = d.map(|d| &d.live);
        InputRouteStats {
            receivers: d.map_or(0, |d| d.receivers.len()),
            queued_samples: l.and_then(|l| l.journal.as_ref()).map_or(0, |j| j.0.len()),
            queued_events: l.map_or(0, |l| l.events.len()),
            leased: l.is_some_and(|l| l.leased.is_some()),
            captures: l.map_or(0, |l| l.captures.len()),
            hover: l.and_then(|l| l.hover),
            epoch: l.map_or(0, |l| l.epoch),
        }
    }
    /// Registry and acknowledgements share one atomic owner command boundary.
    pub fn apply_batch(&mut self, client: ClientId, commands: &[InputRouteCommand]) -> Result<()> {
        ensure!(client.0 > 0, "input routing owner");
        let mut staged = self.clone();
        for command in commands {
            match command {
                InputRouteCommand::Clear => {
                    staged.clients.remove(&client.0);
                }
                InputRouteCommand::Acknowledge { receipt, handled } => {
                    staged.acknowledge(client, *receipt, *handled)?
                }
                InputRouteCommand::Cancel => {
                    let d = staged
                        .clients
                        .get_mut(&client.0)
                        .context("input routing owner missing")?;
                    d.live.init(client.0)?;
                    d.live.journal.as_mut().unwrap().0.discard(client.0)?;
                    d.live.sample = None;
                    d.live.events.clear();
                    d.live.finish();
                    d.live.captures.clear();
                    d.live.hover = None;
                    d.live.hover_marked.clear();
                    d.live.next_delivery = d
                        .live
                        .next_delivery
                        .checked_add(1)
                        .context("input delivery serial overflow")?;
                }
                command => {
                    let d = staged.clients.entry(client.0).or_default();
                    ensure!(
                        d.live.leased.is_none() && d.live.plan.is_none(),
                        "input registry mutation during unacknowledged delivery"
                    );
                    match command {
                        InputRouteCommand::Put { receiver } => {
                            receiver.validate()?;
                            if let Some(old) = d.receivers.iter_mut().find(|r| r.id == receiver.id)
                            {
                                *old = receiver.clone();
                            } else {
                                d.receivers.push(receiver.clone());
                            }
                        }
                        InputRouteCommand::Remove { id } => {
                            let index = d
                                .receivers
                                .iter()
                                .position(|r| r.id == *id)
                                .context("input receiver missing")?;
                            d.receivers.remove(index);
                            d.live.hover_marked.remove(id);
                            if d.live.hover == Some(*id) {
                                d.live.hover = None;
                            }
                        }
                        InputRouteCommand::SetEnabled { id, enabled } => {
                            d.receivers
                                .iter_mut()
                                .find(|r| r.id == *id)
                                .context("input receiver missing")?
                                .enabled = *enabled;
                            if !*enabled {
                                d.live.hover_marked.remove(id);
                                if d.live.hover == Some(*id) {
                                    d.live.hover = None;
                                }
                            }
                        }
                        InputRouteCommand::SetPriority { id, priority } => {
                            d.receivers
                                .iter_mut()
                                .find(|r| r.id == *id)
                                .context("input receiver missing")?
                                .priority = *priority
                        }
                        _ => unreachable!(),
                    }
                    d.live.hover_dirty = true;
                    // Rust's stable sort preserves the last sorted equal-priority
                    // list, including priority changes and later equalization.
                    d.receivers.sort_by(|a, b| b.priority.cmp(&a.priority));
                    if d.receivers.is_empty() {
                        staged.clients.remove(&client.0);
                    }
                }
            }
        }
        staged.validate()?;
        *self = staged;
        Ok(())
    }
    /// Preflight all journals before advancing any clock or admitting a sample.
    pub fn admit_frame(&mut self, delta_ms: u64, frame: InputFrame) -> Result<()> {
        let time = self
            .clock_ms
            .checked_add(delta_ms)
            .context("input routing clock overflow")?;
        let mut staged = self.clone();
        for (client, d) in &mut staged.clients {
            d.live.init(*client)?;
            let j = &mut d.live.journal.as_mut().unwrap().0;
            if j.observed().is_none() {
                d.live.point = Some([frame.pointer_x, frame.pointer_y]);
                d.live.hover_dirty = true;
                // Existing held buttons at registration/restore do not replay.
                // Explicit new edges in this frame are still delivered.
                let mut snapshot = j.snapshot();
                let base = frame.buttons_down & !frame.buttons_pressed;
                snapshot.base_down = base;
                snapshot.observed = Some(InputFrame {
                    buttons_down: base,
                    buttons_pressed: 0,
                    ..frame
                });
                snapshot.observed_time = time;
                *j = InputBuffer::restore(*client, d.live.epoch, snapshot)?;
            }
            j.preflight_push(*client, time, frame)?;
        }
        for (client, d) in &mut staged.clients {
            d.live
                .journal
                .as_mut()
                .unwrap()
                .0
                .push(*client, time, frame)?;
        }
        staged.clock_ms = time;
        *self = staged;
        Ok(())
    }
    /// Explicit device-neutral event admission, e.g. a controller backend. The
    /// ordinary InputFrame path never claims to have observed such a device.
    pub fn dispatch(&mut self, client: ClientId, event: InputRouteEvent) -> Result<()> {
        if event.phase == InputRoutePhase::Drag {
            ensure!(
                event.source == InputRouteSource::Pointer && event.origin.is_some(),
                "pointer drag requires its explicit origin"
            );
        } else {
            ensure!(
                event.origin.is_none(),
                "non-drag input cannot carry a drag origin"
            );
        }
        ensure!(
            event.phase != InputRoutePhase::Move || event.source == InputRouteSource::Pointer,
            "only pointer input can move hover"
        );
        ensure!(
            !matches!(
                event.phase,
                InputRoutePhase::HoverEnter | InputRoutePhase::HoverLeave
            ),
            "hover events are engine-derived"
        );
        let d = self
            .clients
            .get_mut(&client.0)
            .context("input routing owner missing")?;
        d.live.init(client.0)?;
        ensure!(d.live.events.len() < 4096, "input route event capacity");
        ensure!(
            d.live.sample.is_none() && d.live.journal.as_ref().unwrap().0.is_empty(),
            "explicit input backend must drain earlier journal samples first"
        );
        d.live.events.push_back(event);
        Ok(())
    }
    fn start(d: &mut Domain, event: InputRouteEvent) -> Result<()> {
        use InputRoutePhase as P;
        if event.phase == P::Move {
            let hover = d
                .receivers
                .iter()
                .find(|r| {
                    r.enabled && r.hover_enabled && r.sources & 2 != 0 && r.hit(event.point, true)
                })
                .map(|r| r.id);
            let mut enter = hover;
            let mut events = Vec::new();
            // Suspending hover preserves each receiver's marker. It must not
            // manufacture an exit, or re-enter an already marked receiver on
            // resumption. Other active receivers still receive ordered exits.
            for r in &d.receivers {
                if r.enabled && r.hover_enabled && d.live.hover_marked.contains(&r.id) {
                    if Some(r.id) == hover {
                        enter = None;
                    } else {
                        d.live.hover_marked.remove(&r.id);
                        events.push(InputRouteEvent {
                            code: r.id,
                            phase: P::HoverLeave,
                            ..event.clone()
                        });
                    }
                }
            }
            if let Some(id) = enter {
                d.live.hover_marked.insert(id);
                events.push(InputRouteEvent {
                    code: id,
                    phase: P::HoverEnter,
                    ..event.clone()
                });
            }
            ensure!(
                d.live.events.len() + events.len() <= 8192,
                "input hover event capacity"
            );
            for event in events.into_iter().rev() {
                d.live.events.push_front(event);
            }
            d.live.hover = hover;
            d.live.point = Some(event.point);
            d.live.hover_dirty = false;
            return Ok(());
        }
        if event.phase == P::Press {
            d.live.captures.remove(&event.code);
        }
        let eligible = |r: &&InputReceiverSpec| {
            r.enabled
                && r.sources & event.source.mask() != 0
                && (event.source != InputRouteSource::Pointer || r.hit(event.point, false))
        };
        let receivers = match event.phase {
            P::Press => d.receivers.iter().filter(eligible).map(|r| r.id).collect(),
            P::Release => d
                .receivers
                .iter()
                .filter(|r| {
                    r.enabled
                        && d.live
                            .captures
                            .get(&event.code)
                            .is_none_or(|id| *id == r.id)
                })
                .map(|r| r.id)
                .collect(),
            P::Repeat => d
                .receivers
                .iter()
                .find(|r| {
                    d.live
                        .captures
                        .get(&event.code)
                        .is_none_or(|id| *id == r.id)
                })
                .filter(eligible)
                .map(|r| vec![r.id])
                .unwrap_or_default(),
            P::Drag => {
                let origin = event
                    .origin
                    .context("pointer drag origin missing")?
                    .map(i64::from);
                d.receivers
                    .iter()
                    .filter(|r| {
                        r.enabled
                            && r.drag_enabled
                            && r.sources & 2 != 0
                            && r.rect.is_none_or(|rect| {
                                crysirror_core::region_map::contains_rect(rect, origin)
                            })
                    })
                    .map(|r| r.id)
                    .collect()
            }
            P::HoverEnter | P::HoverLeave => vec![event.code],
            P::Move => unreachable!(),
        };
        d.live.next_event = d
            .live
            .next_event
            .checked_add(1)
            .context("input event serial overflow")?;
        d.live.plan = Some(Plan {
            event,
            receivers,
            cursor: 0,
            serial: d.live.next_event,
        });
        Ok(())
    }
    fn sample(d: &mut Domain, client: u32) -> Result<bool> {
        let batch = d.live.journal.as_mut().unwrap().0.read(client, 1)?;
        let Some(sample) = batch.samples.into_iter().next() else {
            return Ok(false);
        };
        d.live.sample = batch.receipt;
        let point = [sample.frame.pointer_x, sample.frame.pointer_y];
        if d.live.hover_dirty || d.live.point != Some(point) {
            d.live.events.push_back(InputRouteEvent {
                source: InputRouteSource::Pointer,
                code: 0,
                phase: InputRoutePhase::Move,
                point,
                origin: None,
            });
            d.live.point = Some(point);
        }
        let pointer =
            buttons::MOUSE_LEFT | buttons::MOUSE_RIGHT | buttons::WHEEL_UP | buttons::WHEEL_DOWN;
        for (bits, phase) in [
            (sample.frame.buttons_pressed, InputRoutePhase::Press),
            (sample.released, InputRoutePhase::Release),
        ] {
            for code in 0..64 {
                if bits & (1u64 << code) != 0 {
                    let source = if pointer & (1u64 << code) != 0 {
                        InputRouteSource::Pointer
                    } else {
                        InputRouteSource::Keyboard
                    };
                    d.live.events.push_back(InputRouteEvent {
                        source,
                        code,
                        phase,
                        point,
                        origin: None,
                    });
                }
            }
        }
        Ok(true)
    }
    /// Repeated polls return the exact same lease until an explicit ACK/cancel.
    pub fn poll(&mut self, client: ClientId) -> Result<Option<InputDelivery>> {
        let Some(d) = self.clients.get_mut(&client.0) else {
            return Ok(None);
        };
        d.live.init(client.0)?;
        if let Some(delivery) = &d.live.leased {
            return Ok(Some(delivery.clone()));
        }
        for _ in 0..512 {
            if let Some(plan) = &d.live.plan {
                if let Some(receiver) = plan.receivers.get(plan.cursor) {
                    d.live.next_delivery = d
                        .live
                        .next_delivery
                        .checked_add(1)
                        .context("input delivery serial overflow")?;
                    let delivery = InputDelivery {
                        receiver: *receiver,
                        event: plan.event.clone(),
                        receipt: InputDeliveryReceipt {
                            client: client.0,
                            epoch: d.live.epoch,
                            event: plan.serial,
                            delivery: d.live.next_delivery,
                        },
                    };
                    d.live.leased = Some(delivery.clone());
                    return Ok(Some(delivery));
                }
                d.live.finish();
            }
            if let Some(event) = d.live.events.pop_front() {
                Self::start(d, event)?;
                continue;
            }
            if let Some(receipt) = d.live.sample.take() {
                d.live
                    .journal
                    .as_mut()
                    .unwrap()
                    .0
                    .acknowledge(client.0, receipt)?;
            }
            if !Self::sample(d, client.0)? {
                if d.live.hover_dirty {
                    if let Some(point) = d.live.point {
                        Self::start(
                            d,
                            InputRouteEvent {
                                source: InputRouteSource::Pointer,
                                code: 0,
                                phase: InputRoutePhase::Move,
                                point,
                                origin: None,
                            },
                        )?;
                        continue;
                    }
                }
                return Ok(None);
            }
        }
        Ok(None) // bounded owner work; queued input remains visible in stats
    }
    pub fn acknowledge(
        &mut self,
        client: ClientId,
        receipt: InputDeliveryReceipt,
        handled: bool,
    ) -> Result<()> {
        let d = self
            .clients
            .get_mut(&client.0)
            .context("input routing owner missing")?;
        ensure!(
            d.live.leased.as_ref().is_some_and(|l| l.receipt == receipt)
                && receipt.client == client.0,
            "stale/foreign input delivery receipt"
        );
        if handled && d.live.plan.as_ref().unwrap().event.phase == InputRoutePhase::Press {
            let code = d.live.plan.as_ref().unwrap().event.code;
            ensure!(
                d.live.captures.contains_key(&code) || d.live.captures.len() < 4096,
                "input capture capacity"
            );
        }
        let leased = d.live.leased.take().unwrap();
        let plan = d.live.plan.as_mut().unwrap();
        if handled && plan.event.phase == InputRoutePhase::Press {
            d.live.captures.insert(plan.event.code, leased.receiver);
        }
        if plan.event.phase == InputRoutePhase::Drag {
            plan.cursor += 1;
        } else if handled
            || matches!(
                plan.event.phase,
                InputRoutePhase::Repeat | InputRoutePhase::HoverEnter | InputRoutePhase::HoverLeave
            )
        {
            d.live.finish();
        } else {
            plan.cursor += 1;
        }
        Ok(())
    }
    /// Owner restore boundary: registry survives, live deliveries never replay.
    pub fn reset_input_after_restore(&mut self) {
        for d in self.clients.values_mut() {
            d.live = Live::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const C: ClientId = ClientId(1);
    fn receiver(id: u32, priority: i32, rect: Option<[i64; 4]>) -> InputReceiverSpec {
        InputReceiverSpec {
            id,
            priority,
            sources: 7,
            rect,
            coverage: None,
            enabled: true,
            hover_enabled: true,
            drag_enabled: true,
        }
    }
    fn put(r: &mut InputRouting, s: InputReceiverSpec) {
        r.apply_batch(C, &[InputRouteCommand::Put { receiver: s }])
            .unwrap();
    }
    fn event(
        phase: InputRoutePhase,
        source: InputRouteSource,
        code: u32,
        point: [i32; 2],
    ) -> InputRouteEvent {
        InputRouteEvent {
            source,
            code,
            phase,
            point,
            origin: None,
        }
    }
    fn send(r: &mut InputRouting, e: InputRouteEvent) -> InputDelivery {
        r.dispatch(C, e).unwrap();
        r.poll(C).unwrap().unwrap()
    }
    fn ack(r: &mut InputRouting, d: &InputDelivery, handled: bool) {
        r.acknowledge(C, d.receipt, handled).unwrap();
    }
    #[test]
    fn signed_priority_stable_history_foreign_ack_and_capture_release_are_engine_owned() {
        let mut r = InputRouting::default();
        put(&mut r, receiver(9, 0, None));
        put(&mut r, receiver(1, 0, None));
        let press = event(
            InputRoutePhase::Press,
            InputRouteSource::Pointer,
            16,
            [9, 9],
        );
        let a = send(&mut r, press.clone());
        assert_eq!(a.receiver, 9);
        assert_eq!(r.poll(C).unwrap(), Some(a.clone()));
        assert!(r.acknowledge(ClientId(2), a.receipt, true).is_err());
        assert!(r
            .apply_batch(C, &[InputRouteCommand::SetPriority { id: 1, priority: 5 }])
            .is_err());
        ack(&mut r, &a, false);
        let b = r.poll(C).unwrap().unwrap();
        assert_eq!(b.receiver, 1);
        assert_ne!(a.receipt, b.receipt);
        assert!(r.acknowledge(C, a.receipt, true).is_err());
        ack(&mut r, &b, true);
        let up = send(
            &mut r,
            event(
                InputRoutePhase::Release,
                InputRouteSource::Pointer,
                16,
                [-999, -999],
            ),
        );
        assert_eq!(up.receiver, 1);
        ack(&mut r, &up, true);
        assert_eq!(r.stats(C).captures, 0);
        r.apply_batch(
            C,
            &[
                InputRouteCommand::SetPriority {
                    id: 9,
                    priority: i32::MIN,
                },
                InputRouteCommand::SetPriority {
                    id: 1,
                    priority: i32::MAX,
                },
                InputRouteCommand::SetPriority { id: 9, priority: 0 },
                InputRouteCommand::SetPriority { id: 1, priority: 0 },
            ],
        )
        .unwrap();
        let a = send(&mut r, press);
        assert_eq!(a.receiver, 1);
        ack(&mut r, &a, true);
        r.apply_batch(C, &[InputRouteCommand::Remove { id: 1 }])
            .unwrap();
        // A removed captured receiver does not redirect its release to others.
        r.dispatch(
            C,
            event(
                InputRoutePhase::Release,
                InputRouteSource::Pointer,
                16,
                [0, 0],
            ),
        )
        .unwrap();
        assert!(r.poll(C).unwrap().is_none());
        assert_eq!(r.stats(C).captures, 0);
        r.validate().unwrap();
    }
    #[test]
    fn pointer_rect_coverage_global_press_and_nonempty_hover_are_distinct() {
        let mut r = InputRouting::default();
        let mut mask = receiver(1, 10, Some([0, 0, 2, 1]));
        mask.coverage = Some(WidgetHitMask {
            size: [2, 1],
            coverage: vec![0, 255].into(),
            threshold: 128,
            strict_bounds: false,
        });
        put(&mut r, mask);
        put(&mut r, receiver(2, 0, None));
        let d = send(
            &mut r,
            event(
                InputRoutePhase::Press,
                InputRouteSource::Pointer,
                16,
                [0, 0],
            ),
        );
        assert_eq!(d.receiver, 2);
        ack(&mut r, &d, true);
        let d = send(
            &mut r,
            event(
                InputRoutePhase::Press,
                InputRouteSource::Pointer,
                17,
                [2, 0],
            ),
        );
        assert_eq!(d.receiver, 2);
        ack(&mut r, &d, true);
        let d = send(
            &mut r,
            event(
                InputRoutePhase::Press,
                InputRouteSource::Keyboard,
                12,
                [-100, -100],
            ),
        );
        assert_eq!(d.receiver, 1);
        ack(&mut r, &d, true);
        let d = send(
            &mut r,
            event(InputRoutePhase::Move, InputRouteSource::Pointer, 0, [1, 0]),
        );
        assert_eq!(
            (d.receiver, d.event.phase),
            (1, InputRoutePhase::HoverEnter)
        );
        ack(&mut r, &d, true);
        let d = send(
            &mut r,
            event(InputRoutePhase::Move, InputRouteSource::Pointer, 0, [2, 0]),
        );
        assert_eq!(
            (d.receiver, d.event.phase),
            (1, InputRoutePhase::HoverLeave)
        );
        ack(&mut r, &d, true);
        assert!(r.poll(C).unwrap().is_none());
        assert_eq!(r.stats(C).hover, None);
    }
    #[test]
    fn explicit_drag_broadcast_uses_origin_and_does_not_sample_pixel_coverage_or_stop_on_handled() {
        let mut r = InputRouting::default();
        let mut a = receiver(1, 10, Some([0, 0, 2, 2]));
        a.coverage = Some(WidgetHitMask {
            size: [2, 2],
            coverage: vec![0; 4].into(),
            threshold: 128,
            strict_bounds: false,
        });
        put(&mut r, a);
        put(&mut r, receiver(2, 0, Some([0, 0, 2, 2])));
        let mut disabled = receiver(3, 20, None);
        disabled.drag_enabled = false;
        put(&mut r, disabled);
        put(&mut r, receiver(4, 30, Some([10, 10, 2, 2])));
        let mut drag = event(
            InputRoutePhase::Drag,
            InputRouteSource::Pointer,
            16,
            [11, 11],
        );
        assert!(r.dispatch(C, drag.clone()).is_err());
        assert_eq!(r.stats(C).epoch, 0);
        drag.origin = Some([1, 1]);
        let first = send(&mut r, drag);
        assert_eq!(first.receiver, 1);
        ack(&mut r, &first, true);
        let second = r.poll(C).unwrap().unwrap();
        assert_eq!(second.receiver, 2);
        ack(&mut r, &second, false);
        assert!(r.poll(C).unwrap().is_none());
        assert_eq!(r.stats(C).captures, 0);
    }
    #[test]
    fn suspended_hover_keeps_receiver_markers_and_recomputes_without_pointer_motion() {
        let mut r = InputRouting::default();
        let a = receiver(1, 10, Some([0, 0, 2, 2]));
        let b = receiver(2, 0, Some([0, 0, 2, 2]));
        put(&mut r, a.clone());
        put(&mut r, b);
        let enter = send(
            &mut r,
            event(InputRoutePhase::Move, InputRouteSource::Pointer, 0, [1, 1]),
        );
        assert_eq!(
            (enter.receiver, enter.event.phase),
            (1, InputRoutePhase::HoverEnter)
        );
        ack(&mut r, &enter, true);
        let mut suspended = a.clone();
        suspended.hover_enabled = false;
        put(&mut r, suspended);
        let enter = r.poll(C).unwrap().unwrap();
        assert_eq!(
            (enter.receiver, enter.event.phase),
            (2, InputRoutePhase::HoverEnter)
        );
        ack(&mut r, &enter, true);
        assert!(r.poll(C).unwrap().is_none());
        // The disabled receiver retained its marker: switching back exits2 but
        // does not emit a second enter1, despite no intervening pointer move.
        put(&mut r, a);
        let exit = r.poll(C).unwrap().unwrap();
        assert_eq!(
            (exit.receiver, exit.event.phase),
            (2, InputRoutePhase::HoverLeave)
        );
        ack(&mut r, &exit, true);
        assert!(r.poll(C).unwrap().is_none());
        assert_eq!(r.stats(C).hover, Some(1));
        r.apply_batch(
            C,
            &[InputRouteCommand::SetEnabled {
                id: 1,
                enabled: false,
            }],
        )
        .unwrap();
        let enter = r.poll(C).unwrap().unwrap();
        assert_eq!(enter.receiver, 2);
        ack(&mut r, &enter, true);
        r.apply_batch(
            C,
            &[InputRouteCommand::SetEnabled {
                id: 1,
                enabled: true,
            }],
        )
        .unwrap();
        let exit = r.poll(C).unwrap().unwrap();
        assert_eq!(
            (exit.receiver, exit.event.phase),
            (2, InputRoutePhase::HoverLeave)
        );
        ack(&mut r, &exit, true);
        let enter = r.poll(C).unwrap().unwrap();
        assert_eq!(
            (enter.receiver, enter.event.phase),
            (1, InputRoutePhase::HoverEnter)
        );
        ack(&mut r, &enter, true);
    }
    #[test]
    fn core_journal_preserves_press_release_and_cancel_restore_never_replays_held_input() {
        let mut r = InputRouting::default();
        put(&mut r, receiver(1, 0, None));
        r.admit_frame(
            1,
            InputFrame {
                buttons_down: buttons::CTRL,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(r.poll(C).unwrap().is_none());
        r.admit_frame(1, InputFrame::default()).unwrap();
        let up = r.poll(C).unwrap().unwrap();
        assert_eq!(up.event.phase, InputRoutePhase::Release);
        ack(&mut r, &up, true);
        r.admit_frame(
            1,
            InputFrame {
                buttons_down: buttons::CTRL,
                buttons_pressed: buttons::CTRL,
                ..Default::default()
            },
        )
        .unwrap();
        let down = r.poll(C).unwrap().unwrap();
        assert_eq!(
            (down.event.code, down.event.phase),
            (12, InputRoutePhase::Press)
        );
        let encoded = serde_json::to_value(&r).unwrap();
        let mut restored: InputRouting = serde_json::from_value(encoded).unwrap();
        assert!(restored.acknowledge(C, down.receipt, true).is_err());
        restored
            .admit_frame(
                1,
                InputFrame {
                    buttons_down: buttons::CTRL,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(restored.poll(C).unwrap().is_none());
        r.apply_batch(C, &[InputRouteCommand::Cancel]).unwrap();
        assert!(r.acknowledge(C, down.receipt, true).is_err());
        r.admit_frame(
            1,
            InputFrame {
                buttons_down: buttons::CTRL,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(r.poll(C).unwrap().is_none());
        r.admit_frame(1, InputFrame::default()).unwrap();
        let up = r.poll(C).unwrap().unwrap();
        assert_eq!(up.event.phase, InputRoutePhase::Release);
        ack(&mut r, &up, true);
        let controller = send(
            &mut restored,
            event(
                InputRoutePhase::Press,
                InputRouteSource::Controller,
                55,
                [0, 0],
            ),
        );
        assert_eq!(controller.event.source, InputRouteSource::Controller);
        assert_ne!(controller.receipt.epoch, down.receipt.epoch);
        ack(&mut restored, &controller, true);
        let json = serde_json::to_value(controller.receipt).unwrap();
        assert!(
            json["epoch"].is_string() && json["event"].is_string() && json["delivery"].is_string()
        );
    }
    #[test]
    fn menu_collision_leaves_ui_journal_and_clock_uncommitted() {
        use crate::ui::UiCommand;
        use crate::{EngineSystems, ServiceBatch};
        let mut s = EngineSystems::default();
        let menu=serde_json::from_value(serde_json::json!({"id":1,"rect":[0,0,100,24],"font":"default","items":[{"id":1,"label":"Menu"}]})).unwrap();
        s.submit(
            C,
            &ServiceBatch {
                ui: vec![UiCommand::SetMenu { menu }],
                input_routes: vec![InputRouteCommand::Put {
                    receiver: receiver(1, 0, None),
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let before = serde_json::to_value(&s).unwrap();
        assert!(s
            .tick(
                16,
                InputFrame {
                    buttons_down: buttons::MOUSE_LEFT,
                    buttons_pressed: buttons::MOUSE_LEFT,
                    pointer_x: 5,
                    pointer_y: 5
                },
                [100, 100]
            )
            .is_err());
        assert_eq!(serde_json::to_value(&s).unwrap(), before);
        assert_eq!(s.input_routes.stats(C).queued_samples, 0);
    }
    #[test]
    fn routing_scene_ownership_and_invalid_batches_roll_back_before_input_admission() {
        use crate::ui::UiCommand;
        use crate::{EngineSystems, ServiceBatch};
        let mut s = EngineSystems::default();
        s.submit(
            C,
            &ServiceBatch {
                input_routes: vec![InputRouteCommand::Put {
                    receiver: receiver(1, 0, None),
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let before = serde_json::to_value(&s).unwrap();
        let widget =
            serde_json::from_value(serde_json::json!({"id":4,"kind":"button","rect":[0,0,10,10]}))
                .unwrap();
        assert!(s
            .submit(
                C,
                &ServiceBatch {
                    ui: vec![UiCommand::Put { widget }],
                    ..Default::default()
                }
            )
            .is_err());
        assert_eq!(serde_json::to_value(&s).unwrap(), before);
        assert!(s
            .submit(
                ClientId(2),
                &ServiceBatch {
                    input_routes: vec![InputRouteCommand::Put {
                        receiver: receiver(2, 0, None)
                    }],
                    ..Default::default()
                }
            )
            .is_err());
        assert_eq!(serde_json::to_value(&s).unwrap(), before);
        assert!(s
            .tick(
                16,
                InputFrame {
                    buttons_pressed: buttons::CTRL,
                    buttons_down: buttons::CTRL,
                    ..Default::default()
                },
                [100, 100]
            )
            .unwrap());
        let d = s.input_routes.poll(C).unwrap().unwrap();
        assert_eq!(d.event.phase, InputRoutePhase::Press);
        let before = serde_json::to_value(&s).unwrap();
        assert!(s
            .submit(
                C,
                &ServiceBatch {
                    input_routes: vec![InputRouteCommand::SetPriority {
                        id: 1,
                        priority: 99
                    }],
                    ..Default::default()
                }
            )
            .is_err());
        assert_eq!(serde_json::to_value(&s).unwrap(), before);
    }
}
