//! Independent shared-service test; no native VM, window, or game resources.
use crysirror_core::{
    image_processing::{ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageProcessState},
    service::HostServices,
    state::ClientId,
};
use crysirror_runtime::Runtime;
use std::time::{Duration, Instant};
#[test]
fn separate_mask_graph_checks_admission_and_preserves_endpoint_and_floor_rules() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let project = dir.path().join("project.json");
    std::fs::write(&project,br#"{"schema":2,"name":"mask graph","content_revision":"1","canvas":[5,2],"assets":"assets","clients":[{"name":"one","adapter":"crysirror-script-v1","game":"one"},{"name":"two","adapter":"crysirror-script-v1","game":"two"}]}"#).unwrap();
    let mut rt = Runtime::load(&project, &dir.path().join("state")).unwrap();
    let images = rt.host.image_metadata(ClientId(1)).unwrap();
    let make = |opacity| ImageInput::Composite {
        base: Box::new(ImageInput::Solid {
            size: [5, 2],
            color: [255, 255, 255, 91],
        }),
        layers: [0, 1, 128, 254, 255]
            .into_iter()
            .enumerate()
            .map(|(x, alpha)| ImageCompositeLayer {
                input: Box::new(ImageInput::Solid {
                    size: [1, 2],
                    color: [255, 255, 255, alpha],
                }),
                source: [0, 0, 1, 2],
                destination: [x as i32, 0],
                clip: Some([0, 0, 5, 1]),
                opacity,
                math: ImageCompositeMath::MaskSeparate256,
            })
            .collect(),
    };
    assert!(images
        .begin_process(make(128), b"[]".as_slice().into())
        .is_err());
    let mut job = images
        .begin_process(make(255), b"[]".as_slice().into())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = loop {
        match job.poll().unwrap() {
            ImageProcessState::Ready(r) => break r,
            ImageProcessState::Failed(e) => panic!("{e}"),
            ImageProcessState::Pending => {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
    };
    let expected: Vec<_> = [255, 254, 254, 254, 255]
        .into_iter()
        .chain([255; 5])
        .flat_map(|v| [v, v, v, 91])
        .collect();
    assert_eq!(result.rgba().as_ref(), expected);
    assert_eq!(rt.host.now_ms(), 0);
    assert!(
        matches!(job.poll().unwrap(), ImageProcessState::Ready(_)),
        "completed receipt is retained"
    );
    assert!(rt
        .host
        .image_metadata(ClientId(2))
        .unwrap()
        .begin_process(ImageInput::Resource(result), b"[]".as_slice().into())
        .is_err());
}
