//! Shared source precedence, without a VM, format-specific retry or game data.
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{FileSystem, HostServices},
    state::ClientId,
    vfs::{MemoryProvider, VfsProvider},
};
use crysirror_runtime::{AdapterRegistry, Project, Runtime};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
struct Origin {
    files: MemoryProvider,
    calls: Arc<AtomicUsize>,
}
impl VfsProvider for Origin {
    fn kind(&self) -> &'static str {
        "authored-archive"
    }
    fn container_source(&self) -> Option<&str> {
        Some("install.cab")
    }
    fn read(&self, path: &str) -> crysirror_core::Result<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.files.read(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.files.exists(path)
    }
    fn list(&self) -> crysirror_core::Result<Vec<String>> {
        self.files.list()
    }
}
fn asset(path: &str) -> VfsPath {
    VfsPath::new(Mount::Asset, path).unwrap()
}
#[test]
fn fallback_prefers_real_loose_bytes_and_keeps_missing_entries_and_origins_scoped() {
    let root = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir(root.path().join(name)).unwrap();
    }
    std::fs::write(root.path().join("a/shared.bin"), b"real-data").unwrap();
    std::fs::write(root.path().join("a/only-loose.bin"), b"loose").unwrap();
    std::fs::write(root.path().join("b/shared.bin"), b"other-client").unwrap();
    let project:Project=serde_json::from_value(serde_json::json!({"schema":2,"name":"fallback","content_revision":"1","assets":"game","canvas":[4,4],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a","asset_prefix":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b","asset_prefix":"b"}]})).unwrap();
    let mut rt = Runtime::with_asset_root(
        project,
        root.path(),
        &root.path().join("state"),
        AdapterRegistry::default(),
    )
    .unwrap();
    let a = ClientId(1);
    let b = ClientId(2);
    let before = rt.host.storage(a).unwrap();
    let mut files = MemoryProvider::new();
    files.insert("shared.bin", b"\0\0\0\0".to_vec()).unwrap();
    files
        .insert("only-archive.bin", b"script".to_vec())
        .unwrap();
    files.insert("folder/child.bin", b"child".to_vec()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    rt.mount_fallback_provider(
        a,
        "",
        Arc::new(Origin {
            files,
            calls: calls.clone(),
        }),
    )
    .unwrap();
    assert_eq!(before.read(&asset("shared.bin")).unwrap(), b"real-data");
    assert_eq!(rt.host.read(a, &asset("shared.bin")).unwrap(), b"real-data");
    assert_eq!(
        rt.assets.read_for(a, "shared.bin").unwrap().as_ref(),
        b"real-data"
    );
    let reader = before.open_asset(&asset("shared.bin")).unwrap();
    let mut bytes = [0; 9];
    reader.read_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"real-data");
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "shadowed placeholder decoder must not be touched"
    );
    assert!(!before.served_by_container(&asset("shared.bin")));
    assert!(before.serving_container(&asset("shared.bin")).is_none());
    assert_eq!(before.read(&asset("only-archive.bin")).unwrap(), b"script");
    assert!(before.served_by_container(&asset("only-archive.bin")));
    assert_eq!(
        before.serving_container(&asset("only-archive.bin")),
        Some(("install.cab".into(), "".into()))
    );
    assert_eq!(
        before.list(Mount::Asset, "").unwrap(),
        vec!["folder", "only-archive.bin", "only-loose.bin", "shared.bin"]
    );
    assert_eq!(
        rt.host
            .storage(b)
            .unwrap()
            .read(&asset("shared.bin"))
            .unwrap(),
        b"other-client"
    );
    assert!(rt
        .host
        .storage(b)
        .unwrap()
        .read(&asset("only-archive.bin"))
        .is_err());
    let mut patch = MemoryProvider::new();
    patch.insert("shared.bin", b"patch".to_vec()).unwrap();
    rt.mount_provider(a, "", Arc::new(patch)).unwrap();
    assert_eq!(
        before.read(&asset("shared.bin")).unwrap(),
        b"patch",
        "normal later overlay remains authoritative"
    );
    let mut old = [0; 9];
    reader.read_at(0, &mut old).unwrap();
    assert_eq!(
        &old, b"real-data",
        "opened reader retains the selected generation"
    );
}
