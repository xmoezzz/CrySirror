// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Shared output-frame linear control interpolation.
#[derive(Clone, Copy)]
pub(crate) struct Ramp {
    pub(crate) value: f32,
    pub(crate) target: f32,
    pub(crate) left: u64,
}
impl Ramp {
    pub(crate) fn new(value: f32) -> Self {
        Self {
            value,
            target: value,
            left: 0,
        }
    }
    pub(crate) fn to(&mut self, target: f32, frames: u64) {
        self.target = target;
        self.left = frames;
        if frames == 0 {
            self.value = target
        }
    }
    pub(crate) fn step(&mut self) -> f32 {
        let out = self.value;
        if self.left > 0 {
            self.value += (self.target - self.value) / self.left as f32;
            self.left -= 1;
        }
        out
    }
}
