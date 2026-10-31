// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Native LineParser calls the shared, VM-independent LineCursor.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;
// Same authored replay reader as the real desktop frontend; test-only reuse.
#[path = "../../crysirror-desktop/src/input_track.rs"]
mod input_track;
fn fixture(source: &str) -> (tempfile::TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    std::fs::write(d.path().join("assets/startup.tjs"), source).unwrap();
    let path = d.path().join("project.json");
    let p=serde_json::from_value(json!({"schema":2,"name":"line plugin","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-line","config":{"canvas":[32,32]}}]})).unwrap();
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let r = Runtime::with_registry(p, &path, &d.path().join("state"), registry).unwrap();
    (d, r)
}
#[test]
fn line_parser_preserves_newlines_target_events_and_storage_encoding() {
    let (dir, mut runtime) = fixture(
        r#"
        Plugins.link('lineParser.dll');
        if(Plugins.getStatus('lineParser.dll').kind!='engine_facade') throw new Exception('status');
        var parser=new LineParser();
        parser.init('one\r\n\r二\n');
        if(parser.currentLineNumber!=0||parser.getNextLine()!='one'||parser.getNextLine()!=''||parser.getNextLine()!='二'||parser.getNextLine()!==void||parser.currentLineNumber!=3)throw new Exception('line cursor');
        var target=%[lines:[],doLine:function(s,n){this.lines.add(n+':'+s);}];
        var events=new LineParser(target);events.parse('a\rb\nc');
        if(target.lines.join('|')!='1:a|2:b|3:c') throw new Exception('events');
        events.parseStorage('unicode.txt',true);
        if(target.lines[3]!='1:日本'||target.lines[4]!='2:tail'||events.currentLineNumber!=2)throw new Exception('UTF8 storage');
        events.init('fresh');if(events.getNextLine()!='fresh'||events.currentLineNumber!=1)throw new Exception('reset');
        if(Plugins.unlink('lineParser.dll')!=0)throw new Exception('unload live facade');
        Debug.message('shared-line-complete');
    "#,
    );
    std::fs::write(dir.path().join("assets/unicode.txt"), "日本\r\ntail").unwrap();
    for _ in 0..100 {
        runtime.tick(1, InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("shared-line-complete")
        {
            return;
        }
    }
    panic!("native LineParser did not complete: {}", runtime.inspect());
}

#[test]
fn var_media_references_live_octets_and_decodes_images_with_shared_engine_services() {
    use std::io::Cursor;
    let mut encoded = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 1, image::Rgba([200, 40, 10, 255]))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let octet = format!(
        "<%{}%>",
        encoded
            .into_inner()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let source = r#"
        Plugins.link('varfile.dll');Plugins.link('lineParser.dll');Plugins.link('fstat.dll');
        global.files=%[image:IMAGE_BYTES];
        var original=['first','日本'];original.save('var://./files/text');
        if(!Storages.isExistentStorage('var://./files/text'))throw new Exception('exists');
        var lines=[];lines.load('var://./files/text');
        if(lines.count!=2||lines[1]!='日本')throw new Exception('text read/write');
        global.files.text=<% 61 0a 62 %>;lines.load('var://./files/text');
        if(lines[0]!='a'||lines[1]!='b')throw new Exception('stale snapshot');
        global.files.nested=[<% 78 0a 79 %>];lines.load('var://./files/nested/0');
        if(lines[0]!='x'||lines[1]!='y')throw new Exception('array path');
        var state=%[answer:42];(Dictionary.saveStruct incontextof state)('var://./files/state','b');
        var restored=%[];restored=(Dictionary.loadStruct incontextof restored)('var://./files/state');
        if(restored.answer!=42)throw new Exception('binary struct roundtrip');
        var parser=new LineParser();parser.initStorage('var://./files/text');
        if(parser.getNextLine()!='a')throw new Exception('media LineCursor');
        if(Storages.dirlist('var://./files/').count!=3)throw new Exception('media directory');
        var bad=false;try{lines.load('var://foreign/files/text');}catch(e){bad=true;}
        if(!bad)throw new Exception('foreign domain');
        bad=false;try{lines.load('var://./../files/text');}catch(e){bad=true;}
        if(!bad)throw new Exception('parent traversal');
        global.files.script=<% 67 6c 6f 62 61 6c 2e 6d 65 64 69 61 53 63 72 69 70 74 3d 37 3b %>;
        Scripts.execStorage('var://./files/script');
        if(global.mediaScript!=7)throw new Exception('media script');
        var window=new Window();var root=new Layer(window,null);var picture=new Layer(window,root);
        picture.loadImages('var://./files/image');
        if(picture.imageWidth!=2||picture.imageHeight!=1)throw new Exception('shared image decode');
        picture.visible=true;window.visible=true;
        Debug.message('var-media-complete');
    "#
    .replace("IMAGE_BYTES", &octet);
    let (_dir, mut runtime) = fixture(&source);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        runtime.tick(1, InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("var-media-complete")
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{}",
            runtime.inspect()
        );
        std::thread::yield_now();
    }
    assert!(
        serde_json::to_string(&runtime.inspect())
            .unwrap()
            .contains("canvas")
    );
}

/// Explicit installed-corpus probe through the real Runtime/SharedHost. It
/// mounts original archives read-only and never executes an EXE or DLL.
#[test]
#[ignore = "requires CRYSIRROR_KRKR_ADLER_GAME and MODULES; user game resources remain local"]
fn installed_rotating_adler_game_uses_real_shared_host() {
    use crysirror_core::{
        path::{Mount, VfsPath},
        service::HostServices,
        state::ClientId,
    };
    use crysirror_resource::{ArchiveFormat, ArchiveMount, DecodeContext};
    use std::sync::Arc;
    let root = std::path::PathBuf::from(
        std::env::var_os("CRYSIRROR_KRKR_ADLER_GAME").expect("explicit game root"),
    );
    let modules: Vec<_> = std::env::var("CRYSIRROR_KRKR_ADLER_MODULES")
        .expect("explicit companion modules")
        .split(',')
        .map(str::to_owned)
        .collect();
    let exe =
        std::env::var("CRYSIRROR_KRKR_EXE").expect("explicit executable basename (data only)");
    let canvas: Vec<u32> = std::env::var("CRYSIRROR_KRKR_CANVAS")
        .expect("verified canvas w,h")
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(canvas.len(), 2);
    let state = tempfile::tempdir().unwrap();
    let project=serde_json::from_value(json!({"schema":2,"name":"installed native resource probe","content_revision":"1","canvas":canvas,"assets":"unused","clients":[{"name":"native","adapter":"krkr","game":"authorized-installed","config":{"canvas":canvas,"executable":exe,"arguments":{"-debugwin":"no"}}}]})).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut runtime = Runtime::with_asset_root(project, &root, state.path(), registry).unwrap();
    // Match the desktop's explicit host font setup; actual bytes stay caller
    // owned and are installed through the same shared Text API.
    if let Some(path)=std::env::var_os("CRYSIRROR_KRKR_FONT") {
        use std::io::Read;
        let mut bytes=Vec::new();
        std::fs::File::open(path).unwrap().take(32*1024*1024+1).read_to_end(&mut bytes).unwrap();
        assert!(bytes.len()<=32*1024*1024,"font byte limit");
        runtime.host.text.add_font("default",bytes.clone()).unwrap();
        runtime.assets.text.add_font("default",bytes).unwrap();
    }
    crysirror_formats_arc::register_images(&mut runtime.assets.decoders).unwrap();
    let client = ClientId(1);
    let storage = runtime.host.storage(client).unwrap();
    let mut archives: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|s| s.to_ascii_lowercase().ends_with(".xp3"))
        .collect();
    archives.sort();
    for name in archives {
        let mut context = DecodeContext::new(client);
        context.source_name = Some(name.clone());
        context.limits.max_entry_size = 256 << 20;
        context.limits.max_entries = 1_000_000;
        context.limits.max_index_size = 64 << 20;
        let companion_storage = storage.clone();
        context.resolver = Some(Arc::new(
            move |name: &str| -> anyhow::Result<Arc<dyn crysirror_core::service::AssetReader>> {
                Ok(companion_storage.open_asset(&VfsPath::new(Mount::Asset, name)?)?)
            },
        ));
        // The same installed Runtime probe also accepts classic CXDEC
        // companions, explicitly selected by the caller. Both profiles pass
        // through shared ArchiveFormat/DecodeContext; no game hook is injected.
        let parameters = if std::env::var_os("CRYSIRROR_KRKR_CXDEC").is_some() {
            json!({"cxdec_modules":modules})
        } else {
            json!({"adler_xor_modules":modules})
        };
        context.parameters.insert("kirikiri/xp3".into(), parameters);
        let source = storage
            .open_asset(&VfsPath::new(Mount::Asset, &name).unwrap())
            .unwrap();
        let archive = crysirror_formats_arc::Xp3.open(source, &context).unwrap();
        let provider = Arc::new(ArchiveMount::new(archive, context).unwrap());
        let target = if name.eq_ignore_ascii_case("data.xp3") {
            String::new()
        } else {
            format!("__xp3/{}", name[..name.len() - 4].to_ascii_lowercase())
        };
        runtime.mount_provider(client, &target, provider).unwrap();
    }
    let frames = std::env::var("CRYSIRROR_KRKR_FRAMES")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 10000);
    let mut error = None;
    let mut completed = 0;
    let input_path = std::env::var_os("CRYSIRROR_KRKR_INPUT_TRACK").map(std::path::PathBuf::from);
    let inputs = input_track::InputTrack::load(input_path.as_deref()).unwrap();
    let await_title=std::env::var_os("CRYSIRROR_KRKR_AWAIT_TITLE").is_some();
    let mut title_ready=None;
    let mut input_clock=0;
    for frame in 0..frames {
        let input=if !await_title||title_ready.is_some(){let input=inputs.at(input_clock);input_clock+=1;input}else{InputFrame::default()};
        match runtime.tick(16, input) {
            Ok(()) => {
                completed += 1;
                if await_title&&title_ready.is_none(){
                    let view=runtime.inspect();let native=&view["sessions"][0]["native"];
                    if native["kag"]["last_tag"]=="s"&&native["kag"]["sleeping"]==true&&native["suspended"]==false {
                        title_ready=Some(frame);
                    }
                }
                std::thread::yield_now();
            }
            Err(e) => {
                error = Some(format!("{e:#}"));
                break;
            }
        }
    }
    if await_title&&title_ready.is_none()&&error.is_none(){error=Some("original title wait was not observed within the bounded probe".into());}
    let report = json!({"title_ready_frame":title_ready,"input_updates":input_clock,"frames_completed":completed,"requested_frames":frames,"gpu_rendered":false,"error":error,"runtime":runtime.inspect()});
    if let Some(path) = std::env::var_os("CRYSIRROR_KRKR_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(error.is_none(), "{}", error.unwrap_or_default());
}

