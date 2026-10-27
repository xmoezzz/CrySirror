// SPDX-License-Identifier: MPL-2.0
//! Real TypeScript -> Oxc -> Boa -> shared CPU bitmap -> retained layer flow.
//! Synthetic pixels only: no GPU, window, audio device, fonts or game assets.
use crysirror_core::service::InputFrame;
use crysirror_engine::App;
use crysirror_runtime::Runtime;
use crysirror_script_js::JsHost;

#[test]
fn typescript_bitmaps_publish_shared_pixels_and_feed_transformed_filtered_layers() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("assets")).unwrap();
    let project = serde_json::json!({"schema":2,"name":"bitmap-ts","content_revision":"1","assets":"assets","canvas":[32,32],"clients":[{"name":"game","adapter":"crysirror-script-v1","game":"bitmap-ts"}]});
    std::fs::write(
        tmp.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("state")).unwrap(),
        "game",
    )
    .unwrap();
    let source = r#"
      interface Rect { x:number; y:number; width:number; height:number }
      const one:Rect={x:0,y:0,width:1,height:1};
      function onStart():void {
        const strip:string=bitmap.create(1,3,1,[10,0,0,128]);
        bitmap.fill(1,{x:1,y:0,width:1,height:1},[20,0,0,128]);
        bitmap.fill(1,{x:2,y:0,width:1,height:1},[30,0,0,128]);
        bitmap.create(2,4,2,[1,2,3,77]);
        bitmap.blit(2,strip,{source_rect:{x:-1,y:0,width:4,height:1},destination:[-1,0],mode:'copy_rgb'});
        bitmap.create(3,4,1,[9,9,9,33]);
        const product:string=bitmap.stretch(3,strip,{source_rect:{x:0,y:0,width:3,height:1},destination:{x:-1,y:0,width:4,height:1},sampling:'nearest',mode:'copy_rgb',color_key:[20,0,0]});
        const mask:string=bitmap.create(5,1,1,[64,64,64,128]);
        bitmap.create(4,1,1,[200,100,50,17]);
        bitmap.blit(4,mask,{source_rect:one,destination:[0,0],mode:'mask255'});
        bitmap.create(6,1,1,[200,100,50,17]);
        bitmap.blit(6,mask,{source_rect:one,destination:[0,0],mode:'premultiplied_mask256'});
        const before=bitmap.info(6).revision;
        try {bitmap.blit(6,mask,{source_rect:one,destination:[0,0],mode:'premultiplied_mask256',opacity:128});}
        catch(error){engine.log('fade-rejected');}
        engine.log(String(bitmap.info(6).revision===before));
        bitmap.create(7,1,1,[0,0,255,0]);
        bitmap.blit(7,mask,{source_rect:one,destination:[0,0],mode:'source_over'});
        const self:string=bitmap.load(8,strip);
        bitmap.blit(8,self,{source_rect:{x:0,y:0,width:2,height:1},destination:[1,0]});
        layers.put({id:42,rect:[2,3,4,1],paint:{kind:'image',asset:product}});
        layers.setTransform(42,{scale_x:2,scale_y:3,z:1});
        layers.setFilter(42,{invert:1,mono:0.5,alpha_test:true});
        engine.log(product);
      }
    "#;
    app.add_script_host(Box::new(
        JsHost::with_source("bitmap", "bitmap.ts", source).unwrap(),
    ))
    .unwrap();
    app.init().unwrap();
    app.start().unwrap();
    app.frame(16, InputFrame::default()).unwrap();
    assert_eq!(
        app.engine()
            .unwrap()
            .log
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["fade-rejected", "true", "__surface/3"]
    );
    let state = app.context().state().unwrap();
    let pixels = |slot| {
        state
            .runtime
            .assets
            .surface(state.client, &format!("__surface/{slot}"))
            .unwrap()
            .rgba
            .as_ref()
    };
    assert_eq!(
        pixels(2),
        &[
            10, 0, 0, 77, 20, 0, 0, 77, 30, 0, 0, 77, 1, 2, 3, 77, 1, 2, 3, 77, 1, 2, 3, 77, 1, 2,
            3, 77, 1, 2, 3, 77
        ]
    );
    assert_eq!(
        pixels(3),
        &[10, 0, 0, 33, 9, 9, 9, 33, 30, 0, 0, 33, 9, 9, 9, 33]
    );
    assert_eq!(pixels(4), &[132, 82, 57, 136]);
    assert_eq!(pixels(6), &[164, 114, 89, 17]);
    assert_eq!(pixels(7), &[64, 64, 64, 128]);
    assert_eq!(pixels(8), &[10, 0, 0, 128, 10, 0, 0, 128, 20, 0, 0, 128]);
    let frame = state.runtime.host.engine.layers.frame([32, 32]).unwrap();
    let layer = frame
        .iter()
        .find(|p| p.owner == state.client.0 && p.id == 42)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&layer.paint).unwrap()["asset"],
        "__surface/3"
    );
    assert!((layer.quad[1][0] - layer.quad[0][0] - 8.).abs() < 1e-5);
    assert!((layer.quad[3][1] - layer.quad[0][1] - 3.).abs() < 1e-5);
    let filter = layer.filter.as_ref().unwrap();
    assert_eq!(
        (filter.invert, filter.mono, filter.alpha_test),
        (1., 0.5, true)
    );
}

#[test]
fn typescript_awaits_shared_image_effects_and_transitions_before_presentation() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("assets")).unwrap();
    let p = tmp.path().join("project.json");
    std::fs::write(&p,serde_json::to_vec(&serde_json::json!({"schema":2,"name":"bitmap-effects-ts","content_revision":"1","assets":"assets","canvas":[32,32],"clients":[{"name":"game","adapter":"crysirror-script-v1","game":"test"}]})).unwrap()).unwrap();
    let mut app = App::new(
        Runtime::load(&p, &tmp.path().join("state")).unwrap(),
        "game",
    )
    .unwrap();
    app.add_script_host(Box::new(JsHost::with_source("effects","effects.ts",r#"
      async function onStart():Promise<void> {
        bitmap.create(1,2,1,[128,0,0,255]);
        const image:string=await bitmap.applyEffectAsync(1,[{kind:'gamma',gamma:[2,2,2],floor:[0,0,0],ceiling:[255,255,255]}]);
        bitmap.create(2,2,1,[0,0,255,255]);
        const mixed:string=await bitmap.transitionAsync(1,2,{progress:0.5,softness:0,blur:0});
        layers.put({id:7,rect:[0,0,2,1],paint:{kind:'image',asset:mixed}});
        engine.log('effects-ready');
      }
    "#).unwrap())).unwrap();
    app.init().unwrap();
    app.start().unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.frame(1, InputFrame::default()).unwrap();
        if app
            .engine()
            .unwrap()
            .log
            .iter()
            .any(|l| l == "effects-ready")
        {
            break;
        }
        assert!(
            std::time::Instant::now() < end,
            "{:?}",
            app.engine().unwrap().log
        );
        std::thread::yield_now();
    }
    let state = app.context().state().unwrap();
    let surface = state
        .runtime
        .assets
        .surface(state.client, "__surface/1")
        .unwrap();
    assert_eq!(&*surface.rgba, &[91, 0, 128, 255, 91, 0, 128, 255]);
    assert!(state
        .runtime
        .host
        .engine
        .layers
        .get(state.client, 7)
        .is_some());
}
