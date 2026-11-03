//! Native descriptor classes, packed scopes and consumption policy over the
//! engine's reusable descriptor ledger. No window/input backend is hosted.
use crysirror_scene::input_ledger::{DescriptorLedger, PriorityScopes};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
/// Exact class descriptor tables at target addresses 0x50670C..0x5069B8.
/// Sys80:1B replaces only the mutable classes accepted by sub_46DFA0.
const TARGET_DEFAULT_INPUT_CLASSES: &[(i32, &[i32])] = &[
    (0x0000_0001, &[1]),
    (0x0000_0002, &[2]),
    (0x0000_0004, &[4]),
    (0x0000_0010, &[5]),
    (0x0000_0020, &[6]),
    (0x0000_0040, &[14]),
    (0x0000_0080, &[15]),
    (0x0000_0100, &[13]),
    (0x0000_0200, &[32]),
    (0x0000_1000, &[38]),
    (0x0000_2000, &[40]),
    (0x0000_4000, &[37]),
    (0x0000_8000, &[39]),
    (0x0001_0000, &[49, 97]),
    (0x0002_0000, &[50, 98]),
    (0x0004_0000, &[51, 99]),
    (0x0008_0000, &[52, 100]),
    (0x0010_0000, &[53, 101]),
    (0x0020_0000, &[54, 102]),
    (0x0040_0000, &[55, 103]),
    (0x0080_0000, &[56, 104]),
    (0x0100_0000, &[57, 105]),
    (0x0200_0000, &[48, 96]),
    (0x4000_0000, &[9]),
    (i32::MIN, &[17]),
];

/// sub_46DC80 reports the nineteen keyboard/configuration classes from
/// 0x40 through 0x40000000. Primitive pointer bits 1/2/4/0x10/0x20 are added
/// separately by sub_46DF00, and the sign-bit class feeds sub_46DE30 only.
pub const TARGET_EVENT_CLASS_MASKS: &[i32] = &[
    0x0000_0040,
    0x0000_0080,
    0x0000_0100,
    0x0000_0200,
    0x0000_1000,
    0x0000_2000,
    0x0000_4000,
    0x0000_8000,
    0x0001_0000,
    0x0002_0000,
    0x0004_0000,
    0x0008_0000,
    0x0010_0000,
    0x0020_0000,
    0x0040_0000,
    0x0080_0000,
    0x0100_0000,
    0x0200_0000,
    0x4000_0000,
];

pub fn target_default_input_classes() -> BTreeMap<i32, Vec<i32>> {
    TARGET_DEFAULT_INPUT_CLASSES
        .iter()
        .map(|&(mask, descriptors)| (mask, descriptors.to_vec()))
        .collect()
}

