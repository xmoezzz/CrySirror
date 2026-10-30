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
