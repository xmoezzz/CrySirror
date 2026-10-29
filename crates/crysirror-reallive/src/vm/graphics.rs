// SPDX-License-Identifier: MPL-2.0
//! The object store of the original `Graphics` (siglus_rs 5f358ef
//! `graphics.rs`: `ObjectRef`, object lookup, promotion, animation queues).
//! Images are engine assets with their g00 pattern regions; pixels and
//! composition belong to the engine, which the adapter drives from here.
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::{Result, anyhow};
use crysirror_image::g00::Region;

use crate::vm::object::{OBJECT_COUNT, Object, ObjectData, TextParams};
use crate::vm::rect::Rect;

/// A bitmap known to the engine: its asset path and pattern regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub asset: String,
    pub width: i32,
    pub height: i32,
    pub regions: Vec<Region>,
}

impl Image {
    // Copied from upstream `image::Image::region`.
    pub fn region(&self, pattern: i32) -> Region {
        usize::try_from(pattern)
            .ok()
            .and_then(|index| self.regions.get(index))
            .copied()
            .unwrap_or_else(|| Region::full(self.width, self.height))
    }
}

/// Resource access the object functions need, provided by the adapter.
pub trait AssetLoader {
    fn image(&self, name: &str) -> Result<Rc<Image>>;
    fn gan(&self, name: &str) -> Result<Vec<u8>>;
    /// Unscaled size of a text object's rendering.
    fn text_size(&self, text: &TextParams) -> (i32, i32);
}

// Copied from upstream `graphics.rs`.
/// Which object slot an operation addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectRef {
    /// false = foreground, true = background.
    pub bg: bool,
    pub buf: i32,
    /// A child of `buf` (modtype 2 functions).
    pub child: Option<i32>,
}

impl ObjectRef {
    pub fn fg(buf: i32) -> Self {
        Self {
            bg: false,
            buf,
            child: None,
        }
    }
}

pub struct Gfx {
    pub fg: Vec<Option<Object>>,
    pub bg: Vec<Option<Object>>,
    pub width: i32,
    pub height: i32,
    pub paused_animations: HashMap<usize, u64>,
    /// `*GANANM_NEXT_*`: animations to start when the running one ends
    /// (a looping one at the end of its cycle): (object, when, animation).
    pub queued_animations: Vec<(ObjectRef, Option<u64>, crate::vm::object::Animation)>,
    pub dirty: bool,
    pub loader: Option<Rc<dyn AssetLoader>>,
}

impl std::fmt::Debug for Gfx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gfx").field("width", &self.width).field("height", &self.height).finish_non_exhaustive()
    }
}

impl Clone for Gfx {
    fn clone(&self) -> Self {
        Self {
            fg: self.fg.clone(),
            bg: self.bg.clone(),
            width: self.width,
            height: self.height,
            paused_animations: self.paused_animations.clone(),
            queued_animations: self.queued_animations.clone(),
            dirty: self.dirty,
            loader: self.loader.clone(),
        }
    }
}

impl Default for Gfx {
    fn default() -> Self {
        Self {
            fg: vec![None; OBJECT_COUNT],
            bg: vec![None; OBJECT_COUNT],
            width: 640,
            height: 480,
            paused_animations: HashMap::new(),
            queued_animations: Vec::new(),
            dirty: true,
            loader: None,
        }
    }
}

impl Gfx {
    fn loader(&self) -> Result<Rc<dyn AssetLoader>> {
        self.loader.clone().ok_or_else(|| anyhow!("reallive: no resource loader"))
    }

    pub fn load_image(&mut self, name: &str) -> Result<Rc<Image>> {
        self.loader()?.image(name)
    }

    pub fn load_gan(&mut self, name: &str) -> Result<Rc<crate::vm::gan::Gan>> {
        let bytes = self.loader()?.gan(name)?;
        Ok(Rc::new(crate::vm::gan::Gan::parse(&bytes)?))
    }

