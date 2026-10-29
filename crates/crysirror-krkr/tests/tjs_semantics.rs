//! Vendored-VM semantics that real game scripts depend on (no game data).
fn run(src: &str) -> crysirror_tjs2::runtime::Variant {
    crysirror_tjs2::execute_source("semantics.tjs", src).unwrap()
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
    assert_eq!(v, crysirror_tjs2::runtime::Variant::Integer(10));
}
#[test]
fn skipped_preprocessor_block_with_division_and_line_comment() {
    let v = run("var fsz = 2;\r\n@if (UNDEFINED_FLAG)\r\nvar x = 26/fsz; // note\r\n@endif\r\nreturn 7;");
    assert_eq!(v, crysirror_tjs2::runtime::Variant::Integer(7));
}
#[test]
fn invalidate_this_under_postfix_if() {
    // MyYesNoDialog.tjs shape. krkrz tjs.y: `expr: comma_expr "if" expr` (the
    // `if` binds lowest); tjsInterCodeGen.cpp T_INVALIDATE is a general unary
    // operator (VM_INV on the operand value), so `this` needs no lvalue.
    let v = run(r#"
class D {
    function D() {}
    function finalize() { invalidate this if (isvalid this); }
}
var d = new D();
d.finalize();
var skipped = 0, taken = 0;
skipped++ if (isvalid d);
taken += 2, taken += 3 if (!isvalid d);
var o = %[];
invalidate o;
function make() { return %[]; }
invalidate make();
return (isvalid d) + ":" + skipped + ":" + taken + ":" + (isvalid o);
"#);
    assert_eq!(v, crysirror_tjs2::runtime::Variant::String("0:0:5:0".into()));
}
#[test]
fn delete_and_increment_still_require_an_lvalue() {
    for src in ["delete 1;", "1++;", "++(1);"] {
        assert!(crysirror_tjs2::execute_source("semantics.tjs", src).is_err(), "{src}");
    }
}
#[test]
fn null_context_property_setter_runs_on_the_holding_object() {
    // Utils.tjs objectHookInjection shape (Yuzusoft): an eval'd property whose
    // setter rebinds a hook closure to `this` -- the object holding the
    // property, because it was stored `incontextof null` -- while the hook
    // reaches a helper the script put on that same object.
    let v = run(r#"
global.__T = %[];
class Owner {
    function Owner() {}
    function helper(name, ref, v) { global.__out = name + "=" + v; }
    function setFace(ref, v) { helper("face", ref, v); }
}
function inject(target, hook) {
    var holder = new Dictionary();
    var make = function(key) {
        ("property prop { setter(v) { with (global.__T['" + key + "']) (.setter incontextof this)((&.orig) incontextof this, v); } getter { return 0; } }")!;
        return &this.prop;
    } incontextof holder;
    var p = make("k");
    p["setter"] = hook;
    global.__T["k"] = p;
    target.face = p incontextof null;
}
var owner = new Owner();
var font = new Dictionary();
font.helper = owner.helper;
inject(font, owner.setFace);
font.face = "b";
return global.__out;
"#);
    assert_eq!(v, crysirror_tjs2::runtime::Variant::String("face=b".into()));
}
