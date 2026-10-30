//! Client-defined pointer cursors: standard shapes and custom RGBA images
//! (single or animated). A presenting host maps the selected cursor onto its
//! window system; definitions are client-scoped and shared by `Arc` so a UI
//! batch clone does not copy pixels.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

/// Most cursor definitions one client may keep.
pub const MAX_CURSORS: usize = 256;
/// Largest custom cursor edge in pixels.
pub const MAX_CURSOR_EDGE: u32 = 256;
/// Most frames/steps of one animated cursor.
pub const MAX_CURSOR_FRAMES: usize = 256;

/// Platform-neutral standard pointer shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardCursor {
    Default,
    Crosshair,
    Text,
    VerticalText,
    Move,
    NeswResize,
    NsResize,
    NwseResize,
    EwResize,
    Wait,
    Progress,
    Grabbing,
    NoDrop,
    ColResize,
    RowResize,
    Copy,
    NotAllowed,
    Help,
    Pointer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CursorSelect {
    Standard {
        shape: StandardCursor,
    },
    /// A cursor previously defined by the same client.
    Custom {
        id: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorFrameSpec {
    pub width: u32,
    pub height: u32,
    pub hotspot: [u32; 2],
    /// Straight (non-premultiplied) RGBA8, row-major, top-left origin.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorSpec {
    pub frames: Vec<CursorFrameSpec>,
    /// `[frame index, milliseconds]`; one step of 0 ms for a still cursor.
    pub steps: Vec<[u32; 2]>,
}

impl CursorSpec {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=MAX_CURSOR_FRAMES).contains(&self.frames.len())
                && (1..=MAX_CURSOR_FRAMES).contains(&self.steps.len()),
            "cursor frame/step count"
        );
        for f in &self.frames {
            ensure!(
                (1..=MAX_CURSOR_EDGE).contains(&f.width)
                    && (1..=MAX_CURSOR_EDGE).contains(&f.height),
                "cursor size"
            );
            ensure!(
                f.rgba.len() == (f.width * f.height * 4) as usize,
                "cursor RGBA length"
            );
            ensure!(
                f.hotspot[0] < f.width && f.hotspot[1] < f.height,
                "cursor hotspot"
            );
        }
        ensure!(
            self.steps
                .iter()
                .all(|[i, _]| (*i as usize) < self.frames.len()),
            "cursor step frame index"
        );
        ensure!(
            self.steps.len() == 1 || self.steps.iter().all(|[_, ms]| *ms > 0),
            "animated cursor step duration"
        );
        Ok(())
    }
}

/// Cursor definitions and the active selection.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CursorSystem {
    /// `(client, id)` → definition. Not persisted: clients redefine on restore.
    #[serde(skip)]
    defs: BTreeMap<(u32, u32), Arc<CursorSpec>>,
    /// Selected cursor and its owning client; `None` is the host default.
    #[serde(default)]
    pub selected: Option<(u32, CursorSelect)>,
    /// Bumped on every definition so a host can rebuild cached OS cursors.
    #[serde(skip)]
    pub revision: u64,
}

impl CursorSystem {
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty() && self.selected.is_none()
    }

    pub fn define(&mut self, client: u32, id: u32, spec: CursorSpec) -> Result<()> {
        spec.validate()?;
        ensure!(
            self.defs.contains_key(&(client, id))
                || self.defs.range((client, 0)..=(client, u32::MAX)).count() < MAX_CURSORS,
            "cursor definition limit"
        );
        self.defs.insert((client, id), Arc::new(spec));
        self.revision += 1;
        Ok(())
    }

    pub fn select(&mut self, client: u32, cursor: CursorSelect) -> Result<()> {
        if let CursorSelect::Custom { id } = cursor {
            ensure!(
                self.defs.contains_key(&(client, id)),
                "cursor {id} is not defined"
            );
        }
        self.selected = Some((client, cursor));
        Ok(())
    }

    /// The custom definition selected now, if any.
    pub fn get(&self, client: u32, id: u32) -> Option<&Arc<CursorSpec>> {
        self.defs.get(&(client, id))
    }
}
