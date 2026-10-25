// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{
    service::{HostServices, InputFrame},
    state::ClientId,
};
use crysirror_image::{ImageDecoder, ImageSurface};
use crysirror_runtime::Runtime;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
struct Decoder(Arc<AtomicUsize>);
impl ImageDecoder for Decoder {
    fn decode(&self, bytes: &[u8]) -> anyhow::Result<ImageSurface> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(ImageSurface {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, bytes[0]],
        })
    }
}
#[test]
fn metadata_uses_shared_decoders_and_caches_opaque_status_per_client() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    std::fs::write(t.path().join("assets/opaque.testimage"), [255]).unwrap();
    std::fs::write(t.path().join("assets/alpha.testimage"), [127]).unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,br#"{"schema":2,"name":"metadata","content_revision":"1","canvas":[16,16],"assets":"assets","clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]}"#).unwrap();
    let mut rt = Runtime::load(&p, &t.path().join("state")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    rt.assets
        .decoders
        .register("testimage", Arc::new(Decoder(calls.clone())))
        .unwrap();
    rt.tick(0, InputFrame::default()).unwrap();
    assert_eq!(
        rt.host.image_size(ClientId(1), "opaque.testimage").unwrap(),
        [2, 1]
    );
    assert!(
        rt.host
            .image_is_opaque(ClientId(1), "opaque.testimage")
            .unwrap()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        !rt.host
            .image_is_opaque(ClientId(1), "alpha.testimage")
            .unwrap()
    );
    assert!(
        rt.host
            .image_is_opaque(ClientId(2), "opaque.testimage")
            .unwrap()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(
        rt.host
            .image_is_opaque(ClientId(99), "opaque.testimage")
            .is_err()
    );
    assert!(rt.host.image_size(ClientId(1), "../escape").is_err());
}
