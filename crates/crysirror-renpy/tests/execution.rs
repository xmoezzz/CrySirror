// SPDX-License-Identifier: MPL-2.0
use crysirror_python::{Dialect, Value};
use crysirror_renpy::{
    engine::{Engine, Interaction, Profile, Progress},
    files::{GameFiles, ScopedFiles},
};
use std::rc::Rc;

struct Script(&'static str);
impl GameFiles for Script {
    fn list(&self) -> Vec<String> {
        vec!["test.rpy".into()]
    }
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        (name == "test.rpy").then(|| self.0.as_bytes().to_vec())
    }
    fn common_list(&self) -> Vec<String> {
        vec![]
    }
    fn common_read(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
}

fn load(files: Rc<dyn GameFiles>) -> Engine {
    let mut engine = Engine::new(
        files,
        Profile {
            version: (6, 99, 12),
            dialect: Dialect::Py2,
        },
    )
    .unwrap();
    engine.load_script().unwrap();
    engine
}

#[test]
fn init_dialogue_conditional_menu_and_call_return() {
    let mut engine = load(Rc::new(Script(
        r#"
define narrator_character = Character("Tester")
init python:
    show_second = True
label start:
    narrator_character "Ready"
    menu:
        "Hidden" if not show_second:
            $ result = -1
        "First":
            $ result = 0
        "Second" if show_second:
            call pick_second
    return
label pick_second:
    $ result = 2
    return
"#,
    )));
    engine.run_init().unwrap();
    engine.start("start").unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert_eq!(
        engine.interaction(),
        Some(Interaction::Say {
            speaker: Some("Tester".into()),
            text: "Ready".into(),
            screen: None,
        })
    );
    engine.resume(None).unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert_eq!(
        engine.interaction(),
        Some(Interaction::Menu {
            caption: None,
            options: vec![("First".into(), 1), ("Second".into(), 2)],
        })
    );
    assert!(engine.resume(Some(2)).is_err());
    engine.resume(Some(1)).unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Finished);
    assert!(matches!(
        engine.store_dict("store").dict_get_str_raw("result"),
        Some(Value::Int(2))
    ));
}

#[test]
fn atl_declaration_retains_program_without_running_animation() {
    let mut engine = load(Rc::new(Script(
        r#"
transform fade_notice:
    xalign .02
    on show:
        alpha 0
        linear .25 alpha 1
label start:
    "Ready"
"#,
    )));
    engine.run_init().unwrap();
    let transform = engine
        .store_dict("store")
        .dict_get_str_raw("fade_notice")
        .unwrap();
    assert_eq!(transform.obj().unwrap().class_name(), "ATLTransform");
    assert!(
        crysirror_python::ops::repr(&mut engine.vm, &transform)
            .unwrap()
            .contains("game/test.rpy")
    );
}

#[test]
fn startup_modules_run_before_regular_early_code_and_defaults() {
    struct Project;
    impl GameFiles for Project {
        fn list(&self) -> Vec<String> {
            vec!["script.rpy".into()]
        }
        fn read(&self, name: &str) -> Option<Vec<u8>> {
            (name == "script.rpy").then(|| {
                br#"
python early:
    defined_early = greeting
default message = defined_early
label start:
    "Ready"
    return
"#
                .to_vec()
            })
        }
        fn common_list(&self) -> Vec<String> {
            vec!["_errorhandling.rpym".into()]
        }
        fn common_read(&self, name: &str) -> Option<Vec<u8>> {
            (name == "_errorhandling.rpym").then(|| {
                br#"
init:
    call prepare
label prepare:
    $ greeting = "prepared"
    return
"#
                .to_vec()
            })
        }
    }
    let mut engine = load(Rc::new(Project));
    engine.run_init().unwrap();
    assert!(
        engine
            .store_dict("store")
            .dict_get_str_raw("message")
            .is_none()
    );
    engine.start("start").unwrap();
    let value = engine
        .store_dict("store")
        .dict_get_str_raw("message")
        .unwrap();
    assert_eq!(value.as_str(), Some("prepared"));
}

