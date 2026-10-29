// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Self-authored CXDEC startup: `Storages.setupArchiveData` exists only with an
//! engine-published profile and accepts only the argument that profile names.
//! The profile file here stands in for the one the shared `kirikiri/xp3`
//! mount derives after authenticating a Special index; no title data.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

fn run(source: &str, profile: Option<serde_json::Value>) -> String {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("assets/__cxdec")).unwrap();
    std::fs::write(d.path().join("assets/startup.tjs"), source).unwrap();
    let mut config = json!({"canvas": [32, 32]});
    if let Some(profile) = profile {
        std::fs::write(
            d.path().join("assets/__cxdec/protection.json"),
            serde_json::to_vec(&profile).unwrap(),
        )
        .unwrap();
        config["bootstrap_profile"] = json!("__cxdec/protection.json");
    }
    let path = d.path().join("project.json");
    let p = serde_json::from_value(json!({"schema":2,"name":"cxdec bootstrap","content_revision":"1",
        "canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr",
        "game":"synthetic-cxdec","config":config}]}))
    .unwrap();
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut r = Runtime::with_registry(p, &path, &d.path().join("state"), registry).unwrap();
    r.tick(0, InputFrame::default()).unwrap();
    r.inspect()["sessions"][0]["native"]["log_tail"].to_string()
}

fn profile(text: &str) -> serde_json::Value {
    json!({"family": "cxdec-setup-archive-data", "setup_archive_data": text,
        "setup_archive_data_units": text.len(), "startup": "data.xp3>$", "records": 2})
}

#[test]
fn setup_archive_data_accepts_only_the_mount_authenticated_argument_once() {
    let log = run(
        r#"
    if(typeof Storages.setupArchiveData != "Object") throw new Exception('facade absent');
    var bad=false;try{Storages.setupArchiveData('another-text');}catch(e){bad=true;}
    if(!bad) throw new Exception('foreign argument accepted');
    Storages.setupArchiveData('synthetic-archive-data');
    bad=false;try{Storages.setupArchiveData('synthetic-archive-data');}catch(e){bad=true;}
    if(!bad) throw new Exception('second setup accepted');
    Debug.message('cxdec-setup-ok');
    "#,
        Some(profile("synthetic-archive-data")),
    );
    assert!(log.contains("cxdec-setup-ok"), "{log}");
}

#[test]
fn setup_archive_data_is_absent_without_a_validated_profile() {
    let log = run(
        r#"
    if(typeof Storages.setupArchiveData != "undefined") throw new Exception('invented facade');
    Debug.message('cxdec-absent-ok');
    "#,
        None,
    );
    assert!(log.contains("cxdec-absent-ok"), "{log}");
}
