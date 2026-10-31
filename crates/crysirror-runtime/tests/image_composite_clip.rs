//! Shared immutable composition and destination clipping, without a VM or GPU.
use crysirror_core::{
    image_processing::{
        ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageProcessState, ImageResource,
    },
    service::HostServices,
    state::ClientId,
};
use crysirror_runtime::Runtime;
use std::time::{Duration, Instant};
#[derive(Debug)]
struct Columns;
impl ImageResource for Columns {
    fn client(&self) -> ClientId {
        ClientId(1)
    }
    fn size(&self) -> [u32; 2] {
        [4, 2]
    }
    fn rgba(&self) -> std::sync::Arc<[u8]> {
        [10u8, 20, 30, 40, 50, 60, 70, 80]
            .into_iter()
            .flat_map(|r| [r, 0, 0, 0])
            .collect::<Vec<_>>()
            .into()
    }
    fn opaque(&self) -> bool {
        false
    }
}

#[test]
fn clipped_rgb_copy_preserves_mapping_alpha_and_rejects_malformed_clip() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    let path = root.path().join("project.json");
    std::fs::write(&path,br#"{"schema":2,"name":"composite","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#).unwrap();
    let mut rt = Runtime::load(&path, &root.path().join("state")).unwrap();
    let images = rt.host.image_metadata(ClientId(1)).unwrap();
    let make = |clip| ImageInput::Composite {
        base: Box::new(ImageInput::Solid {
            size: [4, 2],
            color: [9, 8, 7, 77],
        }),
        layers: vec![ImageCompositeLayer {
            input: Box::new(ImageInput::Resource(std::sync::Arc::new(Columns))),
            source: [0, 0, 4, 2],
            destination: [-1, 0],
            clip,
            opacity: 255,
            math: ImageCompositeMath::CopyRgb,
        }],
    };
    assert!(
        images
            .begin_process(make(Some([0, 0, -1, 1])), b"[]".as_slice().into())
            .is_err()
    );
    let mut job = images
        .begin_process(make(Some([1, 0, 1, 1])), b"[]".as_slice().into())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let pixels = loop {
        match job.poll().unwrap() {
            ImageProcessState::Ready(image) => break image.rgba(),
            ImageProcessState::Failed(error) => panic!("{error}"),
            ImageProcessState::Pending => {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
    };
    let expected = [9, 8, 7, 77];
    for (i, pixel) in pixels.chunks_exact(4).enumerate() {
        assert_eq!(pixel, if i == 1 { &[30, 0, 0, 77] } else { &expected });
    }
}

#[test]
fn bare_asset_preparation_publishes_exact_owned_pixels_and_checks_identity() {
    let root=tempfile::tempdir().unwrap();std::fs::create_dir(root.path().join("assets")).unwrap();
    let path=root.path().join("project.json");
    std::fs::write(&path,br#"{"schema":2,"name":"image preparation","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"},{"name":"other","adapter":"crysirror-script-v1","game":"test"}]}"#).unwrap();
    let rgba=vec![10,20,30,0,40,50,60,64,70,80,90,200,100,110,120,255];
    let encoded=crysirror_image::encode_png(&crysirror_image::ImageSurface{width:2,height:2,rgba:rgba.clone()}).unwrap();
    std::fs::write(root.path().join("assets/coverage.png"),&encoded).unwrap();
    let mut rt=Runtime::load(&path,&root.path().join("state")).unwrap();let images=rt.host.image_metadata(ClientId(1)).unwrap();
    let mut job=images.begin_process(ImageInput::Asset{path:"coverage.png".into(),size:[2,2],fingerprint:Some(*blake3::hash(&encoded).as_bytes())},b"[]".as_slice().into()).unwrap();
    let deadline=Instant::now()+Duration::from_secs(10);
    let image=loop{match job.poll().unwrap(){ImageProcessState::Ready(image)=>break image,ImageProcessState::Failed(e)=>panic!("{e}"),ImageProcessState::Pending=>{assert!(Instant::now()<deadline);std::thread::yield_now();}}};
    assert_eq!(image.client(),ClientId(1));assert_eq!(image.rgba().as_ref(),rgba);assert!(!image.opaque());
    assert_eq!(image.alpha_at([1,0]),Some(64));assert_eq!(image.alpha_region([0,0,2,2]).unwrap().as_ref(),[0,64,200,255]);
    let other=rt.host.image_metadata(ClientId(2)).unwrap();
    assert!(other.begin_process(ImageInput::Resource(image.clone()),b"[]".as_slice().into()).is_err());
    for input in [ImageInput::Resource(image.clone()),ImageInput::Solid{size:[1,1],color:[4,3,2,1]}]{
        let solid=matches!(&input,ImageInput::Solid{..});
        let mut prepared=images.begin_process(input,b"[]".as_slice().into()).unwrap();
        loop{match prepared.poll().unwrap(){ImageProcessState::Ready(published)=>{assert_eq!(published.client(),ClientId(1));assert_eq!(published.rgba().as_ref(),if solid{&[4,3,2,1]}else{rgba.as_slice()});break;},ImageProcessState::Failed(e)=>panic!("{e}"),ImageProcessState::Pending=>{assert!(Instant::now()<deadline);std::thread::yield_now();}}}
    }
    let mut bad=images.begin_process(ImageInput::Asset{path:"coverage.png".into(),size:[2,2],fingerprint:Some([0;32])},b"[]".as_slice().into()).unwrap();
    loop{match bad.poll().unwrap(){ImageProcessState::Failed(e)=>{assert!(e.contains("changed"),"{e}");break;},ImageProcessState::Ready(_)=>panic!("wrong content identity was admitted"),ImageProcessState::Pending=>{assert!(Instant::now()<deadline);std::thread::yield_now();}}}
}

#[test]
fn large_asset_preparation_has_resource_budget_and_keeps_effect_limit() {
    let root=tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    let path=root.path().join("project.json");
    std::fs::write(&path,br#"{"schema":2,"name":"large resource","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"test"}]}"#).unwrap();
    let image=crysirror_image::ImageSurface::filled(3072,2048,[80,90,100,61]).unwrap();
    std::fs::write(root.path().join("assets/sheet.png"),crysirror_image::encode_png(&image).unwrap()).unwrap();
    let mut runtime=Runtime::load(&path,&root.path().join("state")).unwrap();
    let images=runtime.host.image_metadata(ClientId(1)).unwrap();
    let input=ImageInput::Asset{path:"sheet.png".into(),size:[3072,2048],fingerprint:None};
    // Preparation is not a Gaussian/LUT effect; the full shared memory and
    // graph reservations still apply before worker admission.
    assert!(images.begin_process(input.clone(),br#"[{"kind":"invert"}]"#.as_slice().into()).is_err());
    assert!(images.begin_process(ImageInput::Solid{size:[4097,4096],color:[0;4]},b"[]".as_slice().into()).is_err());
    let mut job=images.begin_process(input,b"[]".as_slice().into()).unwrap();
    let until=Instant::now()+Duration::from_secs(10);
    loop {
        match job.poll().unwrap() {
            ImageProcessState::Ready(resource)=> {
                assert_eq!(resource.client(),ClientId(1));assert_eq!(resource.size(),[3072,2048]);
                assert_eq!(resource.alpha_at([3071,2047]),Some(61));
                assert_eq!(&*resource.rgba(),&*image.rgba);break;
            }
            ImageProcessState::Failed(e)=>panic!("{e}"),
            ImageProcessState::Pending=>{assert!(Instant::now()<until);std::thread::yield_now();}
        }
    }
}
