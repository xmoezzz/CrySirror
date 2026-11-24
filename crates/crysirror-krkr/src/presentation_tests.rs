// SPDX-License-Identifier: MPL-2.0
use super::*;

#[test]
fn suspended_redraw_retains_previous_presentation_but_modal_can_be_shown() {
    let mut rt = Runtime::with_host(KrkrHost::new([4, 4], "default".into()));
    let global = rt.global_handle();
    rt.register_object_native(global, "awaitImage", |rt: &mut Runtime<KrkrHost>, _, _| {
        rt.host_mut().bitmap_redraw_pending = true;
        rt.request_suspend();
        Ok(Variant::Void)
    });
    natives::run_source(
        &mut rt,
        "redraw.tjs",
        "global.paintStage = 1; awaitImage(); global.paintStage = 2;",
        None,
    )
    .unwrap();
    assert_eq!(rt.global_member("paintStage").to_integer().unwrap(), 1);
    let config = serde_json::from_value(json!({"canvas":[4,4]})).unwrap();
    let mut vm = KrkrVm::new(ClientId(1), config).unwrap();
    vm.rt = Some(rt);
    assert!(
        vm.presentation_pending(),
        "the prefix of a redraw must not be presented"
    );
    vm.rt.as_mut().unwrap().host_mut().dialog_pending = Some(1);
    assert!(
        !vm.presentation_pending(),
        "a waiting modal must be presented to receive input"
    );
    vm.rt.as_mut().unwrap().host_mut().dialog_pending = None;
    vm.rt.as_mut().unwrap().resume_suspended().unwrap();
    vm.rt.as_mut().unwrap().host_mut().bitmap_redraw_pending = false;
    assert_eq!(
        vm.rt
            .as_ref()
            .unwrap()
            .global_member("paintStage")
            .to_integer()
            .unwrap(),
        2
    );
    assert!(
        !vm.presentation_pending(),
        "the completed redraw must publish normally"
    );
}

#[test]
fn ordinary_resource_wait_and_active_frame_do_not_block_startup_presentation() {
    let mut rt = Runtime::with_host(KrkrHost::new([4, 4], "default".into()));
    let global = rt.global_handle();
    rt.register_object_native(global, "awaitImage", |rt: &mut Runtime<KrkrHost>, _, _| {
        rt.request_suspend();
        Ok(Variant::Void)
    });
    natives::run_source(&mut rt, "startup-wait.tjs", "awaitImage();", None).unwrap();
    let mut vm = KrkrVm::new(
        ClientId(1),
        serde_json::from_value(json!({"canvas":[4,4]})).unwrap(),
    )
    .unwrap();
    vm.rt = Some(rt);
    vm.pass = Some(FramePass {
        fence: 0,
        phase: 4,
        handlers: vec![],
        handler: 0,
        now: 0,
        paints: None,
        paint: 0,
    });
    assert!(!vm.presentation_pending());
}

