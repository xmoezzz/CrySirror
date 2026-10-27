// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Original-style TJS2 scripts -> public engine services. Self-authored
//! scripts and generated images only; no game data, fonts or GPU.
use crysirror_core::{
    service::{HostServices, InputFrame, buttons},
    state::ClientId,
};
use crysirror_runtime::{AdapterRegistry, Runtime};
use crysirror_scene::layer::Paint;
use serde_json::json;

fn png(path: &std::path::Path, w: u32, h: u32, rgba: [u8; 4]) {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    img.save(path).unwrap();
}

fn fixture(startup: &str, extra: &[(&str, &str)]) -> (tempfile::TempDir, Runtime) {
    let t = tempfile::tempdir().unwrap();
    let game = t.path().join("assets/game");
    std::fs::create_dir_all(game.join("system")).unwrap();
    std::fs::create_dir_all(game.join("image")).unwrap();
    std::fs::write(game.join("startup.tjs"), startup).unwrap();
    for (name, body) in extra {
        std::fs::write(game.join(name), body).unwrap();
    }
    png(&game.join("image/Chara.png"), 40, 20, [200, 30, 30, 255]);
    let p: crysirror_runtime::Project = serde_json::from_value(json!({
        "schema": 2, "name": "krkr bridge", "content_revision": "1", "canvas": [640, 480],
        "assets": "assets",
        "clients": [{"name": "native", "adapter": "krkr", "game": "synthetic",
            "config": {"root": "game", "canvas": [640, 480]}}]
    }))
    .unwrap();
    std::fs::write(
        t.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let r = Runtime::with_registry(
        p,
        &t.path().join("project.json"),
        &t.path().join("state"),
        registry,
    )
    .unwrap();
    (t, r)
}

const SCENE: &str = r#"
Debug.message("boot", System.getTickCount());
Storages.addAutoPath("image/");
Scripts.execStorage("system/lib.tjs");
var win = new Window();
win.setInnerSize(640, 480);
var base = new Layer(win, null);
base.fillRect(0, 0, base.imageWidth, base.imageHeight, 0xff203040);
class Sprite extends Layer {
    var clicks = 0;
    function Sprite(w, p) { super.Layer(w, p); }
    function onClick(x, y) { clicks++; Debug.message("click", x, y, clicks); }
}
var chara = new Sprite(win, base);
chara.loadImages("chara");
chara.setPos(100, 50);
chara.visible = true;
chara.opacity = 200;
var panel = new Layer(win, base);
panel.setImageSize(120, 60);
panel.setSizeToImageSize();
panel.colorRect(0, 0, 120, 60, 0x00ff00, 128);
panel.setPos(300, 300);
panel.visible = true;
global.ticks = 0;
var t = new Timer(function() { ticks++; chara.left += 10; if (ticks == 3) t.enabled = false; }, "");
t.interval = 100;
t.enabled = true;
win.visible = true;
Debug.message("ready", helper(2));
"#;

#[test]
fn tjs_layers_timers_and_clicks_drive_engine_owned_nodes() {
    let (_t, mut r) = fixture(
        SCENE,
        &[("system/lib.tjs", "function helper(x) { return x * 21; }")],
    );
    r.tick(0, InputFrame::default()).unwrap();
    let diag = r.inspect()["sessions"][0]["native"].clone();
    let log = diag["log_tail"].to_string();
    assert!(log.contains("ready 42"), "{diag}");
    assert_eq!(diag["layers"], 3);

    let client = ClientId(1);
    let layers = &r.host.engine.layers;
    // Primary container covers the window; chara and panel are its children.
    let primary = layers.get(client, 2).unwrap();
    assert_eq!(primary.rect, [0., 0., 640., 480.]);
    assert!(primary.visible);
    let chara = layers.get(client, 4).unwrap();
    assert_eq!(chara.parent, Some(2));
    assert_eq!(chara.rect, [100., 50., 40., 20.]);
    assert_eq!(chara.opacity, 200);
    assert!(chara.order[0] > primary.order[0]);
    // The image lives in an engine canvas surface, not a VM framebuffer.
    let surface = r.host.engine.canvases.surface(client, 2).unwrap();
    assert_eq!([surface.width, surface.height], [40, 20]);
    let json = serde_json::to_string(&surface.nodes).unwrap();
    assert!(json.contains("image/Chara.png"), "{json}");

    // Timer fires on the engine clock and is disabled by its own handler.
    for ms in [100, 200, 300, 400, 500] {
        r.tick(ms, InputFrame::default()).unwrap();
    }
    assert_eq!(r.host.engine.layers.get(client, 4).unwrap().rect[0], 130.);

    // A click inside the sprite reaches the script subclass with local coordinates.
    let at = |down| InputFrame {
        pointer_x: 135,
        pointer_y: 55,
        buttons_down: if down { buttons::MOUSE_LEFT } else { 0 },
        buttons_pressed: if down { buttons::MOUSE_LEFT } else { 0 },
        ..Default::default()
    };
    r.tick(600, at(true)).unwrap();
    r.tick(616, at(false)).unwrap();
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("click 5 5 1"), "{log}");

    // Engine-owned state is editable without the VM.
    let fill = r.host.engine.layers.frame([640, 480]).unwrap();
    assert!(
        fill.iter()
            .any(|p| matches!(p.paint, Paint::Solid { color } if color == [0, 255, 0, 128]))
    );
}

