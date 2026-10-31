//! Generic file/immutable-resource RGB effects: no VM, GPU or game assets.
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState, ImageResource},
    service::HostServices,
    state::ClientId,
};
use crysirror_image::{
    ImageSurface,
    effects::ImageEffect,
    rgb_lut::{RgbLookup, RgbLookupProfile},
};
use crysirror_runtime::Runtime;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn finish(job: &mut ImageJob) -> Result<Arc<dyn ImageResource>, String> {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        match job.poll().unwrap() {
            ImageProcessState::Ready(r) => return Ok(r),
            ImageProcessState::Failed(e) => return Err(e),
            ImageProcessState::Pending => {
                assert!(Instant::now() < end);
                std::thread::yield_now();
            }
        }
    }
}
fn effect(profile: RgbLookupProfile) -> ImageEffect {
    ImageEffect::RgbLookup {
        spec: RgbLookup {
            red: (0..=255).map(|n| 255 - n).collect(),
            green: (0..=255).map(|n| n ^ 85).collect(),
            blue: (0..=255)
                .map(|n| if n % 2 == 0 { 17 } else { 231 })
                .collect(),
            profile,
        },
    }
}
#[test]
fn file_lut_is_worker_owned_immutable_and_foreign_results_cannot_be_reused() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("assets")).unwrap();
    let path = root.path().join("project.json");
    std::fs::write(&path,br#"{"schema":2,"name":"lookup","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"one","adapter":"crysirror-script-v1","game":"test"},{"name":"two","adapter":"crysirror-script-v1","game":"test"}]}"#).unwrap();
    let rgba = vec![2, 3, 4, 0, 7, 8, 9, 128];
    let encoded = crysirror_image::encode_png(&ImageSurface {
        width: 2,
        height: 1,
        rgba: rgba.clone(),
    })
    .unwrap();
    std::fs::write(root.path().join("assets/input.png"), &encoded).unwrap();
    let mut rt = Runtime::load(&path, &root.path().join("state")).unwrap();
    let images = rt.host.image_metadata(ClientId(1)).unwrap();
    let source = || ImageInput::Asset {
        path: "input.png".into(),
        size: [2, 1],
        fingerprint: Some(*blake3::hash(&encoded).as_bytes()),
    };
    let mut base = images
        .begin_process(source(), b"[]".as_slice().into())
        .unwrap();
    let base = finish(&mut base).unwrap();
    let spec = serde_json::to_vec(&[effect(RgbLookupProfile::StraightRgba)]).unwrap();
    let mut job = images.begin_process(source(), spec.clone().into()).unwrap();
    let result = finish(&mut job).unwrap();
    assert_eq!(result.client(), ClientId(1));
    assert_eq!(result.rgba().as_ref(), [253, 86, 17, 0, 248, 93, 231, 128]);
    assert_eq!(base.rgba().as_ref(), rgba);
    assert_eq!(
        std::fs::read(root.path().join("assets/input.png")).unwrap(),
        encoded
    );
    let published = rt
        .host
        .publish_image_resource(ClientId(1), 100, result.clone())
        .unwrap();
    let frozen = rt
        .assets
        .surface(ClientId(1), published.path.relative())
        .unwrap();
    assert_eq!(frozen.rgba.as_ref(), result.rgba().as_ref());
    assert!(
        rt.host
            .publish_image_resource(ClientId(2), 101, result.clone())
            .is_err()
    );
    assert!(
        rt.host
            .release_image_resource(ClientId(2), published.path.relative())
            .is_err()
    );
    assert!(
        rt.assets
            .surface(ClientId(1), published.path.relative())
            .is_some()
    );
    rt.host
        .release_image_resource(ClientId(1), published.path.relative())
        .unwrap();
    assert!(
        rt.assets
            .surface(ClientId(1), published.path.relative())
            .is_none()
    );
    assert_eq!(frozen.rgba.as_ref(), [253, 86, 17, 0, 248, 93, 231, 128]);
    let other = rt.host.image_metadata(ClientId(2)).unwrap();
    assert!(
        other
            .begin_process(ImageInput::Resource(result), spec.into())
            .is_err()
    );
    // Profile validation fails without replacing/publishing any existing resource.
    let bad = serde_json::to_vec(&[effect(RgbLookupProfile::PremultipliedRgba)]).unwrap();
    let rejected = images.begin_process(ImageInput::Resource(base.clone()), bad.into());
    let error = match rejected {
        Ok(_) => panic!("premultiplied ImageJob was admitted"),
        Err(e) => e,
    };
    assert!(error.to_string().contains("premultiplied"));
    assert_eq!(base.rgba().as_ref(), rgba);
    let mut bad = effect(RgbLookupProfile::StraightRgba);
    if let ImageEffect::RgbLookup { spec } = &mut bad {
        spec.green.pop();
    }
    assert!(
        images
            .begin_process(source(), serde_json::to_vec(&[bad]).unwrap().into())
            .is_err()
    );
    assert_eq!(base.rgba().as_ref(), rgba);
}
