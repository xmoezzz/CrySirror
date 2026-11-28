// SPDX-License-Identifier: MPL-2.0
//! Standalone casino exchange shop. Catalog and display assets come from the installed game.
use super::*;

const ROWS: usize = 9;
const CAMERA: ([f32; 3], f32, [f32; 2]) = ([0.0, 1.61, 0.0], 1.6, [175., 10.]);

struct Item {
    id: i32,
    name: String,
    price: i64,
    icon: String,
    description: String,
    product: Product,
}

enum Product {
    Outfit {
        preview: Vec<String>,
        awarded: Vec<String>,
        mask: String,
    },
    Material(i32),
}

impl Item {
    fn costume(&self) -> bool {
        matches!(self.product, Product::Outfit { .. })
    }
}

#[derive(Default)]
pub(super) struct Shop {
    panel: Vec<Widget>,
    category_parts: Vec<Widget>,
    row_parts: Vec<Widget>,
    items: Vec<Item>,
    costume: bool,
    offset: usize,
    selected: Option<usize>,
    hover: Option<usize>,
    backup: Option<crate::chara::Maid>,
}

fn menus(names: &str) -> impl Iterator<Item = String> + '_ {
    names
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            let name = name.to_ascii_lowercase();
            if name.ends_with(".menu") {
                name
            } else {
                format!("{name}.menu")
            }
        })
}

impl Game {
    pub(super) fn open_casino_shop(
        &mut self,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<String> {
        let (dealer, mut items) = {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state;
            let slot = s
                .retail
                .casino_slot
                .context("casino shop without a selected venue")?;
            let dealer = *s
                .retail
                .casino_dealers
                .get(&slot)
                .context("casino shop without a dealer")?;
            let catalog = s.retail.table("casinoshop_data.nei")?;
            let costumes = s.retail.table("casinoshop_costume_data.nei")?;
            let materials = s.retail.table("casinoshop_facilityitem_data.nei")?;
            let mut items = Vec::new();
            for row in 1..catalog.rows {
                if !catalog.exists(0, row) {
                    continue;
                }
                let id = catalog.int(0, row);
                if !s.retail.enabled("casinoshop_enable_id", id)? {
                    continue;
                }
                let product = if catalog.string(3, row) == "衣装" {
                    let r = (1..costumes.rows)
                        .find(|&r| costumes.int(0, r) == id)
                        .context("casino outfit data")?;
                    Product::Outfit {
                        preview: menus(&costumes.string(3, r)).collect(),
                        awarded: (4..costumes.columns)
                            .flat_map(|col| menus(&costumes.string(col, r)).collect::<Vec<_>>())
                            .collect(),
                        mask: costumes.string(2, r),
                    }
                } else {
                    let r = (1..materials.rows)
                        .find(|&r| materials.int(0, r) == id)
                        .context("casino upgrade material data")?;
                    Product::Material(materials.int(2, r))
                };
                items.push(Item {
                    id,
                    product,
                    name: catalog.string(1, row),
                    price: i64::from(catalog.int(2, row)),
                    icon: catalog.string(4, row),
                    description: catalog.string(5, row),
                });
            }
            (dealer, items)
        };
        for item in &mut items {
            let icon = item.icon.trim_end_matches(".tex");
            item.icon = self.assets.schedule_icon(op, host, c, &self.files, icon)?;
        }
        self.adv_camera = CAMERA;
        self.sound
            .play(host, c, &self.files, Kind::Bgm, "BGM010.ogg", 0., true)?;
        self.leisure_actor(dealer, op, now)?;
        self.leisure.casino_shop = Shop {
            panel: std::mem::take(&mut self.leisure.base),
            category_parts: self.assets.scene(op, host, c, "CasinoShopCategory")?,
            row_parts: self.assets.scene(op, host, c, "CasinoShopItem")?,
            items,
            costume: true,
            ..Default::default()
        };
        self.leisure.base = self
            .assets
            .filtered_canvas(op, host, c, CasinoShop.name(), "Canvas", |path| {
                path.is_empty() || path.starts_with("/TopFlame") || path.starts_with("/ExChangeUI")
            })?
            .base;
        self.draw_casino_shop()?;
        Ok("CasinoMini".into())
    }

    fn draw_casino_shop(&mut self) -> Result<()> {
        self.casino_widgets()?;
        if self.leisure.casino.exchange.is_some() {
            return Ok(());
        }
        let vm = self
            .script
            .tjs
            .lock()
            .map_err(|_| anyhow::anyhow!("TJS lock"))?;
        let s = &vm.runtime.host().state;
        let shop = &self.leisure.casino_shop;
        for w in &shop.panel {
            if w.path.contains("/Window-ItemInfo/") {
                let Some(index) = shop.hover else { continue };
                let item = &shop.items[index];
                let mut w = w.clone();
                if w.path.ends_with("/Icon") {
                    w.asset.clone_from(&item.icon);
                }
                if let Some(label) = w.label.as_mut() {
                    label.set_text(if w.path.ends_with("/Title") {
                        &item.name
                    } else {
                        &item.description
                    });
                }
                w.collider = None;
                self.widgets.push(w);
            } else {
                let mut w = w.clone();
                if w.path.ends_with("/TrialWear") && shop.selected.is_none()
                    || w.path.ends_with("/TrialWearReset") && shop.backup.is_none()
                {
                    w.collider = None;
                    w.tint[3] = 110;
                }
                self.widgets.push(w);
            }
        }
        for (category, name) in ["コスチューム", "施設強化素材"].into_iter().enumerate()
        {
            for w in &shop.category_parts {
                let mut w = w.clone();
                business::move_by(&mut w, 91. - 960., 521. - 540. + category as f32 * 64.);
                if let Some(label) = w.label.as_mut() {
                    label.set_text(name);
                }
                if w.path.ends_with("/Button") {
                    w.path = format!("casino-shop/category/{category}");
                    w.collider = Some(w.rect);
                    if (category == 0) == shop.costume {
                        w.tint = [255, 222, 150, 255];
                    }
                }
                self.widgets.push(w);
            }
        }
        for (row, (index, item)) in shop
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.costume() == shop.costume)
            .skip(shop.offset)
            .take(ROWS)
            .enumerate()
        {
            let sold = s.retail.casino_sold_out.contains(&item.id);
            for template in &shop.row_parts {
                let name = template.path.rsplit('/').next().unwrap_or_default();
                if (name == "Purchased" && !sold)
                    || (name == "BuyButton" && sold)
                    || name == "Impossible"
                    || (name == "Frame" && shop.selected != Some(index))
                {
                    continue;
                }
                let mut w = template.clone();
                business::move_by(&mut w, 460. - 960., 138. - 540. + row as f32 * 110.);
                w.path = format!("casino-shop/item/{index}/{name}");
                if name == "ItemIcon" {
                    w.asset.clone_from(&item.icon);
                    w.collider = Some(w.rect);
                }
                if let Some(label) = w.label.as_mut() {
                    match name {
                        "ItemName" => label.set_text(&item.name),
                        "Price" => {
                            label.set_text(&daily::money(item.price));
                            if !sold && s.retail.casino_coins < item.price {
                                w.tint = [255, 55, 55, 255];
                            }
                        }
                        _ => {}
                    }
                }
                self.widgets.push(w);
            }
        }
        Ok(())
    }

