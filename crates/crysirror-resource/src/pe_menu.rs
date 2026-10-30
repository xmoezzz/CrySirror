// SPDX-License-Identifier: MPL-2.0
//! Safe CPU reader for standard Windows MENU templates. No platform handles,
//! callbacks, executable code, native renderer or user interface is created.
//! Layout references: Microsoft MENUITEMTEMPLATEHEADER/MENUITEMTEMPLATE.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

/// Maximum bytes admitted for a single menu resource template.
pub const MAX_TEMPLATE_BYTES: usize = 1024 * 1024;
/// Maximum entries, including separators and nested popup roots.
pub const MAX_MENU_ITEMS: usize = 1024;
/// Parsed immutable menu tree, independent of any VM/window implementation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuTemplate {
    /// Native format. Only standard MENU version zero is currently admitted.
    pub version: u16,
    /// Ordered root item records, preserving nested popup boundaries.
    pub items: Vec<MenuTemplateItem>,
}
/// Resource template item; numeric IDs remain logical data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuTemplateItem {
    /// Native command ID. Standard MENU popup headers have no ID.
    pub id: Option<u32>,
    /// Unmodified Unicode label, including native mnemonics/shortcut text.
    pub text: String,
    /// Native MF type bits; structural popup/end and state flags are removed.
    pub item_type: u32,
    /// Native disabled/grayed/checked state bits.
    pub state: u32,
    /// Popup descendants in original order; empty for ordinary commands.
    pub children: Vec<MenuTemplateItem>,
}
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    count: usize,
    text_bytes: usize,
}
impl Cursor<'_> {
    fn word(&mut self) -> Result<u16> {
        let end = self.at.checked_add(2).context("menu word overflow")?;
        let b = self
            .bytes
            .get(self.at..end)
            .context("truncated menu word")?;
        self.at = end;
        Ok(u16::from_le_bytes(b.try_into()?))
    }
    fn text(&mut self) -> Result<String> {
        let mut chars = Vec::new();
        loop {
            let c = self.word()?;
            if c == 0 {
                break;
            }
            ensure!(chars.len() < 2048, "menu label word budget");
            chars.push(c);
        }
        let text = String::from_utf16(&chars).context("invalid menu UTF-16 label")?;
        self.text_bytes = self
            .text_bytes
            .checked_add(text.len())
            .context("menu cumulative label overflow")?;
        ensure!(self.text_bytes <= 64 * 1024, "menu cumulative label budget");
        Ok(text)
    }
    fn group(&mut self, depth: usize) -> Result<Vec<MenuTemplateItem>> {
        ensure!(depth <= 16, "menu nesting budget");
        let mut items = Vec::new();
        loop {
            ensure!(self.count < MAX_MENU_ITEMS, "menu item budget");
            self.count += 1;
            let flags = self.word()?;
            let popup = flags & 0x10 != 0;
            let id = if popup {
                None
            } else {
                Some(u32::from(self.word()?))
            };
            let text = self.text()?;
            let children = if popup {
                self.group(depth + 1)?
            } else {
                Vec::new()
            };
            items.push(MenuTemplateItem {
                id,
                text,
                item_type: u32::from(flags & !0x9b),
                state: u32::from(flags & 0xb),
                children,
            });
            if flags & 0x80 != 0 {
                return Ok(items);
            }
        }
    }
}
impl MenuTemplate {
    /// Parse one original standard template without creating a native window
    /// or executing a callback. MENUEX/version one is explicitly unsupported.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(
            (4..=MAX_TEMPLATE_BYTES).contains(&bytes.len()),
            "menu template byte budget"
        );
        let mut c = Cursor {
            bytes,
            at: 0,
            count: 0,
            text_bytes: 0,
        };
        let version = c.word()?;
        let offset = c.word()? as usize;
        ensure!(version == 0, "unsupported menu template version {version}");
        let at = 4usize
            .checked_add(offset)
            .context("menu header offset overflow")?;
        ensure!(
            at <= bytes.len() && at % 2 == 0,
            "menu header offset outside resource/alignment"
        );
        c.at = at;
        let items = c.group(0)?;
        ensure!(
            bytes[c.at..].len() <= 3 && bytes[c.at..].iter().all(|b| *b == 0),
            "menu template trailing data"
        );
        Ok(Self { version, items })
    }
}
