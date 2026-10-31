// Distributed under the GNU Affero General Public License, version 3 or later.
//! Native semantics use the shared surface budget and retain slot reuse.
use crysirror_core::{service::{InputFrame,MAX_CANVAS_SURFACES_PER_CLIENT},state::ClientId};
use crysirror_runtime::{AdapterRegistry,Runtime};
use serde_json::json;

#[test]
fn native_layers_share_admission_and_reuse_invalidated_slots_above_one_thousand() {
    let limit=MAX_CANVAS_SURFACES_PER_CLIENT;
    let temp=tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("assets")).unwrap();
    let source=r#"
        var w=new Window(),root=new Layer(w,null),last,held=[];
        for(var i=1;i<__LIMIT__;i++) { last=new Layer(w,root);held.add(last); }
        var denied=false;
        try { new Layer(w,root); } catch(e) { denied=e.message.indexOf('layer limit')>=0; }
        if(!denied)throw new Exception('native layer admission was unbounded');
        invalidate last;
        last=new Layer(w,root);
        Debug.message('shared-native-budget-complete');
    "#.replace("__LIMIT__",&limit.to_string());
    std::fs::write(temp.path().join("assets/startup.tjs"),source).unwrap();
    let project=serde_json::from_value(json!({"schema":2,"name":"native-budget","content_revision":"1",
        "canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"authored",
        "config":{"canvas":[32,32]}}]})).unwrap();
    let path=temp.path().join("project.json");
    std::fs::write(&path,serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry=AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut runtime=Runtime::with_registry(project,&path,&temp.path().join("state"),registry).unwrap();
    runtime.tick(0,InputFrame::default()).unwrap();
    let inspect=runtime.inspect();
    let native=&inspect["sessions"][0]["native"];
    assert_eq!(native["layers"],limit);
    assert!(native["log_tail"].to_string().contains("shared-native-budget-complete"));
    assert!(runtime.host.engine.canvases.surface(ClientId(1),limit as u32).is_some());
    assert!(runtime.host.engine.canvases.surface(ClientId(1),limit as u32+1).is_none());
    runtime.host.engine.canvases.validate().unwrap();
}