#[test]
fn kaicho_options_publish_shared_effects_and_preserve_native_override_rules() {
    for (kind, options) in [
        (
            "blur",
            "time:1,blur1x:2,blur1y:1,blur1:3,blur2:0,accel:6,dynamic:1",
        ),
        (
            "dim",
            "time:1,rule:'rule.png',blur:3,xblur:2,neg:false,accel:2,vague:1",
        ),
    ] {
        let source = format!(
            r#"
            Plugins.link('KaichoTrans.dll');
            var w=new Window();var root=new Layer(w,null);var before=new Layer(w,root);var after=new Layer(w,root);
            before.setSize(8,4);after.setSize(8,4);before.visible=true;after.visible=false;
            before.fillRect(0,0,8,4,0xff000000);after.fillRect(0,0,8,4,0xffffffff);
            var bad=false;try{{before.beginTransition('blur',true,after,%[time:100,unknownoption:1]);}}catch(e){{bad=true;}}
            if(!bad)throw new Exception('unknown transition option was accepted');
            before.beginTransition('{kind}',true,after,%[{options}]);Debug.message('kaicho-ready');
        "#
        );
        let (dir, mut runtime) = fixture(&source);
        image::RgbaImage::from_pixel(8, 4, image::Rgba([128, 128, 128, 255]))
            .save(dir.path().join("assets/rule.png"))
            .unwrap();
        runtime.tick(0, InputFrame::default()).unwrap();
        let layers = serde_json::to_value(&runtime.host.engine.layers).unwrap();
        let fades = layers["crossfades"]["1"].as_object().unwrap();
        assert_eq!(fades.len(), 1);
        let fade = fades.values().next().unwrap();
        assert_eq!(fade["duration_ms"], 2);
        let effect = &fade["effect"];
        assert_eq!(effect["kind"], kind);
        if kind == "blur" {
            assert_eq!(effect["before"], json!([3, 3]));
            assert_eq!(effect["after"], json!([0, 0]));
            assert_eq!(effect["dynamic"], true);
        } else {
            assert_eq!(effect["blur"], json!([2, 3]));
            assert_eq!(effect["negative"], true);
            assert_eq!(effect["acceleration"], 2.);
        }
    }
}

#[test]
fn dirlist_uses_scoped_engine_directory_metadata_and_rejects_traversal() {
    let (root, mut runtime) = fixture(
        r#"
        Plugins.link('dirlist.dll');
        var names=getDirList('folder/');
        if(names.count!=4||names[0]!='./'||names[1]!='../'||names[2]!='child/'||names[3]!='tiny.bin')throw new Exception('directory entries');
        var bad=false;try{getDirList('folder');}catch(e){bad=true;}if(!bad)throw new Exception('missing slash');
        bad=false;try{getDirList('../');}catch(e){bad=true;}if(!bad)throw new Exception('root traversal');
        bad=false;try{getDirList('missing/');}catch(e){bad=true;}if(!bad)throw new Exception('missing directory');
        Debug.message('shared-dirlist-complete');
    "#,
    );
    std::fs::create_dir_all(root.path().join("assets/folder/child")).unwrap();
    std::fs::write(root.path().join("assets/folder/tiny.bin"), b"data").unwrap();
    runtime.tick(0, InputFrame::default()).unwrap();
    assert!(
        runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("shared-dirlist-complete")
    );
}

