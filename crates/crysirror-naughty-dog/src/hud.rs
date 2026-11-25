// SPDX-License-Identifier: MPL-2.0
//! HUD elements of `hud-settings.bin` (`hud-button` records), shown by
//! `display-button-hud` (`sub_8A26D0`). A record is `{x, y, w, h}`
//! (floats, 1280×720 screen), …, +0x28 text catalog key (string, may be
//! empty), +0x30/+0x34 text offset, +0x38 RGBA colour (floats), +0x48 an
//! array object of texture paths (`general/hud/buttons/…`, in
//! `pak22/hud.pak`).
use crate::dc::{Dc, sid};
use anyhow::{Result, ensure};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct HudButton {
    pub rect: [f32; 4],
    pub text: Option<String>,
    pub text_offset: [f32; 2],
    pub color: [f32; 4],
    pub images: Vec<String>,
}

pub fn parse(bytes: &[u8]) -> Result<HashMap<u32, HudButton>> {
    let mut dc = Dc::default();
    let module = dc.load(sid("hud-settings"), bytes)?.clone();
    let kind = sid("hud-button");
    let m = &dc.mem;
    let mut out = HashMap::new();
    for (name, p) in module.entries {
        if m.type_of(p).ok() != Some(kind) {
            continue;
        }
        let f = |o: u32| m.f32(p + o);
        let text = m.u32(p + 0x28).ok().filter(|t| *t != 0).and_then(|t| m.cstr(t).ok()).filter(|t| !t.is_empty());
        // The image list: an array object `{.., count?, paths…}` whose
        // string pointers follow its `array` tag.
        let mut images = Vec::new();
        if let Ok(list) = m.u32(p + 0x48) {
            for k in 0..8u32 {
                let Ok(q) = m.u32(list + 8 + 4 * k) else { break };
                let Ok(s) = m.cstr(q) else { continue };
                if s.starts_with("general/") && !images.contains(&s) {
                    images.push(s);
                }
            }
        }
        out.insert(name, HudButton { rect: [f(0)?, f(4)?, f(8)?, f(12)?], text, text_offset: [f(0x30)?, f(0x34)?], color: [f(0x38)?, f(0x3c)?, f(0x40)?, f(0x44)?], images });
    }
    ensure!(out.len() <= 4096, "HUD element count");
    Ok(out)
}

/// HUD material names (lowercase, `…/default`) → pass-0 colour and mask
/// texture IDs of a package's `MATERIAL_TABLE_1` (+0xc count, +0x20
/// record pointers).
pub fn materials(p: &crate::mesh::Loaded, directory: &crate::package::Directory) -> Result<HashMap<String, (u32, Option<u32>)>> {
    let mut out = HashMap::new();
    for item in directory.items.iter().filter(|i| i.kind == "MATERIAL_TABLE_1") {
        let payload = item.payload_offset as usize;
        let count = p.u32(payload + 0xc)? as usize;
        ensure!(count <= 4096, "HUD material count");
        let array = p.ptr(payload + 0x20)?;
        for i in 0..count {
            let Ok(m) = p.ptr(array + 4 * i) else { continue };
            let Ok(material) = crate::mesh::material(p, m) else { continue };
            if let Some(&color) = material.samplers.first() {
                out.insert(material.name.to_lowercase(), (color, material.samplers.get(1).copied()));
            }
        }
    }
    Ok(out)
}
