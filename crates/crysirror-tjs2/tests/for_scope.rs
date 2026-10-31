// Derived VM licence: AGPL-3.0-or-later; authored scope regressions.
use crysirror_tjs2::{compile_source_to_bytecode, runtime::{Runtime, Variant}};

fn run(source: &str) -> Variant {
    let file=compile_source_to_bytecode("for-scope.tjs",source).unwrap();
    Runtime::new().execute_file(&file).unwrap()
}

#[test]
fn nested_for_initializers_shadow_without_replacing_outer_metadata_index() {
    let result=run(r#"
        function check() {
            var i=71, seen='';
            var records=[%[filename:'a'],%[filename:'b'],%[filename:'c']];
            for(var i=0;i<records.count;i++) {
                for(var i=0;i<3;i++) { if(i==1)continue; if(i==2)break; }
                seen+=records[i].filename;
            }
            return seen+':'+i;
        }
        return check();
    "#);
    assert!(matches!(result,Variant::String(ref s) if s=="abc:71"),"{result:?}");
}

#[test]
fn declaration_free_for_keeps_outer_index_and_scopes_its_single_statement_body() {
    let result=run(r#"
        function check() {
            var i=9;
            for(i=0;i<3;i++) { for(var i=0;i<1;i++) {} }
            var finalIndex=i,loopOnly=9;
            for(i=0;i<1;i++) var loopOnly=42;
            return finalIndex+':'+loopOnly;
        }
        return check();
    "#);
    assert!(matches!(result,Variant::String(ref s) if s=="3:9"),"{result:?}");
}

#[test]
fn break_restores_outer_binding_and_return_uses_active_loop_binding() {
    let result=run(r#"
        function afterBreak() {
            var i=91;
            for(var i=0;i<3;i++) { break; }
            return i;
        }
        function afterReturn() {
            for(var i=0;i<5;i++) { if(i==2)return i; }
        }
        var i=57;
        return afterBreak()+':'+afterReturn()+':'+i;
    "#);
    assert!(matches!(result,Variant::String(ref s) if s=="91:2:57"),"{result:?}");
}

#[test]
fn top_level_loop_local_does_not_overwrite_existing_global() {
    let result=run(r#"
        var index=99,seen='';
        for(var index=0;index<3;index++) { seen+=index; }
        return seen+':'+index;
    "#);
    assert!(matches!(result,Variant::String(ref s) if s=="012:99"),"{result:?}");
}
