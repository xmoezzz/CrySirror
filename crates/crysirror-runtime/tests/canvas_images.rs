//! Shared Canvas image jobs without a legacy VM, GPU or window.
use crysirror_core::{image_processing::*, service::*, state::ClientId};
use crysirror_runtime::Runtime;
use crysirror_scene::ServiceBatch;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn runtime() -> (tempfile::TempDir, Runtime) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    let p = root.path().join("project.json");
    std::fs::write(&p,br#"{"schema":2,"name":"capture","content_revision":"1","canvas":[4,4],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]}"#).unwrap();
    let mut rt = Runtime::load(&p, &root.path().join("state")).unwrap();
    submit(
        &mut rt,
        serde_json::json!({"canvases":[{"op":"create","id":1,"width":2,"height":2},{"op":"clear","id":1,"color":[255,0,0,255]}]}),
    );
    (root, rt)
}
fn submit(rt: &mut Runtime, value: serde_json::Value) {
    rt.host
        .engine
        .submit(
            ClientId(1),
            &serde_json::from_value::<ServiceBatch>(value).unwrap(),
        )
        .unwrap();
}
fn finish(job: &mut ImageJob) -> Arc<dyn ImageResource> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        match job.poll().unwrap() {
            ImageProcessState::Ready(image) => return image,
            ImageProcessState::Failed(reason) => panic!("{reason}"),
            ImageProcessState::Pending => {
                assert!(Instant::now() < until);
                std::thread::yield_now();
            }
        }
    }
}
struct WorkerReader {
    owner: std::thread::ThreadId,
    bytes: Arc<[u8]>,
}
impl AssetReader for WorkerReader {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.bytes.len() as u64)
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> crysirror_core::Result<usize> {
        assert_ne!(
            std::thread::current().id(),
            self.owner,
            "readback ran on owner"
        );
        let start = offset as usize;
        let n = output
            .len()
            .min(3)
            .min(self.bytes.len().saturating_sub(start));
        output[..n].copy_from_slice(&self.bytes[start..start + n]);
        Ok(n)
    }
}
#[test]
fn immutable_canvas_capture_awaits_presenter_then_reads_only_on_worker_and_retains_receipt() {
    let (_root, mut rt) = runtime();
    let owner = ClientId(1);
    assert!(rt.host.begin_canvas_image(owner, 1).is_err());
    rt.set_canvas_image_presenter(true);
    assert!(rt.host.begin_canvas_image(ClientId(2), 1).is_err());
    assert!(rt.host.begin_canvas_image(ClientId(99), 1).is_err());
    let mut job = rt.host.begin_canvas_image(owner, 1).unwrap();
    assert!(matches!(job.poll().unwrap(), ImageProcessState::Pending));
    submit(
        &mut rt,
        serde_json::json!({"canvases":[{"op":"clear","id":1,"color":[0,0,255,255]}]}),
    );
    let mut attempts = 0;
    rt.prepare_canvas_images(|request,_assets| {
        attempts+=1;
        assert_eq!(request.size,[2,2]);
        assert!(request.layers.iter().any(|layer|matches!(&layer.paint,crysirror_scene::layer::Paint::Solid{color} if *color==[255,0,0,255])));
        Ok(None)
    }).unwrap();
    assert_eq!(attempts, 1);
    assert!(matches!(job.poll().unwrap(), ImageProcessState::Pending));
    let pixels: Arc<[u8]> = [255, 0, 0, 255].repeat(4).into();
    rt.prepare_canvas_images(|_, _| {
        Ok(Some(Arc::new(WorkerReader {
            owner: std::thread::current().id(),
            bytes: pixels.clone(),
        })))
    })
    .unwrap();
    let image = finish(&mut job);
    assert_eq!(image.client(), owner);
    assert_eq!(image.size(), [2, 2]);
    assert_eq!(image.rgba(), pixels);
    assert!(image.opaque());
    let ImageProcessState::Ready(again) = job.poll().unwrap() else {
        panic!("retained completion expected")
    };
    assert!(Arc::ptr_eq(&image, &again));
}
#[derive(Debug)]
struct Resource(Arc<[u8]>);
impl ImageResource for Resource {
    fn client(&self) -> ClientId {
        ClientId(1)
    }
    fn size(&self) -> [u32; 2] {
        [2, 2]
    }
    fn rgba(&self) -> Arc<[u8]> {
        self.0.clone()
    }
    fn opaque(&self) -> bool {
        true
    }
}
#[test]
fn hidden_snapshot_and_world_material_hold_retired_images_until_their_last_reference_is_removed() {
    let (_root,mut rt)=runtime();let owner=ClientId(1);
    let pixels:Arc<[u8]>=[1,2,3,255].repeat(4).into();
    let image=rt.host.publish_image_resource(owner,8,Arc::new(Resource(pixels.clone()))).unwrap();
    let path=image.path.relative().to_owned();
    submit(&mut rt,serde_json::json!({"layers":[{"op":"put","layer":{"id":9,"visible":false,"rect":[0,0,2,2],"paint":{"kind":"snapshot","size":[2,2],"layers":[{"id":1,"rect":[0,0,2,2],"paint":{"kind":"image","asset":path}}]}}}],"world":[{"op":"spawn","entity":{"id":4,"name":"image-holder","geometry":{"kind":"cube"},"material":{"base_color_texture":path}}}]}));
    let frozen=rt.assets.freeze_surfaces();
    rt.host.release_image_resource(owner,&path).unwrap();rt.tick(0,InputFrame::default()).unwrap();
    assert!(rt.assets.surface(owner,&path).is_some());
    submit(&mut rt,serde_json::json!({"layers":[{"op":"remove","id":9}]}));rt.tick(0,InputFrame::default()).unwrap();
    assert!(rt.assets.surface(owner,&path).is_some(),"World still owns the texture");
    submit(&mut rt,serde_json::json!({"world":[{"op":"despawn","id":4}]}));rt.tick(0,InputFrame::default()).unwrap();
    assert!(rt.assets.surface(owner,&path).is_none());
    assert_eq!(frozen.surface(owner,&path).unwrap().rgba,pixels,"already-frozen frame keeps its own lease");
}
#[test]
fn capture_pins_published_inputs_while_release_mutation_and_failed_presenter_remain_explicit() {
    let (_root, mut rt) = runtime();
    rt.set_canvas_image_presenter(true);
    let owner = ClientId(1);
    let pixels: Arc<[u8]> = [0, 255, 0, 255].repeat(4).into();
    let published = rt
        .host
        .publish_image_resource(owner, 7, Arc::new(Resource(pixels.clone())))
        .unwrap();
    let path = published.path.relative().to_owned();
    submit(
        &mut rt,
        serde_json::json!({"canvases":[{"op":"set","id":1,"layer":{"id":5,"rect":[0,0,2,2],"paint":{"kind":"image","asset":path}}}]}),
    );
    let mut job = rt.host.begin_canvas_image(owner, 1).unwrap();
    rt.host.release_image_resource(owner, &path).unwrap();
    assert!(
        rt.assets.surface(owner, &path).is_some(),
        "Canvas still owns the retired image path"
    );
    assert!(
        rt.host.release_image_resource(owner, &path).is_err(),
        "caller ownership ended exactly once"
    );
    submit(
        &mut rt,
        serde_json::json!({"canvases":[{"op":"clear","id":1,"color":[0,0,255,255]}]}),
    );
    rt.tick(0, InputFrame::default()).unwrap();
    assert!(
        rt.assets.surface(owner, &path).is_none(),
        "unreferenced retired path collected"
    );
    rt.prepare_canvas_images(|_, assets| {
        assert_eq!(assets.surface(owner, &path).unwrap().rgba, pixels);
        Ok(Some(Arc::new(MemoryAssetReader(pixels.clone()))))
    })
    .unwrap();
    assert!(rt.assets.surface(owner, &path).is_none());
    assert_eq!(finish(&mut job).rgba(), pixels);
    let mut failed = rt.host.begin_canvas_image(owner, 1).unwrap();
    rt.prepare_canvas_images(|_, _| anyhow::bail!("authored presenter failure"))
        .unwrap();
    assert!(
        matches!(failed.poll().unwrap(),ImageProcessState::Failed(reason) if reason.contains("authored presenter failure"))
    );
    assert!(matches!(
        failed.poll().unwrap(),
        ImageProcessState::Failed(_)
    ));
    // A failed or cancelled request releases its task/memory admission; later
    // requests neither silently succeed nor get stuck behind dead queue entries.
    for _ in 0..32 {
        let dead = rt.host.begin_canvas_image(owner, 1).unwrap();
        drop(dead);
    }
    let mut pending = rt.host.begin_canvas_image(owner, 1).unwrap();
    rt.set_canvas_image_presenter(false);
    assert!(matches!(
        pending.poll().unwrap(),
        ImageProcessState::Failed(_)
    ));
}