#[test]
fn tiled_overlay_batches_preserve_pixels_and_native_instruction_order() {
    use crysirror_core::{
        image_processing::{ImageInput, ImageJob},
        service::ImageMetadata,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CountImages {
        inner: Arc<dyn ImageMetadata>,
        calls: Arc<AtomicUsize>,
    }
    impl ImageMetadata for CountImages {
        fn size(&self, asset: &str) -> crysirror_core::Result<[u32; 2]> {
            self.inner.size(asset)
        }
        fn begin_process(
            &self,
            input: ImageInput,
            effects: Arc<[u8]>,
        ) -> crysirror_core::Result<ImageJob> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.begin_process(input, effects)
        }
    }
    for callback in [false, true] {
        for hold in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir(dir.path().join("assets")).unwrap();
            std::fs::write(dir.path().join("assets/startup.tjs"), "").unwrap();
            let project:crysirror_runtime::Project=serde_json::from_value(json!({"schema":2,"name":"bitmap batch","content_revision":"1","canvas":[256,4],"assets":"assets","clients":[{"name":"game","adapter":"krkr","game":"synthetic","config":{"canvas":[256,4]}}]})).unwrap();
            std::fs::write(
                dir.path().join("project.json"),
                serde_json::to_vec(&project).unwrap(),
            )
            .unwrap();
            let mut registry = crysirror_runtime::AdapterRegistry::default();
            crate::register(&mut registry).unwrap();
            let mut owner = crysirror_runtime::Runtime::with_registry(
                project,
                &dir.path().join("project.json"),
                &dir.path().join("state"),
                registry,
            )
            .unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let mut host = KrkrHost::new([256, 4], "default".into());
            host.client = 1;
            host.can_suspend = true;
            host.images = Some(Arc::new(CountImages {
                inner: owner.host.image_metadata(ClientId(1)).unwrap(),
                calls: calls.clone(),
            }));
            let mut rt = Runtime::with_host(host);
            natives::install(&mut rt);
            let source = format!(
                r#"
var w=new Window(),root=new Layer(w,null),dest=new Layer(w,root),tile=new Layer(w,root),copied=new Layer(w,root);
dest.setImageSize(130,1);dest.setSizeToImageSize();dest.fillRect(0,0,130,1,0xc8285078);
tile.setImageSize(1,1);tile.setSizeToImageSize();tile.fillRect(0,0,1,1,0xffa0c0e0);
copied.setImageSize(130,1);copied.setSizeToImageSize();
dest.holdAlpha={hold};var written=0;
function paintBatch() {{
for(var x=0;x<130;x++) {{ written++;dest.operateRect(x,0,tile,0,0,1,1,18,255); }}
copied.copyRect(0,0,dest,0,0,130,1);
if(written!=130)throw new Exception('native retry repeated loop side effects');
global.batchDone=true;
}}
if({callback}) {{
    dest.onPaint=paintBatch;dest.update();
    copied.onPaint=function(){{if(!global.batchDone)throw new Exception('paint entered before previous callback finished');global.secondPaintDone=true;}};
    copied.update();
}} else paintBatch();
"#
            );
            natives::run_source(&mut rt, "tile-batch.tjs", &source, None).unwrap();
            let mut vm = KrkrVm::new(
                ClientId(1),
                serde_json::from_value(json!({"canvas":[256,4]})).unwrap(),
            )
            .unwrap();
            vm.rt = Some(rt);
            for _ in 0..2000 {
                if callback {
                    vm.execute(&mut owner.host, InputFrame::default()).unwrap();
                } else {
                    vm.drive_sound(&mut owner.host).unwrap();
                }
                let rt = vm.rt.as_ref().unwrap();
                if rt.global_member("batchDone").is_truthy()
                    && (!callback || rt.global_member("secondPaintDone").is_truthy())
                {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let rt = vm.rt.as_ref().unwrap();
            assert!(
                rt.global_member("batchDone").is_truthy(),
                "native continuation did not finish"
            );
            assert_eq!(
                calls.load(Ordering::SeqCst),
                3,
                "130 small writes use three bounded jobs, not 130 whole-image jobs"
            );
            let copied = rt.global_member("copied").object_handle().unwrap();
            let result = crate::image_api::input(rt, copied).unwrap();
            // The copy follows completion of the final job and retains its exact
            // immutable source even though the original Canvas was never read back.
            let ready = owner.host.image_metadata(ClientId(1)).unwrap();
            let mut job = ready.begin_process(result, Arc::from(&b"[]"[..])).unwrap();
            let mut actual = None;
            for _ in 0..2000 {
                match job.poll().unwrap() {
                    crysirror_core::image_processing::ImageProcessState::Ready(image) => {
                        actual = Some(image.rgba());
                        break;
                    }
                    crysirror_core::image_processing::ImageProcessState::Failed(e) => panic!("{e}"),
                    _ => std::thread::sleep(std::time::Duration::from_millis(1)),
                }
            }
            let mut expected = [40u8, 80, 120, 200].repeat(130);
            use crysirror_image::bitmap::{BitmapRect, BlitOptions, PixelMath, PsBlend};
            for x in 0..130 {
                crysirror_image::bitmap::blit_rgba8(
                    &mut expected,
                    [130, 1],
                    &[160, 192, 224, 255],
                    [1, 1],
                    BlitOptions {
                        source_rect: BitmapRect::new(0, 0, 1, 1),
                        destination: [x, 0],
                        opacity: 255,
                        mode: PixelMath::Ps {
                            blend: PsBlend::Overlay,
                            hold_alpha: hold,
                        },
                        color_key: None,
                    },
                )
                .unwrap();
            }
            assert_eq!(
                &*actual.unwrap(),
                &expected,
                "batch output differs from sequential reference"
            );
            assert!(!vm.presentation_pending());
        }
    }
}

#[test]
fn masked_paint_keeps_the_previous_frame_until_the_whole_paint_pass_finishes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let project = serde_json::from_value(json!({
        "schema": 2, "name": "cut-in", "content_revision": "1", "canvas": [8, 8],
        "assets": "assets", "clients": [{"name": "game", "adapter": "krkr", "game": "synthetic",
            "config": {"canvas": [8, 8]}}]
    })).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry = crysirror_runtime::AdapterRegistry::default();
    crate::register(&mut registry).unwrap();
    let mut owner = crysirror_runtime::Runtime::with_registry(
        project, &path, &dir.path().join("state"), registry,
    ).unwrap();

    let client = ClientId(1);
    let mut host = KrkrHost::new([8, 8], "default".into());
    host.client = client.0;
    host.images = Some(owner.host.image_metadata(client).unwrap());
    let mut rt = Runtime::with_host(host);
    natives::install(&mut rt);
    rt.register_global_native("waitForFrameArt", |rt: &mut Rt, _, _| {
        if !rt.global_member("frameArtReady").is_truthy() {
            return Err(crysirror_tjs2::TjsError::resource_pending("cut-in frame art"));
        }
        Ok(Variant::Void)
    });
    natives::run_source(&mut rt, "cut-in.tjs", r#"
        Plugins.link('layerExBTOA.dll');
        var w=new Window(),root=new Layer(w,null);
        var head=new Layer(w,root),mask=new Layer(w,root),frame=new Layer(w,root);
        mask.setImageSize(4,1);mask.fillRect(0,0,4,1,0x80ffffff);
        head.setImageSize(4,1);
        head.onPaint=function() {
            this.fillRect(0,0,4,1,0xff405060);
            this.clipAlphaRect(0,0,mask,0,0,4,1,0);
            global.headClipped=true;
        };
        head.update();
        frame.onPaint=function() {
            global.frameStarted=true;
            waitForFrameArt();
            global.framePainted=true;
        };
        frame.update();
    "#, None).unwrap();
    let mut vm = KrkrVm::new(client, serde_json::from_value(json!({"canvas": [8, 8]})).unwrap()).unwrap();
    vm.rt = Some(rt);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        vm.execute(&mut owner.host, InputFrame::default()).unwrap();
        let rt = vm.rt.as_ref().unwrap();
        if rt.global_member("frameStarted").is_truthy() {
            assert!(rt.global_member("headClipped").is_truthy());
            assert!(!rt.global_member("framePainted").is_truthy());
            assert!(vm.presentation_pending(), "the frame art is still waiting");
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }

    vm.rt.as_mut().unwrap().set_global_member("frameArtReady", Variant::Integer(1));
    vm.execute(&mut owner.host, InputFrame::default()).unwrap();
    assert!(vm.rt.as_ref().unwrap().global_member("framePainted").is_truthy());
    assert!(!vm.presentation_pending(), "both parts of the redraw are ready");
}
