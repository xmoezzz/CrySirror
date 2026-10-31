//! Dynamic font tests use no VM or renderer. Font binaries remain outside the repository.
use crysirror_core::{
    path::{Mount, VfsPath},
    service::*,
    state::{ClientId, Color, Rect, TextRun},
};
use crysirror_runtime::font_resources::FontResources;
use crysirror_tasks::TaskPool;
use crysirror_text::TextSystem;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
struct Empty;
impl ClientStorage for Empty {
    fn read(&self, _: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
        Err(crysirror_core::Error::NotFound)
    }
    fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
    fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
        Err(crysirror_core::Error::ReadOnly)
    }
    fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
        Ok(vec![])
    }
}
fn setup() -> (
    FontResources,
    TextSystem,
    Arc<dyn FontLibrary>,
    Arc<dyn FontLibrary>,
) {
    let text = TextSystem::default();
    let mut render = TextSystem::default();
    render.attach_dynamic_fonts(&text);
    let resources = FontResources::new(
        text,
        TaskPool::new("font-test-io", 1, 32).unwrap(),
        TaskPool::new("font-test-cpu", 1, 32).unwrap(),
    );
    let a = resources.for_client(ClientId(1), Arc::new(Empty));
    let b = resources.for_client(ClientId(2), Arc::new(Empty));
    (resources, render, a, b)
}
fn ready(library: &dyn FontLibrary, job: FontJob) -> FontLoadState {
    let start = Instant::now();
    loop {
        let state = library.status(job).unwrap();
        if state != FontLoadState::Pending {
            return state;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::yield_now();
    }
}
#[test]
fn malformed_font_reports_failure_and_tickets_are_client_scoped() {
    let (resources, _, a, b) = setup();
    let job = a
        .begin(FontLoadSpec {
            name: "bad".into(),
            source: FontLoadSource::Bytes(Arc::from(&b"not a font"[..])),
            face_index: 0,
        })
        .unwrap();
    assert!(resources.has_resources());
    assert!(b.status(job).is_err());
    assert!(b.release_job(job).is_err());
    assert!(matches!(ready(a.as_ref(), job), FontLoadState::Failed(_)));
    a.release_job(job).unwrap();
    assert!(!resources.has_resources());
    assert!(
        a.begin(FontLoadSpec {
            name: "__font/2/fake".into(),
            source: FontLoadSource::Bytes(Arc::from(&b"x"[..])),
            face_index: 0
        })
        .is_err()
    );
}
#[test]
fn bounded_font_jobs_release_capacity_and_cancel_workers() {
    let (resources, _, a, _) = setup();
    let mut jobs = vec![];
    for i in 0..16 {
        jobs.push(
            a.begin(FontLoadSpec {
                name: format!("font{i}"),
                source: FontLoadSource::Bytes(Arc::from(&b"bad"[..])),
                face_index: 0,
            })
            .unwrap(),
        );
    }
    assert!(
        a.begin(FontLoadSpec {
            name: "overflow".into(),
            source: FontLoadSource::Bytes(Arc::from(&b"bad"[..])),
            face_index: 0
        })
        .is_err()
    );
    for job in jobs {
        a.release_job(job).unwrap();
    }
    assert!(!resources.has_resources());
}
fn load(lib: &dyn FontLibrary, name: &str, bytes: Arc<[u8]>, index: u32) -> FontInfo {
    let job = lib
        .begin(FontLoadSpec {
            name: name.into(),
            source: FontLoadSource::Bytes(bytes),
            face_index: index,
        })
        .unwrap();
    let FontLoadState::Ready(info) = ready(lib, job) else {
        panic!("font preparation failed")
    };
    lib.release_job(job).unwrap();
    info
}
#[test]
#[ignore = "requires two runtime system font paths; never commits font binaries"]
fn dynamic_font_publication_is_shared_scoped_idempotent_and_immutable() {
    let first: Arc<[u8]> = std::fs::read(std::env::var("CRYSIRROR_TEST_FONT").unwrap())
        .unwrap()
        .into();
    let second: Arc<[u8]> = std::fs::read(std::env::var("CRYSIRROR_TEST_FONT_SECOND").unwrap())
        .unwrap()
        .into();
    let (resources, mut render, a, b) = setup();
    // Desktop installs its default font after constructing the resource actor.
    // A newly scoped library must see that current static alias as well as the
    // live dynamic catalog, despite TextSystem static storage being copy-on-write.
    render.add_font("late-default", first.to_vec()).unwrap();
    assert!(!a.is_loaded("late-default").unwrap());
    let late = resources.for_client_with_text(ClientId(1), Arc::new(Empty), &render);
    assert!(late.is_loaded("late-default").unwrap());
    assert!(render.for_client(ClientId(1)).measure_simple("late-default", 24., "ABC").is_ok());
    let measurement = resources.measurement(ClientId(1));
    let info = load(a.as_ref(), "game", first.clone(), 0);
    let duplicate = load(a.as_ref(), "game", first, 0);
    assert_eq!(duplicate, info);
    assert_eq!(a.resolve("game").unwrap(), Some(info.clone()));
    assert!(!info.families.is_empty());
    assert_eq!(a.resolve(&info.families[0]).unwrap(), Some(info.clone()));
    assert!(b.resolve("game").unwrap().is_none());
    assert!(!b.is_loaded(&info.alias).unwrap());
    assert!(render.font_digest(&info.alias).is_err());
    let view = render.for_client(ClientId(1));
    assert_eq!(view.font_digest(&info.alias).unwrap(), info.digest);
    let metrics = measurement.measure_simple(&info.alias, 32., "ABC").unwrap();
    assert_eq!(
        view.measure_simple(&info.alias, 32., "ABC").unwrap(),
        metrics
    );
    let run = TextRun {
        text: "ABC".into(),
        font: info.alias.clone(),
        size_px: 32,
        bounds: Rect {
            x: 0,
            y: 0,
            width: 200,
            height: 60,
        },
        color: Color([255; 4]),
        layout_profile: UNWRAPPED_TEXT_LAYOUT.into(),
    };
    let pixels = view.rasterize(&run).unwrap();
    assert!(pixels.chunks_exact(4).any(|p| p[3] > 0));
    let newer = load(a.as_ref(), "game", second, 0);
    assert_ne!(newer.digest, info.digest);
    assert_ne!(newer.alias, info.alias);
    assert_eq!(a.resolve("game").unwrap(), Some(newer.clone()));
    assert_eq!(
        measurement.measure_simple(&info.alias, 32., "ABC").unwrap(),
        metrics
    );
    assert_eq!(view.rasterize(&run).unwrap(), pixels);
    assert!(b.release_font(info.handle).is_err());
    a.release_font(info.handle).unwrap();
    assert!(measurement.measure_simple(&info.alias, 32., "ABC").is_err());
    assert!(view.rasterize(&run).is_err());
    a.release_font(newer.handle).unwrap();
    assert!(!resources.has_resources());
}
