// SPDX-License-Identifier: MPL-2.0
//! Front-end menu pages of `dc1.psarc/menu.bin`.
//!
//! `*menu-pages*` is `{count, page array}`; a page is 0x44 bytes: name
//! (string pointer, `StringId`), title text key at +0x10 (string pointer,
//! `StringId`), flags at +0x1c and its button list at +0x20 (`{count,
//! array}`). A button is 0x48 bytes: name at +0, type +8, action +0xc,
//! text key at +0x14 (pointer, `StringId`) and a visibility condition at
//! +0x1c (0 always shown; for example 0xf on Continue, 0x13 on Load Game
//! and Medals, which depend on saved progress). The condition codes are
//! evaluated by the EBOOT and are not decoded here: buttons with a
//! condition are hidden, matching a profile without saves.
use crate::dc::{Dc, sid};
use anyhow::{Context, Result, ensure};

#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub name: String,
    pub text: String,
    pub condition: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub name: String,
    pub flags: u32,
    pub title: Option<String>,
    pub buttons: Vec<Button>,
}

/// Every page of `menu.bin`.
pub fn pages(bytes: &[u8]) -> Result<Vec<Page>> {
    let mut dc = Dc::default();
    let module = dc.load(sid("menu"), bytes)?.clone();
    let root = module.entries.iter().find(|(n, _)| *n == sid("*menu-pages*")).map(|(_, a)| *a).context("menu.bin without *menu-pages*")?;
    let m = &dc.mem;
    let text = |at: u32| -> Option<String> { m.u32(at).ok().filter(|p| *p != 0).and_then(|p| m.cstr(p).ok()) };
    let count = m.u32(root)?;
    ensure!(count <= 1024, "menu page count");
    let array = m.u32(root + 4)?;
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let page = array + 0x44 * i;
        let mut buttons = Vec::new();
        let list = m.u32(page + 0x20)?;
        if list != 0 {
            let n = m.u32(list)?;
            ensure!(n <= 256, "menu button count");
            let at = m.u32(list + 4)?;
            for b in 0..n {
                let button = at + 0x48 * b;
                buttons.push(Button {
                    name: text(button).unwrap_or_default(),
                    text: text(button + 0x14).unwrap_or_default(),
                    condition: m.u32(button + 0x1c)?,
                });
            }
        }
        out.push(Page { name: text(page).unwrap_or_default(), flags: m.u32(page + 0x1c)?, title: text(page + 0x10), buttons });
    }
    Ok(out)
}
