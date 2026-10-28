// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or later.
//! Self-authored native calls publish shared frozen Canvas trees, never pixels.
use crysirror_core::{service::InputFrame, state::ClientId};
use crysirror_runtime::{AdapterRegistry, Runtime};
use crysirror_scene::layer::Paint;
use serde_json::json;
fn fixture(source: &str) -> (tempfile::TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/startup.tjs"), source).unwrap();
    let project=serde_json::from_value(json!({"schema":2,"name":"piled copy","content_revision":"1","canvas":[64,64],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-piled-copy","config":{"canvas":[64,64]}}]})).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let runtime =
        Runtime::with_registry(project, &path, &dir.path().join("state"), registry).unwrap();
    (dir, runtime)
}
#[test]
fn native_piled_copy_freezes_hidden_subtree_in_local_space_after_paint_and_before_source_mutation()
{
    let (_dir, mut runtime) = fixture(
        r#"
var w=new Window();var base=new Layer(w,null);w.visible=true;
var source=new Layer(w,base);source.setImageSize(8,8);source.setSizeToImageSize();
source.setPos(100,100);source.opacity=17;source.visible=false;
var child=new Layer(w,source);child.setImageSize(4,4);child.setSizeToImageSize();child.setPos(1,1);child.visible=true;
var dest=new Layer(w,base);dest.setImageSize(8,8);dest.setSizeToImageSize();dest.setPos(20,20);dest.visible=true;
dest.fillRect(0,0,8,8,0xff0000ff);dest.face=dfMask;dest.holdAlpha=true;dest.setClip(3,4,2,2);
var paints=0;source.onPaint=function(){paints++;child.fillRect(0,0,4,4,0xffff0000);};source.update();
dest.piledCopy(2,3,source incontextof global.Layer,0,0,8,8);
if(paints!=1||!dest.imageModified)throw new Exception('piledCopy completion/dirty semantics');
child.fillRect(0,0,4,4,0xff00ff00);source.setPos(0,0);
Debug.message('piled-static-complete');
"#,
    );
    runtime.tick(0, InputFrame::default()).unwrap();
    let log = runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("piled-static-complete"), "{log}");
    let frame = runtime.host.engine.layers.frame([64, 64]).unwrap();
    let snapshot = frame
        .iter()
        .find(|p| matches!(p.paint, Paint::Snapshot { .. }))
        .expect("isolated frozen snapshot");
    assert_eq!(snapshot.opacity, 255);
    assert_eq!(snapshot.clip, Some([20., 20., 8., 8.]));
    assert_eq!([snapshot.quad[0][0], snapshot.quad[0][1]], [23., 24.]);
    assert_eq!([snapshot.quad[2][0], snapshot.quad[2][1]], [25., 26.]);
    let Paint::Snapshot { size, layers } = &snapshot.paint else {
        unreachable!()
    };
    let mut frozen = crysirror_scene::layer::LayerSystem::default();
    frozen
        .apply_batch(
            ClientId(1),
            &layers
                .iter()
                .cloned()
                .map(|layer| crysirror_scene::layer::LayerCommand::Put { layer })
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let inner = frozen.frame(*size).unwrap();
    let red = inner
        .iter()
        .find(|p| {
            matches!(
                p.paint,
                Paint::Solid {
                    color: [255, 0, 0, 255]
                }
            )
        })
        .expect("frozen source child");
    assert_eq!(red.opacity, 255, "source root opacity must not be baked");
    assert_eq!([red.quad[0][0], red.quad[0][1]], [1., 1.]);
    assert!(!inner.iter().any(|p| matches!(
        p.paint,
        Paint::Solid {
            color: [0, 255, 0, 255]
        }
    )));
    let dest = runtime
        .host
        .engine
        .canvases
        .surface(ClientId(1), 4)
        .unwrap();
    let data = serde_json::to_string(&dest.nodes).unwrap();
    assert!(
        data.contains("[255,0,0,255]") && !data.contains("[0,255,0,255]"),
        "{data}"
    );
    assert_eq!(
        runtime.inspect()["sessions"][0]["native"]["owns_framebuffer"],
        false
    );
}
#[test]
fn piled_copy_rejects_missing_images_and_logs_opaque_sources_but_empty_clip_is_noop() {
    let (_dir, mut runtime) = fixture(
        r#"
var w=new Window();var base=new Layer(w,null);var source=new Layer(w,base);var dest=new Layer(w,base);
dest.setClip(0,0,0,0);dest.imageModified=false;dest.piledCopy(0,0,source,0,0,8,8);
if(dest.imageModified)throw new Exception('empty clip mutated destination');dest.setClip();
var bad=false;try{dest.piledCopy(0,0,null,0,0,1,1);}catch(e){bad=true;}if(!bad)throw new Exception('non-layer accepted');
var raw=new Layer();bad=false;try{dest.piledCopy(0,0,raw,0,0,1,1);}catch(e){bad=true;}if(!bad)throw new Exception('unconstructed accepted');
var child=new Layer(w,source);child.visible=true;source.type=ltOpaque;dest.piledCopy(0,0,source,0,0,1,1);source.type=ltAlpha;
source.freeImage();bad=false;try{dest.piledCopy(0,0,source,0,0,1,1);}catch(e){bad=true;}if(!bad)throw new Exception('missing source image accepted');
source.hasImage=true;dest.freeImage();bad=false;try{dest.piledCopy(0,0,source,0,0,1,1);}catch(e){bad=true;}if(!bad)throw new Exception('missing destination image accepted');
Debug.message('piled-rejections-complete');
"#,
    );
    runtime.tick(0, InputFrame::default()).unwrap();
    let log = runtime.inspect()["sessions"][0]["native"]["log_tail"].to_string();
    assert!(log.contains("piled-rejections-complete"));
    // The composed opaque source is copied with straight alpha, and says so.
    assert!(log.contains("opaque source composed with straight alpha"), "{log}");
}

#[test]
fn raw_opaque_piled_copy_preserves_rgba_without_completion_or_display_opacity() {
    let (dir, mut runtime) = fixture(
        r#"
var w=new Window();var base=new Layer(w,null);w.visible=true;
var source=new Layer(w,base);source.setImageSize(3,1);source.setSizeToImageSize();
source.loadImages("raw.png");
source.type=ltOpaque;source.opacity=17;source.visible=false;
var hidden=new Layer(w,source);hidden.type=ltAdditive;hidden.visible=false;
var paints=0;source.onPaint=function(){paints++;};source.update();
var dest=new Layer(w,base);dest.setImageSize(3,1);dest.setSizeToImageSize();dest.visible=true;dest.type=ltAlpha;dest.hitType=htProvince;
dest.fillRect(0,0,3,1,0xffffffff);dest.face=dfMask;dest.holdAlpha=true;
dest.piledCopy(0,0,source,0,0,3,1);
if(paints!=0)throw new Exception('raw Complete must not call onPaint');
source.fillRect(0,0,3,1,0xff000000);
"#,
    );
    let pixels = vec![17, 34, 51, 0, 68, 85, 102, 128, 119, 136, 153, 255];
    image::RgbaImage::from_raw(3, 1, pixels.clone())
        .unwrap()
        .save(dir.path().join("assets/raw.png"))
        .unwrap();
    runtime.tick(0, InputFrame::default()).unwrap();
    let dest = runtime
        .host
        .engine
        .canvases
        .surface(ClientId(1), 4)
        .unwrap();
    let data = serde_json::to_string(&dest.nodes).unwrap();
    assert!(
        data.contains("raw.png"),
        "raw source image was not retained: {data}"
    );
    let frame = runtime.host.engine.layers.frame([64, 64]).unwrap();
    assert!(
        !frame.iter().any(|p| matches!(
            p.paint,
            Paint::Solid {
                color: [255, 255, 255, 255]
            }
        )),
        "raw copy retained the old destination fill"
    );
    let p = frame
        .iter()
        .find(|p| matches!(&p.paint, Paint::Image { asset, .. } if asset.ends_with("raw.png")))
        .unwrap();
    assert_eq!(p.opacity, 255);
    let Paint::Image { asset, .. } = &p.paint else {
        unreachable!()
    };
    let bytes = runtime.assets.read_for(ClientId(1), asset).unwrap();
    assert_eq!(
        image::load_from_memory(&bytes)
            .unwrap()
            .to_rgba8()
            .into_raw(),
        pixels
    );
}
