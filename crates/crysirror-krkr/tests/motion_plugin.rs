// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or later.
//! Native semantics and engine-owned motion resources; never a legacy framebuffer.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;
fn fixture(script: &str, motion: Option<&[u8]>) -> (tempfile::TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    std::fs::write(d.path().join("assets/startup.tjs"), script).unwrap();
    if let Some(bytes) = motion {
        std::fs::write(d.path().join("assets/motion.mtn"), bytes).unwrap()
    }
    let p=serde_json::from_value(json!({"schema":2,"name":"motion plugin","content_revision":"1","canvas":[1920,1080],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-motion","config":{"canvas":[1920,1080]}}]})).unwrap();
    let path = d.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let runtime = Runtime::with_registry(p, &path, &d.path().join("state"), registry).unwrap();
    (d, runtime)
}
#[test]
fn motion_class_defaults_and_unsupported_profiles_are_explicit() {
    let (_d, mut r) = fixture(
        r#"
Plugins.link('motionplayer_nod3d.dll');
var rm=new Motion.ResourceManager();var p=new Motion.Player(rm);
if(p.playing||p.motion!=''||p.speed!=1||p.opacity!=255)throw new Exception('player defaults');
p.speed=2;p.tickCount=500;if(p.frameTickCount!=30)throw new Exception('clock unit');
p.setVariable('test',3);if(p.getVariable('test')!=3||p.variableKeys[0]!='test')throw new Exception('parameter mapping');
if(p.varibleKeys.count!=1||p.varibleKeys[0]!='test')throw new Exception('legacy parameter keys');
p.setVariable('alpha',2);if(p.varibleKeys.count!=2||p.variableKeys.count!=2)throw new Exception('live parameter keys');
var bad=false;try{p.play('missing',Motion.PlayFlagJoin);}catch(e){bad=true;}if(!bad)throw new Exception('unsupported join accepted');
bad=false;try{p.setVariable('test',4,60);}catch(e){bad=true;}if(!bad)throw new Exception('unsupported tween accepted');
var w=new Window();var base=new Layer(w,null);var child=new Layer(w,base);child.setSize(320,200);child.visible=true;
var a=new Motion.SeparateLayerAdaptor(child incontextof global.Layer);
if(!(a instanceof 'Layer')||a.getSubImageLayers()!==void)throw new Exception('adaptor owner closure');
Debug.message('motion-facade-ok');
"#,
        None,
    );
    r.tick(0, InputFrame::default()).unwrap();
    assert!(r.inspect()["sessions"][0]["native"]["log_tail"]
        .to_string()
        .contains("motion-facade-ok"));
    let child = r.host.engine.layers.get(ClientId(1), 6).unwrap();
    assert_eq!(child.parent, Some(4));
    assert_eq!(child.rect, [0., 0., 320., 200.]);
}
#[test]
#[ignore = "requires owner-provided MOTION_FILE/MOTION_OBJECT/MOTION_NAME; no game data committed"]
fn external_mtn_native_draw_publishes_engine_textures_and_meshes() {
    let bytes = std::fs::read(std::env::var("MOTION_FILE").unwrap()).unwrap();
    let object = serde_json::to_string(&std::env::var("MOTION_OBJECT").unwrap()).unwrap();
    let name = serde_json::to_string(&std::env::var("MOTION_NAME").unwrap()).unwrap();
    let script = format!(
        r#"
Plugins.link('motionplayer_nod3d.dll');
var w=new Window();var base=new Layer(w,null);var target=new Layer(w,base);
target.setImageSize(1920,1080);target.setSizeToImageSize();target.visible=true;
var a=new Motion.SeparateLayerAdaptor(target incontextof global.Layer);target.type=ltBinder;
var rm=new Motion.ResourceManager();var loaded=rm.load('motion.mtn');var p=new Motion.Player(rm);
p.chara={object};p.play({name},Motion.PlayFlagForce);p.progress(500);
p.setDrawAffineTranslateMatrix(1,0,0,1,0,0);p.clear(a,0);p.draw(a);
var syncs=0;p.onSync=function(){{syncs++;}};p.progress(100000);p.progress(100000);
if(p.playing||syncs!=1)throw new Exception('one-shot completion');
rm.unload('motion.mtn');Debug.message('motion-native-complete');w.visible=true;
"#
    );
    let (_d, mut r) = fixture(&script, Some(&bytes));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        r.tick(1, InputFrame::default()).unwrap();
        let frame = r.host.engine.layers.frame([1920, 1080]).unwrap();
        if let Some(p) = frame
            .iter()
            .find(|p| p.puppet.as_ref().is_some_and(|f| !f.parts.is_empty()))
        {
            assert_eq!(
                p.size,
                [1920, 1080],
                "adaptor canvas refit must reach the engine"
            );
            let mesh = p.puppet.as_ref().unwrap();
            mesh.validate().unwrap();
            assert!(mesh.parts.iter().any(|p| !p.indices.is_empty()));
            for texture in &mesh.textures {
                let data = r.assets.read_for(ClientId(1), texture).unwrap();
                assert!(
                    data.starts_with(b"\x89PNG"),
                    "missing engine texture {texture}"
                )
            }
            assert!(r.inspect()["sessions"][0]["native"]["log_tail"]
                .to_string()
                .contains("motion-native-complete"));
            assert_eq!(
                r.inspect()["sessions"][0]["native"]["owns_framebuffer"],
                false
            );
            eprintln!(
                "engine motion publication: parts={} textures={} layer={}",
                mesh.parts.len(),
                mesh.textures.len(),
                p.id
            );
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "motion job did not publish engine geometry: {}",
            r.inspect()
        );
        std::thread::yield_now();
    }
}
