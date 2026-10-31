//! Shared persistent CPU pattern resources, without a native VM or device.
use crysirror_core::{image_processing::ImageInput, service::*, state::ClientId};
use crysirror_image::temporal_pattern::{PatternRng, TemporalPattern, TemporalPatternSnapshot};
use crysirror_runtime::Runtime;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn runtime() -> (tempfile::TempDir, Runtime) {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,br#"{"schema":2,"name":"pattern","content_revision":"1","canvas":[16,4],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"one"},{"name":"b","adapter":"crysirror-script-v1","game":"two"}]}"#).unwrap();
    let r = Runtime::load(&p, &t.path().join("state")).unwrap();
    (t, r)
}
fn encoded<T: serde::Serialize>(v: &T) -> Arc<[u8]> {
    serde_json::to_vec(v).unwrap().into()
}
fn create() -> PatternRequest {
    PatternRequest::Create {
        spec: encoded(&serde_json::json!({"size":[16,4],"cell":[2,2]})),
        rng: encoded(&PatternRng::msvc(1)),
    }
}
fn finish(mut job: PatternJob) -> PatternOutput {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        match job.poll().unwrap() {
            PatternProcessState::Ready(v) => return v,
            PatternProcessState::Failed(e) => panic!("{e}"),
            PatternProcessState::Pending => {
                assert!(Instant::now() < until);
                std::thread::yield_now();
            }
        }
    }
}
fn state(v: PatternOutput) -> (Arc<dyn PatternResource>, Arc<[u8]>) {
    match v {
        PatternOutput::Resource {
            resource,
            rng_after,
        } => (resource, rng_after),
        _ => panic!("resource expected"),
    }
}
#[test]
fn pattern_workers_preserve_scopes_versions_rng_and_render_exact_shared_pixels() {
    let (_t, mut rt) = runtime();
    let a = ClientId(1);
    let b = ClientId(2);
    assert!(rt.host.begin_pattern(ClientId(99), create()).is_err());
    let (first, random) = state(finish(rt.host.begin_pattern(a, create()).unwrap()));
    assert_eq!(first.info().client, a);
    assert_eq!(first.info().size, [16, 4]);
    assert_eq!(first.info().completed_steps, 0);
    assert_eq!(
        serde_json::from_slice::<PatternRng>(&random).unwrap().draws,
        8
    );
    let before = first.snapshot();
    assert!(rt
        .host
        .begin_pattern(
            b,
            PatternRequest::Advance {
                source: first.clone(),
                rng: random.clone()
            }
        )
        .is_err());
    let (_other, mut other) = runtime();
    assert!(other
        .host
        .begin_pattern(
            a,
            PatternRequest::Advance {
                source: first.clone(),
                rng: random.clone()
            }
        )
        .is_err());
    let (second, after) = state(finish(
        rt.host
            .begin_pattern(
                a,
                PatternRequest::Advance {
                    source: first.clone(),
                    rng: random.clone(),
                },
            )
            .unwrap(),
    ));
    assert_eq!(second.info().completed_steps, 1);
    assert_eq!(&*first.snapshot(), &*before);
    assert_ne!(&*second.snapshot(), &*before);
    assert_eq!(
        serde_json::from_slice::<PatternRng>(&after).unwrap().draws,
        12
    );
    let render = || PatternRequest::Render {
        source: second.clone(),
        destination: ImageInput::Solid {
            size: [16, 4],
            color: [100, 110, 120, 191],
        },
        rect: [0, 0, 16, 4],
        opacity256: 128,
    };
    let output = finish(rt.host.begin_pattern(a, render()).unwrap());
    let PatternOutput::Image(image) = output else {
        panic!("image expected")
    };
    assert_eq!(image.client(), a);
    assert!(!image.opaque());
    let kernel = TemporalPattern::from_snapshot(
        serde_json::from_slice::<TemporalPatternSnapshot>(&second.snapshot()).unwrap(),
        &|| Ok(()),
    )
    .unwrap();
    let expected = kernel
        .composite(
            &crysirror_image::ImageSurface::filled(16, 4, [100, 110, 120, 191]).unwrap(),
            [0, 0, 16, 4],
            128,
            &|| Ok(()),
        )
        .unwrap();
    assert_eq!(&*image.rgba(), &*expected.rgba);
    let (restored, unchanged) = state(finish(
        rt.host
            .begin_pattern(
                b,
                PatternRequest::Restore {
                    snapshot: second.snapshot(),
                    rng: after.clone(),
                },
            )
            .unwrap(),
    ));
    assert_eq!(restored.client(), b);
    assert_eq!(restored.info().completed_steps, 1);
    assert_eq!(&*unchanged, &*after);
    assert_eq!(&*restored.snapshot(), &*second.snapshot());
    assert_eq!(first.info().resident_bytes, 16 + before.len());
}
#[test]
fn malformed_pattern_data_and_render_contexts_fail_without_mutating_resources() {
    let (_t, mut rt) = runtime();
    let a = ClientId(1);
    assert!(rt
        .host
        .begin_pattern(
            a,
            PatternRequest::Create {
                spec: encoded(&serde_json::json!({"size":[8192,8192],"cell":[2,2]})),
                rng: encoded(&PatternRng::msvc(1))
            }
        )
        .is_err());
    let (source, random) = state(finish(rt.host.begin_pattern(a, create()).unwrap()));
    let before = source.snapshot();
    assert!(rt
        .host
        .begin_pattern(
            a,
            PatternRequest::Render {
                source: source.clone(),
                destination: ImageInput::Solid {
                    size: [16, 4],
                    color: [0; 4]
                },
                rect: [0, 0, 15, 4],
                opacity256: 128
            }
        )
        .is_err());
    let mut bad: serde_json::Value = serde_json::from_slice(&before).unwrap();
    bad["words"][0] = serde_json::json!(0x80008000u32);
    let mut job = rt
        .host
        .begin_pattern(
            a,
            PatternRequest::Restore {
                snapshot: encoded(&bad),
                rng: random,
            },
        )
        .unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        match job.poll().unwrap() {
            PatternProcessState::Failed(e) => {
                assert!(e.contains("word domain"), "{e}");
                break;
            }
            PatternProcessState::Ready(_) => panic!("invalid words accepted"),
            PatternProcessState::Pending => {
                assert!(Instant::now() < until);
                std::thread::yield_now();
            }
        }
    }
    assert_eq!(&*source.snapshot(), &*before);
    let PatternProcessState::Failed(_) = job.poll().unwrap() else {
        panic!("terminal failure not retained")
    };
    // Failure did not poison the common memory/worker reservation pool.
    state(finish(rt.host.begin_pattern(a, create()).unwrap()));
}
