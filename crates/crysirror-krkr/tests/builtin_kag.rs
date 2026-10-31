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

#[test]
fn parser_ex_ordered_names_include_tagname_and_repeated_attributes() {
    let dir=tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/startup.tjs"),r#"
        var legacy=new KAGParser();legacy.ignoreCR=true;
        legacy.onScenarioLoad=function(name){return '[surface show]';};
        Plugins.link('KAGParserEx.dll');
        legacy.loadScenario('old.ks');
        if(legacy.getNextTag().taglist!==void)throw new Exception('existing parser format changed');
        var parser=new KAGParser();parser.ignoreCR=true;
        parser.onScenarioLoad=function(name){return '[surface show][surface x=1 x=2][plain]';};
        parser.loadScenario('ordered.ks');
        var tag=parser.getNextTag();
        if(tag.taglist.count!=2||tag.taglist[0]!='tagname'||tag.taglist[1]!='show')
            throw new Exception('single attribute command cannot pass native creation policy');
        if(tag.tagname!='surface'||!+tag.show)throw new Exception('native tag values');
        tag=parser.getNextTag();
        if(tag.taglist.count!=3||tag.taglist[0]!='tagname'||tag.taglist[1]!='x'||tag.taglist[2]!='x'||tag.x!='2')
            throw new Exception('written duplicate attribute order was lost');
        tag=parser.getNextTag();
        if(tag.taglist.count!=1||tag.taglist[0]!='tagname')throw new Exception('empty tag order');
        Debug.message('kag-ex-ordered-complete');
    "#).unwrap();
    let project=serde_json::from_value(json!({"schema":2,"name":"kag-ex-order","content_revision":"1",
        "canvas":[32,32],"assets":"assets","clients":[{"name":"native","adapter":"krkr","game":"authored",
        "config":{"canvas":[32,32]}}]})).unwrap();
    let path=dir.path().join("project.json");
    std::fs::write(&path,serde_json::to_vec(&project).unwrap()).unwrap();
    let mut registry=AdapterRegistry::default();
    crysirror_krkr::register(&mut registry).unwrap();
    let mut runtime=Runtime::with_registry(project,&path,&dir.path().join("state"),registry).unwrap();
    runtime.tick(0,InputFrame::default()).unwrap();
    let native=&runtime.inspect()["sessions"][0]["native"];
    assert!(native["log_tail"].to_string().contains("kag-ex-ordered-complete"));
    assert!(native["plugin_shells"].as_array().unwrap().is_empty());
}