fn failure(r: &mut Runtime) -> String {
    assert!(r.tick(0, InputFrame::default()).is_err());
    r.inspect()["sessions"][0]["native"]["error"].to_string()
}

#[test]
fn unsupported_natives_fail_visibly() {
    let (_t, mut r) = fixture(
        r#"var win = new Window(); var base = new Layer(win, null);
           var l = new Layer(win, base); l.loadImages("image/chara");
           l.fillRect(1, 1, 4, 4, 0x80ffffff);"#,
        &[],
    );
    let e = failure(&mut r);
    assert!(e.contains("partial translucent Layer.fillRect"), "{e}");

    let (_t, mut r) = fixture(r#"Plugins.link("extrans.dll");"#, &[]);
    let e = failure(&mut r);
    assert!(e.contains("native DLLs are never loaded"), "{e}");

    let (_t, mut r) = fixture(r#"var w = new Window(); w.setInnerSize(800, 600);"#, &[]);
    let e = failure(&mut r);
    assert!(e.contains("project canvas"), "{e}");
}

#[test]
fn shift_jis_scripts_and_case_insensitive_storage() {
    let (sjis, _, _) = encoding_rs::SHIFT_JIS
        .encode("Debug.message(\"吉里吉里\", Storages.isExistentStorage(\"IMAGE/chara.PNG\"));");
    let t = tempfile::tempdir().unwrap();
    let game = t.path().join("assets/game");
    std::fs::create_dir_all(game.join("image")).unwrap();
    std::fs::write(game.join("startup.tjs"), &*sjis).unwrap();
    png(&game.join("image/Chara.png"), 2, 2, [0, 0, 0, 255]);
    let p: crysirror_runtime::Project = serde_json::from_value(json!({
        "schema": 2, "name": "krkr sjis", "content_revision": "1", "canvas": [640, 480], "assets": "assets",
        "clients": [{"name": "native", "adapter": "krkr", "game": "s", "config": {"root": "game", "canvas": [640, 480]}}]
    }))
    .unwrap();
    std::fs::write(
        t.path().join("project.json"),
        serde_json::to_vec(&p).unwrap(),
    )
    .unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut r = Runtime::with_registry(
        p,
        &t.path().join("project.json"),
        &t.path().join("state"),
        registry,
    )
    .unwrap();
    r.tick(0, InputFrame::default()).unwrap();
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("吉里吉里 1"), "{log}");
}

#[test]
fn nominal_font_height_uses_absolute_height_and_rejects_unmapped_styles() {
    let (_t, mut r) = fixture(
        r#"
        var w = new Window(); var l = new Layer(w, null);
        l.font.height = -37;
        Debug.message("heights", l.font.getTextHeight("A"), l.font.getTextHeight(""));
    "#,
        &[],
    );
    r.tick(0, InputFrame::default()).unwrap();
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("heights 37 37"), "{log}");
    let (_t, mut r) = fixture(
        r#"
        var w = new Window(); var l = new Layer(w, null);
        l.font.italic = true; l.drawText(0, 0, "A", 0xffffff);
    "#,
        &[],
    );
    assert!(failure(&mut r).contains("Font.italic"));
}