    pub(super) fn casino_shop_scroll(&mut self, input: InputFrame) -> Result<()> {
        if self.leisure.casino.exchange.is_some() {
            return Ok(());
        }
        let hover = self.widgets.iter().rev().find_map(|w| {
            let (id, name) = w.path.strip_prefix("casino-shop/item/")?.split_once('/')?;
            (name == "ItemIcon" && business::contains(w.rect, &input))
                .then(|| id.parse().ok())
                .flatten()
        });
        let shop = &mut self.leisure.casino_shop;
        let wheel = input.buttons_pressed & (buttons::WHEEL_UP | buttons::WHEEL_DOWN);
        if wheel != 0 && business::contains([145., 0., 596., 1080.], &input) {
            self.scroll_casino_shop(if wheel & buttons::WHEEL_DOWN != 0 {
                1
            } else {
                -1
            })
        } else if hover != shop.hover {
            shop.hover = hover;
            self.draw_casino_shop()
        } else {
            Ok(())
        }
    }

    fn scroll_casino_shop(&mut self, delta: isize) -> Result<()> {
        let shop = &mut self.leisure.casino_shop;
        let count = shop
            .items
            .iter()
            .filter(|i| i.costume() == shop.costume)
            .count();
        shop.offset = shop
            .offset
            .saturating_add_signed(delta)
            .min(count.saturating_sub(ROWS));
        shop.hover = None;
        self.draw_casino_shop()
    }

