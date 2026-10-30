// Distributed under the GNU Affero General Public License, version 3 or later.
//! Legacy built-in parser registration uses the original VM, not a plugin shell.
use crysirror_core::service::InputFrame;
use crysirror_runtime::{AdapterRegistry, Runtime};
use serde_json::json;

#[test]
fn builtin_parser_inheritance_executes_scenario_without_a_dll_link() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/startup.tjs"), r#"
        class LegacyConductor extends KAGParser {
            function LegacyConductor(){super.KAGParser();ignoreCR=true;}
        }
        var parser=new LegacyConductor();
        parser.onScenarioLoad=function(name){return '[wait time=4]';};
        parser.loadScenario('inline.ks');
        var tag=parser.getNextTag();
        if(tag.tagname!='wait'||tag.time!='4')throw new Exception('builtin parser did not parse');
        Debug.message('builtin-kag-complete');
    "#).unwrap();
    let project = serde_json::from_value(json!({"schema":2,"name":"builtin-kag","content_revision":"1",
        "canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"authored",
        "config":{"canvas":[32,32]}}]})).unwrap();
    let path = dir.path().join("project.json");
    std::fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry=AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut runtime=Runtime::with_registry(project,&path,&dir.path().join("state"),registry).unwrap();
    runtime.tick(0,InputFrame::default()).unwrap();
    let native=&runtime.inspect()["sessions"][0]["native"];
    assert!(native["log_tail"].to_string().contains("builtin-kag-complete"));
    assert!(native["plugin_shells"].as_array().unwrap().is_empty());
}