#[test]
fn hard_pause_only_resumes_on_the_clock_and_returns_false() {
    let mut engine = load(Rc::new(Script(
        r#"
label start:
    $ dismissed = renpy.pause(.25, hard=True)
    return
"#,
    )));
    engine.run_init().unwrap();
    engine.start("start").unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert_eq!(
        engine.interaction(),
        Some(Interaction::Pause {
            seconds: Some(0.25),
            hard: true
        })
    );
    assert!(engine.resume(None).is_err());
    engine.finish_pause_timeout().unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Finished);
    assert!(matches!(
        engine.store_dict("store").dict_get_str_raw("dismissed"),
        Some(Value::Bool(false))
    ));
}

#[test]
fn parameterized_label_restores_shadowed_store_values() {
    let mut engine = load(Rc::new(Script(
        r#"
init python:
    first = "outer"
    default_tail = "tail"
label start:
    call inner("inner", "extra", key="value")
    return
label inner(first, tail=default_tail, *extra, **named):
    $ result = (first, tail, extra, named)
    return
"#,
    )));
    engine.run_init().unwrap();
    engine.start("start").unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Finished);
    let first = engine
        .eval(&crysirror_renpy::lexer::PyExpr::new(
            "first",
            "test".into(),
            1,
        ))
        .unwrap();
    assert_eq!(first.as_str(), Some("outer"));
    let result = engine
        .eval(&crysirror_renpy::lexer::PyExpr::new(
            "result == ('inner', 'extra', (), {'key': 'value'})",
            "test".into(),
            1,
        ))
        .unwrap();
    assert!(matches!(result, Value::Bool(true)));
}

