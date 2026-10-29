//! Native menu-bar presentation of the engine MenuSystem (macOS, via muda).
//!
//! The engine keeps owning menu trees, radio state and activation events;
//! this module only mirrors the visible bars into the OS menu bar and turns a
//! native selection back into `UiSystem::activate_menu_item`. Platforms
//! without a safe native path keep the engine-drawn bar.
use anyhow::Result;
use crysirror_scene::ui::{UiSystem, menu::MenuItemSpec};

/// What the desktop must do after draining native menu selections.
#[derive(Default)]
pub struct Selections {
    pub quit: bool,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use crysirror_scene::ui::menu::MenuSpec;
    use muda::{
        CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::Accelerator,
    };

    const QUIT: &str = "crysirror:quit";

    pub struct NativeMenu {
        menu: Menu,
        app_name: String,
        /// Bars last mirrored; a native check toggle forces a rebuild.
        shown: Option<Vec<(u32, MenuSpec)>>,
    }

    fn id(owner: u32, bar: u32, item: u32) -> String {
        format!("{owner}:{bar}:{item}")
    }

    fn parse_id(id: &str) -> Option<(u32, u32, u32)> {
        let mut parts = id.split(':').map(str::parse::<u32>);
        let (Some(Ok(o)), Some(Ok(b)), Some(Ok(i)), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return None;
        };
        Some((o, b, i))
    }

    /// KiriKiri-style `Ctrl+F12`; an accelerator muda cannot parse stays
    /// display-free rather than failing the whole bar.
    fn accelerator(text: &str) -> Option<Accelerator> {
        (!text.is_empty()).then(|| text.parse().ok()).flatten()
    }

    fn append_items(
        target: &dyn Fn(&dyn muda::IsMenuItem) -> muda::Result<()>,
        owner: u32,
        bar: u32,
        items: &[MenuItemSpec],
    ) -> Result<()> {
        for item in items.iter().filter(|i| i.visible) {
            if item.separator {
                target(&PredefinedMenuItem::separator())?;
            } else if !item.children.is_empty() {
                let sub = Submenu::with_id(id(owner, bar, item.id), &item.label, item.enabled);
                append_items(&|i| sub.append(i), owner, bar, &item.children)?;
                target(&sub)?;
            } else if item.checked || item.radio {
                target(&CheckMenuItem::with_id(
                    id(owner, bar, item.id),
                    &item.label,
                    item.enabled,
                    item.checked,
                    accelerator(&item.shortcut),
                ))?;
            } else {
                target(&MenuItem::with_id(
                    id(owner, bar, item.id),
                    &item.label,
                    item.enabled,
                    accelerator(&item.shortcut),
                ))?;
            }
        }
        Ok(())
    }

    impl NativeMenu {
        pub fn new(app_name: &str) -> Self {
            Self { menu: Menu::new(), app_name: app_name.to_owned(), shown: None }
        }

        /// Mirror the engine's visible bars when they changed.
        pub fn sync(&mut self, ui: &mut UiSystem) -> Result<()> {
            ui.menus.set_native_bars(true);
            let bars: Vec<(u32, MenuSpec)> =
                ui.menus.bars().map(|(owner, spec)| (owner, spec.clone())).collect();
            if self.shown.as_ref() == Some(&bars) {
                return Ok(());
            }
            let menu = Menu::new();
            // macOS shows the first submenu as the application menu.
            let app = Submenu::new(&self.app_name, true);
            app.append_items(&[
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::separator(),
                // Routed through the engine's close path, never NSApp terminate.
                &MenuItem::with_id(QUIT, "Quit", true, "CmdOrCtrl+Q".parse().ok()),
            ])?;
            menu.append(&app)?;
            for (owner, spec) in &bars {
                // An empty bar title has nothing to show or click on macOS.
                for item in spec.items.iter().filter(|i| i.visible && !i.separator && !i.label.is_empty()) {
                    // A menu-bar title must be a submenu on macOS: a top-level
                    // leaf becomes a submenu holding that one item.
                    let sub = Submenu::with_id(
                        format!("bar:{}", id(*owner, spec.id, item.id)),
                        &item.label,
                        item.enabled,
                    );
                    let children = if item.children.is_empty() {
                        std::slice::from_ref(item)
                    } else {
                        item.children.as_slice()
                    };
                    append_items(&|i| sub.append(i), *owner, spec.id, children)?;
                    menu.append(&sub)?;
                }
            }
            menu.init_for_nsapp();
            let titles: Vec<String> = menu.items().iter().filter_map(|i| i.as_submenu().map(|s| s.text())).collect();
            log::info!("native menu bar installed: {titles:?}");
            self.menu = menu;
            self.shown = Some(bars);
            Ok(())
        }

        pub fn drain(&mut self, ui: &mut UiSystem) -> Result<Selections> {
            let mut out = Selections::default();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let name = event.id().as_ref();
                if name == QUIT {
                    out.quit = true;
                } else if let Some((owner, bar, item)) = parse_id(name) {
                    if let Err(error) = ui.activate_menu_item(owner, bar, item) {
                        log::warn!("native menu {name}: {error:#}");
                    }
                    // Native check items toggle themselves; re-mirror the
                    // engine's authoritative state.
                    self.shown = None;
                } else {
                    log::warn!("native menu: unknown item {name}");
                }
            }
            Ok(out)
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    /// No safe native menu path on this platform: bars stay engine-drawn.
    pub struct NativeMenu;
    impl NativeMenu {
        pub fn new(_: &str) -> Self {
            log::info!("native menu bar unavailable on this platform; engine-drawn bar kept");
            Self
        }
        pub fn sync(&mut self, _: &mut UiSystem) -> Result<()> {
            Ok(())
        }
        pub fn drain(&mut self, _: &mut UiSystem) -> Result<Selections> {
            Ok(Selections::default())
        }
    }
}

pub use imp::NativeMenu;

