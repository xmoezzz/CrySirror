// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Self-authored native solid-brush paths -> shared vector worker -> resources.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;
fn fixture(source: &str) -> (tempfile::TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    std::fs::write(d.path().join("assets/startup.tjs"), source).unwrap();
    let p=serde_json::from_value(json!({"schema":2,"name":"vector plugin","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-vector","config":{"canvas":[32,32]}}]})).unwrap();
    let path = d.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut a = AdapterRegistry::default();
    crysirror_krkr::register(&mut a).unwrap();
    let r = Runtime::with_registry(p, &path, &d.path().join("state"), a).unwrap();
    (d, r)
}
#[test]
fn ordered_brushes_affine_append_and_path_bounds_use_shared_vector_workers() {
    let (_d, mut r) = fixture(
        r#"
 Plugins.link('layerExDraw.dll');if(Plugins.getStatus('layerExDraw.dll').kind!='engine_facade')throw new Exception('fake facade');
 var w=new Window();var root=new Layer(w,null);var l=new Layer(w,root);l.setImageSize(12,12);l.clear(0);
 var app=new GdiPlus.Appearance();app.addBrush(0xffff0000,0,0);app.addBrush(0xff0000ff,1,0);
 var bad=false;try{app.addPen(0xffffffff,1,0,0);}catch(e){bad=true;}if(!bad)throw new Exception('fake pen');
 l.scaleTransform(2,2);l.translateTransform(1,1);l.translateViewTransform(0,1);
 var a=l.drawRectangle(app,1,1,2,2);
 if(a.x!=3||a.y!=4||a.width!=6||a.height!=4||a.right!=9||a.bottom!=8)throw new Exception('native bounds/order');
 l.resetTransform();l.resetViewTransform();l.smoothingMode=2;if(l.smoothingMode!=2)throw new Exception('mode state');
 bad=false;try{l.smoothingMode=3;}catch(e){bad=true;}if(!bad||l.smoothingMode!=2)throw new Exception('mode rejection');
 app.clear();app.addBrush(0xff00ff00,0,0);var p=new GdiPlus.Path();p.drawRectangle(0,0,3,3);p.drawRectangle(1,1,1,1);
 var b=l.drawPath(app,p);if(b.x!=0||b.y!=0||b.width!=3||b.height!=3)throw new Exception('path bounds');
 var line=new GdiPlus.Path();line.startFigure();line.drawLine(0,9,2,9);line.drawLine(2,9,2,11);line.closeFigure();
 var c=l.drawPath(app,line);if(c.x!=0||c.y!=9||c.width!=2||c.height!=2)throw new Exception('connected line figure');
 Debug.message('shared-vectors-complete');
 "#,
    );
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        r.tick(1, InputFrame::default()).unwrap();
        if r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("shared-vectors-complete")
        {
            break;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::yield_now()
    }
    let p = r
        .assets
        .surface(ClientId(1), "__surface/1073741826")
        .unwrap();
    let at = |x: usize, y: usize| &p.rgba[(y * 12 + x) * 4..(y * 12 + x) * 4 + 4];
    assert_eq!(at(3, 4), &[255, 0, 0, 255]);
    assert_eq!(at(5, 4), &[0, 0, 255, 255]);
    assert_eq!(at(8, 7), &[0, 0, 255, 255]);
    assert_eq!(at(9, 7), &[0; 4]);
    assert_eq!(at(0, 0), &[0, 255, 0, 255]);
    assert_eq!(at(1, 1), &[0; 4]);
    assert!(at(1, 9)[3] > 0);
    assert_eq!(
        r.inspect()["sessions"][0]["native"]["owns_framebuffer"],
        false
    );
}
#[test]
fn native_invalidation_releases_appearance_state_even_with_script_finalizer() {
    let (_d, mut r) = fixture(
        r#"
 Plugins.link('layerExDraw.dll');
 for(var i=0;i<300;i++){var a=new GdiPlus.Appearance();a.addBrush(0xffffffff,0,0);a.finalize=function(){};invalidate a;}
 var app=new GdiPlus.Appearance();app.addBrush(0xff123456,0,0);
 var w=new Window();var root=new Layer(w,null);var l=new Layer(w,root);l.update=function(){};
 var bad=false;try{l.clear(0);}catch(e){bad=true;}if(!bad)throw new Exception('script update bypassed');
 Debug.message('appearance-release-ok');
 "#,
    );
    r.tick(0, InputFrame::default()).unwrap();
    assert!(
        r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("appearance-release-ok")
    );
}

#[test]
fn native_method_override_before_plugin_link_is_not_mistaken_for_builtin_update() {
    let (_d,mut r)=fixture(r#"
    var original=Layer.update;Layer.update=Layer.fillRect;
    Plugins.link('layerExDraw.dll');var w=new Window();var root=new Layer(w,null);var l=new Layer(w,root);
    var bad=false;try{l.clear(0);}catch(e){bad=true;}if(!bad)throw new Exception('native override bypassed');
    Layer.update=original;l.clear(0);Debug.message('pre-link-override-ok');
    "#);
    r.tick(0,InputFrame::default()).unwrap();
    assert!(r.inspect()["sessions"][0]["native"]["log_tail"].to_string().contains("pre-link-override-ok"));
}