    pub(super) fn casino_shop_click(
        &mut self,
        index: usize,
        path: &str,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        if path.starts_with("/TopFlame/")
            || path.starts_with("/ExChangeUI/")
            || path.starts_with("exchange/")
        {
            self.casino_click(index, path, op, host, c, now)?;
            if self.leisure.dialog.is_none() {
                self.draw_casino_shop()?;
            }
            return Ok(());
        }
        let shop = &mut self.leisure.casino_shop;
        if let Some(category) = path.strip_prefix("casino-shop/category/") {
            shop.costume = category == "0";
            shop.offset = 0;
            shop.hover = None;
            shop.selected = None;
        } else if let Some((id, control)) = path
            .strip_prefix("casino-shop/item/")
            .and_then(|p| p.split_once('/'))
        {
            let id = id.parse::<usize>()?;
            let item = &shop.items[id];
            if control == "ItemIcon" && item.costume() {
                shop.selected = (shop.selected != Some(id)).then_some(id);
            } else if control == "BuyButton" {
                let (message, response) = {
                    let vm = self
                        .script
                        .tjs
                        .lock()
                        .map_err(|_| anyhow::anyhow!("TJS lock"))?;
                    let state = &vm.runtime.host().state.retail;
                    if state.casino_sold_out.contains(&item.id) {
                        return Ok(());
                    }
                    if state.casino_coins < item.price {
                        (
                            "カジノコインが不足しています。".to_string(),
                            Response::Dismiss,
                        )
                    } else {
                        (
                            format!(
                                "{}\nを購入しますか?\nカジノコイン -{}枚",
                                item.name,
                                daily::money(item.price)
                            ),
                            Response::CasinoBuy(id),
                        )
                    }
                };
                let style = if matches!(response, Response::Dismiss) {
                    "ok"
                } else {
                    "okcancel"
                };
                return self.leisure_prompt(op, host, c, now, &message, style, response);
            }
        } else if path.ends_with("/ItemList/Scroll Bar/Down") {
            return self.scroll_casino_shop(1);
        } else if path.ends_with("/ItemList/Scroll Bar/Up") {
            return self.scroll_casino_shop(-1);
        } else if path.ends_with("/ViewReset") {
            self.adv_camera = CAMERA;
        } else if path.ends_with("/TrialWear")
            && let Some(i) = shop.selected
        {
            let Product::Outfit { preview, mask, .. } = &shop.items[i].product else {
                return Ok(());
            };
            let actor = self.actors.actor(false, 0).context("casino dealer")?;
            let mut fitted = shop.backup.as_ref().unwrap_or(actor).clone();
            for name in preview {
                let menu =
                    crate::menu::MenuFile::parse(&self.files.read(op.storage.as_ref(), name)?)?;
                fitted.set_prop_file(&menu.category, name, true)?;
            }
            fitted.set_mask_mode(mask)?;
            fitted.all_proc_prop(&self.files, op.storage.as_ref())?;
            shop.backup.get_or_insert_with(|| actor.clone());
            *actor = fitted;
        } else if path.ends_with("/TrialWearReset") {
            self.cancel_casino_fitting();
        } else if path == "UI Root/Parent/OK" {
            self.cancel_casino_fitting();
            return self.resume_script(op, host, c, now);
        }
        self.draw_casino_shop()
    }

    fn cancel_casino_fitting(&mut self) {
        if let Some(original) = self.leisure.casino_shop.backup.take()
            && let Some(actor) = self.actors.actor(false, 0)
        {
            *actor = original;
        }
    }

    pub(super) fn buy_casino_item(
        &mut self,
        index: usize,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
        now: u64,
    ) -> Result<()> {
        let item = &self.leisure.casino_shop.items[index];
        let success = format!("{}を購入しました", item.name);
        {
            let mut vm = self
                .script
                .tjs
                .lock()
                .map_err(|_| anyhow::anyhow!("TJS lock"))?;
            let s = &mut vm.runtime.host_mut().state.retail;
            ensure!(
                item.price >= 0 && s.casino_coins >= item.price,
                "insufficient casino coins"
            );
            ensure!(
                !s.casino_sold_out.contains(&item.id),
                "casino item sold out"
            );
            match &item.product {
                Product::Outfit { awarded, .. } => s.award_casino_parts(awarded),
                Product::Material(id) => {
                    s.materials.insert(*id);
                }
            }
            s.casino_coins -= item.price;
            s.casino_sold_out.insert(item.id);
        }
        self.draw_casino_shop()?;
        self.leisure_prompt(op, host, c, now, &success, "ok", Response::Dismiss)
    }
}