#[test]
#[ignore = "requires licensed runtime font in CRYSIRROR_TEST_FONT; no font bundled"]
fn font_width_and_canvas_text_share_engine_font_resources() {
    let (_t, mut r) = fixture(
        r#"
        var w = new Window(); var l = new Layer(w, null);
        l.font.height = 24;
        Debug.message("width", l.font.getTextWidth("AVATAR"), l.font.getTextWidth(""));
        l.drawText(5, 10, "AVATAR", 0xffffff);
    "#,
        &[],
    );
    let bytes = std::fs::read(std::env::var_os("CRYSIRROR_TEST_FONT").unwrap()).unwrap();
    r.host.text.add_font("default", bytes.clone()).unwrap();
    r.assets.text.add_font("default", bytes).unwrap();
    let expected = r
        .assets
        .text
        .measure_simple("default", 24., "AVATAR")
        .unwrap()
        .advance
        .ceil() as i64;
    r.tick(0, InputFrame::default()).unwrap();
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains(&format!("width {expected} 0")), "{log}");
    let surface = r.host.engine.canvases.surface(ClientId(1), 1).unwrap();
    let nodes = serde_json::to_string(&surface.nodes).unwrap();
    assert!(nodes.contains("AVATAR"));
    assert!(nodes.contains("simple-horizontal-unwrapped-v1"));
    assert!(r.host.text_measurement(ClientId(999)).is_err());
}

#[test]
fn menu_plugin_routes_native_objects_to_shared_ui_and_dispatches_clicks() {
    let (_t, mut r) = fixture(
        r#"
        Plugins.link("plugin/MENU.dll"); Plugins.link("menu.dll");
        Debug.message("plugin",Plugins.getStatus("menu.dll").kind,Plugins.getList().count);
        var w=new Window();w.visible=true;
        var root=new MenuItem(w,"File(&F)");w.menu.add(root);
        class Choice extends MenuItem {
            function Choice(w,c) {super.MenuItem(w,c);}
            function onClick(){Debug.message("native_menu_click",caption,checked);}
        }
        var fast=new Choice(w,"Fast");fast.radio=true;fast.group=1;fast.checked=true;root.add(fast);
        var slow=new Choice(w,"Slow");slow.radio=true;slow.group=1;root.add(slow);
        var removed=new MenuItem(w,"Removed");root.add(removed);invalidate removed;
        Debug.message("children",root.children.count);
        Debug.message("unload",Plugins.unlink("menu.dll"));
    "#,
        &[],
    );
    r.tick(0, InputFrame::default()).unwrap();
    let spec = r.host.engine.ui.menus.get(1, 1).unwrap();
    assert_eq!(spec.items[0].label, "File");
    assert!(spec.items[0].children[0].checked);
    assert!(!spec.items[0].children[1].checked);
    let click = |x, y| InputFrame {
        pointer_x: x,
        pointer_y: y,
        buttons_pressed: buttons::MOUSE_LEFT,
        ..Default::default()
    };
    r.tick(16, click(20, 10)).unwrap();
    r.tick(32, InputFrame::default()).unwrap();
    r.tick(48, click(20, 60)).unwrap();
    r.tick(64, InputFrame::default()).unwrap();
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("plugin engine_facade 1"), "{log}");
    assert!(log.contains("children 2"), "{log}");
    assert!(log.contains("native_menu_click Slow 1"), "{log}");
    assert!(log.contains("unload 0"), "{log}");
    let spec = r.host.engine.ui.menus.get(1, 1).unwrap();
    assert!(!spec.items[0].children[0].checked);
    assert!(spec.items[0].children[1].checked);
}

#[test]
fn configured_plugin_shell_only_registers_and_never_exports_a_fake_capability() {
    let (t, _) = fixture(
        r#"
        Debug.message("available",CanLoadPlugin("optional.dll"),CanLoadPlugin("unknown.dll"));
        Plugins.link("optional.dll");
        Debug.message("shell",Plugins.getStatus("optional.dll").kind,Plugins.getStatus("optional.dll").capability);
        Debug.message("unlink",Plugins.unlink("optional.dll"),Plugins.unlink("optional.dll"));
        Debug.message("export",typeof global.OptionalRenderer);
    "#,
        &[],
    );
    let path = t.path().join("project.json");
    let mut p: crysirror_runtime::Project =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    p.clients[0].config["plugin_shells"] = json!(["OPTIONAL.dll"]);
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut r = Runtime::with_registry(p, &path, &t.path().join("state2"), registry).unwrap();
    r.tick(0, InputFrame::default()).unwrap();
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("available 1 0"), "{log}");
    assert!(log.contains("shell registration_only"), "{log}");
    assert!(log.contains("unlink 1 0"), "{log}");
    assert!(log.contains("export undefined"), "{log}");
}

