//! WIN32Dialog.messageBox on the engine's modal dialog (no game data): the
//! caller suspends until a button is chosen through engine input and gets
//! the Win32 result id.
use crysirror_core::service::{InputFrame, buttons};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

fn fixture(source: &str) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/startup.tjs"), source).unwrap();
    let project = serde_json::from_value(json!({"schema":2,"name":"dialog","content_revision":"1","canvas":[640,480],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-dialog","config":{"canvas":[640,480]}}]})).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let runtime = Runtime::with_registry(project, &path, &dir.path().join("state"), registry).unwrap();
    (dir, runtime)
}
fn log(r: &Runtime) -> String {
    r.inspect()["sessions"][0]["native"]["log_tail"].to_string()
}
fn press(r: &mut Runtime, key: u64) {
    r.tick(16, InputFrame { buttons_pressed: key, buttons_down: key, ..Default::default() }).unwrap();
    r.tick(16, InputFrame::default()).unwrap();
}

#[test]
fn system_confirm_waits_inside_nested_startup_and_resumes_with_the_answer() {
    let (dir, mut r) = fixture(r#"
Scripts.execStorage("first-run.tjs");
Debug.message("startup finished");
"#);
    std::fs::write(dir.path().join("assets/first-run.tjs"), r#"
var declined = System.confirm("First question", "Startup");
var accepted = System.confirm("Second question");
if (declined !== 0 || accepted !== 1) throw "wrong confirmation result";
Debug.message("answers verified");
"#).unwrap();
    r.tick(0, InputFrame::default()).unwrap();
    let dialog = r.host.engine.ui.dialogs.get(1).expect("startup dialog");
    assert_eq!(dialog.title, "Startup");
    assert_eq!(dialog.message, "First question");
    assert_eq!(dialog.default, 0);
    assert!(!r.presentation_pending());
    assert!(!log(&r).contains("startup finished"));
    press(&mut r, buttons::ESCAPE);
    assert_eq!(r.host.engine.ui.dialogs.get(1).unwrap().message, "Second question");
    press(&mut r, buttons::LEFT);
    press(&mut r, buttons::ENTER);
    assert!(r.host.engine.ui.dialogs.is_empty());
    assert!(log(&r).contains("answers verified"), "{}", log(&r));
    assert!(log(&r).contains("startup finished"), "{}", log(&r));
}

#[test]
fn edit_template_dispatches_script_overrides_and_returns_text_only_on_ok() {
    let (_dir, mut r) = fixture(r#"
        Plugins.link("win32dialog.dll");
        class Edit extends WIN32Dialog {
            var answer, clicks=0;
            function Edit() {
                super.WIN32Dialog(null);
                var h=new WIN32Dialog.Header(); h.store(%["title"=>"test"]);
                var e=new WIN32Dialog.Items(); e.store(%["windowClass"=>129,"id"=>7]);
                var a=new WIN32Dialog.Items(); a.store(%["windowClass"=>128,"id"=>1,"title"=>"OK"]);
                var c=new WIN32Dialog.Items(); c.store(%["windowClass"=>128,"id"=>2,"title"=>"Cancel"]);
                makeTemplate(h,e,a,c);
            }
            function onInit(m,w,l) {
                setItemText(7,"old");
                sendItemMessage(7,0xb1,0,-1);
            }
            function onCommand(m,w,l) {
                clicks++;
                if (w==1) answer=getItemText(7);
                close(w);
            }
            function open() { var id=super.open();return id==1 ? answer : void; }
        }
        var d=new Edit();
        var accepted=d.open(), cancelled=d.open();
        if (accepted!=="备注" || cancelled!==void || d.clicks!=2) throw "dialog result";
        Debug.message("edit-template-verified");
    "#);
    r.tick(0, InputFrame::default()).unwrap();
    r.tick(16, InputFrame::default()).unwrap(); // publish onInit's initial text
    let field = r.host.engine.ui.focus.expect("editor focused");
    assert_eq!(r.host.engine.ui.get(field.owner, field.id).unwrap().text, "old");
    let input = serde_json::from_value(json!({"ui":[{"op":"insert_text","text":"备注"}]})).unwrap();
    r.host.engine.submit(crysirror_core::state::ClientId(field.owner), &input).unwrap();
    // 640x480 canvas: the dialog's OK button is in the lower right of the panel.
    let x = 552;
    let y = 304;
    r.tick(16, InputFrame { pointer_x:x,pointer_y:y,buttons_down:buttons::MOUSE_LEFT,buttons_pressed:buttons::MOUSE_LEFT,..Default::default() }).unwrap();
    r.tick(16, InputFrame {pointer_x:x,pointer_y:y,..Default::default()}).unwrap();
    for _ in 0..3 { r.tick(16,InputFrame::default()).unwrap(); }
    press(&mut r,buttons::ESCAPE);
    assert!(log(&r).contains("edit-template-verified"), "{}", log(&r));
}

#[test]
fn message_box_suspends_until_an_engine_dialog_button_is_chosen() {
    let (_dir, mut r) = fixture(
        r#"
Plugins.link("win32dialog.dll");
var a = WIN32Dialog.messageBox(null, "終了しますか？", "確認", WIN32Dialog.MB_YESNO | WIN32Dialog.MB_ICONQUESTION);
Debug.message("first " + a);
var b = WIN32Dialog.messageBox(null, "続けますか？", "", WIN32Dialog.MB_OKCANCEL);
Debug.message("second " + b);
"#,
    );
    r.tick(0, InputFrame::default()).unwrap();
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
    }
    let dialog = r.host.engine.ui.dialogs.get(1).expect("dialog open").clone();
    assert!(!r.presentation_pending(), "the owner must be able to see and answer the modal");
    assert_eq!(dialog.title, "確認");
    assert_eq!(dialog.buttons.iter().map(|b| b.id).collect::<Vec<_>>(), [6, 7]);
    assert!(!log(&r).contains("first"));
    // YES/NO has no Cancel: Escape is ignored, as in Win32.
    press(&mut r, buttons::ESCAPE);
    assert!(r.host.engine.ui.dialogs.get(1).is_some());
    press(&mut r, buttons::RIGHT);
    press(&mut r, buttons::ENTER);
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
    }
    assert!(log(&r).contains("first 7"), "{}", log(&r));
    // OK/CANCEL: Escape answers IDCANCEL.
    assert_eq!(r.host.engine.ui.dialogs.get(1).expect("second dialog").buttons.len(), 2);
    press(&mut r, buttons::ESCAPE);
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
    }
    assert!(log(&r).contains("second 2"), "{}", log(&r));
    assert!(r.host.engine.ui.dialogs.is_empty());
}

#[test]
fn user_close_runs_on_close_query_and_closes_only_when_accepted() {
    let (_dir, mut r) = fixture(
        r#"
Plugins.link("win32dialog.dll");
class MainWin extends Window {
    var asked = 0;
    function MainWin() { super.Window(); }
    function onCloseQuery(canclose) {
        asked++;
        var a = WIN32Dialog.messageBox(this, "終了しますか？", "確認", WIN32Dialog.MB_YESNO);
        Debug.message("answer " + a);
        super.onCloseQuery(a == WIN32Dialog.IDYES);
    }
}
var w = new MainWin();
// A program close whose handler refuses does not close.
class Refuse extends Window {
    function Refuse() { super.Window(); }
    function onCloseQuery(canclose) { super.onCloseQuery(false); }
}
Debug.message("ready");
"#,
    );
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
    }
    assert!(log(&r).contains("ready"), "{}", log(&r));
    assert!(r.request_close(), "the krkr game handles the close request");
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
    }
    assert!(r.host.engine.ui.dialogs.get(1).is_some(), "close confirmation open");
    press(&mut r, buttons::RIGHT); // いいえ
    press(&mut r, buttons::ENTER);
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
    }
    assert!(log(&r).contains("answer 7"), "{}", log(&r));
    assert!(!r.vm_sessions_halted(), "refused close keeps the game running");
    assert!(r.request_close());
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
    }
    press(&mut r, buttons::ENTER); // はい (default)
    for _ in 0..4 {
        r.tick(16, InputFrame::default()).unwrap();
    }
    assert!(log(&r).contains("answer 6"), "{}", log(&r));
    assert!(r.vm_sessions_halted(), "accepted close halts the game");
}

#[test]
fn program_close_asks_on_close_query_synchronously() {
    let (_dir, mut r) = fixture(
        r#"
class Refuse extends Window {
    function Refuse() { super.Window(); }
    function onCloseQuery(canclose) { Debug.message("query " + canclose); super.onCloseQuery(false); }
}
var w = new Refuse();
w.close();
Debug.message("still running");
"#,
    );
    for _ in 0..3 {
        r.tick(16, InputFrame::default()).unwrap();
    }
    assert!(log(&r).contains("query 1") && log(&r).contains("still running"), "{}", log(&r));
    assert!(!r.vm_sessions_halted());
}
