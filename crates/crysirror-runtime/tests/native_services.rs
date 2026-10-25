// SPDX-License-Identifier: MPL-2.0
//! Native host capabilities tested without a window, font or user game.
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AudioParameters, AudioRequest, EncodedAudio, HostServices, InputFrame},
    state::ClientId,
    vfs::MemoryProvider,
    Error,
};
use crysirror_runtime::{vfs::DiskVfs, AdapterRegistry, Project, Runtime};
use std::{
    io::{Seek, SeekFrom, Write},
    sync::Arc,
};

fn asset(name: &str) -> VfsPath {
    VfsPath::new(Mount::Asset, name).unwrap()
}

#[test]
fn large_sparse_assets_stream_by_offset_without_materializing_the_archive() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let size = 512 * 1024 * 1024 + 17;
    let mut file = std::fs::File::create(root.path().join("large.arc")).unwrap();
    file.set_len(size).unwrap();
    file.write_all(b"HEAD").unwrap();
    file.seek(SeekFrom::Start(size - 4)).unwrap();
    file.write_all(b"TAIL").unwrap();
    file.flush().unwrap();
    let mut fs = DiskVfs::new(root.path(), state.path()).unwrap();
    fs.register(ClientId(1), "bgi", "stream-test").unwrap();
    let storage = fs.storage(ClientId(1)).unwrap();
    let stream = storage.open_asset(&asset("large.arc")).unwrap();
    assert_eq!(stream.len().unwrap(), size);
    let mut bytes = [0; 4];
    assert_eq!(stream.read_at(size - 4, &mut bytes).unwrap(), 4);
    assert_eq!(&bytes, b"TAIL");
    assert_eq!(stream.read_at(0, &mut bytes).unwrap(), 4);
    assert_eq!(&bytes, b"HEAD");
    // Reading a later mutation proves the opened capability is not an eager
    // resident copy. The sparse fixture exceeds the 64 MiB read() contract.
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"LIVE").unwrap();
    file.flush().unwrap();
    stream.read_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"LIVE");
    assert_eq!(stream.read_at(size, &mut bytes).unwrap(), 0);
    assert_eq!(stream.read_at(u64::MAX, &mut bytes).unwrap(), 0);
    assert!(storage
        .open_asset(&VfsPath::new(Mount::Save, "large.arc").unwrap())
        .is_err());
}

#[test]
fn streaming_providers_shadow_disk_but_cannot_cross_client_capabilities() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    for name in ["one", "two"] {
        std::fs::create_dir_all(root.path().join(name).join("pack")).unwrap();
    }
    std::fs::write(root.path().join("one/pack/value.bin"), b"disk-one").unwrap();
    std::fs::write(root.path().join("one/pack/fallback.bin"), b"fallback").unwrap();
    std::fs::write(root.path().join("two/pack/value.bin"), b"private-two").unwrap();
    let mut fs = DiskVfs::new(root.path(), state.path()).unwrap();
    fs.register_with_assets(ClientId(1), "bgi", "one", "one")
        .unwrap();
    fs.register_with_assets(ClientId(2), "bgi", "two", "two")
        .unwrap();
    for value in [b"older".as_slice(), b"newest".as_slice()] {
        let mut provider = MemoryProvider::new();
        provider.insert("value.bin", value.to_vec()).unwrap();
        fs.mount_provider(ClientId(1), "pack", Arc::new(provider))
            .unwrap();
    }
    let one = fs.storage(ClientId(1)).unwrap();
    let two = fs.storage(ClientId(2)).unwrap();
    let read = |storage: &Arc<dyn crysirror_core::service::ClientStorage>, name: &str| {
        let stream = storage.open_asset(&asset(name)).unwrap();
        let mut out = vec![0; stream.len().unwrap() as usize];
        stream.read_at(0, &mut out).unwrap();
        out
    };
    assert_eq!(read(&one, "pack/value.bin"), b"newest");
    assert_eq!(read(&one, "pack/fallback.bin"), b"fallback");
    assert_eq!(read(&two, "pack/value.bin"), b"private-two");
    assert!(fs.storage(ClientId(99)).is_err());
    assert!(VfsPath::new(Mount::Asset, "../two/pack/value.bin").is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("../two/pack/value.bin", root.path().join("one/leak")).unwrap();
        assert!(one.open_asset(&asset("leak")).is_err());
    }
}

