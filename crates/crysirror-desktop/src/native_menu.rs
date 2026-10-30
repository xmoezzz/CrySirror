//! Native menu-bar presentation of the engine MenuSystem (muda).
//!
//! The engine keeps owning menu trees, radio state and activation events;
//! this module only mirrors the visible bars into the OS menu bar and turns a
//! native selection back into `UiSystem::activate_menu_item`. Menus never
//! draw over the game canvas.
//!
//! - macOS: the application menu bar (`init_for_nsapp`), with an app menu.
//! - Windows: the window's own menu bar (`init_for_hwnd`); accelerators are
//!   translated in the event-loop message hook (`accelerator_hook`).
//! - Other platforms: muda's menu bar needs a GTK window, which a winit window
//!   is not; retain the visible engine menu instead of hiding its controls.
use anyhow::Result;
use crysirror_scene::ui::{UiSystem, menu::MenuItemSpec};
use winit::window::Window;

/// What the desktop must do after draining native menu selections.
#[derive(Default)]
pub struct Selections {
    pub quit: bool,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
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
        #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
        app_name: String,
        /// Bars last mirrored; a native check toggle forces a rebuild.
        shown: Option<Vec<(u32, MenuSpec)>>,
        #[cfg(target_os = "windows")]
        hwnd: isize,
        #[cfg(target_os = "windows")]
        attached: bool,
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

    /// Both pointer selections and native accelerators enter through this
    /// engine-owned validation path. Stale/disabled items cannot call the VM.
    fn dispatch(ui: &mut UiSystem, name: &str) -> Result<Selections> {
        if name == QUIT {
            return Ok(Selections { quit: true });
        }
        let (owner, bar, item) =
            parse_id(name).ok_or_else(|| anyhow::anyhow!("unknown native menu identity {name}"))?;
        ui.activate_menu_item(owner, bar, item)?;
        log::info!("native menu activated: {name}");
        Ok(Selections::default())
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

    #[cfg(target_os = "windows")]
    fn hwnd(window: &Window) -> Result<isize> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match window.window_handle()?.as_raw() {
            RawWindowHandle::Win32(h) => Ok(h.hwnd.get()),
            other => anyhow::bail!("unexpected Windows window handle {other:?}"),
        }
    }

    /// The live Windows accelerator table, read by the message hook.
    #[cfg(target_os = "windows")]
    pub(crate) static HACCEL: std::sync::atomic::AtomicIsize =
        std::sync::atomic::AtomicIsize::new(0);

    impl NativeMenu {
        pub fn new(app_name: &str, window: &Window) -> Result<Self> {
            let _ = window;
            Ok(Self {
                menu: Menu::new(),
                app_name: app_name.to_owned(),
                shown: None,
                #[cfg(target_os = "windows")]
                hwnd: hwnd(window)?,
                #[cfg(target_os = "windows")]
                attached: false,
            })
        }

        /// Mirror the engine's visible bars when they changed.
        #[cfg_attr(target_os = "windows", allow(unsafe_code))]
        pub fn sync(&mut self, ui: &mut UiSystem) -> Result<()> {
            ui.menus.set_native_bars(true);
            let bars: Vec<(u32, MenuSpec)> = ui
                .menus
                .bars()
                .map(|(owner, spec)| (owner, spec.clone()))
                .collect();
            if self.shown.as_ref() == Some(&bars) {
                return Ok(());
            }
            let menu = Menu::new();
            // macOS shows the first submenu as the application menu.
            #[cfg(target_os = "macos")]
            {
                let app = Submenu::new(&self.app_name, true);
                app.append_items(&[
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::separator(),
                    // Routed through the engine's close path, never NSApp terminate.
                    &MenuItem::with_id(QUIT, "Quit", true, "CmdOrCtrl+Q".parse().ok()),
                ])?;
                menu.append(&app)?;
            }
            for (owner, spec) in &bars {
                // An empty bar title has nothing to show or click.
                for item in spec
                    .items
                    .iter()
                    .filter(|i| i.visible && !i.separator && !i.label.is_empty())
                {
                    if item.children.is_empty() && cfg!(target_os = "windows") {
                        // Windows menu bars take top-level command items.
                        append_items(
                            &|i| menu.append(i),
                            *owner,
                            spec.id,
                            std::slice::from_ref(item),
                        )?;
                        continue;
                    }
                    // A macOS menu-bar title must be a submenu: a top-level
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
            #[cfg(target_os = "macos")]
            menu.init_for_nsapp();
            #[cfg(target_os = "windows")]
            // SAFETY: `hwnd` is the live top-level window this host owns; the
            // previous bar is detached from it before the new one attaches.
            unsafe {
                if self.attached {
                    self.menu.remove_for_hwnd(self.hwnd).ok();
                }
                if !bars.is_empty() {
                    menu.init_for_hwnd(self.hwnd)?;
                }
                self.attached = !bars.is_empty();
                HACCEL.store(menu.haccel(), std::sync::atomic::Ordering::Release);
            }
            let titles: Vec<String> = menu
                .items()
                .iter()
                .filter_map(|i| i.as_submenu().map(|s| s.text()))
                .collect();
            log::info!("native menu bar installed: {titles:?}");
            self.menu = menu;
            self.shown = Some(bars);
            Ok(())
        }

        pub fn drain(&mut self, ui: &mut UiSystem) -> Result<Selections> {
            let mut out = Selections::default();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let name = event.id().as_ref();
                match dispatch(ui, name) {
                    Ok(selection) => out.quit |= selection.quit,
                    Err(error) => log::warn!("native menu {name}: {error:#}"),
                }
                // Native check items toggle themselves; re-mirror even a
                // rejected selection from the engine's authoritative state.
                self.shown = None;
            }
            Ok(out)
        }
    }

