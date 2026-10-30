// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! OS menu presenters call the same shared activation method exercised here.
//! No window, operating-system event injection, game files or font is required.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

fn fixture(script: &str) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/startup.tjs"), script).unwrap();
    let project: crysirror_runtime::Project = serde_json::from_value(json!({
        "schema":2,"name":"native-menu","content_revision":"1",
        "canvas":[640,480],"assets":"assets",
        "clients":[{"name":"game","game":"synthetic","adapter":"krkr",
            "config":{"canvas":[640,480]}}]
    }))
    .unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let runtime = Runtime::with_registry(
        project,
        &dir.path().join("project.json"),
        &dir.path().join("state"),
        registry,
    )
    .unwrap();
    (dir, runtime)
}

#[test]
fn host_menu_selection_reaches_tjs_callback_without_canvas_pointer_input() {
    let (_dir, mut runtime) = fixture(
        r#"
        var w = new Window(); w.visible = true;
        var file = new MenuItem(w, "File"); w.menu.add(file);
        class Action extends MenuItem {
            function Action(w, name) { super.MenuItem(w, name); }
            function onClick() { Debug.message("host_callback", caption, checked); }
        }
        var first = new Action(w, "First"); first.radio = true;
        first.group = 1; first.checked = true; file.add(first);
        var second = new Action(w, "Second"); second.radio = true;
        second.group = 1; file.add(second);
    "#,
    );
    runtime.tick(0, InputFrame::default()).unwrap();
    runtime.host.engine.ui.menus.set_native_bars(true);
    let menu = runtime.host.engine.ui.menus.get(1, 1).unwrap();
    let second = menu.items[0].children[1].id;
    runtime
        .host
        .engine
        .ui
        .activate_menu_item(1, 1, second)
        .unwrap();
    runtime.tick(16, InputFrame::default()).unwrap();
    let report = runtime.inspect();
    let log = report["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("host_callback Second 1"), "{log}");
    let menu = runtime.host.engine.ui.menus.get(1, 1).unwrap();
    assert!(!menu.items[0].children[0].checked);
    assert!(menu.items[0].children[1].checked);
}

#[test]
fn unsupported_popup_ex_is_not_reported_as_a_cancelled_selection() {
    let (_dir, mut runtime) = fixture(
        r#"
        Plugins.link("windowEx.dll");
        var w = new Window();
        var menu = new MenuItem(w, "Popup");
        menu.popupEx(0, 10, 20, w, null, []);
        Debug.message("popup_was_falsely_acknowledged");
    "#,
    );
    let error = runtime.tick(0, InputFrame::default()).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("MenuItem.popupEx"), "{text}");
    assert!(
        text.contains("native synchronous popup continuation"),
        "{text}"
    );
    let log = runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(!log.contains("popup_was_falsely_acknowledged"), "{log}");
}
