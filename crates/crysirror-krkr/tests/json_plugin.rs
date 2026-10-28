// Copyright (c) 2026 CrySirror contributors.
// Distributed under the GNU Affero General Public License, version 3 or
// (at your option) any later version, https://www.gnu.org/licenses/agpl-3.0.html.
//! Self-authored native json.dll -> shared document services -> scoped storage.
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
    let p=serde_json::from_value(json!({"schema":2,"name":"json plugin","content_revision":"1","canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"synthetic-json","config":{"canvas":[32,32]}}]})).unwrap();
    std::fs::write(&path, serde_json::to_vec(&p).unwrap()).unwrap();
    let mut registry = AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let r = Runtime::with_registry(p, &path, &d.path().join("state"), registry).unwrap();
    (d, r)
}
#[test]
fn json_plugin_calls_shared_parser_and_writer_and_waits_for_scoped_atomic_write() {
    let (_d, mut r) = fixture(
        r#"
    if(!CanLoadPlugin('json.dll'))throw new Exception('availability');
    Plugins.link('json.dll');if(Plugins.getStatus('json.dll').kind!='engine_facade')throw new Exception('fake status');
    var d=Scripts.evalJSON("/* comment */ {'n'=>1e5; 'values':[true,,void,], 's':'text'}");
    if(d.n!=1||d.values.count!=3||d.values[1]!==void||d.s!='text')throw new Exception('parse mismatch');
    var serialized=Scripts.toJSONString(d,1);var round=Scripts.evalJSON(serialized);
    if(round.values.count!=3||round.n!=1)throw new Exception('roundtrip');
    var fail=false;try{Scripts.evalJSON('{bare:1}');}catch(e){fail=true;}if(!fail)throw new Exception('invented bare grammar');
    var c=%[];c.self=c;fail=false;try{Scripts.toJSONString(c);}catch(e){fail=true;}if(!fail)throw new Exception('cycle admitted');
    Scripts.saveJSON('savedata/config.json',d,true,1);
    var restored=Scripts.evalJSONStorage('savedata/config.json',true);
    if(restored.n!=1||restored.s!='text')throw new Exception('save publication');
    if(Plugins.unlink('json.dll')!=0)throw new Exception('live facade unload');
    Debug.message('shared-json-complete');
    "#,
    );
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        r.tick(1, InputFrame::default()).unwrap();
        if r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("shared-json-complete")
        {
            break;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::yield_now()
    }
    let bytes = r
        .host
        .storage(ClientId(1))
        .unwrap()
        .read(&VfsPath::new(Mount::Save, "config.json").unwrap())
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("\n"));
    assert!(!text.contains("\r"));
    assert!(text.contains("\"n\":1"));
}
#[test]
fn json_storage_rejects_oversized_stream_before_reading_it() {
    let (d, mut r) = fixture(
        "Plugins.link('json.dll'); var bad=false;try{Scripts.evalJSONStorage('large.json',true);}catch(e){bad=true;}if(!bad)throw new Exception('oversize admitted');Debug.message('json-size-ok');",
    );
    let f = std::fs::File::create(d.path().join("assets/large.json")).unwrap();
    f.set_len(1024 * 1024 + 1).unwrap();
    r.tick(0, InputFrame::default()).unwrap();
    assert!(
        r.inspect()["sessions"][0]["native"]["log_tail"]
            .to_string()
            .contains("json-size-ok")
    );
}
