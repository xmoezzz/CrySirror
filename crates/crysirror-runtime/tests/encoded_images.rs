//! Shared archive-member byte decoding and exact integer transitions, without
//! any native VM, window, audio device or proprietary data.
use crysirror_core::{
    image_processing::{ImageInput, ImageJob, ImageProcessState, ImageResource},
    service::HostServices,
    state::ClientId,
};
use crysirror_image::{
    effects::ImageEffect,
    fixed_mix::{FixedMixAlpha, FixedMixRounding, FixedMixSpec},
    transition::ImageTransition,
    ImageSurface,
};
use crysirror_runtime::Runtime;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn finish(mut job: ImageJob) -> Result<Arc<dyn ImageResource>, String> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        match job.poll().unwrap() {
            ImageProcessState::Ready(image) => return Ok(image),
            ImageProcessState::Failed(error) => return Err(error),
            ImageProcessState::Pending => {
                assert!(Instant::now() < until);
                std::thread::yield_now();
            }
        }
    }
}
#[test]
fn encoded_bytes_use_shared_decoder_then_exact_pair_worker() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    let file = dir.path().join("project.json");
    std::fs::write(&file,br#"{"schema":2,"name":"encoded","content_revision":"1","canvas":[8,8],"assets":"assets","clients":[{"name":"one","adapter":"crysirror-script-v1","game":"authored"}]}"#).unwrap();
    let mut runtime = Runtime::load(&file, &dir.path().join("state")).unwrap();
    let images = runtime.host.image_metadata(ClientId(1)).unwrap();
    let bytes: Arc<[u8]> =
        crysirror_image::encode_png(&ImageSurface::filled(1, 1, [255, 1, 100, 0]).unwrap())
            .unwrap()
            .into();
    let left = finish(
        images
            .begin_process(
                ImageInput::Encoded {
                    name: "archive/member.png".into(),
                    bytes: bytes.clone(),
                    size: [1, 1],
                },
                b"[]".as_slice().into(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(left.rgba().as_ref(), [255, 1, 100, 0]);
    let changed = finish(
        images
            .begin_process(
                ImageInput::Encoded {
                    name: "member.png".into(),
                    bytes: bytes.clone(),
                    size: [2, 1],
                },
                b"[]".as_slice().into(),
            )
            .unwrap(),
    );
    assert!(changed.unwrap_err().contains("dimensions"));
    assert!(images
        .begin_process(
            ImageInput::Encoded {
                name: "../escape.png".into(),
                bytes,
                size: [1, 1]
            },
            b"[]".as_slice().into()
        )
        .is_err());
    let right = finish(
        images
            .begin_process(
                ImageInput::Solid {
                    size: [1, 1],
                    color: [1, 255, 100, 0],
                },
                b"[]".as_slice().into(),
            )
            .unwrap(),
    )
    .unwrap();
    let effect = ImageEffect::Transition {
        spec: ImageTransition {
            progress: 0.5,
            softness: 0.,
            blur: 0.,
            fixed_weighted: Some(FixedMixSpec {
                left_weight: 128,
                right_weight: 128,
                fractional_bits: 8,
                rounding: FixedMixRounding::SumThenShift,
                alpha: FixedMixAlpha::Opaque,
            }),
        },
    };
    let mixed = finish(
        images
            .begin_process(
                ImageInput::Transition {
                    from: left.clone(),
                    to: right,
                    mask: None,
                },
                serde_json::to_vec(&[effect]).unwrap().into(),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(mixed.rgba().as_ref(), [128, 128, 100, 255]);
    assert_eq!(left.rgba().as_ref(), [255, 1, 100, 0]);
}