#[test]
fn kag_parser_preserves_native_tags_callbacks_control_flow_and_shared_draws() {
    let (_t, mut r) = fixture(
        r#"
Plugins.link('KAGParser.dll');
global.score=7;
class P extends KAGParser {
 function P(){super.KAGParser();ignoreCR=true;}
 function onLabel(name,page){Debug.message('label',name);}
 function onScript(script,name,line){Scripts.exec(script,name,line);return true;}
}
var p=new P();p.loadScenario('first.ks');
var w=new Window();var l=new Layer(w,null);
var item=p.getNextTag();if(item.tagname!='item'||item.x!='9')throw new Exception('macro attribute');
item=p.getNextTag();if(item.x!='7')throw new Exception('expression attribute');
item=p.getNextTag();if(item.x!='sub')throw new Exception('call');
item=p.getNextTag();if(item.x!='after')throw new Exception('return');
if(p.getNextTag()!==void||score!=42)throw new Exception('script/EOF');
l.fillRect(0,0,l.imageWidth,l.imageHeight,0xff123456);
if(Plugins.getStatus('KAGParser.dll').kind!='engine_facade')throw new Exception('plugin status');
Debug.message('kag-native-ok');
"#,
        &[(
            "first.ks",
            "*start\n[macro name=m][item x=%value][endmacro]\n[m value=9]\n[if exp=\"score==7\"][item x=&score][else][item x=bad][endif]\n[call target=*sub]\n[item x=after]\n[jump target=*end]\n*sub\n[item x=sub][return]\n*end\n[iscript]\nscore=42;\n[endscript]\n",
        )],
    );
    r.tick(0, InputFrame::default()).unwrap();
    assert!(
        r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("kag-native-ok")
    );
    assert!(r.host.engine.layers.get(ClientId(1), 2).is_some());
}

#[test]
fn kag_nested_load_callback_keeps_its_label_change_and_unknown_saves_fail_visibly() {
    let (_t, mut r) = fixture(
        r#"
 Plugins.link('KAGParser.dll');
 class P extends KAGParser {function P(){super.KAGParser();ignoreCR=true;} function onScenarioLoaded(name){goToLabel('*chosen');}}
 var p=new P();p.loadScenario('first.ks');
 if(p.getNextTag().value!='chosen')throw new Exception('nested label lost');
 var bad=false;try{p.store();}catch(e){bad=true;}if(!bad)throw new Exception('unmapped save succeeded');
 Debug.message('kag-reentry-ok');
 "#,
        &[(
            "first.ks",
            "[item value=wrong]\n*chosen\n[item value=chosen]\n",
        )],
    );
    r.tick(0, InputFrame::default()).unwrap();
    assert!(
        r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("kag-reentry-ok")
    );
}

#[test]
fn async_trigger_uses_shared_turn_fences_cached_posts_cancel_and_zero_interval_timer() {
    let (_t, mut r) = fixture(
        r#"
 global.n=0;
 var a=new AsyncTrigger(function(){n++;Debug.message('fire',n);if(n<3)a.trigger();},'');
 a.cached=true;a.trigger();a.trigger();
 var canceled=new AsyncTrigger(function(){throw new Exception('canceled delivered');},'');canceled.trigger();canceled.cancel();
 var t=new Timer(function(){throw new Exception('zero timer delivered');},'');t.interval=0;t.enabled=true;
 "#,
        &[],
    );
    for n in 1..=3 {
        r.tick(0, InputFrame::default()).unwrap();
        let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
        assert!(log.contains(&format!("fire {n}")));
        assert!(!log.contains(&format!("fire {}", n + 1)));
    }
}