#[test]
fn creator_defined_statement_uses_original_parse_and_execute_callbacks() {
    struct Project;
    impl GameFiles for Project {
        fn list(&self) -> Vec<String> {
            vec!["register.rpy".into(), "script.rpy".into()]
        }
        fn read(&self, name: &str) -> Option<Vec<u8>> {
            let contents = match name {
                "register.rpy" => {
                    r#"
python early hide:
    from helper import half
    def parse_signal(lexer):
        expression = lexer.simple_expression()
        lexer.expect_eol()
        return expression
    def execute_signal(expression, half=half):
        renpy.pause(half(eval(expression)))
    renpy.register_statement("test signal", parse=parse_signal, execute=execute_signal)
"#
                }
                "script.rpy" => {
                    r#"
label start:
    test signal 5
    "Resumed"
    return
"#
                }
                "python-packages/helper.py" => "def half(value):\n    return value / 2\n",
                _ => return None,
            };
            Some(contents.as_bytes().to_vec())
        }
        fn common_list(&self) -> Vec<String> {
            vec![]
        }
        fn common_read(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
    }
    let mut engine = load(Rc::new(Project));
    engine.run_init().unwrap();
    engine.start("start").unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert_eq!(
        engine.interaction(),
        Some(Interaction::Pause {
            seconds: Some(2.0),
            hard: false
        })
    );
    engine.resume(None).unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert_eq!(
        engine.interaction(),
        Some(Interaction::Say {
            speaker: None,
            text: "Resumed".into(),
            screen: None,
        })
    );
}

#[test]
fn temporary_file_lock_is_scoped_and_released_on_close() {
    use crysirror_core::state::ClientId;
    let game = tempfile::tempdir().unwrap();
    std::fs::create_dir(game.path().join("game")).unwrap();
    let writable = tempfile::tempdir().unwrap();
    let mut vfs = crysirror_runtime::vfs::DiskVfs::new(game.path(), writable.path()).unwrap();
    let client = ClientId(1);
    vfs.register(client, "renpy", "test").unwrap();
    let files = ScopedFiles::new(vfs.storage(client).unwrap(), &[]).unwrap();
    let mut engine = Engine::new(
        Rc::new(files),
        Profile::from_version([6, 99, 12, 4]).unwrap(),
    )
    .unwrap();
    crysirror_python::run_source(
        &mut engine.vm,
        r#"
import fcntl
import tempfile
import logging
path = tempfile.gettempdir() + "/lease.lock"
log = logging.getLogger("lease.test")
log.addHandler(logging.StreamHandler())
log.warning("lease %s", "test")
first = open(path, "w")
first.flush()
fcntl.lockf(first, fcntl.LOCK_EX | fcntl.LOCK_NB)
second = open(path, "w")
try:
    fcntl.lockf(second, fcntl.LOCK_EX | fcntl.LOCK_NB)
except OSError:
    pass
else:
    raise AssertionError("an exclusive lease is already held")
first.close()
fcntl.lockf(second, fcntl.LOCK_EX | fcntl.LOCK_NB)
second.close()
try:
    blocked = open("/tmp/../outside", "w")
    blocked.flush()
except OSError:
    pass
else:
    raise AssertionError("temporary storage escaped its scope")
"#,
        Dialect::Py3,
    )
    .unwrap();
    assert!(
        writable
            .path()
            .join("renpy/test/cache/python-tmp-lease.lock")
            .is_file()
    );
    assert!(!game.path().join("outside").exists());
    assert!(
        engine
            .st
            .log
            .borrow()
            .iter()
            .any(|line| line == "lease test\n")
    );
}

#[test]
#[ignore = "requires an installed copy of DDLC via CRYSIRROR_RENPY_DDLC"]
fn ddlc_native_script_index() {
    let root = std::path::PathBuf::from(std::env::var_os("CRYSIRROR_RENPY_DDLC").unwrap());
    let (files, _writable) = installed_files(&root);
    let scripts = crysirror_renpy::script::discover(&files).unwrap();
    let mut engine = Engine::new(
        Rc::new(files),
        Profile::from_version([6, 99, 12, 4]).unwrap(),
    )
    .unwrap();
    let mut shells = crysirror_renpy::rpyc::Shells::new(&mut engine.vm).unwrap();
    let mut count = [0, 0];
    for script in scripts {
        if let crysirror_renpy::script::Source::Rpyc(bytes) = script.source {
            let (_, nodes) =
                crysirror_renpy::rpyc::unpickle_script(&mut engine.vm, &mut shells, &bytes)
                    .unwrap_or_else(|error| panic!("{}: {error}", script.name));
            assert!(!nodes.is_empty(), "{}", script.name);
            count[usize::from(script.common)] += 1;
        }
    }
    assert!(count[0] > 0 && count[1] > 0);
    eprintln!(
        "DDLC compiled files decoded: {} game, {} common",
        count[0], count[1]
    );
    if std::env::var_os("CRYSIRROR_RENPY_CHECK_INIT").is_some() {
        engine.load_script().unwrap();
        engine.run_init().unwrap();
    }
}

fn installed_files(root: &std::path::Path) -> (ScopedFiles, tempfile::TempDir) {
    use crysirror_core::{
        path::{Mount, VfsPath},
        state::ClientId,
    };
    use crysirror_resource::{ArchiveMount, DecodeContext, FormatRegistry};
    use std::sync::Arc;
    let writable = tempfile::tempdir().unwrap();
    let mut vfs = crysirror_runtime::vfs::DiskVfs::new(root, writable.path()).unwrap();
    let client = ClientId(1);
    vfs.register(client, "renpy", "test").unwrap();
    let storage = vfs.storage(client).unwrap();
    let mut formats = FormatRegistry::default();
    crysirror_formats_arc::register(&mut formats).unwrap();
    let mut mounts = Vec::new();
    for archive in ["scripts", "images", "fonts", "audio"] {
        let path = VfsPath::new(Mount::Asset, &format!("game/{archive}.rpa")).unwrap();
        let context = DecodeContext::new(client);
        let (_, archive_data) = formats
            .open(
                storage.open_asset(&path).unwrap(),
                Some("renpy/rpa"),
                &context,
            )
            .unwrap();
        let target = format!("renpy-archives/{archive}");
        vfs.mount_provider(
            client,
            &target,
            Arc::new(ArchiveMount::new(archive_data, context).unwrap()),
        )
        .unwrap();
        mounts.push(target);
    }
    (ScopedFiles::new(storage, &mounts).unwrap(), writable)
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_music_uses_shared_decoder_and_channel_stop() {
    use crysirror_core::service::{HostServices, InputFrame, buttons};
    use crysirror_core::state::ClientId;
    use crysirror_runtime::{AdapterRegistry, Project, Runtime};
    use serde_json::json;
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let game = root.path().join("game");
    std::fs::create_dir(&game).unwrap();
    std::fs::write(
        game.join("script.rpy"),
        r#"
init python:
    renpy.music.register_channel(7, "music", loop=True)
    renpy.music.alias_channel(7, "music")

label start:
    $ renpy.music.play("tone.wav", fadein=.02, fadeout=0)
    $ renpy.pause()
    $ renpy.music.stop(fadeout=0)
    $ renpy.pause()
    return
"#,
    )
    .unwrap();
    let frames = 24_000u32;
    let sample_bytes = frames * 2;
    let mut wave = b"RIFF".to_vec();
    wave.extend_from_slice(&(36 + sample_bytes).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16u32.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wave.extend_from_slice(&1u16.to_le_bytes()); // Mono
    wave.extend_from_slice(&48_000u32.to_le_bytes());
    wave.extend_from_slice(&96_000u32.to_le_bytes());
    wave.extend_from_slice(&2u16.to_le_bytes());
    wave.extend_from_slice(&16u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&sample_bytes.to_le_bytes());
    for frame in 0..frames {
        let sample = if frame % 64 < 32 { 5000i16 } else { -5000i16 };
        wave.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(game.join("tone.wav"), wave).unwrap();
    let project: Project = serde_json::from_value(json!({
        "schema":2, "name":"renpy-audio", "content_revision":"1", "canvas":[800,600],
        "assets":"game", "clients":[{"name":"game", "game":"fixture", "adapter":"renpy",
            "config":{"version":[6,99,12,4], "canvas":[800,600]}}]
    }))
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_renpy::adapter::register(&mut registry).unwrap();
    let mut runtime =
        Runtime::with_asset_root(project, root.path(), state.path(), registry).unwrap();
    runtime.set_audio_nonblocking(false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while runtime.poll_vm_preparations().unwrap() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    for _ in 0..12 {
        runtime.tick(16, InputFrame::default()).unwrap();
    }
    assert!(runtime.host.audio_playing(ClientId(1), 0).unwrap());
    let info = runtime.host.audio_info(ClientId(1), 0).unwrap().unwrap();
    assert!(info.position_frames > 0);
    assert!(runtime.inspect()["channel_audio"]["peak"].as_f64().unwrap() > 0.0);
    runtime
        .tick(
            16,
            InputFrame {
                buttons_pressed: buttons::ENTER,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!runtime.host.audio_playing(ClientId(1), 0).unwrap());
}

#[test]
fn rollback_preserves_shared_identity_and_excludes_unique_no_rollback_descendants() {
    let mut engine = load(Rc::new(Script(
        r#"
init python:
    class Untracked(NoRollback):
        pass
    cache = Untracked()
    cache.unique = []
    cache.shared = []
    shared = cache.shared

label start:
    "checkpoint"
    python:
        cache.unique.append("live")
        shared.append("revert")
        branch = 42
    menu:
        "Continue":
            "after choosing"
    return
"#,
    )));
    engine.run_init().unwrap();
    engine.start("start").unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    engine.resume(None).unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    engine.resume(Some(0)).unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert!(engine.rewind(2));
    assert_eq!(
        engine.interaction(),
        Some(Interaction::Say {
            speaker: None,
            text: "checkpoint".to_string(),
            screen: None,
        })
    );
    let value = engine.eval(&crysirror_renpy::lexer::PyExpr::new(
        "cache.unique == ['live'] and cache.shared is shared and shared == [] and 'branch' not in globals()",
        "test".into(), 1,
    )).unwrap();
    assert!(matches!(value, Value::Bool(true)));
    assert!(engine.roll_forward().unwrap());
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert!(engine.roll_forward().unwrap());
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert!(
        matches!(engine.interaction(), Some(Interaction::Say { text, .. }) if text == "after choosing")
    );
}

#[test]
fn persistent_protocol_2_reopens_in_a_fresh_engine() {
    use crysirror_core::state::ClientId;
    let root = tempfile::tempdir().unwrap();
    let writable = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("game")).unwrap();
    let client = ClientId(1);
    let mut vfs = crysirror_runtime::vfs::DiskVfs::new(root.path(), writable.path()).unwrap();
    vfs.register(client, "renpy", "test").unwrap();
    let open_engine = || {
        let files = ScopedFiles::new(vfs.storage(client).unwrap(), &[]).unwrap();
        Engine::new(
            Rc::new(files),
            Profile::from_version([6, 99, 12, 4]).unwrap(),
        )
        .unwrap()
    };
    let mut first = open_engine();
    crysirror_python::run_source(
        &mut first.vm,
        r#"
import renpy
import store
store.persistent.chapter = 17
store._preferences.text_cps = 42
renpy.save_persistent()
"#,
        Dialect::Py3,
    )
    .unwrap();
    let file = writable.path().join("renpy/test/save/persistent");
    let encoded = std::fs::read(file).unwrap();
    let mut raw = Vec::new();
    use std::io::Read;
    flate2::read::ZlibDecoder::new(&encoded[..])
        .read_to_end(&mut raw)
        .unwrap();
    assert!(raw.starts_with(&[0x80, 2]));
    assert!(raw.windows(16).any(|bytes| bytes == b"renpy.persistent"));
    let mut second = open_engine();
    let val = second.eval(&crysirror_renpy::lexer::PyExpr::new(
        "persistent.chapter == 17 and _preferences.text_cps == 42 and persistent._preferences is _preferences",
        "fixture".into(), 1,
    )).unwrap();
    assert!(matches!(val, Value::Bool(true)));
}

#[cfg(not(target_arch = "wasm32"))]
fn game_runtime(
    script: &str,
) -> (
    crysirror_runtime::Runtime,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    use crysirror_runtime::{AdapterRegistry, Project, Runtime};
    use serde_json::json;
    let root = tempfile::tempdir().unwrap();
    let writable = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("game")).unwrap();
    std::fs::write(root.path().join("game/script.rpy"), script).unwrap();
    let project: Project = serde_json::from_value(json!({
        "schema":2, "name":"renpy-route", "content_revision":"1", "canvas":[800,600],
        "assets":"game", "clients":[{"name":"game", "game":"fixture", "adapter":"renpy",
            "config":{"version":[6,99,12,4],"canvas":[800,600]}}]
    }))
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_renpy::adapter::register(&mut registry).unwrap();
    let mut runtime =
        Runtime::with_asset_root(project, root.path(), writable.path(), registry).unwrap();
    runtime.set_audio_nonblocking(false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while runtime.poll_vm_preparations().unwrap() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    (runtime, root, writable)
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn screen_parameters_render_shared_widgets_and_activate_original_callback() {
    use crysirror_core::service::{InputFrame, buttons};
    let (mut runtime, _root, _writable) = game_runtime(
        r#"
screen action_box(title):
    vbox:
        xsize 300
        text title
        transclude
screen actions(caption):
    modal True
    use action_box(caption):
        for i in range(2):
            textbutton "Pick " + str(i) action Return(i + 100)
label start:
    $ code = renpy.call_screen("actions", "Original arguments")
    $ renpy.say(None, "Selected " + str(code))
    return
"#,
    );
    runtime.tick(16, InputFrame::default()).unwrap();
    let inspect = runtime.inspect();
    assert_eq!(inspect["sessions"][0]["native"]["interaction"], "screen");
    let view = &inspect["engine"]["ui"]["clients"]["1"];
    assert!(
        view.as_object()
            .unwrap()
            .values()
            .any(|widget| widget["spec"]["text"] == "Original arguments")
    );
    let pointer = InputFrame {
        pointer_x: 20,
        pointer_y: 90,
        ..Default::default()
    };
    runtime
        .tick(
            16,
            InputFrame {
                buttons_down: buttons::MOUSE_LEFT,
                buttons_pressed: buttons::MOUSE_LEFT,
                ..pointer
            },
        )
        .unwrap();
    runtime.tick(16, pointer).unwrap();
    runtime.tick(16, InputFrame::default()).unwrap();
    let inspect = runtime.inspect();
    assert_eq!(inspect["sessions"][0]["native"]["interaction"], "say");
    let layers = &inspect["engine"]["layers"]["clients"]["1"];
    assert!(
        layers
            .as_object()
            .unwrap()
            .values()
            .any(|layer| { layer["paint"]["text"] == "Selected 101" })
    );
    assert!(
        inspect["engine"]["ui"]["clients"]["1"]
            .as_object()
            .is_none_or(|widgets| widgets.is_empty())
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn dissolve_waits_on_the_shared_clock_then_releases_script() {
    use crysirror_core::service::InputFrame;
    let (mut runtime, _root, _writable) = game_runtime(
        r##"
image start_plate = Solid("#f00")
image end_plate = Solid("#00f")
label start:
    scene start_plate
    with None
    scene end_plate
    with Dissolve(.25)
    "Ready"
    return
"##,
    );
    runtime.tick(10, InputFrame::default()).unwrap();
    assert_eq!(
        runtime.inspect()["engine"]["layers"]["clients"]["1"]["1"]["paint"]["color"],
        serde_json::json!([255, 0, 0, 255])
    );
    runtime.tick(10, InputFrame::default()).unwrap();
    assert_eq!(
        runtime.inspect()["engine"]["layers"]["clients"]["1"]["1"]["paint"]["color"],
        serde_json::json!([0, 0, 255, 255])
    );
    assert_eq!(
        runtime.inspect()["sessions"][0]["native"]["interaction"],
        "transition"
    );
    for _ in 0..20 {
        runtime.tick(10, InputFrame::default()).unwrap();
    }
    assert_eq!(
        runtime.inspect()["sessions"][0]["native"]["interaction"],
        "transition"
    );
    for _ in 0..6 {
        runtime.tick(10, InputFrame::default()).unwrap();
    }
    assert_eq!(
        runtime.inspect()["sessions"][0]["native"]["interaction"],
        "say"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn atl_uses_integer_placement_and_runs_while_waiting_for_input() {
    use crysirror_core::service::InputFrame;
    use serde_json::json;
    let (mut runtime, _root, _writable) = game_runtime(
        r##"
transform drift(destination, delay=0.2):
    xpos 10 alpha 0.0
    linear delay xpos destination alpha 1.0
image moving = Solid("#c11")
label start:
    show moving at drift(110)
    "Animation stays live during dialogue"
    return
"##,
    );
    let tick = |runtime: &mut crysirror_runtime::runtime::Runtime, dt| {
        runtime.tick(dt, InputFrame::default()).unwrap();
        let state = runtime.inspect();
        let plate = state["engine"]["layers"]["clients"]["1"]
            .as_object()
            .unwrap()
            .values()
            .find(|layer| layer["paint"]["color"] == json!([204, 17, 17, 255]))
            .unwrap();
        (
            plate["rect"][0].as_f64().unwrap(),
            plate["opacity"].as_u64().unwrap(),
        )
    };
    assert_eq!(tick(&mut runtime, 16), (10., 0));
    assert_eq!(tick(&mut runtime, 100), (60., 127));
    assert_eq!(tick(&mut runtime, 100), (110., 255));
    assert_eq!(tick(&mut runtime, 100), (110., 255));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn authored_say_screen_replaces_fallback_dialogue_panel() {
    use crysirror_core::service::InputFrame;
    let (mut runtime, _root, _writable) = game_runtime(
        r##"
define fixture_speaker = Character("Speaker")
style default:
    size 34
style spoken:
    color "#4bb"
screen say(who, what):
    vbox:
        text who
        text "Authored: " + what style "spoken"
label start:
    fixture_speaker "line"
    return
"##,
    );
    runtime.tick(16, InputFrame::default()).unwrap();
    let state = runtime.inspect();
    assert_eq!(state["sessions"][0]["native"]["interaction"], "say");
    let widgets = state["engine"]["ui"]["clients"]["1"].as_object().unwrap();
    assert!(
        widgets
            .values()
            .any(|widget| widget["spec"]["text"] == "Speaker")
    );
    assert!(
        widgets
            .values()
            .any(|widget| widget["spec"]["text"] == "Authored: line"
                && widget["spec"]["text_size"] == 34
                && widget["spec"]["text_color"] == serde_json::json!([68, 187, 187, 255]))
    );
    assert!(
        state["engine"]["layers"]["clients"]["1"]
            .as_object()
            .is_none_or(|layers| layers.is_empty())
    );
}

#[test]
fn nested_label_resumes_python_with_child_result_and_restores_dynamic_state() {
    let fixture = Script(
        r#"
default state = "parent"
label start:
    $ parent_context = renpy.context()
    $ result = renpy.call_in_new_context("child", 42, suffix="!")
    $ assert renpy.context() is parent_context
    $ assert result == "42!"
    $ renpy.say(None, state)
    return
label child(number, suffix):
    $ assert renpy.context() is not parent_context
    $ renpy.dynamic("state")
    $ state = "child"
    $ ui.interact(pause=0.01)
    return str(number) + suffix
"#,
    );
    let profile = Profile::from_version([6, 99, 12, 4]).unwrap();
    let mut engine = Engine::new(Rc::new(fixture), profile).unwrap();
    engine.load_script().unwrap();
    engine.run_init().unwrap();
    engine.start("start").unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert!(matches!(
        engine.interaction(),
        Some(Interaction::Screen {
            timeout_ms: Some(10),
            ..
        })
    ));
    assert_eq!(engine.st.contexts.borrow().len(), 2);
    engine.finish_pause_timeout().unwrap();
    assert_eq!(engine.tick(100).unwrap(), Progress::Waiting);
    assert_eq!(engine.st.contexts.borrow().len(), 1);
    assert!(
        matches!(engine.interaction(), Some(Interaction::Say { text, .. }) if text == "parent")
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn screen_action_jumps_from_child_context_to_parent_label() {
    use crysirror_core::service::{InputFrame, buttons};
    let (mut runtime, _root, _writable) = game_runtime(
        r#"
default mode = "story"
screen child_menu:
    textbutton "Start" action renpy.partial(renpy.jump_out_of_context, "chapter")
label start:
    $ renpy.call_in_new_context("menu_context")
    "Should only appear after normal return"
    return
label menu_context:
    $ renpy.context_dynamic("mode")
    $ mode = "menu"
    $ renpy.call_screen("child_menu")
    return
label chapter:
    $ assert renpy.context().rollback
    $ renpy.say(None, "Started in " + mode)
    return
"#,
    );
    runtime.tick(16, InputFrame::default()).unwrap();
    assert_eq!(
        runtime.inspect()["sessions"][0]["native"]["interaction"],
        "screen"
    );
    let pointer = InputFrame {
        pointer_x: 20,
        pointer_y: 10,
        ..Default::default()
    };
    runtime
        .tick(
            16,
            InputFrame {
                buttons_down: buttons::MOUSE_LEFT,
                buttons_pressed: buttons::MOUSE_LEFT,
                ..pointer
            },
        )
        .unwrap();
    runtime.tick(16, pointer).unwrap();
    runtime.tick(16, InputFrame::default()).unwrap();
    let state = runtime.inspect();
    assert_eq!(state["sessions"][0]["native"]["interaction"], "say");
    assert!(
        state["engine"]["layers"]["clients"]["1"]
            .as_object()
            .unwrap()
            .values()
            .any(|layer| layer["paint"]["text"] == "Started in story")
    );
}
