// SPDX-License-Identifier: MPL-2.0
// Original siglus_rs RealLive VM source, commit 5f358ef1360b0439e92aed2b749e8f25d8721954.
//! Graphics objects: retained sprites composited over DC 0.
//!
//! Geometry follows the RealLive model: the object's origin (its own
//! `objOrigin`, or the origin stored with the g00 pattern) is placed at
//! `(x, y)` plus the eight adjustment offsets; scaling and rotation happen
//! around the origin plus the "rep origin".

use std::rc::Rc;

use crate::vm::gan::Gan;
use crate::vm::graphics::Image;
use crate::vm::rect::Rect;

pub const OBJECT_COUNT: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct TextParams {
    pub text: String,
    pub size: i32,
    pub xspace: i32,
    pub yspace: i32,
    /// Characters per line (0 = unlimited); `objTextOpts`' vertical flag
    /// makes this 1.
    pub char_count: i32,
    pub colour: i32,
    /// Shadow colour index, or -1 for none.
    pub shadow: i32,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            text: String::new(),
            size: 14,
            xspace: 0,
            yspace: 0,
            char_count: 0,
            colour: 0,
            shadow: -1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DigitParams {
    pub value: i32,
    pub digits: i32,
    pub zero: i32,
    pub sign: i32,
    pub pack: i32,
    pub space: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftParams {
    pub count: i32,
    pub use_animation: i32,
    pub start_pattern: i32,
    pub end_pattern: i32,
    pub animation_time: i32,
    pub y_time: i32,
    pub period: i32,
    pub amplitude: i32,
    pub use_drift: i32,
    pub unknown: i32,
    pub drift_speed: i32,
    pub area: Rect,
}

impl Default for DriftParams {
    fn default() -> Self {
        Self {
            count: 1,
            use_animation: 0,
            start_pattern: 0,
            end_pattern: 0,
            animation_time: 0,
            y_time: 1000,
            period: 0,
            amplitude: 0,
            use_drift: 0,
            unknown: 0,
            drift_speed: 0,
            area: Rect::new(0, 0, 640, 480),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ButtonParams {
    pub is_button: i32,
    pub action: i32,
    pub se: i32,
    pub group: i32,
    pub number: i32,
    /// 0 normal, 1 hover, 2 pressed (pattern offsets).
    pub state: i32,
}

/// Everything scripts can set on an object.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectParams {
    pub visible: bool,
    pub x: i32,
    pub y: i32,
    pub adjust_x: [i32; 8],
    pub adjust_y: [i32; 8],
    pub adjust_vert: i32,
    pub origin: (i32, i32),
    pub rep_origin: (i32, i32),
    /// Percent.
    pub width: i32,
    pub height: i32,
    /// Thousandths.
    pub hq_width: i32,
    pub hq_height: i32,
    /// Tenths of a degree, clockwise.
    pub rotation: i32,
    pub pattern: i32,
    pub alpha: i32,
    pub adjust_alpha: [i32; 8],
    pub clip: Option<Rect>,
    pub own_clip: Option<Rect>,
    pub mono: i32,
    pub invert: i32,
    pub light: i32,
    pub tint: [i32; 3],
    /// r, g, b, level.
    pub colour: [i32; 4],
    /// 0 normal, 1 add, 2 subtract.
    pub composite: i32,
    pub scroll_rate: (i32, i32),
    pub z_order: i32,
    pub z_layer: i32,
    pub z_depth: i32,
    pub quarter_view: i32,
    pub wipe_copy: bool,
    pub text: TextParams,
    pub digits: DigitParams,
    pub drift: DriftParams,
    pub button: ButtonParams,
    /// `objFadeOpts` values (min alpha, max alpha, ...).
    pub fade: [i32; 5],
}

impl Default for ObjectParams {
    fn default() -> Self {
        Self {
            visible: false,
            x: 0,
            y: 0,
            adjust_x: [0; 8],
            adjust_y: [0; 8],
            adjust_vert: 0,
            origin: (0, 0),
            rep_origin: (0, 0),
            width: 100,
            height: 100,
            hq_width: 1000,
            hq_height: 1000,
            rotation: 0,
            pattern: 0,
            alpha: 255,
            adjust_alpha: [255; 8],
            clip: None,
            own_clip: None,
            mono: 0,
            invert: 0,
            light: 0,
            tint: [0; 3],
            colour: [0; 4],
            composite: 0,
            scroll_rate: (0, 0),
            z_order: 0,
            z_layer: 0,
            z_depth: 0,
            quarter_view: 0,
            wipe_copy: false,
            text: TextParams::default(),
            digits: DigitParams::default(),
            drift: DriftParams::default(),
            button: ButtonParams::default(),
            fade: [0, 255, 0, 0, 0],
        }
    }
}

impl ObjectParams {
    pub fn adjust_sum(&self) -> (i32, i32) {
        (
            self.adjust_x.iter().sum(),
            self.adjust_y.iter().sum::<i32>() + self.adjust_vert,
        )
    }

    pub fn scale(&self) -> (f64, f64) {
        (
            f64::from(self.width) / 100.0 * f64::from(self.hq_width) / 1000.0,
            f64::from(self.height) / 100.0 * f64::from(self.hq_height) / 1000.0,
        )
    }

    pub fn computed_alpha(&self) -> i32 {
        self.adjust_alpha
            .iter()
            .fold(self.alpha.clamp(0, 255), |alpha, adjust| {
                alpha * (*adjust).clamp(0, 255) / 255
            })
    }

    /// Whether any colour effect applies.
    pub fn has_effects(&self) -> bool {
        self.mono != 0
            || self.invert != 0
            || self.light != 0
            || self.tint != [0; 3]
            || self.colour[3] != 0
    }
}

/// How a pattern or GAN animation ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterAnimation {
    /// Stay on the last frame (`Play`).
    Stop,
    /// Delete the object (`PlayOnce`).
    Clear,
    /// Start again (`Loop`).
    Loop,
    /// Play forwards and back, pause about three seconds, repeat (`Blink`).
    Blink,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Animation {
    pub start: u64,
    pub after: AfterAnimation,
    /// Pattern animations: ms per pattern. GAN animations: the set index.
    pub parameter: i32,
    pub finished: bool,
}

/// Pause between blinks.
const BLINK_PAUSE: u64 = 3000;

#[derive(Debug, Clone)]
pub enum ObjectData {
    File {
        name: String,
        image: Rc<Image>,
    },
    Gan {
        image_name: String,
        image: Rc<Image>,
        gan_name: String,
        gan: Rc<Gan>,
    },
    Text,
    Digits {
        name: String,
        image: Rc<Image>,
    },
    Drift {
        name: String,
        image: Rc<Image>,
    },
    /// A filter area (`objOfArea`, `objOfRect`).
    Rect(Rect),
    /// A parent holding child objects.
    Parent(Vec<Option<Object>>),
}

/// A property that `objEve*` functions can animate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Property {
    X,
    Y,
    Alpha,
    AdjustX(usize),
    AdjustY(usize),
    Mono,
    Invert,
    Light,
    TintR,
    TintG,
    TintB,
    ColR,
    ColG,
    ColB,
    ColLevel,
    AdjustVert,
    AdjustAlpha(usize),
    Width,
    Height,
    Rotation,
    RepOriginX,
    RepOriginY,
    OriginX,
    OriginY,
    HqWidth,
    HqHeight,
    Visible,
    /// The clip rectangle (`objRectEria`, `objBoxEria`): left, top,
    /// width, height, right, bottom.
    ClipX,
    ClipY,
    ClipW,
    ClipH,
    ClipRight,
    ClipBottom,
    /// The object's own clip (`objOwnEria` and friends), likewise.
    OwnClipX,
    OwnClipY,
    OwnClipW,
    OwnClipH,
    OwnClipRight,
    OwnClipBottom,
}

/// An unset clip, as a rectangle: large enough not to clip.
const NO_CLIP: Rect = Rect {
    x: 0,
    y: 0,
    w: 0x4000,
    h: 0x4000,
};

impl Property {
    pub fn get(self, p: &ObjectParams) -> i32 {
        match self {
            Property::X => p.x,
            Property::Y => p.y,
            Property::Alpha => p.alpha,
            Property::AdjustX(i) => p.adjust_x[i & 7],
            Property::AdjustY(i) => p.adjust_y[i & 7],
            Property::Mono => p.mono,
            Property::Invert => p.invert,
            Property::Light => p.light,
            Property::TintR => p.tint[0],
            Property::TintG => p.tint[1],
            Property::TintB => p.tint[2],
            Property::ColR => p.colour[0],
            Property::ColG => p.colour[1],
            Property::ColB => p.colour[2],
            Property::ColLevel => p.colour[3],
            Property::AdjustVert => p.adjust_vert,
            Property::AdjustAlpha(i) => p.adjust_alpha[i & 7],
            Property::Width => p.width,
            Property::Height => p.height,
            Property::Rotation => p.rotation,
            Property::RepOriginX => p.rep_origin.0,
            Property::RepOriginY => p.rep_origin.1,
            Property::OriginX => p.origin.0,
            Property::OriginY => p.origin.1,
            Property::HqWidth => p.hq_width,
            Property::HqHeight => p.hq_height,
            Property::Visible => i32::from(p.visible),
            Property::ClipX => p.clip.unwrap_or(NO_CLIP).x,
            Property::ClipY => p.clip.unwrap_or(NO_CLIP).y,
            Property::ClipW => p.clip.unwrap_or(NO_CLIP).w,
            Property::ClipH => p.clip.unwrap_or(NO_CLIP).h,
            Property::ClipRight => p.clip.unwrap_or(NO_CLIP).right(),
            Property::ClipBottom => p.clip.unwrap_or(NO_CLIP).bottom(),
            Property::OwnClipX => p.own_clip.unwrap_or(NO_CLIP).x,
            Property::OwnClipY => p.own_clip.unwrap_or(NO_CLIP).y,
            Property::OwnClipW => p.own_clip.unwrap_or(NO_CLIP).w,
            Property::OwnClipH => p.own_clip.unwrap_or(NO_CLIP).h,
            Property::OwnClipRight => p.own_clip.unwrap_or(NO_CLIP).right(),
            Property::OwnClipBottom => p.own_clip.unwrap_or(NO_CLIP).bottom(),
        }
    }

    pub fn set(self, p: &mut ObjectParams, value: i32) {
        match self {
            Property::X => p.x = value,
            Property::Y => p.y = value,
            Property::Alpha => p.alpha = value,
            Property::AdjustX(i) => p.adjust_x[i & 7] = value,
            Property::AdjustY(i) => p.adjust_y[i & 7] = value,
            Property::Mono => p.mono = value,
            Property::Invert => p.invert = value,
            Property::Light => p.light = value,
            Property::TintR => p.tint[0] = value,
            Property::TintG => p.tint[1] = value,
            Property::TintB => p.tint[2] = value,
            Property::ColR => p.colour[0] = value,
            Property::ColG => p.colour[1] = value,
            Property::ColB => p.colour[2] = value,
            Property::ColLevel => p.colour[3] = value,
            Property::AdjustVert => p.adjust_vert = value,
            Property::AdjustAlpha(i) => p.adjust_alpha[i & 7] = value,
            Property::Width => p.width = value,
            Property::Height => p.height = value,
            Property::Rotation => p.rotation = value,
            Property::RepOriginX => p.rep_origin.0 = value,
            Property::RepOriginY => p.rep_origin.1 = value,
            Property::OriginX => p.origin.0 = value,
            Property::OriginY => p.origin.1 = value,
            Property::HqWidth => p.hq_width = value,
            Property::HqHeight => p.hq_height = value,
            Property::Visible => p.visible = value != 0,
            Property::ClipX
            | Property::ClipY
            | Property::ClipW
            | Property::ClipH
            | Property::ClipRight
            | Property::ClipBottom => {
                let r = p.clip.get_or_insert(NO_CLIP);
                match self {
                    Property::ClipX => r.x = value,
                    Property::ClipY => r.y = value,
                    Property::ClipW => r.w = value,
                    Property::ClipH => r.h = value,
                    Property::ClipRight => r.w = value - r.x,
                    _ => r.h = value - r.y,
                }
            }
            Property::OwnClipX
            | Property::OwnClipY
            | Property::OwnClipW
            | Property::OwnClipH
            | Property::OwnClipRight
            | Property::OwnClipBottom => {
                let r = p.own_clip.get_or_insert(NO_CLIP);
                match self {
                    Property::OwnClipX => r.x = value,
                    Property::OwnClipY => r.y = value,
                    Property::OwnClipW => r.w = value,
                    Property::OwnClipH => r.h = value,
                    Property::OwnClipRight => r.w = value - r.x,
                    _ => r.h = value - r.y,
                }
            }
        }
    }
}

/// An `objEve*` animation of one or two properties.
#[derive(Debug, Clone, PartialEq)]
pub struct Mutator {
    /// The unified property id (base id of the `obj*` function family).
    pub id: i32,
    pub repno: i32,
    pub targets: Vec<(Property, i32, i32)>,
    pub start: u64,
    pub duration: i32,
    pub delay: i32,
    /// 0 linear, 1 accelerating, 2 decelerating; 3 is a damped wave:
    /// targets are `(property, amplitude, cycles)` and settle at 0.
    /// `CURVE_WAVE_AROUND + n` is a damped wave of `n` cycles around a
    /// base: targets are `(property, base, amplitude)`.
    pub curve: i32,
}

pub const CURVE_WAVE: i32 = 3;
/// A mutator id that matches every running `objEve*` animation
/// (`*EVE_ALLEVE_*`).
pub const ALL_MUTATORS: i32 = -1;
pub const CURVE_WAVE_AROUND: i32 = 1000;
/// `CURVE_FLASH + n`: `n` damped pulses from the base towards base +
/// amplitude and back.
pub const CURVE_FLASH: i32 = 2000;
/// `CURVE_BLINK + n`: `n` times base + amplitude, then base (on/off).
pub const CURVE_BLINK: i32 = 3000;
/// `CURVE_REPEAT + n`: `n` undamped trips to base + amplitude and back.
pub const CURVE_REPEAT: i32 = 4000;

impl Mutator {
    fn value_at(&self, from: i32, to: i32, now: u64) -> (i32, bool) {
        let begin = self.start + self.delay.max(0) as u64;
        if now < begin {
            return (from, false);
        }
        if self.duration <= 0 || now >= begin + self.duration as u64 {
            return (to, true);
        }
        let t = (now - begin) as f64 / f64::from(self.duration);
        if self.curve >= CURVE_REPEAT {
            let cycles = f64::from(self.curve - CURVE_REPEAT);
            let phase = (t * cycles).fract();
            let tri = 1.0 - (phase * 2.0 - 1.0).abs();
            return (
                (f64::from(from) + f64::from(to) * tri).round() as i32,
                false,
            );
        }
        if self.curve >= CURVE_BLINK {
            let cycles = f64::from(self.curve - CURVE_BLINK);
            let on = (t * cycles).fract() < 0.5;
            return (if on { from + to } else { from }, false);
        }
        if self.curve >= CURVE_FLASH {
            let cycles = f64::from(self.curve - CURVE_FLASH);
            let pulse = (t * cycles * std::f64::consts::PI).sin().abs() * (1.0 - t);
            return (
                (f64::from(from) + f64::from(to) * pulse).round() as i32,
                false,
            );
        }
        if self.curve >= CURVE_WAVE_AROUND {
            let cycles = f64::from(self.curve - CURVE_WAVE_AROUND);
            let wave = (t * cycles * std::f64::consts::TAU).sin() * (1.0 - t);
            return (
                (f64::from(from) + f64::from(to) * wave).round() as i32,
                false,
            );
        }
        if self.curve == CURVE_WAVE {
            let wave = (t * f64::from(to) * std::f64::consts::TAU).sin() * (1.0 - t);
            return ((f64::from(from) * wave).round() as i32, false);
        }
        let shaped = match self.curve {
            1 => t * t,
            2 => 1.0 - (1.0 - t) * (1.0 - t),
            _ => t,
        };
        (
            (f64::from(from) + f64::from(to - from) * shaped).round() as i32,
            false,
        )
    }

    /// Applies the value for `now`; true when finished.
    pub fn apply(&self, params: &mut ObjectParams, now: u64) -> bool {
        let mut done = true;
        for &(property, from, to) in &self.targets {
            let (value, finished) = self.value_at(from, to, now);
            if finished && self.curve >= CURVE_WAVE_AROUND {
                property.set(params, from);
            } else if finished {
                self.finish_one(params, property, to);
            } else if property == Property::Visible {
                // Shown while the effect runs, whatever it ends with.
                property.set(params, 1);
            } else {
                property.set(params, value);
            }
            done &= finished;
        }
        done
    }

    fn finish_one(&self, params: &mut ObjectParams, property: Property, to: i32) {
        let value = if self.curve == CURVE_WAVE { 0 } else { to };
        property.set(params, value);
    }

    pub fn finish(&self, params: &mut ObjectParams) {
        for &(property, from, to) in &self.targets {
            if self.curve >= CURVE_WAVE_AROUND {
                property.set(params, from);
            } else {
                self.finish_one(params, property, to);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Object {
    pub params: ObjectParams,
    pub data: Option<ObjectData>,
    pub animation: Option<Animation>,
    pub mutators: Vec<Mutator>,
}

impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        self.params == other.params
            && self.animation == other.animation
            && self.mutators == other.mutators
    }
}

impl Default for Object {
    fn default() -> Self {
        Self::new(ObjectData::Text)
    }
}

impl Object {
    pub fn new(data: ObjectData) -> Self {
        Self {
            params: ObjectParams::default(),
            data: Some(data),
            animation: None,
            mutators: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        Self {
            params: ObjectParams::default(),
            data: None,
            animation: None,
            mutators: Vec::new(),
        }
    }

    pub fn image_mut(&mut self) -> Option<&mut Rc<Image>> {
        match self.data.as_mut()? {
            ObjectData::File { image, .. }
            | ObjectData::Gan { image, .. }
            | ObjectData::Digits { image, .. }
            | ObjectData::Drift { image, .. } => Some(image),
            _ => None,
        }
    }

    pub fn image(&self) -> Option<&Rc<Image>> {
        match self.data.as_ref()? {
            ObjectData::File { image, .. }
            | ObjectData::Gan { image, .. }
            | ObjectData::Digits { image, .. }
            | ObjectData::Drift { image, .. } => Some(image),
            _ => None,
        }
    }

    /// Advances animations and mutators. Returns true if anything changed.
    pub fn update(&mut self, now: u64) -> bool {
        let mut changed = false;
        if !self.mutators.is_empty() {
            let params = &mut self.params;
            self.mutators.retain(|mutator| !mutator.apply(params, now));
            changed = true;
        }
        if let Some(animation) = &mut self.animation {
            if !animation.finished {
                changed = true;
                let frames = match self.data.as_ref() {
                    Some(ObjectData::Gan { gan, .. }) => gan
                        .sets
                        .get(animation.parameter.max(0) as usize)
                        .map_or(0, |set| set.len()),
                    Some(ObjectData::File { image, .. }) => image.regions.len(),
                    _ => 0,
                };
                if frames == 0 {
                    animation.finished = true;
                } else if let Some(ObjectData::Gan { gan, .. }) = self.data.as_ref() {
                    let set = &gan.sets[animation.parameter.max(0) as usize];
                    let total: u64 = set.iter().map(|f| f.time.max(1) as u64).sum();
                    let elapsed = now.saturating_sub(animation.start);
                    match animation.after {
                        AfterAnimation::Loop | AfterAnimation::Blink => {}
                        _ if elapsed >= total => animation.finished = true,
                        _ => {}
                    }
                } else {
                    let per = animation.parameter.max(1) as u64;
                    let elapsed = now.saturating_sub(animation.start);
                    if matches!(
                        animation.after,
                        AfterAnimation::Stop | AfterAnimation::Clear
                    ) && elapsed >= per * frames as u64
                    {
                        animation.finished = true;
                    }
                }
                if animation.finished && animation.after == AfterAnimation::Clear {
                    self.data = None;
                }
            }
        }
        if let Some(ObjectData::Parent(children)) = &mut self.data {
            for child in children.iter_mut().flatten() {
                changed |= child.update(now);
            }
        }
        changed
    }

    /// Length of one cycle of the running pattern animation, in ms.
    pub fn animation_cycle(&self) -> Option<u64> {
        let animation = self.animation.as_ref()?;
        match self.data.as_ref()? {
            ObjectData::Gan { gan, .. } => {
                let set = gan.sets.get(animation.parameter.max(0) as usize)?;
                Some(set.iter().map(|f| f.time.max(1) as u64).sum())
            }
            ObjectData::File { image, .. } => {
                Some(image.regions.len().max(1) as u64 * animation.parameter.max(1) as u64)
            }
            _ => None,
        }
    }

    pub fn is_animating(&self) -> bool {
        self.animation.as_ref().is_some_and(|a| {
            !a.finished && matches!(a.after, AfterAnimation::Stop | AfterAnimation::Clear)
        })
    }

    pub fn has_mutator(&self, id: i32, repno: Option<i32>) -> bool {
        self.mutators
            .iter()
            .any(|m| (id == ALL_MUTATORS || m.id == id) && repno.is_none_or(|r| r == m.repno))
    }

    /// Finishes matching mutators immediately.
    pub fn end_mutators(&mut self, id: i32, repno: Option<i32>) {
        let params = &mut self.params;
        self.mutators.retain(|m| {
            let matches = (id == ALL_MUTATORS || m.id == id) && repno.is_none_or(|r| r == m.repno);
            if matches {
                m.finish(params);
            }
            !matches
        });
    }

    /// The pattern and GAN offset shown at `now`.
    pub fn current_frame(&self, now: u64) -> (i32, (i32, i32), i32) {
        let pattern = self.params.pattern;
        let Some(animation) = &self.animation else {
            return (pattern, (0, 0), 255);
        };
        let elapsed = now.saturating_sub(animation.start);
        match self.data.as_ref() {
            Some(ObjectData::Gan { gan, .. }) => {
                let Some(set) = gan.sets.get(animation.parameter.max(0) as usize) else {
                    return (pattern, (0, 0), 255);
                };
                if set.is_empty() {
                    return (pattern, (0, 0), 255);
                }
                let total: u64 = set.iter().map(|f| f.time.max(1) as u64).sum();
                let mut t = match animation.after {
                    AfterAnimation::Loop => elapsed % total.max(1),
                    AfterAnimation::Blink => {
                        let cycle = total + BLINK_PAUSE;
                        let phase = elapsed % cycle;
                        if phase >= total { total } else { phase }
                    }
                    _ => elapsed.min(total),
                };
                // `GAN_CUTNO_REP` shifts the GAN's patterns (digit objects
                // animated by one GAN).
                for frame in set {
                    let time = frame.time.max(1) as u64;
                    if t < time {
                        return (frame.pattern + pattern, (frame.x, frame.y), frame.alpha);
                    }
                    t -= time;
                }
                let last = set.last().expect("non-empty");
                (last.pattern + pattern, (last.x, last.y), last.alpha)
            }
            Some(ObjectData::File { image, .. }) => {
                let frames = image.regions.len().max(1) as u64;
                let per = animation.parameter.max(1) as u64;
                let step = elapsed / per;
                let index = match animation.after {
                    AfterAnimation::Loop => step % frames,
                    AfterAnimation::Blink => {
                        // 0..n-1..0, then a pause on frame 0.
                        let sweep = frames * 2 - 1;
                        let pause = BLINK_PAUSE / per;
                        let phase = step % (sweep + pause);
                        if phase < frames {
                            phase
                        } else if phase < sweep {
                            sweep - 1 - phase
                        } else {
                            0
                        }
                    }
                    _ => step.min(frames - 1),
                };
                (index as i32, (0, 0), 255)
            }
            _ => (pattern, (0, 0), 255),
        }
    }

    /// Size of the current pattern (for `objGetDims`).
    pub fn dimensions(&self, now: u64, text_size: &dyn Fn(&TextParams) -> (i32, i32)) -> (i32, i32) {
        let (scale_x, scale_y) = self.params.scale();
        let (w, h) = match self.data.as_ref() {
            Some(ObjectData::File { image, .. } | ObjectData::Gan { image, .. }) => {
                let (pattern, ..) = self.current_frame(now);
                let region = image.region(pattern);
                (region.width(), region.height())
            }
            Some(ObjectData::Text) => {
                text_size(&self.params.text)
            }
            Some(ObjectData::Rect(rect)) => (rect.w, rect.h),
            Some(ObjectData::Digits { image, .. } | ObjectData::Drift { image, .. }) => {
                let region = image.region(0);
                (region.width(), region.height())
            }
            _ => (0, 0),
        };
        (
            (f64::from(w) * scale_x).round() as i32,
            (f64::from(h) * scale_y).round() as i32,
        )
    }

    /// Unrotated screen bounds (button hit testing). `offset` is the
    /// parent's position for child objects.
    pub fn bounds(
        &self,
        now: u64,
        text_size: &dyn Fn(&TextParams) -> (i32, i32),
        offset: (i32, i32),
    ) -> Rect {
        let p = &self.params;
        let (w, h) = self.dimensions(now, text_size);
        let pattern_origin = match self.data.as_ref() {
            Some(ObjectData::File { image, .. } | ObjectData::Gan { image, .. }) => {
                let region = image.region(self.current_frame(now).0);
                (region.origin_x, region.origin_y)
            }
            _ => (0, 0),
        };
        let anchor = if p.origin != (0, 0) {
            p.origin
        } else {
            pattern_origin
        };
        let (sx, sy) = p.scale();
        let pivot = (anchor.0 + p.rep_origin.0, anchor.1 + p.rep_origin.1);
        let (ax, ay) = p.adjust_sum();
        let x = f64::from(p.x + ax + offset.0 + pivot.0 - anchor.0) - f64::from(pivot.0) * sx;
        let y = f64::from(p.y + ay + offset.1 + pivot.1 - anchor.1) - f64::from(pivot.1) * sy;
        Rect::new(x.round() as i32, y.round() as i32, w, h)
    }
}

pub fn digit_patterns(d: &DigitParams) -> Vec<i32> {
    let negative = d.value < 0;
    let digits: Vec<i32> = d
        .value
        .unsigned_abs()
        .to_string()
        .bytes()
        .map(|b| i32::from(b - b'0'))
        .collect();
    let sign = if negative {
        Some(11)
    } else if d.sign != 0 {
        Some(if d.value == 0 { 12 } else { 10 })
    } else {
        None
    };
    let width = d.digits.max(0) as usize;
    let counted_sign = usize::from(sign.is_some() && d.sign == 0);
    let pad_len = width.saturating_sub(digits.len() + counted_sign);
    let pad_value = if d.zero != 0 { 0 } else { -1 };
    let mut out = Vec::new();
    if d.zero != 0 || d.pack == 0 {
        // Sign first, then padding.
        out.extend(sign);
        out.extend(std::iter::repeat_n(pad_value, pad_len));
    } else {
        // Padding first, sign next to the number.
        out.extend(std::iter::repeat_n(pad_value, pad_len));
        out.extend(sign);
    }
    out.extend(digits);
    out
}

