// SPDX-License-Identifier: MPL-2.0
//! Smoke tests of the native runtime's module layer (no script, no host).
use crysirror_python::Vm;
use crysirror_renpy::rt::{
    boot,
    state::{Files, Profile},
};
use std::sync::Arc;

struct NoFiles;

impl Files for NoFiles {
    fn list(&self) -> Vec<String> {
        Vec::new()
    }
    fn read(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
    fn common_list(&self) -> Vec<String> {
        Vec::new()
    }
    fn common_read(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
}

fn vm(version: [u32; 4]) -> Vm {
    boot::new_vm(Profile::new(version), Arc::new(NoFiles))
}

fn run(vm: &mut Vm, src: &str) {
    if let Err(e) = crysirror_python::run_source(vm, src, crysirror_python::Dialect::Py3) {
        panic!("{e}");
    }
}

#[test]
fn stores_track_changes_and_revertables_journal_mutations() {
    let mut vm = vm([8, 3, 7, 0]);
    run(&mut vm, r#"
import renpy, renpy.game, renpy.python, renpy.rollback, renpy.revertable
assert renpy.version_tuple[:2] == (8, 3)
renpy.python.create_store("store")
import store
d = renpy.python.store_dicts["store"]
assert d["list"] is renpy.revertable.RevertableList
d.begin()
d["x"] = 1
changes, delta = d.get_changes(True, None)
assert changes["x"] is renpy.rollback.deleted, changes
assert "x" in d.ever_been_changed
class Log(object):
    def __init__(self):
        self.mutated = {}
renpy.game.log = Log()
l = renpy.revertable.RevertableList([1, 2])
assert renpy.game.log.mutated[id(l)] is None
renpy.game.log.mutated.clear()
l.append(3)
ref, clean = renpy.game.log.mutated[id(l)]
assert ref() is l and clean == [1, 2]
l._rollback(clean)
assert l == [1, 2]
s = renpy.revertable.RevertableSet([1])
assert isinstance(s | {2}, renpy.revertable.RevertableSet)
o = renpy.revertable.RevertableObject()
renpy.game.log.mutated.clear()
o.a = 5
assert renpy.game.log.mutated[id(o)][1] == {}
"#);
}

#[test]
fn minstore_names_follow_the_release() {
    let mut old = vm([6, 99, 12, 4]);
    run(&mut old, r#"
import renpy.minstore as m
assert not hasattr(m, "eval") and not hasattr(m, "absolute")
"#);
    let mut new = vm([8, 3, 7, 0]);
    run(&mut new, r#"
import renpy.minstore as m
assert m.eval is not None and m.position is not None and m.defaultdict is not None
"#);
}

/// Reports re-exported names whose source module is not ported yet.
#[test]
fn exports_report_unavailable_sources() {
    let mut vm = vm([8, 3, 7, 0]);
    run(&mut vm, r#"
import renpy.exports as e
missing = sorted(e._crysirror_unavailable)
# Variable-font axes need a font-variation query in the shared font service.
assert missing == ["variable_font_info (renpy.text.font)"], missing
"#);
}

#[test]
fn styles_inherit_and_expand_properties() {
    let mut vm = vm([8, 3, 7, 0]);
    run(&mut vm, r##"
import renpy.style as rs
style = rs.StyleManager()
style.default = rs.Style(None)
style.default.color = "#fff"
style.default.xminimum = None
style.button = rs.Style("default")
style.button.hover_color = "#f00"
style.button.area = (1, 2, 3, 4)
b = rs.get_style("button")
assert b.xpos == 1 and b.ymaximum == 4 and b.xfill is True, (b.xpos, b.ymaximum)
assert b.xminimum == 3
c = b.color
b.set_prefix("hover_")
assert b.color != c
t = rs.get_style("my_button")
assert t.parent == ("button",)
idx = b["sub"]
assert idx.name == ("button", "sub") and idx.xpos == 1
assert rs.get_style("default").xminimum == 0
assert b.get_placement()[0] == 1
try:
    b.not_a_property = 1
    raise AssertionError
except Exception as e:
    assert "not known" in str(e)
st = b.__getstate__()
assert st["name"] == ("button",)
"##);
}

struct MemFiles(Vec<(String, String)>);

impl Files for MemFiles {
    fn list(&self) -> Vec<String> {
        self.0.iter().map(|(n, _)| n.clone()).collect()
    }
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_bytes().to_vec())
    }
    fn common_list(&self) -> Vec<String> {
        Vec::new()
    }
    fn common_read(&self, _: &str) -> Option<Vec<u8>> {
        None
    }
}

#[test]
fn script_loads_with_creator_defined_statements() {
    let files = MemFiles(vec![
        ("a.rpy".into(), r#"
python early:
    def parse_greet(l):
        who = l.simple_expression()
        l.expect_eol()
        return who
    def exec_greet(o):
        store.greeted = o
    renpy.register_statement("greet", parse=parse_greet, execute=exec_greet)
"#.into()),
        ("b.rpy".into(), r#"
init 5 python:
    x = 1

label start:
    greet "eileen"
    return
"#.into()),
    ]);
    let mut vm = boot::new_vm(Profile::new([8, 3, 7, 0]), Arc::new(files));
    run(&mut vm, r#"
import renpy, renpy.game, renpy.script, renpy.parser
renpy.game.script = renpy.script.Script()
init = renpy.game.script.load_script()
assert not renpy.parser.parse_errors, renpy.parser.parse_errors
assert [p for p, _ in init] == [0, 5], init
n = renpy.game.script.lookup("start").next
assert type(n).__name__ == "UserStatement", n
assert n.parsed[0] == ("greet",) and n.parsed[1] == '"eileen"', n.parsed
"#);
}

#[test]
fn characters_parse_dialogue_tags_and_inherit_properties() {
    let mut vm = vm([8, 3, 7, 0]);
    run(&mut vm, r##"
import renpy, renpy.character as c, renpy.python
renpy.python.create_store("store")
d = c.DialogueTextTags("Hello{w=0.5} world{{x}{nw}")
assert d.text == "Hello{w=0.5} world{{x}{nw}", d.text
assert d.pause_start == [0, 5] and d.pause_end == [5, 26] and d.pause_delay == [0.5, 0], (d.pause_start, d.pause_end, d.pause_delay)
assert d.no_wait
d = c.DialogueTextTags("a{done}b{w}")
assert d.has_done and d.pause_end == [7]
adv = c.ADVCharacter(None, kind=False, who_prefix='', who_suffix='', what_prefix='', what_suffix='',
    show_function=c.show_display_say, predict_function=c.predict_show_display_say, condition=None,
    dynamic=False, image=None, interact=True, slow=True, slow_abortable=True, afm=True, ctc=None,
    ctc_pause=None, ctc_timedpause=None, ctc_position="nestled", all_at_once=False, with_none=None,
    callback=None, type='say', advance=True, who_style='say_label', what_style='say_dialogue',
    window_style='say_window', screen='say', mode='say', voice_tag=None)
import store
store.adv = adv
e = c.Character("Eileen", color="#c8ffc8", what_color="#fff", window_background=None)
assert e.name == "Eileen"
assert e.who_args["color"] == "#c8ffc8" and e.what_args["color"] == "#fff", (e.who_args, e.what_args)
assert e.window_args["background"] is None and e.what_args["style"] == "say_dialogue"
assert e.display_args["ctc_position"] == "nestled"
assert repr(e) == "<Character: 'Eileen'>"
"##);
}

#[test]
fn substitutions_follow_the_release() {
    let src = r#"
import renpy, renpy.python, renpy.substitutions as s, renpy.game, renpy.script
renpy.python.create_store("store")
renpy.game.script = renpy.script.Script()
import store
store.name = "eileen"
store.pts = [1, 2]
class O(object):
    pass
store.o = O()
store.o.v = 7
assert s.substitute("Hi [name]!")[0] == "Hi eileen!"
assert s.substitute("[[x] [pts[1]] [o.v:03d]")[0] == "[x] 2 007", s.substitute("[[x] [pts[1]] [o.v:03d]")
assert s.substitute("no subs") == ("no subs", False)
"#;
    let mut old = vm([6, 99, 12, 4]);
    run(&mut old, src);
    run(&mut old, r#"
import renpy.substitutions as s
assert s.substitute("[name!q]")[0] == "eileen"
"#);
    let mut new = vm([8, 3, 7, 0]);
    run(&mut new, src);
    run(&mut new, r#"
import renpy.substitutions as s
assert s.substitute("[name!u] [name!c]")[0] == "EILEEN Eileen"
"#);
}

#[test]
fn every_engine_module_imports() {
    for version in [[6, 99, 12, 4], [7, 4, 11, 0], [8, 3, 7, 0]] {
        let mut vm = vm(version);
        run(&mut vm, "import renpy\nrenpy.import_all()\n");
    }
}

/// Every engine function the start-up code (renpy.main, display.error)
/// calls exists once the engine is imported.
#[test]
fn startup_entry_points_exist() {
    let mut vm = vm([8, 3, 7, 0]);
    run(&mut vm, r#"
import renpy, importlib
renpy.import_all()
missing = []
for mod, name in [
    ("_crysirror_renpy", "savedir" ),
    ("_crysirror_renpy", "variants" ),
    ("builtins", "set" ),
    ("json", "load" ),
    ("renpy", "version" ),
    ("renpy.arguments", "Namespace" ),
    ("renpy.arguments", "commands" ),
    ("renpy.arguments", "display" ),
    ("renpy.arguments", "post_init" ),
    ("renpy.arguments", "pre_init" ),
    ("renpy.ast", "Node" ),
    ("renpy.atl", "compile_all" ),
    ("renpy.audio.audio", "fadeout_all" ),
    ("renpy.config", "init" ),
    ("renpy.config", "post_init" ),
    ("renpy.display", "draw" ),
    ("renpy.display", "interface" ),
    ("renpy.display.behavior", "init_keymap" ),
    ("renpy.display.core", "Interface" ),
    ("renpy.display.error", "error_handled" ),
    ("renpy.display.im", "ImageBase" ),
    ("renpy.display.im", "cache" ),
    ("renpy.display.image", "image_names" ),
    ("renpy.display.presplash", "sleep" ),
    ("renpy.display.render", "check_at_shutdown" ),
    ("renpy.display.screen", "analyze_screens" ),
    ("renpy.display.screen", "before_restart" ),
    ("renpy.display.screen", "prepare_screens" ),
    ("renpy.dump", "dump" ),
    ("renpy.execution", "Context" ),
    ("renpy.execution", "not_infinite_loop" ),
    ("renpy.execution", "run_context" ),
    ("renpy.exports", "block_rollback" ),
    ("renpy.exports", "load_module" ),
    ("renpy.exports", "loadable" ),
    ("renpy.exports", "log" ),
    ("renpy.exports", "open_file" ),
    ("renpy.exports", "quit" ),
    ("renpy.exports", "utter_restart" ),
    ("renpy.game", "CONTROL_EXCEPTIONS" ),
    ("renpy.game", "ParseErrorException" ),
    ("renpy.game", "QuitException" ),
    ("renpy.game", "RestartContext" ),
    ("renpy.game", "after_rollback" ),
    ("renpy.game", "args" ),
    ("renpy.game", "basepath" ),
    ("renpy.game", "build_info" ),
    ("renpy.game", "context" ),
    ("renpy.game", "contexts" ),
    ("renpy.game", "exception_info" ),
    ("renpy.game", "initcode_ast_id" ),
    ("renpy.game", "interface" ),
    ("renpy.game", "log" ),
    ("renpy.game", "persistent" ),
    ("renpy.game", "preferences" ),
    ("renpy.game", "script" ),
    ("renpy.game", "seen_session" ),
    ("renpy.game", "style" ),
    ("renpy.gl2.live2d", "reset" ),
    ("renpy.lint", "lint" ),
    ("renpy.loader", "auto_init" ),
    ("renpy.loader", "auto_quit" ),
    ("renpy.loader", "index_archives" ),
    ("renpy.loadsave", "autosave_not_running" ),
    ("renpy.loadsave", "init" ),
    ("renpy.loadsave", "location" ),
    ("renpy.log", "post_init" ),
    ("renpy.main", "choose_variants" ),
    ("renpy.main", "load_build_info" ),
    ("renpy.parser", "report_parse_errors" ),
    ("renpy.persistent", "init" ),
    ("renpy.persistent", "save_on_quit_MP" ),
    ("renpy.persistent", "should_save_persistent" ),
    ("renpy.persistent", "update" ),
    ("renpy.pyanalysis", "load_cache" ),
    ("renpy.pyanalysis", "save_cache" ),
    ("renpy.python", "RollbackLog" ),
    ("renpy.python", "clean_stores" ),
    ("renpy.python", "make_clean_stores" ),
    ("renpy.savelocation", "init" ),
    ("renpy.savelocation", "quit" ),
    ("renpy.savetoken", "init" ),
    ("renpy.savetoken", "upgrade_all_savefiles" ),
    ("renpy.script", "Script" ),
    ("renpy.scriptedit", "lines" ),
    ("renpy.sl2.slast", "load_cache" ),
    ("renpy.sl2.slast", "save_cache" ),
    ("renpy.sl2.slparser", "init" ),
    ("renpy.style", "StyleManager" ),
    ("renpy.style", "build_styles" ),
    ("renpy.test.testast", "_test" ),
    ("renpy.translation", "init_translation" ),
    ("renpy.translation", "write_updated_strings" ),
    ("renpy.warp", "warp" ),
    ("time", "ctime" ),
]:
    try:
        m = importlib.import_module(mod)
        if not hasattr(m, name):
            missing.append(mod + "." + name)
    except ImportError as e:
        missing.append(mod + " (" + str(e) + ")")
assert not missing, missing
"#);
}

#[test]
fn installation_versions_are_detected_per_release() {
    use crysirror_renpy::rt::state::Profile as P;
    let i618 = "    vc_version = 0\n\nversion_tuple = (6, 18, 3, vc_version)\n";
    assert_eq!(P::detect(i618, Some("vc_version = 761\n")), Some([6, 18, 3, 761]));
    assert_eq!(P::detect(i618, None), Some([6, 18, 3, 0]));
    let i699 = "version_tuple = (6, 99, 12, 4, vc_version)\n";
    assert_eq!(P::detect(i699, Some("vc_version = 2187\n")), Some([6, 99, 12, 4]));
    let i74 = "if PY2:\n    version_tuple = (7, 4, 11, vc_version)\nelse:\n    version_tuple = (8, 0, 0, vc_version)\n";
    assert_eq!(P::detect(i74, Some("vc_version = 2266\n")), Some([7, 4, 11, 2266]));
    let vc83 = "official = True\nnightly = False\nversion_name = 'Second Star'\nversion = '8.3.7.25031702'\n";
    assert_eq!(P::detect("version_tuple = VersionTuple(*(int(i) for i in version.split(\".\")))", Some(vc83)), Some([8, 3, 7, 25031702]));
    assert!(P::supported([6, 99, 12, 4]) && P::supported([8, 6, 1, 0]) && !P::supported([6, 17, 0, 0]));
}

#[test]
fn screen_size_hints_follow_gui_and_config() {
    use crysirror_renpy::rt::state::screen_size_hint as h;
    assert_eq!(h(["init python:\n    gui.init(1280, 720)\n"]), Some([1280, 720]));
    assert_eq!(h(["init -1 python hide:\n    config.screen_width = 1024\n    config.screen_height = 576\n"]), Some([1024, 576]));
    assert_eq!(h(["define config.screen_width = 1920\n", "define config.screen_height = 1080\n"]), Some([1920, 1080]));
    assert_eq!(h(["# gui.init(1, 1)\n    if config.screen_width == 800:\n"]), None);
}

#[test]
fn scry_objects_answer_unknown_fields_with_none() {
    let mut vm = vm([8, 3, 7, 0]);
    run(&mut vm, r#"
import renpy
renpy.import_all()
s = renpy.ast.Scry()
assert s.interacts is None and s.whatever is None
assert s.next() is None, s.next
"#);
}
