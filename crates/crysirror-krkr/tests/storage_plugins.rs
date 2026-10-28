// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Self-authored native storage plugins -> shared engine services -> scoped storage.
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{HostServices, InputFrame},
    state::ClientId,
};
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;
fn fixture(source: &str) -> (tempfile::TempDir, Runtime) {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("assets")).unwrap();
    std::fs::write(d.path().join("assets/startup.tjs"), source).unwrap();
    let path = d.path().join("project.json");
    let p=serde_json::from_value(json!({"schema":2,"name":"storage plugins","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-storage","config":{"canvas":[32,32]}}]})).unwrap();
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let r = Runtime::with_registry(p, &path, &d.path().join("state"), registry).unwrap();
    (d, r)
}
fn finish(r: &mut Runtime, marker: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        r.tick(1, InputFrame::default()).unwrap();
        if r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains(marker)
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{}", r.inspect());
        std::thread::yield_now();
    }
}
#[test]
fn save2_old_and_new_arrays_encode_exactly_and_publish_before_resume() {
    let (_dir, mut runtime) = fixture(
        r#"
        var old = ["日本", "text"];
        Plugins.link("savestruct.dll");
        if(Plugins.getStatus("savestruct.dll").kind != "engine_facade") throw new Exception("status");
        old.save2("savedata/user.cfu");
        if(!Storages.isExistentStorage("savedata/user.cfu")) throw new Exception("early resume");
        var fresh = new Array("日本", "next"); fresh.save2("savedata/utf8.cfu", 1, 1);
        var split = new Array(); split.split(",", "a,b"); split.save2("savedata/split.cfu", 1, 1);
        var bad = false; try { ["😀"].save2("savedata/bad.cfu"); } catch(e) {bad=true;}
        if(!bad) throw new Exception("lossy ACP conversion");
        bad=false;try{["bad"].save2("../escape.cfu");}catch(e){bad=true;}
        if(!bad)throw new Exception("root escaped");
        bad=false;try{[123].save2("savedata/number.cfu");}catch(e){bad=true;}
        if(!bad)throw new Exception("coercion mismatch");
        Debug.message("storage-save-complete");
    "#,
    );
    finish(&mut runtime, "storage-save-complete");
    let storage = runtime.host.storage(ClientId(1)).unwrap();
    assert_eq!(
        storage
            .read(&VfsPath::new(Mount::Save, "user.cfu").unwrap())
            .unwrap(),
        b"\x93\xfa\x96\x7b\r\ntext\r\n"
    );
    assert_eq!(
        storage
            .read(&VfsPath::new(Mount::Save, "utf8.cfu").unwrap())
            .unwrap(),
        "日本\nnext\n".as_bytes()
    );
    assert_eq!(
        storage
            .read(&VfsPath::new(Mount::Save, "split.cfu").unwrap())
            .unwrap(),
        b"a\nb\n"
    );
    assert!(!storage.exists(&VfsPath::new(Mount::Save, "bad.cfu").unwrap()));
}
#[test]
fn fstat_metadata_listing_and_explicit_unsupported_operations() {
    let (dir, mut runtime) = fixture(
        r#"
        Plugins.link("savestruct.dll");Plugins.link("fstat.dll");
        if(!Storages.isExistentDirectory("empty/"))throw new Exception("empty dir absent");
        if(Storages.isExistentDirectory("missing/"))throw new Exception("phantom directory");
        var items=Storages.dirlist("folder/");if(items.count!=2||items[0]!="child/"||items[1]!="tiny.bin")throw new Exception("dirlist");
        var stat=Storages.fstat("folder/tiny.bin");if(stat.size!=3||stat.mtime.getTime()<=0)throw new Exception("metadata");
        ["saved"].save2("savedata/check.cfu",1,1);
        if(Storages.fstat("savedata/check.cfu").size!=6)throw new Exception("save metadata");
        if(Storages.dirlist("savedata/")[0]!="check.cfu")throw new Exception("save root");
        var fail=false;try{Storages.dirlist("missing/");}catch(e){fail=true;}if(!fail)throw new Exception("missing directory");
        fail=false;try{Storages.fstat("../outside");}catch(e){fail=true;}if(!fail)throw new Exception("metadata escaped");
        fail=false;try{Storages.copyFile("folder/tiny.bin","savedata/copy.bin");}catch(e){fail=true;}if(!fail)throw new Exception("fake mutation");
        Debug.message("storage-stat-complete");
    "#,
    );
    std::fs::create_dir_all(dir.path().join("assets/folder/child")).unwrap();
    std::fs::create_dir_all(dir.path().join("assets/empty")).unwrap();
    std::fs::write(dir.path().join("assets/folder/tiny.bin"), b"abc").unwrap();
    finish(&mut runtime, "storage-stat-complete");
}

#[test]
fn save2_worker_failure_does_not_resume_as_success() {
    let (_dir, mut runtime) = fixture(
        r#"
        Plugins.link("savestruct.dll");
        ["cannot commit"].save2("savedata/blocked/child.cfu");
        Debug.message("unexpected-write-success");
    "#,
    );
    runtime
        .host
        .storage(ClientId(1))
        .unwrap()
        .write_atomic(
            &VfsPath::new(Mount::Save, "blocked").unwrap(),
            b"file instead of directory",
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match runtime.tick(1, InputFrame::default()) {
            Err(e) => {
                assert!(format!("{e:#}").contains("native atomic write failed"), "{e}");
                break;
            }
            Ok(_) => {
                assert!(
                    !runtime.inspect()["sessions"][0]["native"]["log_tail"]
                        .to_string()
                        .contains("unexpected-write-success")
                );
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
    }
}