    pub fn text_size(&self) -> impl Fn(&TextParams) -> (i32, i32) + '_ {
        move |p| self.loader.as_ref().map_or((0, 0), |l| l.text_size(p))
    }

    pub fn screen_rect(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    // ---- objects (copied from upstream `graphics.rs`) -----------------------

    fn layer_mut(&mut self, bg: bool) -> &mut Vec<Option<Object>> {
        if bg { &mut self.bg } else { &mut self.fg }
    }

    pub fn object(&self, r: ObjectRef) -> Option<&Object> {
        let layer = if r.bg { &self.bg } else { &self.fg };
        let parent = layer.get(usize::try_from(r.buf).ok()?)?.as_ref()?;
        match r.child {
            None => Some(parent),
            Some(child) => match parent.data.as_ref()? {
                ObjectData::Parent(children) => {
                    children.get(usize::try_from(child).ok()?)?.as_ref()
                }
                _ => None,
            },
        }
    }

    /// The object at `r`, created empty if absent (a parent is created for
    /// child references).
    pub fn object_mut(&mut self, r: ObjectRef) -> Result<&mut Object> {
        let index = usize::try_from(r.buf)
            .ok()
            .filter(|&i| i < OBJECT_COUNT)
            .ok_or_else(|| anyhow!("reallive: invalid object {}", r.buf))?;
        self.dirty = true;
        let slot = &mut self.layer_mut(r.bg)[index];
        let parent = slot.get_or_insert_with(Object::empty);
        let Some(child) = r.child else {
            return Ok(parent);
        };
        let child = usize::try_from(child)
            .ok()
            .filter(|&i| i < OBJECT_COUNT)
            .ok_or_else(|| anyhow!("reallive: invalid child object {child}"))?;
        if !matches!(parent.data, Some(ObjectData::Parent(_))) {
            parent.data = Some(ObjectData::Parent(vec![None; OBJECT_COUNT]));
        }
        let Some(ObjectData::Parent(children)) = &mut parent.data else {
            unreachable!("just made a parent");
        };
        Ok(children[child].get_or_insert_with(Object::empty))
    }

    /// Layer promotion (`grpOpenBg`, `grpDisplay`, ...): foreground objects
    /// without the wipe-copy flag are deleted and background objects move
    /// to the foreground.
    pub fn promote_objects(&mut self) {
        for index in 0..OBJECT_COUNT {
            let background = self.bg[index].take();
            match background {
                Some(object) if object.data.is_some() => self.fg[index] = Some(object),
                _ => {
                    if self.fg[index]
                        .as_ref()
                        .is_some_and(|object| !object.params.wipe_copy)
                    {
                        self.fg[index] = None;
                    }
                }
            }
        }
        self.dirty = true;
    }

    /// Advances queued animations, pattern animations and mutators.
    pub fn update(&mut self, now: u64) -> bool {
        let mut changed = false;
        for (r, due, mut animation) in std::mem::take(&mut self.queued_animations) {
            let Ok(object) = self.object_mut(r) else {
                continue;
            };
            let ready = match due {
                Some(due) => now >= due,
                None => !object.is_animating(),
            };
            if ready {
                animation.start = due.unwrap_or(now).min(now);
                object.animation = Some(animation);
                changed = true;
            } else {
                self.queued_animations.push((r, due, animation));
            }
        }
        for layer in [&mut self.fg, &mut self.bg] {
            for object in layer.iter_mut().flatten() {
                changed |= object.update(now);
            }
        }
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// `OBJFRONTANM_PAUSE` / `_PAUSEALL`.
    pub fn pause_animation(&mut self, index: usize, now: u64) {
        self.paused_animations.entry(index).or_insert(now);
    }

    /// `OBJFRONTANM_RESUME` / `_RESUMEALL`: continue from where it stopped.
    pub fn resume_animation(&mut self, index: usize, now: u64) {
        if let Some(since) = self.paused_animations.remove(&index)
            && let Some(Some(object)) = self.fg.get_mut(index)
            && let Some(animation) = &mut object.animation
        {
            animation.start += now.saturating_sub(since);
        }
    }
}
