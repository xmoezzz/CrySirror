// SPDX-License-Identifier: MPL-2.0
use crysirror_core::{
    service::{HostServices, MediaRequest},
    state::ClientId,
};
use crysirror_runtime::Runtime;
#[test]
fn media_snapshots_reject_even_stopped_resources_whose_textures_cannot_be_reconstructed() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join("assets")).unwrap();
    std::fs::write(
        t.path().join("assets/movie.mpg"),
        include_bytes!("../../crysirror-video/tests/fixtures/media_av.mpg"),
    )
    .unwrap();
    let p = t.path().join("project.json");
    std::fs::write(&p,br#"{"schema":2,"name":"media","content_revision":"1","canvas":[16,16],"assets":"assets","clients":[{"name":"app","adapter":"crysirror-script-v1","game":"fixture"}]}"#).unwrap();
    let mut rt = Runtime::load(&p, &t.path().join("state")).unwrap();
    assert!(rt.snapshot().is_ok());
    rt.host
        .media_request(
            ClientId(1),
            MediaRequest::Play {
                id: 0,
                asset: "movie.mpg".into(),
                looping: false,
                audio: false,
                gain: 1.,
            },
        )
        .unwrap();
    assert!(
        rt.snapshot()
            .unwrap_err()
            .to_string()
            .contains("resource reconstruction")
    );
    rt.host
        .media_request(ClientId(1), MediaRequest::Stop { id: 0 })
        .unwrap();
    assert!(!rt.host.media.has_active());
    assert!(rt.host.media.has_resources());
    assert!(
        rt.snapshot()
            .unwrap_err()
            .to_string()
            .contains("retained EOF frames")
    );
    assert!(rt.host.media_request(ClientId(2), MediaRequest::Remove { id: 0 }).is_err());
    assert!(rt.host.media_info(ClientId(1), 0).is_ok());
    rt.host.media_request(ClientId(1), MediaRequest::Remove { id: 0 }).unwrap();
    assert!(rt.host.media_info(ClientId(1), 0).is_err());
    assert!(!rt.host.media.has_resources());
    assert!(rt.snapshot().is_ok(), "explicit removal releases the reconstruction obligation");
}
