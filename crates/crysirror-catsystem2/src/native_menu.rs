// SPDX-License-Identifier: MPL-2.0
//! Native resource IDs/caption policy translated onto shared retained menus.
use anyhow::{ensure, Context, Result};
use crysirror_resource::{
    pe::PeResourceName,
    pe_menu::{MenuTemplate, MenuTemplateItem},
};
use crysirror_scene::ui::menu::{MenuItemSpec, MenuSpec};
use std::collections::BTreeSet;
/// Production menu resource in the statically reviewed native ABI.
pub const PRODUCTION_MENU: u32 = 104;
/// Native command disabled by GMenuReturnTitle during system initialization.
pub const RETURN_TITLE: u32 = 119;
pub fn build(template: &MenuTemplate, canvas: [u32; 2], font: &str) -> Result<MenuSpec> {
    let mut used = BTreeSet::new();
    fn collect(items: &[MenuTemplateItem], used: &mut BTreeSet<u32>) -> Result<()> {
        for i in items {
            if let Some(id) = i.id.filter(|id| *id != 0) {
                ensure!(
                    (1..=65535).contains(&id) && used.insert(id),
                    "unsupported duplicate/out-of-range native menu command"
                );
            }
            collect(&i.children, used)?;
        }
        Ok(())
    }
    collect(&template.items, &mut used)?;
    ensure!(
        used.contains(&RETURN_TITLE),
        "native production menu has no Return to title command"
    );
    let mut next = 65535;
    fn convert(
        items: &[MenuTemplateItem],
        used: &mut BTreeSet<u32>,
        next: &mut u32,
    ) -> Result<Vec<MenuItemSpec>> {
        items
            .iter()
            .map(|item| {
                ensure!(
                    item.item_type & !0x800 == 0,
                    "unsupported native menu type flags {:#x}",
                    item.item_type
                );
                let id = if let Some(id) = item.id.filter(|id| *id != 0) {
                    id
                } else {
                    while used.contains(next) {
                        *next = next
                            .checked_sub(1)
                            .context("native menu logical ID exhaustion")?;
                    }
                    let id = *next;
                    used.insert(id);
                    *next = next
                        .checked_sub(1)
                        .context("native menu logical ID exhaustion")?;
                    id
                };
                let (label, shortcut) = item.text.split_once('\t').unwrap_or((&item.text, ""));
                Ok(MenuItemSpec {
                    id,
                    label: label.into(),
                    visible: true,
                    enabled: item.state & 3 == 0,
                    checked: item.state & 8 != 0,
                    radio: false,
                    group: 0,
                    separator: item.item_type & 0x800 != 0
                        || (item.id == Some(0) && item.text.is_empty()),
                    shortcut: shortcut.into(),
                    children: convert(&item.children, used, next)?,
                })
            })
            .collect()
    }
    let menu = MenuSpec {
        id: PRODUCTION_MENU,
        rect: [0., 0., canvas[0] as f32, 24.],
        font: font.into(),
        text_size: 18,
        cell_width: 120.,
        popup_width: 360.,
        row_height: 24.,
        visible: true,
        context: false,
        items: convert(&template.items, &mut used, &mut next)?,
    };
    menu.validate()?;
    Ok(menu)
}
pub fn production(resources: &[crysirror_resource::pe::PeMenuResource]) -> Result<&MenuTemplate> {
    let mut menus = resources
        .iter()
        .filter(|m| m.name == PeResourceName::Id(PRODUCTION_MENU) && m.language == 1041);
    let first = menus
        .next()
        .context("reviewed native RT_MENU #104 language 1041 missing")?;
    ensure!(menus.next().is_none(), "duplicate native production menu");
    Ok(&first.template)
}