#[test]
fn ext_kag_executes_native_loops_locals_multiline_macros_and_call_scopes() {
    let (_dir, mut runtime) = fixture(
        r#"
        Plugins.link('ExtKAGParser.dll');
        if(Plugins.getStatus('ExtKAGParser.dll').kind!='engine_facade')throw new Exception('provider');
        global.tf=%[];
        class P extends KAGParser {function P(){super.KAGParser();ignoreCR=true;multiLineTagEnabled=true;numericMacroArgumentsEnabled=true;}}
        var p=new P();
        p.onScenarioLoad=function(name){return global.source;};
        global.source='[localvar value=outer][while init="tf.i=0" exp="tf.i<3" each="tf.i++"][continue cond="tf.i==1"][item number=&tf.i local=$value][endwhile][pushlocalvar copyvar=true value=inner][item local=$value][poplocalvar][call target=*sub arg=call][item local=$value][jump target=*end]\n*sub\n[item arg=$arg][pushlocalvar copyvar=true extra=child][item old=$arg][return]\n*end\n[item final=true]';
        p.loadScenario('loops.ks');
        var values=[];var tag;
        while((tag=p.getNextTag())!==void){values.add(tag.number!==void?tag.number:(tag.local!==void?tag.local:(tag.arg!==void?tag.arg:(tag.old!==void?tag.old:'end'))));}
        if(values.join('|')!='0|2|inner|call|call|outer|end'||p.localVariablesDepth!=1||p.whileStackDepth!=0||p.lf.value!='outer')throw new Exception('loop/local/call '+values.join('|'));
        global.source='[pmacro name=paint color=red][macro name=m fallback=yes][item value=%fallback first=%1][endmacro][item paint explicit=true \\\n amount=42][m chosen=hi][done]';
        p.loadScenario('macros.ks');
        tag=p.getNextTag();if(tag.color!='red'||tag.explicit!='true'||tag.amount!='42')throw new Exception('multiline/pmacro');
        tag=p.getNextTag();if(tag.tagname!='eval')throw new Exception('native default eval');Scripts.eval(tag.exp,'',0,p);
        tag=p.getNextTag();if(tag.value!='yes'||tag.first!='chosen')throw new Exception('macro defaults/numeric '+tag.value+' '+tag.first);
        if(p.getNextTag().tagname!='done')throw new Exception('macro end');
        var bad=false;try{p.fuzzyReturn=true;}catch(e){bad=true;}if(!bad)throw new Exception('unimplemented fuzzy return');
        global.source='*saved\n[item a=one][item a=two]';p.loadScenario('save.ks');
        if(p.getNextTag().a!='one')throw new Exception('save scenario');
        p.lf.saved=%[nested:[1]];var saved=p.store();p.lf.saved.nested[0]=9;p.ignoreCR=false;p.restore(saved);
        if(p.lf.saved.nested[0]!=1||!p.ignoreCR||!p.multiLineTagEnabled)throw new Exception('original Ext native quiescent save');
        var missing=false;try{var invalid=p.prelf;}catch(e){missing=true;}if(!missing)throw new Exception('prelf underflow');
        p.clearCallStack();if(p.localVariablesDepth!=1||p.lf.saved!==void)throw new Exception('clear locals');
        Debug.message('ext-kag-native-complete');
    "#,
    );
    for _ in 0..100 {
        runtime.tick(1, InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("ext-kag-native-complete")
        {
            return;
        }
    }
    panic!("ExtKAGParser did not complete: {}", runtime.inspect());
}

#[test]
fn ext_kag_assignment_copies_execution_but_preserves_native_value_ownership() {
    let (_dir, mut runtime) = fixture(
        r#"
        Plugins.link('ExtKAGParser.dll');
        var source=new KAGParser();var copy=new KAGParser();
        source.ignoreCR=true;source.multiLineTagEnabled=true;source.numericMacroArgumentsEnabled=true;
        source.onScenarioLoad=function(name){return global.scenario;};
        global.scenario='[localvar value=outer][call target=*sub arg=call][done][jump target=*end]\n*sub\n[localvar child=yes][while exp=true][item arg=$arg][break][endwhile][return]\n*end\n';
        source.loadScenario('assign.ks');
        if(source.getNextTag().arg!='call'||source.callStackDepth!=1)throw new Exception('source nested state');
        source.pmacros.color=%[color:'red'];
        copy.processSpecialTags=false;copy.assign(source);
        if(copy.callStackDepth!=1||copy.whileStackDepth!=source.whileStackDepth||copy.localVariablesDepth!=2)throw new Exception('execution stacks');
        if(!copy.ignoreCR||copy.multiLineTagEnabled||copy.numericMacroArgumentsEnabled||copy.processSpecialTags)throw new Exception('owner switches');
        if(copy.localVariables===source.localVariables||copy.lf!==source.lf||copy.pmacros.color!==source.pmacros.color)throw new Exception('shallow native ownership');
        copy.lf.child='shared';if(source.lf.child!='shared')throw new Exception('shared local dictionary');
        copy.processSpecialTags=true;
        if(copy.getNextTag().tagname!='done'||source.getNextTag().tagname!='done'||copy.callStackDepth!=0||source.callStackDepth!=0)throw new Exception('independent continuation');
        if(copy.pmacros.color!==source.pmacros.color)throw new Exception('parameter macro identity after parsing');
        global.scenario='[macro name=m][item value=%value][item next=done][endmacro][m value=source]';
        source.loadScenario('macro-assign.ks');var tag=source.getNextTag();
        if(tag.tagname=='eval'){Scripts.eval(tag.exp,'',0,source);tag=source.getNextTag();}
        if(tag.value!='source')throw new Exception('active macro');
        copy.assign(source);
        if(copy.mp===source.mp||copy.mp.value!='source')throw new Exception('macro argument dictionary copy');
        copy.mp.value='destination';if(source.mp.value!='source')throw new Exception('isolated macro arguments');
        if(copy.getNextTag().next!='done'||source.getNextTag().next!='done')throw new Exception('macro continuation');
        Debug.message('ext-assign-complete');
    "#,
    );
    for _ in 0..100 {
        runtime.tick(1, InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("ext-assign-complete")
        {
            return;
        }
    }
    panic!("Ext assignment did not complete: {}", runtime.inspect());
}

#[test]
fn ext_macro_dictionaries_keep_typed_values_across_tags_nested_calls_and_slot_reuse() {
    let (_dir, mut runtime) = fixture(
        r#"
        Plugins.link('ExtKAGParser.dll');
        var parser=new KAGParser();parser.ignoreCR=true;
        parser.onScenarioLoad=function(name){return global.scenario;};
        global.scenario='[macro name=outer][item value=&(mp.state===void)][eval exp="mp.state=%[f:1,array:[7]]"][eval exp="global.saved=mp"][item value=\"&mp.state.array[0]\"][inner alias=%state *][item value=&(mp===global.saved)][item value=&mp.extra][item value=&mp.state.f][item value=\"&mp.state.array[0]\"][endmacro]';
        global.scenario+='[macro name=inner][eval exp="global.saved.state.f=2"][eval exp="global.saved.state.array[0]=9"][eval exp="global.saved.extra=17"][eval exp="mp.own=[1,2]"][item value=\"&mp.own[1]\"][check object=%own * state=override][endmacro][outer][outer][done]';
        parser.loadScenario('typed-macro.ks');
        var values=[];var tag;
        while((tag=parser.getNextTag())!==void){
            if(tag.tagname=='eval')Scripts.eval(tag.exp,'',0,parser);
            else if(tag.tagname=='item')values.add(tag.value);
            else if(tag.tagname=='check'){if(!(tag.object instanceof 'Array')||!(tag.own instanceof 'Array')||tag.alias!==global.saved.state||tag.state!='override')throw new Exception('typed native tag forwarding');}
            else if(tag.tagname=='done')values.add('done');
        }
        if(values.join('|')!='1|7|2|1|17|2|9|1|7|2|1|17|2|9|done')throw new Exception('typed macro state '+values.join('|'));
        Debug.message('typed-macro-complete');
    "#,
    );
    for _ in 0..100 {
        runtime.tick(1, InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("typed-macro-complete")
        {
            return;
        }
    }
    panic!("typed macro test did not complete: {}", runtime.inspect());
}

#[test]
fn piled_copy_keeps_canonical_addalpha_child_in_shared_frozen_snapshot() {
    use crysirror_core::state::ClientId;
    use crysirror_scene::layer::{LayerCommand, LayerSystem, Paint};
    let (_dir, mut runtime) = fixture(
        r#"
        var window=new Window();var root=new Layer(window,null);window.visible=true;
        var source=new Layer(window,root);source.setImageSize(4,4);source.setSizeToImageSize();source.type=ltOpaque;
        source.fillRect(0,0,4,4,0xff0000ff);
        var child=new Layer(window,source);child.setImageSize(2,2);child.setSizeToImageSize();child.visible=true;child.type=ltAddAlpha;
        child.fillRect(0,0,2,2,0x80ff0000);child.opacity=128;
        var destination=new Layer(window,root);destination.setImageSize(4,4);destination.setSizeToImageSize();destination.visible=true;destination.hitType=htProvince;
        destination.piledCopy(0,0,source,0,0,4,4);
        child.fillRect(0,0,2,2,0xff00ff00);
    "#,
    );
    runtime.tick(0, InputFrame::default()).unwrap();
    let frame = runtime.host.engine.layers.frame([32, 32]).unwrap();
    let snapshot = frame
        .iter()
        .find_map(|primitive| match &primitive.paint {
            Paint::Snapshot { size, layers } => Some((*size, layers)),
            _ => None,
        })
        .expect("shared frozen subtree");
    let mut frozen = LayerSystem::default();
    frozen
        .apply_batch(
            ClientId(1),
            &snapshot
                .1
                .iter()
                .cloned()
                .map(|layer| LayerCommand::Put { layer })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let inner = frozen.frame(snapshot.0).unwrap();
    let group = inner
        .iter()
        .find(|p| p.opacity == 128 && matches!(p.paint, Paint::Snapshot { .. }))
        .expect("isolated native group alpha");
    let Paint::Snapshot { size, layers } = &group.paint else {
        unreachable!()
    };
    let mut group_tree = LayerSystem::default();
    group_tree.install(ClientId(1), layers).unwrap();
    let contents = group_tree.frame(*size).unwrap();
    assert!(
        contents.iter().any(|p| matches!(
            p.paint,
            Paint::Solid {
                color: [255, 0, 0, 128]
            }
        ) && p.opacity == 255),
        "pixel alpha must be inside, group alpha outside"
    );
    assert!(!contents.iter().any(|p| matches!(
        p.paint,
        Paint::Solid {
            color: [0, 255, 0, 255]
        }
    )));
    assert_eq!(
        runtime.inspect()["sessions"][0]["native"]["owns_framebuffer"],
        false
    );
}

#[test]
fn ext_original_call_macro_save_roundtrips_typed_values_and_label_rewind() {
    let (dir, mut runtime) = fixture(
        r#"
        Plugins.link('ExtKAGParser.dll');
        var parser=new KAGParser();parser.ignoreCR=true;parser.multiLineTagEnabled=true;
        parser.loadScenario('main.ks');
        var tag;while((tag=parser.getNextTag())!==void){
            if(tag.tagname=='eval')Scripts.eval(tag.exp,'',0,parser);
            else if(tag.name=='callee')break;
        }
        parser.lf.localData=[3];
        var saved=parser.store();
        if(saved.callStack.count!=1||saved.whileStack.count!=1||saved.macroArgs.count!=2||saved.macroArgStackBase!=2||saved.macroArgStackDepth!=2||saved.LocalVariables.count!=2)throw new Exception('original active save fields');
        if(!saved.callStack[0].lineBufferUsing||saved.callStack[0].lineBuffer.indexOf('inner-after')<0||saved.callStack[0].lineBuffer.indexOf('outer-after')<0||saved.callStack[0].WhileStackDepth!=1||saved.callStack[0].LocalVariablesCount!=2)throw new Exception('native return buffer');
        parser.mp.alias.list[0]=99;parser.lf.localData[0]=99;
        if(saved.macroArgs[1].alias.list[0]!=7||saved.LocalVariables[1].localData[0]!=3)throw new Exception('structured snapshot independence');
        parser.clear();
        global.restoreCallbacks=0;
        parser.onScenarioLoaded=function(name){
            if(this.callStackDepth!=1||this.localVariablesDepth!=2||this.mp.alias.list[0]!=7)throw new Exception('restore callback ownership');
            global.restoreCallbacks++;
        };
        parser.restore(saved);
        if(global.restoreCallbacks!=1||!parser.multiLineTagEnabled||!parser.ignoreCR||parser.lf.localData[0]!=3)throw new Exception('restored owner values');
        var names=[];
        while((tag=parser.getNextTag())!==void){
            if(tag.tagname=='eval')Scripts.eval(tag.exp,'',0,parser);
            else if(tag.name!==void){names.add(tag.name);if(tag.value!==void&&tag.value!='7')throw new Exception('restored typed macro value');}
            else if(tag.tagname=='done')names.add('done');
        }
        if(names.join('|')!='callee|inner-after|outer-after|done'||parser.callStackDepth!=0||parser.localVariablesDepth!=1)throw new Exception('native restored continuation '+names.join('|'));
        delete parser.onScenarioLoaded;
        parser.loadScenario('while.ks');parser.getNextTag();
        var failed=false;try{parser.store();}catch(e){failed=e.message.indexOf('while-loop')>=0;}if(!failed)throw new Exception('unknown active while save must fail');
        Debug.message('native-active-save-complete');
    "#,
    );
    std::fs::write(dir.path().join("assets/main.ks"),r#"*root
[macro name=outer][eval exp="mp.payload=%[list:[7]]"][inner alias=%payload *][item name=outer-after value="&mp.payload.list[0]"][endmacro]
[macro name=inner][call storage=callee.ks target=*save][item name=inner-after value="&mp.alias.list[0]"][endmacro]
[outer]
[done]"#).unwrap();
    std::fs::write(
        dir.path().join("assets/callee.ks"),
        "*save\n[item name=callee][return]",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("assets/while.ks"),
        "[while exp=true][item name=loop][endwhile]",
    )
    .unwrap();
    for _ in 0..100 {
        runtime.tick(1, InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("native-active-save-complete")
        {
            return;
        }
    }
    panic!(
        "original active save did not complete: {}",
        runtime.inspect()
    );
}

#[test]
fn native_font_class_keeps_nominal_metrics_and_reports_portable_rasterizer_mapping() {
    let (_dir, mut runtime) = fixture(
        r#"
        if(Font.rasterizer!=frGDI)throw new Exception('original default profile');
        Font.rasterizer=frFreeType;
        var font=new Font();font.height=-17;
        if(!(font instanceof 'Font')||font.getTextHeight('')!=17||font.rasterizer!=frFreeType)throw new Exception('standalone engine font');
        var window=new Window();var root=new Layer(window,null);
        root.font.rasterizer=frGDI;
        if(Font.rasterizer!=frGDI||!(root.font instanceof 'Font'))throw new Exception('static rasterizer selector');
        var bad=false;try{Font.rasterizer=99;}catch(e){bad=true;}if(!bad)throw new Exception('unknown rasterizer');
        Debug.message('portable-font-complete');
    "#,
    );
    runtime.tick(0, InputFrame::default()).unwrap();
    let log = runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(
        log.contains("portable-font-complete")
            && log.contains("native GDI/FreeType raster pixels are not implemented"),
        "{log}"
    );
}

#[test]
#[ignore = "requires CRYSIRROR_CODEC_FIXTURES with locally generated Opus/Vorbis sine fixtures"]
fn native_codec_modules_use_shared_decoders_and_reject_unmapped_replay_gain() {
    let root = std::path::PathBuf::from(
        std::env::var_os("CRYSIRROR_CODEC_FIXTURES").expect("explicit authored codec fixture path"),
    );
    let (dir, mut runtime) = fixture(
        r#"
        Plugins.link('kropus.dll');Plugins.link('wuvorbis.dll');
        var opus=new WaveSoundBuffer();opus.open('tone.opus');
        if(opus.frequency!=48000||opus.totalSamples<100000)throw new Exception('Opus metadata');
        opus.looping=true;opus.play();
        var vorbis=new WaveSoundBuffer();vorbis.open('tone.ogg');
        if(vorbis.frequency!=48000||vorbis.totalSamples<100000)throw new Exception('Vorbis metadata');
        vorbis.looping=true;vorbis.play();
        Debug.message('codec-native-complete');
    "#,
    );
    for name in ["tone.opus", "tone.ogg"] {
        std::fs::copy(root.join(name), dir.path().join("assets").join(name)).unwrap();
    }
    let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut audible = false;
    loop {
        runtime.tick(10, InputFrame::default()).unwrap();
        audible |= runtime
            .host
            .channel_audio
            .take_output()
            .iter()
            .any(|v| v[0].abs() > 0.01 || v[1].abs() > 0.01);
        if audible
            && runtime.inspect()["sessions"][0]["native"]["log_tail"]
                .to_string()
                .contains("codec-native-complete")
        {
            break;
        }
        assert!(std::time::Instant::now() < until, "{}", runtime.inspect());
        std::thread::yield_now();
    }
    let (dir, mut runtime) = fixture(
        "Plugins.link('wuvorbis.dll');var sound=new WaveSoundBuffer();sound.open('gain.ogg');throw new Exception('ReplayGain should fail before playback');",
    );
    std::fs::copy(root.join("gain.ogg"), dir.path().join("assets/gain.ogg")).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        match runtime.tick(10, InputFrame::default()) {
            Err(e) => {
                assert!(format!("{e:#}").contains("ReplayGain-tagged"), "{e:#}");
                break;
            }
            Ok(_) => {}
        }
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    assert!(
        runtime
            .host
            .channel_audio
            .take_output()
            .iter()
            .all(|sample| sample[0] == 0.0 && sample[1] == 0.0)
    );
}

#[test]
fn native_layer_constructor_injection_runs_after_the_real_font_is_created() {
    let (_dir, mut runtime) = fixture(
        r#"
        global.originalLayer=Layer.Layer;global.layerHookCalls=0;
        Layer.Layer=function(*) {
            global.layerHookCalls++;
            var result=(global.originalLayer incontextof this)(*);
            this.font.injected=42;
            return result;
        } incontextof null;
        class Parent extends Layer {function Parent(w,p){super.Layer(w,p);}}
        class Child extends Parent {function Child(w,p){super.Parent(w,p);}}
        var window=new Window();var layer=new Child(window,null);
        if(global.layerHookCalls!=1||layer.font.injected!=42||layer.width!=32)throw new Exception('native constructor hook');
        var direct=new Layer(window,layer);
        if(global.layerHookCalls!=2||direct.font.injected!=42)throw new Exception('direct native constructor hook');
        if(layer.Layer!=Layer.Layer||direct.Layer!=Layer.Layer)throw new Exception('constructor member overwritten');
        Debug.message('layer-native-hook-complete');
    "#,
    );
    runtime.tick(0, InputFrame::default()).unwrap();
    assert!(
        runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("layer-native-hook-complete")
    );
}

#[test]
fn native_idle_sound_buffers_do_not_consume_the_playing_voice_limit() {
    let (_dir, mut runtime) = fixture(
        r#"
        var buffers=[];
        class Sound extends WaveSoundBuffer {function Sound(){super.WaveSoundBuffer();}}
        for(var i=0;i<100;i++)buffers.add(new Sound());
        if(buffers.count!=100||buffers[99].status!='unload')throw new Exception('idle sound buffer pool');
        Debug.message('logical-sound-buffer-pool-complete');
    "#,
    );
    runtime.tick(0, InputFrame::default()).unwrap();
    assert!(
        runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("logical-sound-buffer-pool-complete")
    );
    assert!(
        runtime
            .host
            .channel_audio
            .take_output()
            .iter()
            .all(|v| v[0] == 0.0 && v[1] == 0.0)
    );
}

#[test]
fn native_ciphered_and_compressed_struct_saves_wait_for_atomic_shared_commit() {
    let (_dir, mut runtime) = fixture(
        r#"
        Plugins.link('varfile.dll');global.files=%[];
        var data=%[number:42,text:'保存😀'];
        var modes=['c1','z','z0','z9'];
        for(var i=0;i<modes.count;i++) {
            var filename=System.dataPath+'state-'+modes[i]+'.ksd';
            (Dictionary.saveStruct incontextof data)(filename,modes[i]);
            if(!Storages.isExistentStorage(filename))throw new Exception('atomic commit not visible');
            var restored=Scripts.evalStorage(filename);
            if(restored.number!=42||restored.text!='保存😀')throw new Exception('encoded native struct');
        }
        (Dictionary.saveStruct incontextof data)('var://./files/compressed','z');
        var restored=Scripts.evalStorage('var://./files/compressed');
        if(restored.number!=42||restored.text!='保存😀')throw new Exception('shared memory stream codec');
        Debug.message('encoded-native-save-complete');
    "#,
    );
    let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        runtime.tick(0, InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("encoded-native-save-complete")
        {
            break;
        }
        assert!(std::time::Instant::now() < until, "{}", runtime.inspect());
        std::thread::yield_now();
    }
    let storage = crysirror_core::service::HostServices::storage(
        &mut runtime.host,
        crysirror_core::state::ClientId(1),
    )
    .unwrap();
    for (mode, header) in [("c1", 1), ("z", 2), ("z0", 2), ("z9", 2)] {
        let bytes = storage
            .read(
                &crysirror_core::path::VfsPath::new(
                    crysirror_core::path::Mount::Save,
                    &format!("state-{mode}.ksd"),
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(&bytes[..5], &[0xfe, 0xfe, header, 0xff, 0xfe]);
        assert!(
            crysirror_resource::text_stream::decode(&bytes, 1 << 20)
                .unwrap()
                .unwrap()
                .contains("42")
        );
    }
}

#[test]
fn storage_scripts_use_the_explicit_third_context_in_split_class_bodies() {
    let (dir, mut runtime) = fixture(
        r#"
        class Base {var table=%[];}
        class Child extends Base {
            Scripts.execStorage('part.tjs', , this);
            function Child(){}
        }
        var object=new Child();
        if(object.answer()!=42||Scripts.evalStorage('expression.tjs', , object)!=42)throw new Exception('storage class context');
        var denied=false;try{Scripts.execStorage('part.tjs','',4);}catch(e){denied=true;}if(!denied)throw new Exception('invalid context');
        Debug.message('storage-context-complete');
    "#,
    );
    std::fs::write(
        dir.path().join("assets/part.tjs"),
        "table['entry']=41;function answer(){return table['entry']+1;}",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("assets/expression.tjs"),
        "this.table['entry']+1",
    )
    .unwrap();
    runtime.tick(0, InputFrame::default()).unwrap();
    assert!(
        runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("storage-context-complete")
    );
}

#[test]
fn native_application_focus_defaults_are_null_and_edges_use_shared_host_focus() {
    let (_dir, mut runtime) = fixture(
        r#"
        if(System.onActivate!==null||System.onDeactivate!==null)throw new Exception('native focus defaults');
        global.systemEvents=0;global.windowEvents=0;
        System.onActivate=function{if(this===System)throw new Exception('application callback receiver');global.systemEvents++;Debug.message('system-focus',global.systemEvents);} incontextof null;
        System.onDeactivate=System.onActivate;
        class W extends Window {
            function W(){super.Window();}
            function onActivate(){global.windowEvents++;Debug.message('window-focus',global.windowEvents);}
            function onDeactivate(){global.windowEvents++;Debug.message('window-focus',global.windowEvents);}
        }
        var window=new W();Debug.message('focus-ready');
    "#,
    );
    runtime.host.window_focused = Some(true);
    runtime.tick(0, InputFrame::default()).unwrap();
    runtime.host.window_focused = Some(false);
    runtime.tick(0, InputFrame::default()).unwrap();
    runtime.tick(0, InputFrame::default()).unwrap();
    runtime.host.window_focused = Some(true);
    runtime.tick(0, InputFrame::default()).unwrap();
    runtime.host.window_focused = None;
    runtime.tick(0, InputFrame::default()).unwrap();
    let logs = runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(
        logs.contains("system-focus 2") && logs.contains("window-focus 2"),
        "{logs}"
    );
    assert!(
        !logs.contains("system-focus 3") && !logs.contains("window-focus 3"),
        "{logs}"
    );
}

#[test]
fn monitor_info_preserves_original_hresult_truthiness_and_null_handle_policy() {
    let (_dir, mut runtime) = fixture(
        r#"
        Plugins.link('windowEx.dll');
        var plugin=%[];
        var nearest=System.getMonitorInfo(true,plugin);
        if(nearest.monitor.w!=32||nearest.work.h!=32)throw new Exception('nearest NULL monitor');
        if(System.getMonitorInfo(false,plugin)!==void)throw new Exception('NULL without nearest');
        var bad=false;try{System.getMonitorInfo(true,42);}catch(e){bad=true;}if(!bad)throw new Exception('primitive monitor argument');
        bad=false;try{System.getMonitorInfo(true,%[HWND:42]);}catch(e){bad=true;}if(!bad)throw new Exception('foreign window capability');
        var window=new Window();if(System.getMonitorInfo(false,window).primary!=1)throw new Exception('owned window display');
        Debug.message('monitor-reference-quirk-complete');
    "#,
    );
    runtime.tick(0, InputFrame::default()).unwrap();
    assert!(
        runtime.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("monitor-reference-quirk-complete")
    );
}

#[test]
#[ignore="requires an explicitly supplied runtime font; no font files enter the repository"]
fn native_addfont_registers_engine_resource_before_continuation_and_uses_shared_metrics() {
    use crysirror_core::{service::HostServices,state::ClientId};
    let (dir,mut runtime)=fixture(r#"
        Plugins.link('addFont.dll');
        if(System.addFont('runtime.ttf')!=1)throw new Exception('registration count');
        if(System.addFont('runtime.ttf')!=1)throw new Exception('repeat registration');
        var font=new Font();font.height=-20;font.face='unavailable family';
        global.measured=font.getTextWidth('ABC');
        if(global.measured<=0)throw new Exception('shared font not published');
        var window=new Window();window.visible=true;
        var root=new Layer(window,null);root.setSize(32,32);root.setImageSize(32,32);root.visible=true;
        root.font.height=-20;root.drawText(0,0,'ABC',0xffffff);
        Debug.message('dynamic-native-font-complete:'+global.measured);
    "#);
    std::fs::copy(std::env::var("CRYSIRROR_TEST_FONT").unwrap(),dir.path().join("assets/runtime.ttf")).unwrap();
    let start=std::time::Instant::now();
    loop {
        runtime.tick(1,InputFrame::default()).unwrap();
        let log=runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
        if log.contains("dynamic-native-font-complete:") {
            let lib=runtime.host.font_library(ClientId(1)).unwrap();
            let info=lib.resolve("runtime.ttf").unwrap().unwrap();
            let metric=runtime.host.text_measurement(ClientId(1)).unwrap().measure_simple(&info.alias,20.,"ABC").unwrap();
            assert!(log.contains(&format!("dynamic-native-font-complete:{}",metric.advance.ceil() as i64)),"{log}");
            assert_eq!(runtime.assets.text.for_client(ClientId(1)).font_digest(&info.alias).unwrap(),info.digest);
            return;
        }
        assert!(start.elapsed()<std::time::Duration::from_secs(10),"{}",runtime.inspect());
        std::thread::yield_now();
    }
}

#[test]
fn live_transition_endpoint_sizes_publish_in_one_engine_layer_batch() {
    use crysirror_core::state::ClientId;
    let (_dir, mut runtime) = fixture(r#"
        var w = new Window(); w.visible = true;
        var root = new Layer(w,null); root.visible = true;
        var a = new Layer(w,root); a.visible = true;
        var b = new Layer(w,root); b.visible = false;
        a.beginTransition("crossfade",true,b,%[time:500]);
        a.setSize(96,64); b.setSize(96,64);
    "#);
    runtime.tick(0,InputFrame::default()).unwrap();
    let layers = &runtime.host.engine.layers;
    assert_eq!(layers.get(ClientId(1),4).unwrap().rect[2..], [96.,64.]);
    assert_eq!(layers.get(ClientId(1),6).unwrap().rect[2..], [96.,64.]);
    assert!(layers.crossfade(ClientId(1),4).is_some());
    runtime.tick(16,InputFrame::default()).unwrap();
}

#[test]
fn positive_native_alpha_hits_use_prepared_shared_image_coverage() {
    use crysirror_core::service::buttons;
    let (dir,mut runtime)=fixture(r#"
        var w=new Window();w.visible=true;var root=new Layer(w,null);
        class Hit extends Layer {
            function Hit(w,p){super.Layer(w,p);}
            function onClick(){Debug.message("alpha-click");}
        }
        var sprite=new Hit(w,root);sprite.visible=true;
        sprite.hitType=htMask;sprite.hitThreshold=16;sprite.loadImages("alpha.png");
        Debug.message("alpha-ready");
    "#);
    let mut image=image::RgbaImage::new(2,1);image.put_pixel(0,0,image::Rgba([240,0,0,0]));image.put_pixel(1,0,image::Rgba([240,0,0,255]));
    image.save(dir.path().join("assets/alpha.png")).unwrap();
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(10);
    loop {
        runtime.tick(1,InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string().contains("alpha-ready"){break;}
        assert!(std::time::Instant::now()<deadline,"image preparation timeout");std::thread::yield_now();
    }
    for x in [0,1] {
        runtime.tick(1,InputFrame{buttons_down:buttons::MOUSE_LEFT,buttons_pressed:0,pointer_x:x,pointer_y:0}).unwrap();
        runtime.tick(1,InputFrame{buttons_down:0,buttons_pressed:0,pointer_x:x,pointer_y:0}).unwrap();
    }
    let log=runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert_eq!(log.matches("alpha-click").count(),1,"{log}");
}

#[test]
fn temporary_image_alpha_preparation_preserves_copied_immutable_asset() {
    use crysirror_core::{
        image_processing::{ImageInput, ImageProcessState},
        service::{HostServices, buttons},
        state::ClientId,
    };
    use crysirror_scene::{canvas::CanvasNode, layer::Paint};
    let (dir, mut runtime) = fixture(r#"
        var w=new Window();w.visible=true;var root=new Layer(w,null);
        class Hit extends Layer {
            function Hit(w,p){super.Layer(w,p);}
            function onClick(x,y){Debug.message("replacement-alpha-click",x);}
        }
        var temp=new Hit(w,root);temp.visible=true;
        temp.hitType=htMask;temp.hitThreshold=16;temp.loadImages("a.png");
        var dest=new Layer(w,root);dest.setImageSize(2,1);dest.setSize(2,1);
        dest.setPos(4,4);dest.visible=true;dest.copyRect(0,0,temp,0,0,2,1);
        temp.loadImages("b.png");
        Debug.message("immutable-copy-ready");
    "#);
    let a = [210, 20, 30, 0, 20, 220, 30, 255];
    let b = [30, 40, 220, 255, 220, 40, 20, 0];
    for (name, bytes) in [("a.png", a), ("b.png", b)] {
        image::RgbaImage::from_raw(2, 1, bytes.to_vec()).unwrap()
            .save(dir.path().join("assets").join(name)).unwrap();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        runtime.tick(1, InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string()
            .contains("immutable-copy-ready") { break; }
        assert!(std::time::Instant::now() < deadline, "image preparation timeout");
        std::thread::yield_now();
    }
    fn image_assets(nodes: &[CanvasNode]) -> Vec<String> {
        let mut assets=Vec::new();
        for node in nodes {
            match node {
                CanvasNode::Draw { layer } => if let Paint::Image { asset, .. } = &layer.paint {
                    assets.push(asset.clone());
                },
                CanvasNode::Group { nodes, .. } => assets.extend(image_assets(nodes)),
            }
        }
        assets
    }
    let copied = runtime.host.engine.canvases.surface(ClientId(1), 3).unwrap();
    let assets = image_assets(&copied.nodes);
    assert_eq!(assets.len(), 1, "one copied immutable image paint");
    let asset = &assets[0];
    assert_eq!(asset, "a.png", "copy must retain A's immutable asset, not temp's live slot");
    let temporary = runtime.host.engine.canvases.surface(ClientId(1), 2).unwrap();
    assert_eq!(image_assets(&temporary.nodes), ["b.png"], "temporary Layer must now display distinct B");
    let metadata = runtime.host.image_metadata(ClientId(1)).unwrap();
    let mut pixels = metadata.begin_process(ImageInput::Asset {
        path: asset.clone(), size: [2, 1], fingerprint: Some(metadata.fingerprint(asset).unwrap()),
    }, std::sync::Arc::from(&b"[]"[..])).unwrap();
    loop {
        match pixels.poll().unwrap() {
            ImageProcessState::Ready(image) => {
                assert_eq!(&*image.rgba(), &a);
                assert_eq!(image.alpha_at([0,0]), Some(0));
                assert_eq!(image.alpha_at([1,0]), Some(255));
                break;
            }
            ImageProcessState::Failed(e) => panic!("copied immutable A failed: {e}"),
            ImageProcessState::Pending => {
                assert!(std::time::Instant::now() < deadline, "copied A pixel preparation timeout");
                std::thread::yield_now();
            }
        }
    }
    // The native hit path must use the separately prepared B coverage after
    // reload: its first pixel is opaque, its second transparent (opposite A).
    for x in [0, 1] {
        runtime.tick(1, InputFrame { buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed:0, pointer_x:x, pointer_y:0 }).unwrap();
        runtime.tick(1, InputFrame { buttons_down:0, buttons_pressed:0,
            pointer_x:x, pointer_y:0 }).unwrap();
    }
    let log=runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert_eq!(log.matches("replacement-alpha-click").count(), 1, "{log}");
    assert!(log.contains("replacement-alpha-click 0"), "B's opaque first pixel: {log}");
}

fn mouse_fixture(extra: &str) -> (tempfile::TempDir, Runtime) {
    fixture(&format!(r#"
        class W extends Window {{
            function W(){{super.Window();}}
            function onKeyDown(key,shift){{if(key==32){{{extra}}}}}
            function onMouseMove(x,y,shift){{if(global.window_trace)note("window:move:"+x);}}
            function onMouseDown(x,y,button,shift){{if(global.window_trace)note("window:down:"+x);}}
            function onMouseUp(x,y,button,shift){{if(global.window_trace)note("window:up:"+x);}}
            function onClick(x,y){{if(global.window_trace)note("window:click:"+x);}}
        }}
        var w=new W();w.visible=true;var root=new Layer(w,null);
        var branch=new Layer(w,root);branch.visible=true;
        function note(s){{Debug.message("mouse:"+s);}}
        global.mutate=false;global.cancel=false;global.window_trace=false;
        class H extends Layer {{
            var tag;
            function H(w,p,n,x){{super.Layer(w,p);tag=n;setSize(8,8);setImageSize(8,8);left=x;visible=true;hitThreshold=0;}}
            function onMouseEnter(){{
                note(tag+":enter");super.onMouseEnter();
                if(global.mutate&&tag=="C"){{visible=false;b.visible=true;}}
                else if(global.mutate&&tag=="B"){{visible=false;c.visible=true;}}
            }}
            function onMouseLeave(){{
                note(tag+":leave");super.onMouseLeave();
                if(global.mutate&&tag=="A"){{b.visible=false;c.visible=true;}}
            }}
            function onMouseMove(x,y,shift){{note(tag+":move:"+x);}}
            function onMouseDown(x,y,button,shift){{note(tag+":down");if(global.cancel)releaseCapture();}}
            function onMouseUp(x,y,button,shift){{note(tag+":up:"+x);}}
            function onClick(x,y){{note(tag+":click:"+x);}}
        }}
        var a=new H(w,root,"A",0);var b=new H(w,root,"B",12);
        var c=new H(w,root,"C",12);c.visible=false;
    "#))
}
fn mouse_tick(runtime:&mut Runtime,x:i32,down:bool,key:bool) {
    use crysirror_core::service::buttons;
    runtime.tick(1,InputFrame { pointer_x:x,pointer_y:1,
        buttons_down:if down {buttons::MOUSE_LEFT}else{0},
        buttons_pressed:if key {buttons::SPACE}else{0} }).unwrap();
}
fn mouse_events(runtime:&Runtime)->Vec<String> {
    runtime.inspect()["sessions"][0]["native"]["log_tail"].as_array().unwrap()
        .iter().rev().filter_map(|v|v.as_str()?.strip_prefix("mouse:").map(str::to_owned)).collect()
}
#[test]
fn native_hover_orders_cross_layer_events_without_repeating_same_layer_enter() {
    let (_dir,mut r)=mouse_fixture("");
    mouse_tick(&mut r,1,false,false);mouse_tick(&mut r,2,false,false);
    mouse_tick(&mut r,14,false,false);mouse_tick(&mut r,-1,false,false);
    assert_eq!(mouse_events(&r),["A:enter","A:move:1","A:move:2","A:leave","B:enter","B:move:2","B:leave"]);
}
#[test]
fn native_capture_routes_drag_and_click_before_up_using_press_coordinates() {
    let (_dir,mut r)=mouse_fixture("global.window_trace=true;");
    mouse_tick(&mut r,-1,false,true);
    mouse_tick(&mut r,1,true,false);mouse_tick(&mut r,15,true,false);mouse_tick(&mut r,15,false,false);
    assert_eq!(mouse_events(&r),["window:move:1","A:enter","A:move:1","window:down:1","A:down","window:move:15","A:move:15","window:click:1","A:click:1","window:up:15","A:up:15","A:leave","B:enter"]);
    let (_dir,mut cancel)=mouse_fixture("global.cancel=true;");
    mouse_tick(&mut cancel,-1,false,true);mouse_tick(&mut cancel,1,true,false);
    mouse_tick(&mut cancel,15,true,false);mouse_tick(&mut cancel,15,false,false);
    let events=mouse_events(&cancel);
    assert!(!events.iter().any(|e|e.contains(":click:")),"releaseCapture must suppress native click: {events:?}");
    assert!(events.contains(&"B:up:3".into()),"releaseCapture must route subsequent Up by hit: {events:?}");
    let (_dir,mut deactivated)=mouse_fixture("");
    deactivated.host.window_focused=Some(true);
    mouse_tick(&mut deactivated,1,true,false);
    deactivated.host.window_focused=Some(false);
    mouse_tick(&mut deactivated,15,true,false);mouse_tick(&mut deactivated,15,false,false);
    let events=mouse_events(&deactivated);
    assert!(events.contains(&"B:up:3".into()),"deactivation releases native capture: {events:?}");
    assert!(!events.iter().any(|e|e.starts_with("A:up:")||e.contains(":click:")),"deactivated capture cannot receive Up/Click: {events:?}");
    // WindowForm has one LastMouseDown point shared by all mouse buttons.
    // Right Down during left capture changes the following left Click point.
    use crysirror_core::service::buttons;
    let (_dir,mut chord)=mouse_fixture("");
    for (x,down) in [(1,buttons::MOUSE_LEFT),(15,buttons::MOUSE_LEFT|buttons::MOUSE_RIGHT),
        (15,buttons::MOUSE_RIGHT),(15,0)] {
        chord.tick(1,InputFrame {pointer_x:x,pointer_y:1,buttons_down:down,buttons_pressed:0}).unwrap();
    }
    let events=mouse_events(&chord);
    assert!(!events.iter().any(|e|e.contains(":click:")),"shared last Down point is over B while A captures: {events:?}");
    assert_eq!(events.iter().filter(|e|*e=="A:up:15").count(),2,"capture survives until all buttons release: {events:?}");
}
#[test]
fn native_idle_recheck_updates_hover_after_layer_motion_without_synthetic_move() {
    let (_dir,mut r)=mouse_fixture("a.left=12;b.left=0;");
    mouse_tick(&mut r,1,false,false);mouse_tick(&mut r,1,false,true);
    assert_eq!(mouse_events(&r),["A:enter","A:move:1"]);
    r.tick(1001,InputFrame {pointer_x:1,pointer_y:1,..Default::default()}).unwrap();
    assert_eq!(mouse_events(&r),["A:enter","A:move:1","A:leave","B:enter"]);
}
#[test]
fn native_hover_rechecks_after_handlers_once_and_capture_survives_hide_but_not_part() {
    let (_dir,mut r)=mouse_fixture("global.mutate=true;");
    mouse_tick(&mut r,1,false,true);mouse_tick(&mut r,14,false,false);
    assert_eq!(mouse_events(&r),["A:enter","A:move:1","A:leave","C:enter","C:leave","B:enter","B:move:2"]);
    let (_dir,mut hidden)=mouse_fixture("a.visible=false;");
    mouse_tick(&mut hidden,1,true,false);mouse_tick(&mut hidden,1,true,true);
    assert_eq!(mouse_events(&hidden).last().unwrap(),"A:leave");
    mouse_tick(&mut hidden,15,true,false);mouse_tick(&mut hidden,15,false,false);
    let events=mouse_events(&hidden);
    assert!(events.contains(&"A:up:15".into()),"hidden capture still owns Up: {events:?}");
    assert!(!events.iter().any(|e|e=="B:up:3"),"hidden capture may not fall through: {events:?}");
    let (_dir,mut parted)=mouse_fixture("a.parent=branch;");
    mouse_tick(&mut parted,1,true,false);mouse_tick(&mut parted,1,true,true);
    assert_eq!(mouse_events(&parted).last().unwrap(),"A:leave");
    mouse_tick(&mut parted,15,true,false);mouse_tick(&mut parted,15,false,false);
    let events=mouse_events(&parted);
    assert!(events.contains(&"B:up:3".into()),"Part releases capture: {events:?}");
    assert!(!events.iter().any(|e|e.starts_with("A:up:")||e.contains(":click:")),"parted capture may not receive Up/Click: {events:?}");
    let (_dir,mut removed)=mouse_fixture("invalidate a;");
    mouse_tick(&mut removed,1,true,false);mouse_tick(&mut removed,1,true,true);
    mouse_tick(&mut removed,15,true,false);mouse_tick(&mut removed,15,false,false);
    let events=mouse_events(&removed);
    assert!(!events.iter().any(|e|e=="A:leave"||e.starts_with("A:up:")||e.starts_with("A:click:")),"Shutdown suppresses callbacks: {events:?}");
    assert!(events.contains(&"B:up:3".into()));
}

#[test]
fn layer_movie_metadata_frames_loop_stop_and_retirement_use_shared_media() {
    use crysirror_core::service::buttons;
    let (dir,mut runtime)=fixture(r#"
        Plugins.link("layerExMovie.dll");
        var w=new Window();w.visible=true;var root=new Layer(w,null);
        class Movie extends Layer {
            var starts=0,stops=0,updates=0;
            function Movie(w,p){super.Layer(w,p);}
            function onStartMovie(){starts++;Debug.message("movie-start");}
            function onUpdateMovie(){updates++;}
            function onStopMovie(){stops++;Debug.message("movie-stop");}
        }
        var m=new Movie(w,root);
        var rejected=false;try{m.openMovie("movie.mpg",true);}catch(e){rejected=true;}
        if(!rejected)throw new Exception("side-by-side alpha was silently admitted");
        m.openMovie("movie.mpg",false);
        if(m.imageWidth!=64||m.imageHeight!=48||m.type!=ltOpaque)throw new Exception("movie metadata geometry");
        m.startMovie(true);if(!m.isPlayingMovie()||m.starts!=1)throw new Exception("movie start state");
        System.addContinuousHandler(function(tick){
            if(System.getKeyState(32)&&m.isPlayingMovie()){
                m.stopMovie();if(m.stops!=1||m.isPlayingMovie()||m.updates<1)throw new Exception("movie stop state");
                invalidate m;Debug.message("movie-retired");
            }
        });
    "#);
    std::fs::write(dir.path().join("assets/movie.mpg"),include_bytes!("../../crysirror-video/tests/fixtures/red_blue.mpg")).unwrap();
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(15);
    loop {
        runtime.tick(16,InputFrame::default()).unwrap();
        let state=runtime.inspect();
        if state["media"]["resources"].as_array().is_some_and(|resources|resources.iter().any(|r|r["id"]==640&&r["status"]["loops_completed"].as_u64().unwrap_or(0)>0)){break;}
        assert!(std::time::Instant::now()<deadline,"shared movie did not publish and loop: {}",state["media"]);std::thread::yield_now();
    }
    runtime.tick(16,InputFrame{buttons_down:buttons::SPACE,buttons_pressed:0,pointer_x:0,pointer_y:0}).unwrap();
    runtime.tick(16,InputFrame::default()).unwrap();
    let state=runtime.inspect();let log=state["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("movie-retired"),"{log}");
    assert!(!state["media"]["resources"].as_array().unwrap().iter().any(|r|r["id"]==640),"{}",state["media"]);
}

#[test]
fn layer_movie_real_eof_sends_one_stop_after_the_last_picture() {
    let (dir,mut runtime)=fixture(r#"
        Plugins.link("layerExMovie.dll");var w=new Window();w.visible=true;var root=new Layer(w,null);
        class Movie extends Layer {
            var updates=0,stops=0;
            function Movie(w,p){super.Layer(w,p);}
            function onUpdateMovie(){if(!isPlayingMovie())throw new Exception("last frame saw early stop");updates++;}
            function onStopMovie(){stops++;if(stops!=1||updates<1||isPlayingMovie())throw new Exception("EOF policy");Debug.message("movie-eof");}
        }
        var m=new Movie(w,root);m.openMovie("movie.mpg",false);m.startMovie(false);
    "#);
    std::fs::write(dir.path().join("assets/movie.mpg"),include_bytes!("../../crysirror-video/tests/fixtures/red_blue.mpg")).unwrap();
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(15);
    loop {
        runtime.tick(16,InputFrame::default()).unwrap();
        if runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string().contains("movie-eof"){break;}
        assert!(std::time::Instant::now()<deadline,"media EOF callback did not arrive");std::thread::yield_now();
    }
    for _ in 0..4{runtime.tick(16,InputFrame::default()).unwrap();}
    let state=runtime.inspect();let log=state["sessions"][0]["native"]["log_tail"].to_string();assert_eq!(log.matches("movie-eof").count(),1);
    assert_eq!(state["media"]["resources"][0]["status"]["state"],"stopped");
}

#[test]
fn composite_alpha_hits_wait_for_shared_workers_and_refresh_after_copy() {
    use crysirror_core::{service::buttons, state::ClientId};
    let (dir, mut runtime)=fixture(r#"
        var w=new Window();w.visible=true;var root=new Layer(w,null);
        class Hit extends Layer {
            function Hit(w,p){super.Layer(w,p);}
            function onClick(x,y){Debug.message('composite-hit:'+x);}
        }
        global.a=new Layer(w,root);a.loadImages('a.png');
        global.b=new Layer(w,root);b.loadImages('b.png');
        global.dest=new Hit(w,root);dest.setSize(4,1);dest.setImageSize(4,1);
        dest.hitType=htMask;dest.hitThreshold=128;dest.visible=true;
        dest.copyRect(1,0,a,0,0,2,1);
        w.onKeyDown=function(key,shift){
            if(key==32){dest.copyRect(1,0,b,0,0,2,1);Debug.message('composite-changed');}
        };
        Debug.message('composite-ready');
    "#);
    for (name, alpha) in [("a.png",[0,255]),("b.png",[255,0])] {
        let bytes=vec![200,10,20,alpha[0],20,200,30,alpha[1]];
        image::RgbaImage::from_raw(2,1,bytes).unwrap().save(dir.path().join("assets").join(name)).unwrap();
    }
    fn log(runtime:&Runtime)->String {
        runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string()
    }
    let until=std::time::Instant::now()+std::time::Duration::from_secs(10);
    while !log(&runtime).contains("composite-ready") {
        runtime.tick(1,InputFrame::default()).unwrap();
        assert!(std::time::Instant::now()<until);std::thread::yield_now();
    }
    let before=serde_json::to_value(runtime.host.engine.canvases.surface(ClientId(1),4).unwrap()).unwrap();
    fn deliver(runtime:&mut Runtime, down:u64,x:i32) {
        runtime.tick(1,InputFrame{buttons_down:down,buttons_pressed:0,pointer_x:x,pointer_y:0}).unwrap();
        // Input remains queued while coverage is pending; stable samples may
        // coalesce but their press/release edges must be consumed exactly once.
        for _ in 0..200 {
            runtime.tick(1,InputFrame{buttons_down:down,buttons_pressed:0,pointer_x:x,pointer_y:0}).unwrap();
            std::thread::yield_now();
        }
    }
    for x in [0,1,2,3] {deliver(&mut runtime,buttons::MOUSE_LEFT,x);deliver(&mut runtime,0,x);}
    let first=log(&runtime);
    assert_eq!(first.matches("composite-hit:").count(),1,"{first}");
    assert!(first.contains("composite-hit:2"),"{first}");
    let after=serde_json::to_value(runtime.host.engine.canvases.surface(ClientId(1),4).unwrap()).unwrap();
    assert_eq!(before,after,"coverage must not replace the retained composite canvas");
    deliver(&mut runtime,buttons::SPACE,3);deliver(&mut runtime,0,3);
    assert!(log(&runtime).contains("composite-changed"));
    for x in [1,2] {deliver(&mut runtime,buttons::MOUSE_LEFT,x);deliver(&mut runtime,0,x);}
    let second=log(&runtime);
    assert_eq!(second.matches("composite-hit:").count(),2,"{second}");
    assert!(second.contains("composite-hit:1"),"updated graph must invalidate old coverage: {second}");
}

#[test]
fn province_copy_uses_shared_region_map_and_preserves_native_absent_source_policy() {
    let (_dir,mut runtime)=fixture(r#"
        var w=new Window();w.visible=true;var root=new Layer(w,null);
        var source=new Layer(w,root);source.setImageSize(4,2);source.face=dfProvince;
        source.fillRect(0,0,4,2,7);source.fillRect(1,0,1,2,0);
        var dest=new Layer(w,root);dest.setImageSize(6,2);dest.face=dfProvince;
        dest.fillRect(0,0,6,2,9);dest.setClip(2,0,3,2);
        dest.copyRect(1,0,source,0,0,4,2);
        if(dest.getProvincePixel(0,0)!=9||dest.getProvincePixel(1,0)!=9||dest.getProvincePixel(2,0)!=0||dest.getProvincePixel(3,0)!=7||dest.getProvincePixel(4,0)!=7||dest.getProvincePixel(5,0)!=9)throw new Exception('clipped identifiers');
        var empty=new Layer(w,root);empty.setImageSize(6,2);
        dest.setClip();dest.copyRect(4,0,empty,1,0,1,1);
        if(dest.getProvincePixel(1,0)!=0||dest.getProvincePixel(4,0)!=7)throw new Exception('native absent source coordinates');
        dest.copyRect(1,1,dest,0,0,5,1);
        if(dest.getProvincePixel(1,1)!=9||dest.getProvincePixel(2,1)!=0||dest.getProvincePixel(4,1)!=7)throw new Exception('overlapping snapshot copy');
        var assigned=new Layer(w,root);assigned.assignImages(dest);assigned.setImageSize(8,3);
        if(assigned.getProvincePixel(4,0)!=7||assigned.getProvincePixel(7,2)!=0)throw new Exception('assign and resize identifiers');
        dest.fillRect(0,0,6,2,0);
        assigned.face=dfProvince;assigned.copyRect(4,0,dest,0,0,1,1);
        if(assigned.getProvincePixel(0,0)!=0||assigned.getProvincePixel(4,0)!=7)throw new Exception('full zero fill deallocates native plane');
        Debug.message('province-copy-complete');
    "#);
    runtime.tick(1,InputFrame::default()).unwrap();
    assert!(runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string().contains("province-copy-complete"));
}

#[test]
fn native_focus_defaults_forward_action_and_subclass_super_handlers() {
    let (_dir,mut runtime)=fixture(r#"
        var w=new Window();w.visible=true;var root=new Layer(w,null);
        var events=[];
        w.action=function(ev){
            var other=ev.type=='onBlur'?ev.focused:ev.blurred;
            events.add(ev.type+':'+ev.target.name+':'+(other===null?'null':other.name)+(ev.type=='onFocus'?':'+ev.direction:''));
        };
        class Focusable extends Layer {
            function Focusable(w,p){super.Layer(w,p);visible=true;focusable=true;}
            function onFocus(){super.onFocus(...);}
            function onBlur(){super.onBlur(...);}
        }
        var a=new Focusable(w,root);a.name='A';var b=new Focusable(w,root);b.name='B';
        a.focus();b.focus();w.focusedLayer=null;
        if(events.join('|')!='onFocus:A:null:1|onBlur:A:B|onFocus:B:A:1|onBlur:B:null')throw new Exception(events.join('|'));
        Debug.message('focus-default-action-complete');
    "#);
    runtime.tick(1,InputFrame::default()).unwrap();
    assert!(runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string().contains("focus-default-action-complete"));
}