#[test]
fn save_writer_lease_conflicts_across_handles_and_reopens_after_drop() {
    let root = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut fs = DiskVfs::new(root.path(), state.path()).unwrap();
    fs.register(ClientId(1), "bgi", "same-game").unwrap();
    fs.register(ClientId(2), "bgi", "same-game").unwrap();
    fs.register(ClientId(3), "bgi", "other-game").unwrap();
    let first = fs.storage(ClientId(1)).unwrap();
    let second = fs.storage(ClientId(2)).unwrap();
    let held = first.acquire_lease("bgi-session").unwrap();
    assert!(second.acquire_lease("bgi-session").is_err());
    let independent = fs
        .storage(ClientId(3))
        .unwrap()
        .acquire_lease("bgi-session")
        .unwrap();
    assert!(second.acquire_lease("../host-path").is_err());
    assert!(second.acquire_lease("nested/name").is_err());
    drop(held);
    let reacquired = second.acquire_lease("bgi-session").unwrap();
    assert!(first.acquire_lease("bgi-session").is_err());
    drop(reacquired);
    drop(independent);
    assert!(first.acquire_lease("bgi-session").is_ok());
}

fn runtime(root: &std::path::Path) -> Runtime {
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let project: Project = serde_json::from_value(serde_json::json!({
        "schema":2,"name":"native-host-test","content_revision":"1","canvas":[32,32],"assets":"assets",
        "clients":[{"name":"one","adapter":"crysirror-script-v1","game":"one","program":[{"op":"halt"}]},
                   {"name":"two","adapter":"crysirror-script-v1","game":"two","program":[{"op":"halt"}]}]
    })).unwrap();
    std::fs::write(
        root.join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    Runtime::with_registry(
        project,
        &root.join("project.json"),
        &root.join("state"),
        AdapterRegistry::default(),
    )
    .unwrap()
}

#[test]
fn pcm_submission_is_scoped_bounded_atomic_and_fresh_each_frame() {
    let root = tempfile::tempdir().unwrap();
    let mut rt = runtime(root.path());
    rt.host
        .submit_pcm(ClientId(1), 48_000, &[[0.25, -0.5]])
        .unwrap();
    assert!(matches!(
        rt.host.submit_pcm(ClientId(99), 48_000, &[[1.; 2]]),
        Err(Error::NotFound)
    ));
    assert!(rt.host.submit_pcm(ClientId(1), 44_100, &[[1.; 2]]).is_err());
    assert!(rt
        .host
        .submit_pcm(ClientId(1), 48_000, &[[1.; 2], [f32::NAN, 0.]])
        .is_err());
    assert!(rt
        .host
        .submit_pcm(ClientId(1), 48_000, &[[257., 0.]])
        .is_err());
    assert!(rt
        .host
        .submit_pcm(ClientId(1), 48_000, &vec![[0.; 2]; 96_000])
        .is_err());
    rt.host
        .submit_pcm(ClientId(2), 48_000, &[[0.5, 0.25], [0.75, 0.5]])
        .unwrap();
    assert_eq!(rt.host.take_pcm_output(), [[0.75, -0.25], [0.75, 0.5]]);
    assert!(rt.host.take_pcm_output().is_empty());
    rt.host
        .submit_pcm(ClientId(1), 48_000, &[[0.8; 2]])
        .unwrap();
    rt.tick(1, InputFrame::default()).unwrap();
    assert!(
        rt.host.take_pcm_output().is_empty(),
        "unread prior-frame PCM must not replay"
    );
    let request = AudioRequest::Play {
        channel: 1,
        source: EncodedAudio {
            bytes: Arc::from(b"invalid".as_slice()),
            frame_limit: None,
        },
        intro: None,
        parameters: AudioParameters::default(),
    };
    assert!(matches!(
        rt.host.submit_audio(ClientId(99), request),
        Err(Error::NotFound)
    ));
}