fn authored_wave() -> Vec<u8> {
    let mut b = b"RIFF".to_vec();
    b.extend(164u32.to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(8000u32.to_le_bytes());
    b.extend(16000u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(128u32.to_le_bytes());
    for _ in 0..64 {
        b.extend(8192i16.to_le_bytes());
    }
    b
}
#[test]
fn native_sound_parks_the_original_caller_until_shared_resources_and_voice_are_ready() {
    let (d, mut r) = fixture(
        r#"
 var w=new Window();
 class Sound extends WaveSoundBuffer {function Sound(w){super.WaveSoundBuffer(w);}function onStatusChanged(){Debug.message('status',status);}}
 var sound=new Sound(w);sound.open('tone.wav');
 if(sound.totalSamples!=64||sound.frequency!=8000||sound.status!='stop')throw new Exception('open metadata');
 Debug.message('opened');sound.looping=true;sound.volume=50000;sound.play();
 if(sound.status!='play')throw new Exception('voice published early');
 Debug.message('native-audio-ready');
 "#,
        &[],
    );
    std::fs::write(d.path().join("assets/game/tone.wav"), authored_wave()).unwrap();
    r.set_audio_nonblocking(true);
    let pool = r.assets.io_tasks();
    let permits = (0..pool.stats().capacity)
        .map(|_| pool.reserve().unwrap())
        .collect::<Vec<_>>();
    r.tick(0, InputFrame::default()).unwrap();
    let before = r.inspect();
    assert_eq!(before["sessions"][0]["native"]["suspended"], true);
    assert!(
        !before["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("opened")
    );
    for _ in 0..2 {
        r.tick(
            1,
            InputFrame {
                buttons_pressed: 1,
                ..Default::default()
            },
        )
        .unwrap();
    }
    assert_eq!(r.inspect()["sessions"][0]["native"]["queued_input"], 3);
    drop(permits);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut audible = false;
    loop {
        r.tick(1, InputFrame::default()).unwrap();
        audible |= r
            .host
            .channel_audio
            .take_output()
            .iter()
            .any(|s| s[0].abs() > 0.01);
        if r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("native-audio-ready")
            && audible
        {
            break;
        }
        assert!(std::time::Instant::now() < end, "{}", r.inspect());
        std::thread::yield_now();
    }
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert_eq!(log.matches("native-audio-ready").count(), 1);
    assert_eq!(log.matches("status play").count(), 1);
    assert!(r.host.audio_info(ClientId(1), 0).unwrap().unwrap().looping);
}

#[test]
fn original_dictionary_save_struct_uses_atomic_save_capability_and_original_utf16_format() {
    let (_d, mut r) = fixture(
        r#"
 var data=%[number:42,text:'保存'];
 (Dictionary.saveStruct incontextof data)(System.dataPath+'state.ksd');
 if(!Storages.isExistentStorage(System.dataPath+'state.ksd'))throw new Exception('commit missing');
 var read=Scripts.evalStorage(System.dataPath+'state.ksd');
 if(read.number!=42||read.text!='保存')throw new Exception('original text save roundtrip');
 var denied=false;try{(Dictionary.saveStruct incontextof data)('startup.tjs');}catch(e){denied=true;}if(!denied)throw new Exception('asset write admitted');
 Debug.message('native-file-ready');
 "#,
        &[],
    );
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        r.tick(0, InputFrame::default()).unwrap();
        if r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("native-file-ready")
        {
            break;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::yield_now();
    }
    let storage = r.host.storage(ClientId(1)).unwrap();
    let b = storage
        .read(
            &crysirror_core::path::VfsPath::new(crysirror_core::path::Mount::Save, "state.ksd")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(&b[..2], &[0xff, 0xfe]);
    assert!(
        storage
            .read(
                &crysirror_core::path::VfsPath::new(
                    crysirror_core::path::Mount::Asset,
                    "game/startup.tjs"
                )
                .unwrap()
            )
            .unwrap()
            .starts_with(b"\n var data")
    );
}

#[test]
fn parser_save_uses_original_dictionary_fields_and_reloads_a_label_instead_of_exact_runtime_cursor()
{
    let (_d, mut r) = fixture(
        r#"
 Plugins.link('KAGParser.dll');var p=new KAGParser();p.ignoreCR=true;p.loadScenario('first.ks');
 var save=p.store();if(save.snapshot!==void||save.storageName!='first.ks'||save.macroArgs.count!=0||save.callStack.count!=0)throw new Exception('native layout');
 if(p.getNextTag().value!='first')throw new Exception('first');if(p.getNextTag().value!='second')throw new Exception('second');
 p.restore(save);if(p.getNextTag().value!='first')throw new Exception('native rewind');
 var m=p.macros;(Dictionary.assign incontextof m)(%[hello:'[item value=macro][macropop]']);
 if(p.getNextTag().value!='second')throw new Exception('second replay');
 if(p.getNextTag().value!='macro')throw new Exception('live macro dictionary');
 Debug.message('native-parser-save-ready');
 "#,
        &[(
            "first.ks",
            "*start\n[item value=first][item value=second][hello]\n",
        )],
    );
    r.tick(0, InputFrame::default()).unwrap();
    assert!(
        r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("native-parser-save-ready")
    );
}

#[test]
fn native_regions_reparent_visibility_and_identity_gamma_use_shared_state() {
    let (_d, mut r) = fixture(
        r#"
    var w=new Window();var root=new Layer(w,null);
    var a=new Layer(w,root);var b=new Layer(w,root);var child=new Layer(w,a);
    root.visible=true;a.visible=true;b.visible=true;child.visible=true;
    b.left=100;child.setImageSize(40,40);child.setSizeToImageSize();
    child.face=dfProvince;child.fillRect(0,0,40,40,0);child.fillRect(2,3,8,9,7);
    if(child.getProvincePixel(3,4)!=7||child.getProvincePixel(0,0)!=0)throw new Exception('region mismatch');
    child.face=dfAuto;child.fillRect(0,0,40,40,0xff123456);
    child.parent=b;
    if(child.parent!==b||a.children.count!=0||b.children.count!=1)throw new Exception('reparent mismatch');
    if(!child.nodeVisible)throw new Exception('visible ancestry');
    b.visible=false;if(child.nodeVisible)throw new Exception('hidden ancestry');b.visible=true;
    b.enabled=false;if(child.nodeEnabled)throw new Exception('disabled ancestry');b.enabled=true;
    var bad=false;try{b.parent=child;}catch(e){bad=true;}if(!bad)throw new Exception('cycle admitted');
    child.adjustGamma();child.adjustGamma(1,0,255,1,0,255,1,0,255);
    bad=false;try{child.adjustGamma(0);}catch(e){bad=true;}if(!bad)throw new Exception('invalid gamma silently accepted');
    Debug.message('native-region-tree-ok');
    "#,
        &[],
    );
    r.tick(0, InputFrame::default()).unwrap();
    assert!(
        r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("native-region-tree-ok")
    );
    let child = r.host.engine.layers.get(ClientId(1), 8).unwrap();
    assert_eq!(child.parent, Some(6));
    assert!(child.visible);
}

#[test]
fn native_hit_filter_uses_super_work_flag_and_disabled_hit_blocks_fallthrough() {
    let (_d, mut r) = fixture(
        r#"
    global.block=false;global.presses=0;
    class W extends Window { function W(){super.Window();} function onKeyDown(k,s){block=true;top.enabled=false;} }
    class Button extends Layer {function Button(w,p){super.Layer(w,p);}function onMouseDown(x,y,b,s){presses++;Debug.message('background-press',presses);} }
    class Filter extends Layer {function Filter(w,p){super.Layer(w,p);}function onHitTest(x,y,b){super.onHitTest(x,y,block);return !block;} }
    var w=new W();var root=new Layer(w,null);root.visible=true;
    var back=new Button(w,root);back.setImageSize(40,40);back.setSizeToImageSize();back.fillRect(0,0,40,40,0xff000000);back.visible=true;
    var top=new Filter(w,root);top.setImageSize(40,40);top.setSizeToImageSize();top.fillRect(0,0,40,40,0xff000000);top.visible=true;
    w.visible=true;
    "#,
        &[],
    );
    r.tick(0, InputFrame::default()).unwrap();
    let click = InputFrame {
        buttons_pressed: buttons::MOUSE_LEFT,
        pointer_x: 5,
        pointer_y: 5,
        ..Default::default()
    };
    r.tick(1, click).unwrap();
    let log = || r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log().contains("background-press 1"));
    r.tick(
        1,
        InputFrame {
            buttons_pressed: buttons::ENTER,
            pointer_x: 5,
            pointer_y: 5,
            ..Default::default()
        },
    )
    .unwrap();
    r.tick(1, click).unwrap();
    let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(!log.contains("background-press 2"));
}

#[test]
fn invalidation_releases_shared_native_voice_even_without_super_finalize() {
    let (d, mut r) = fixture(
        r#"
    class Sound extends WaveSoundBuffer {
      function Sound(){super.WaveSoundBuffer();}
      function finalize(){Debug.message('custom-finalize');}
    }
    var s=new Sound();s.open('tone.wav');s.looping=true;s.play();
    invalidate s;
    Debug.message('sound-invalidated');
    "#,
        &[],
    );
    std::fs::write(d.path().join("assets/game/tone.wav"), authored_wave()).unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        r.tick(1, InputFrame::default()).unwrap();
        let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
        if log.contains("sound-invalidated") {
            assert!(log.contains("custom-finalize"));
            assert!(r.host.audio_info(ClientId(1), 0).unwrap().is_none());
            break;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::yield_now();
    }
}

#[test]
fn native_image_filters_resume_original_tjs_and_publish_shared_resource_pixels() {
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;
    let (d, mut r) = fixture(
        r#"
    var w=new Window();var root=new Layer(w,null);var l=new Layer(w,root);
    l.loadImages('image/effects.png');l.face=dfProvince;l.fillRect(0,0,1,1,9);l.face=dfAuto;l.adjustGamma(2,0,255,2,0,255,2,0,255);
    Debug.message('gamma-ready');l.flipLR();if(l.getProvincePixel(1,0)!=9||l.getProvincePixel(0,0)!=0)throw new Exception('flip province');l.doGrayScale();
    l.visible=true;w.visible=true;Debug.message('shared-image-done');
    "#,
        &[],
    );
    image::RgbaImage::from_raw(2, 1, vec![128, 0, 0, 255, 0, 255, 0, 255])
        .unwrap()
        .save(d.path().join("assets/game/image/effects.png"))
        .unwrap();
    let pool = r.assets.cpu_tasks();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    let mut jobs = vec![];
    for _ in 0..pool.stats().workers {
        let gate = gate.clone();
        let tx = tx.clone();
        jobs.push(
            pool.spawn(move |_| {
                tx.send(())?;
                let (lock, wake) = &*gate;
                let (value, _) = wake
                    .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(10), |v| !*v)
                    .unwrap();
                anyhow::ensure!(*value, "image gate timeout");
                Ok(())
            })
            .unwrap(),
        );
    }
    for _ in 0..jobs.len() {
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    r.tick(0, InputFrame::default()).unwrap();
    let n = &r.inspect()["sessions"][0]["native"];
    assert_eq!(n["suspended"], true);
    assert!(!n["log_tail"].to_string().contains("gamma-ready"));
    r.tick(
        1,
        InputFrame {
            buttons_pressed: buttons::ENTER,
            ..Default::default()
        },
    )
    .unwrap();
    {
        let (lock, wake) = &*gate;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }
    let end = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        r.tick(1, InputFrame::default()).unwrap();
        let log = r.inspect()["sessions"][0]["native"]["log_tail"].to_string();
        if log.contains("shared-image-done") {
            assert_eq!(log.matches("gamma-ready").count(), 1);
            break;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::yield_now();
    }
    let surface = r
        .assets
        .surface(ClientId(1), "__surface/1073741826")
        .unwrap();
    assert_eq!(&*surface.rgba, &[182, 182, 182, 255, 38, 38, 38, 255]);
    assert_eq!(
        r.inspect()["sessions"][0]["native"]["owns_framebuffer"],
        false
    );
    drop(jobs);
}

#[test]
fn layer_ex_raster_plugin_routes_real_copy_to_shared_image_worker() {
    let (d, mut r) = fixture(
        r#"
 Plugins.link('layerExRaster.dll');
 var w=new Window();var root=new Layer(w,null);var src=new Layer(w,root);src.loadImages('image/raster.png');
 var dst=new Layer(w,root);dst.setImageSize(5,2);dst.copyRaster(src,3,4,4,1);
 if(Plugins.getStatus('layerExRaster.dll').kind!='engine_facade')throw new Exception('fake raster plugin');
 Debug.message('raster-facade-ready');
 "#,
        &[],
    );
    let mut image = image::RgbaImage::new(5, 2);
    for (i, p) in image.pixels_mut().enumerate() {
        *p = image::Rgba([(i as u8 + 1) * 10, 0, 0, 255]);
    }
    image
        .save(d.path().join("assets/game/image/raster.png"))
        .unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        r.tick(1, InputFrame::default()).unwrap();
        if r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("raster-facade-ready")
        {
            break;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::yield_now();
    }
    let image = r
        .assets
        .surface(ClientId(1), "__surface/1073741827")
        .unwrap();
    assert_eq!(
        image.rgba.chunks_exact(4).map(|p| p[0]).collect::<Vec<_>>(),
        [20, 30, 40, 50, 255, 255, 255, 60, 70, 80]
    );
    assert_eq!(image.rgba[19], 0);
    assert_eq!(image.rgba[23], 0);
}