    #[cfg(target_os = "windows")]
    impl Drop for NativeMenu {
        fn drop(&mut self) {
            // Never leave the event-loop hook pointing into a dropped menu's
            // accelerator storage. The window may already have been destroyed.
            HACCEL.store(0, std::sync::atomic::Ordering::Release);
        }
    }

    /// winit Windows message hook: menu accelerators (TranslateAccelerator).
    #[cfg(target_os = "windows")]
    #[allow(unsafe_code)]
    pub fn accelerator_hook(msg: *const std::ffi::c_void) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, TranslateAcceleratorW};
        let haccel = HACCEL.load(std::sync::atomic::Ordering::Acquire);
        if haccel == 0 || msg.is_null() {
            return false;
        }
        // SAFETY: winit passes a valid MSG for the duration of the hook; the
        // accelerator table belongs to the menu currently attached.
        unsafe {
            let msg = &*(msg as *const MSG);
            TranslateAcceleratorW(msg.hwnd, haccel as _, msg) != 0
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn bar() -> MenuSpec {
            serde_json::from_value(serde_json::json!({
                "id": 7, "rect": [0,0,640,24], "font": "default",
                "items": [{"id":1,"label":"View","children":[
                    {"id":2,"label":"Small","radio":true,"group":1,"checked":true},
                    {"id":3,"label":"Large","radio":true,"group":1},
                    {"id":4,"label":"Unavailable","enabled":false}
                ]}]
            }))
            .unwrap()
        }

        #[test]
        fn native_selection_preserves_owner_radio_state_and_event_identity() {
            let mut ui = UiSystem::default();
            ui.menus.put(1, bar()).unwrap();
            ui.menus.put(2, bar()).unwrap();
            assert!(!dispatch(&mut ui, "2:7:3").unwrap().quit);
            assert!(ui.drain_events(1, None).is_empty());
            let events = ui.drain_events(2, None);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].menu_bar, Some(7));
            assert_eq!(events[0].widget.id, 3);
            assert_eq!(events[0].value, 1.);
            assert!(ui.menus.get(1, 7).unwrap().items[0].children[0].checked);
            assert!(!ui.menus.get(2, 7).unwrap().items[0].children[0].checked);
            assert!(ui.menus.get(2, 7).unwrap().items[0].children[1].checked);
        }

        #[test]
        fn stale_native_selection_cannot_activate_hidden_or_disabled_items() {
            let mut ui = UiSystem::default();
            ui.menus.put(1, bar()).unwrap();
            assert!(dispatch(&mut ui, "1:7:4").is_err());
            let mut hidden = bar();
            hidden.items[0].visible = false;
            ui.menus.put(1, hidden).unwrap();
            assert!(dispatch(&mut ui, "1:7:2").is_err());
            ui.menus.remove(1, 7).unwrap();
            assert!(dispatch(&mut ui, "1:7:3").is_err());
            assert!(ui.drain_events(1, None).is_empty());
        }

        #[test]
        fn quit_uses_host_close_path_and_unknown_id_is_rejected() {
            let mut ui = UiSystem::default();
            assert!(dispatch(&mut ui, QUIT).unwrap().quit);
            for bad in ["1:2:3:4", "1:2", "bar:1:2:3", "1:2:x"] {
                assert!(dispatch(&mut ui, bad).is_err());
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use super::*;
    /// muda's menu bar needs a GTK window; winit windows here are X11/Wayland
    /// surfaces. Keep the engine-drawn fallback visible; do not claim this is
    /// an OS-native presentation.
    pub struct NativeMenu;
    impl NativeMenu {
        pub fn new(_: &str, _: &Window) -> Result<Self> {
            log::warn!(
                "native menu bar unavailable on this platform (needs a GTK window); using engine-drawn menu bar"
            );
            Ok(Self)
        }
        pub fn sync(&mut self, ui: &mut UiSystem) -> Result<()> {
            ui.menus.set_native_bars(false);
            Ok(())
        }
        pub fn drain(&mut self, _: &mut UiSystem) -> Result<Selections> {
            Ok(Selections::default())
        }
    }
}

pub use imp::NativeMenu;
#[cfg(target_os = "windows")]
pub use imp::accelerator_hook;