#[derive(Debug)]
pub struct NativeInputPolicy {
    physical: BTreeSet<i32>,
    pub now_ms: u64,
    hold_deadlines: BTreeMap<i32,u64>,
    pub ledger: DescriptorLedger,
    pub keyboard_scopes: PriorityScopes,
    pub pointer_scopes: PriorityScopes,
    pub pointer_nodes: Vec<(u32, bool)>,
    pub classes: BTreeMap<i32, Vec<i32>>,
    pub configuration_enabled: i32,
    pub master_gate: i32,
    pub latched_state: i32,
    pub poll_active: bool,
    pub focused: bool,
    pub restored: bool,
    pub background_polling: i32,
    pub mouse_mapping_mode: i32,
    pub pointer: [i32; 2],
    pub press_points: [[i32; 2]; 5],
    pub serial: i32,
    pub window_serial: i32,
    pub repeat: [i32; 256],
    pending_state: Option<i32>,
    pending_descriptor: Option<i32>,
    pending_consumed: bool,
    pub system_events: VecDeque<[i32; 3]>,
    window_messages: VecDeque<(i32, i32, i32, i32)>,
}
impl Default for NativeInputPolicy {
    fn default() -> Self {
        Self {
            physical: BTreeSet::new(),
            now_ms: 0,
            hold_deadlines: BTreeMap::new(),
            ledger: Default::default(),
            keyboard_scopes: Default::default(),
            pointer_scopes: Default::default(),
            pointer_nodes: Vec::new(),
            classes: target_default_input_classes(),
            configuration_enabled: 1,
            master_gate: 1,
            latched_state: 0,
            poll_active: false,
            focused: true,
            restored: true,
            background_polling: 0,
            mouse_mapping_mode: 0,
            pointer: [0; 2],
            press_points: [[-1; 2]; 5],
            serial: 0,
            window_serial: 0,
            repeat: [0; 256],
            pending_state: None,
            pending_descriptor: None,
            pending_consumed: false,
            system_events: VecDeque::new(),
            window_messages: VecDeque::new(),
        }
    }
}
impl NativeInputPolicy {
    pub fn update(&mut self, held: &[i32], pointer: [i32; 2]) {
        self.pending_state = None;
        self.pending_descriptor = None;
        self.pending_consumed = false;
        if self.pointer != pointer {
            let point =
                ((pointer[0] as u32 & 0xffff) | ((pointer[1] as u32 & 0xffff) << 16)) as i32;
            self.window_message(0x200, 0, point);
        }
        self.pointer = pointer;
        let old = self.physical.iter().copied().collect::<Vec<_>>();
        for key in old {
            if !held.contains(&key) {
                self.release(key);
            }
        }
        for &key in held {
            if !self.physical.contains(&key) {
                self.press(key);
            }
        }
    }
    fn mapped(&self, key: i32) -> i32 {
        if self.mouse_mapping_mode == 1 && key == 2 {
            1
        } else {
            key
        }
    }
    fn window_message(&mut self, message: i32, w: i32, l: i32) {
        self.window_serial = self.window_serial.wrapping_add(1);
        if self.window_messages.len() == 256 {
            self.window_messages.pop_front();
        }
        self.window_messages
            .push_back((self.window_serial, message, w, l));
    }
    pub fn press(&mut self, physical: i32) {
        self.physical.insert(physical);
        let key = self.mapped(physical);
        if self.repeat.get(key as usize).copied().unwrap_or(0) != 0 {
            self.ledger.release(key);
        }
        let fresh = self.ledger.press(key, matches!(key, 1 | 2 | 4 | 5 | 6));
        if fresh {
            self.hold_deadlines.insert(key, self.now_ms.saturating_add(500));
            self.serial = self.serial.wrapping_add(1);
            if self.system_events.len() == 256 {
                self.system_events.pop_front();
            }
            self.system_events.push_back([3, key, 0]);
            self.pending_state = Some(0x10000002);
            self.pending_descriptor = Some(key);
            self.pending_consumed = false;
        }
        if let Some(slot) = match key {
            1 => Some(0),
            2 => Some(1),
            4 => Some(2),
            5 => Some(3),
            6 => Some(4),
            _ => None,
        } {
            self.press_points[slot] = self.pointer;
        }
        let point =
            ((self.pointer[0] as u32 & 0xffff) | ((self.pointer[1] as u32 & 0xffff) << 16)) as i32;
        self.window_message(
            match key {
                1 => 0x201,
                2 => 0x204,
                4 => 0x207,
                5 | 6 => 0x20b,
                _ => 0x100,
            },
            key,
            if key < 7 { point } else { 0 },
        );
    }
    pub fn release(&mut self, physical: i32) {
        self.physical.remove(&physical);
        let key = self.mapped(physical);
        self.ledger.release(key);
        self.pending_state = Some(0x10000006);
        self.pending_descriptor = Some(key);
        self.pending_consumed = false;
        let point =
            ((self.pointer[0] as u32 & 0xffff) | ((self.pointer[1] as u32 & 0xffff) << 16)) as i32;
        self.window_message(
            match key {
                1 => 0x202,
                2 => 0x205,
                4 => 0x208,
                5 | 6 => 0x20c,
                _ => 0x101,
            },
            if key < 7 { 0 } else { key },
            if key < 7 { point } else { 0 },
        );
    }
    pub fn pulse(&mut self, key: i32) {
        self.press(key);
        self.release(key);
    }
    pub fn repeat_ready(&self, descriptor: i32) -> bool {
        self.query_held(descriptor) && self.hold_deadlines.get(&descriptor).is_some_and(|deadline| self.now_ms >= *deadline)
    }
    pub fn query_held(&self, key: i32) -> bool {
        if !self.focused && self.background_polling == 0 {
            return false;
        }
        match (self.mouse_mapping_mode, key) {
            (1, 1) => self.physical.contains(&1) || self.physical.contains(&2),
            (1, 2) => false,
            _ => self.physical.contains(&key),
        }
    }
    pub fn keyboard_state(&self) -> [u8; 256] {
        std::array::from_fn(|key| {
            u8::from(self.ledger.toggle(key as i32))
                | if self.query_held(key as i32) { 0x80 } else { 0 }
        })
    }
    pub fn read_state(&mut self, key: i32) -> i32 {
        if self.pending_consumed {
            return 0;
        }
        let state = if self.pending_descriptor == Some(key) || key == 0 {
            self.pending_state.unwrap_or(0)
        } else {
            0
        };
        if state != 0 {
            self.pending_consumed = true;
        }
        state
    }
    pub fn peek_state(&self, key: i32) -> i32 {
        self.ledger.activation_count(key).min(i32::MAX as u32) as i32
    }
    pub fn clear(&mut self) {
        self.ledger.clear_transient();
    }
    pub fn drain(&mut self, key: i32) -> i32 {
        let children = match key {
            193 => self.classes.get(&0x100).cloned().unwrap_or_default(),
            194 => self.classes.get(&0x200).cloned().unwrap_or_default(),
            195 => vec![38, 14],
            196 => vec![40, 15],
            197..=215 => Vec::new(),
            _ => return self.ledger.drain(key) as i32,
        };
        let (mut count, mut held) = (0i32, 0i32);
        for child in children {
            let value = self.drain(child);
            count = count.wrapping_add(value & i32::MAX);
            held |= value & i32::MIN;
        }
        count | held
    }
    pub fn keyboard_eligible(&self, scope: i32) -> bool {
        self.configuration_enabled != 0
            && self
                .keyboard_scopes
                .top()
                .is_none_or(|top| scope as u32 >= top)
    }
    pub fn pointer_eligible(&self, scope: i32) -> bool {
        self.configuration_enabled != 0
            && self
                .pointer_scopes
                .accepts_pointer(scope as u32, &self.pointer_nodes)
    }
    pub fn register_scope(&mut self, scope: i32, already_packed: bool) {
        let packed = if already_packed {
            scope
        } else {
            scope.wrapping_shl(16) | 0xffff
        };
        self.keyboard_scopes.register(packed as u32);
        self.pointer_scopes.register(packed as u32);
        let _ = self.query_bits_inner(packed, true, already_packed);
    }
    pub fn unregister_scope(&mut self, scope: i32, already_packed: bool) {
        let packed = if already_packed {
            scope
        } else {
            scope.wrapping_shl(16) | 0xffff
        };
        if !already_packed {
            let _ = self.query_bits_inner(packed, true, false);
        }
        self.keyboard_scopes.unregister(packed as u32);
        self.pointer_scopes.unregister(packed as u32);
    }
    pub fn query_bits(&mut self, scope: i32, already_packed: bool) -> i32 {
        self.query_bits_inner(scope, already_packed, true)
    }
    fn query_bits_inner(&mut self, scope: i32, already_packed: bool, consume: bool) -> i32 {
        let packed = if already_packed {
            scope
        } else {
            scope.wrapping_shl(16) | 0xffff
        };
        self.query_bits_scoped(packed, packed, consume)
    }
    pub fn query_bits_split(&mut self, keyboard_scope: i32, pointer_scope: i32) -> i32 {
        self.query_bits_scoped(keyboard_scope, pointer_scope, true)
    }
    pub fn register_selection_scopes(&mut self, scope: i32, keyboard: bool, pointer: bool) {
        if keyboard {
            self.keyboard_scopes.register(scope as u32);
        }
        if pointer {
            self.pointer_scopes.register(scope as u32);
        }
    }
    pub fn unregister_selection_scopes(&mut self, scope: i32, keyboard: bool, pointer: bool) {
        if keyboard {
            self.keyboard_scopes.unregister(scope as u32);
        }
        if pointer {
            self.pointer_scopes.unregister(scope as u32);
        }
    }
    fn query_bits_scoped(&mut self, keyboard_scope: i32, pointer_scope: i32, consume: bool) -> i32 {
        let mut result = 0;
        if self.keyboard_eligible(keyboard_scope) {
            for &mask in TARGET_EVENT_CLASS_MASKS {
                let descriptors = self.classes.get(&mask).cloned().unwrap_or_default();
                if descriptors.into_iter().any(|d| self.drain(d) != 0) {
                    result |= mask;
                }
            }
            if self.configured_gate() != 0 {
                result |= i32::MIN;
            }
        }
        if self.pointer_eligible(pointer_scope) {
            for (key, bit) in [(1, 1), (2, 2), (4, 4), (5, 0x10), (6, 0x20)] {
                if self.drain(key) != 0 {
                    result |= bit;
                }
            }
        }
        if consume && result != 0 {
            self.pending_consumed = true;
        }
        result
    }
    pub fn configured_gate(&mut self) -> i32 {
        let mut latched = self.latched_state != 0;
        let mut any_down = false;
        if self.focused && self.configuration_enabled != 0 {
            for key in self.classes.get(&i32::MIN).cloned().unwrap_or_default() {
                let down = self.query_held(key);
                any_down |= down;
                if self.poll_active {
                    if self.drain(key) != 0 && self.restored {
                        latched = true;
                    }
                } else if down && self.restored {
                    latched = true;
                    break;
                }
            }
        }
        if self.poll_active && !any_down {
            self.poll_active = false;
        }
        i32::from(latched && self.master_gate != 0)
    }
    pub fn sample_configured(&mut self) {
        self.poll_active = true;
        for key in self.classes.get(&i32::MIN).cloned().unwrap_or_default() {
            let _ = self.drain(key);
        }
    }
    pub fn register_class(&mut self, mask: i32, descriptors: &[i32]) {
        if matches!(
            mask,
            0x40 | 0x80 | 0x100 | 0x200 | 0x1000 | 0x2000 | 0x4000 | 0x8000 | 0x40000000 | i32::MIN
        ) && descriptors.len() < 16
        {
            self.classes.insert(mask, descriptors.to_vec());
        }
    }
    pub fn class_state(&self, mask: i32) -> i32 {
        let mut result = 0i32;
        for (&class, list) in &self.classes {
            if class != i32::MIN && mask & class != 0 {
                for key in list {
                    result = result.wrapping_add(self.ledger.activation_count(*key) as i32);
                }
            }
        }
        result
    }
    pub fn scoped_event(&mut self, key: i32, scope: i32) -> i32 {
        let packed = scope.wrapping_shl(16) | 0xffff;
        if if matches!(key, 1 | 2) {
            self.pointer_eligible(packed)
        } else {
            self.keyboard_eligible(packed)
        } {
            self.drain(key)
        } else {
            0
        }
    }
    pub fn poll_window(&self, message: i32, after: i32) -> Option<(i32, i32)> {
        self.window_messages
            .iter()
            .rev()
            .find(|(serial, id, _, _)| *id == message && serial.wrapping_sub(after) > 0)
            .map(|(_, _, w, l)| (*w, *l))
    }
}
