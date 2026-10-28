//! Vendored-VM semantics that real game scripts depend on (no game data).
fn run(src: &str) -> krkr_tjs2::runtime::Variant {
    krkr_tjs2::execute_source("semantics.tjs", src).unwrap()
}
#[test]
fn ignore_prop_reaches_incontextof_left_operand() {
    let v = run(r#"
var target = %[ v: 5 ];
function f() {
    function local { property p { getter() { return v * 2; } } }
    with (target) { &.p = &local.p incontextof target; }
}
f();
return target.p;
"#);
    assert_eq!(v, krkr_tjs2::runtime::Variant::Integer(10));
}
#[test]
fn skipped_preprocessor_block_with_division_and_line_comment() {
    let v = run("var fsz = 2;\r\n@if (UNDEFINED_FLAG)\r\nvar x = 26/fsz; // note\r\n@endif\r\nreturn 7;");
    assert_eq!(v, krkr_tjs2::runtime::Variant::Integer(7));
}
