//! Vendored-VM semantics that real game scripts depend on (no game data).
fn run(src: &str) -> crysirror_tjs2::runtime::Variant {
    crysirror_tjs2::execute_source("semantics.tjs", src).unwrap()
}

#[test]
fn krkrz_rect_constructs_copies_and_applies_signed_edge_operations() {
    use crysirror_krkr::{host::KrkrHost, natives};
    use crysirror_tjs2::runtime::{Runtime, Variant};

    let mut rt = Runtime::with_host(KrkrHost::new([640, 480], "ui".into()));
    natives::install(&mut rt);
    let file = rt.compile_source("rect.tjs", r#"
var r = new Rect(-4, 3, 16, 13);
var copy = new Rect(r);
if (!(r instanceof "Rect") || r.width != 20 || r.height != 10 || !copy.equal(r))
    throw "construction: " + (r instanceof "Rect") + ", " + r.width + ", " + r.height + ", " + copy.equal(r);
r.setSize(2, 4);
r.setOffset(11, -2);
r.addOffset(1, 2);
if (r.left != 12 || r.top != 0 || r.right != 14 || r.bottom != 4 || copy.left != -4) throw "offset";
if (!r.includedPos(12, 0) || r.includedPos(14, 4)) throw "half-open edges";
var other = new Rect(13, 1, 17, 5);
if (!r.intersects(other) || !r.clip(other) || !r.included(other)) throw "clip";
if (r.clip(new Rect(0, 0, 1, 1)) || r.left != 13 || r.right != 14) throw "unchanged on empty clip";
if (!r.union(copy) || !copy.included(r) || r.bottom != 13) throw "union";
r.clear();
if (!r.isEmpty() || r.left != 0 || r.top != 0) throw "clear";
r.set(5, 6, 12, 10);
r.width = -2;
r.height = 3;
if (r.right != 3 || r.bottom != 9 || !r.isEmpty() || r.included(copy)) throw "signed extents";
if (r.clip(null) !== void || (new Rect(1, 2)).isEmpty() !== 1) throw "native edge cases";
var invalid = false;
try { var bad = new Rect(%[]); } catch (e) { invalid = true; }
if (!invalid) throw "non-Rect copy";
return 1;
"#).unwrap();
    assert_eq!(rt.execute_file(&file).unwrap(), Variant::Integer(1));
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
fn preprocessor_values_persist_across_scripts_of_one_engine() {
    // Yuzusoft Config.tjs `@set (KAGHOOK_EVENTACTION=1)` is tested by
    // yuzu_touchui.tjs later (`tTJS::PPValues` is per engine instance).
    let mut rt = crysirror_tjs2::runtime::Runtime::new();
    let first = rt.compile_source("Config.tjs", "@set (FLAG_X=1)\nreturn 0;").unwrap();
    rt.execute_file(&first).unwrap();
    let second = rt
        .compile_source("later.tjs", "@if (FLAG_X)\nreturn 1;\n@endif\nreturn 2;")
        .unwrap();
    assert_eq!(rt.execute_file(&second).unwrap(), crysirror_tjs2::runtime::Variant::Integer(1));
    // A separate engine instance starts clean.
    let other = crysirror_tjs2::runtime::Runtime::new();
    let third = other
        .compile_source("later.tjs", "@if (FLAG_X)\nreturn 1;\n@endif\nreturn 2;")
        .unwrap();
    assert_eq!(
        crysirror_tjs2::runtime::Runtime::new().execute_file(&third).unwrap(),
        crysirror_tjs2::runtime::Variant::Integer(2)
    );
}
#[test]
fn eval_expression_mode_accepts_a_trailing_semicolon() {
    // tjsLex.cpp expression mode: `return` + text + `;` (envinit.tjs ends `];`).
    let v = run(r#"return ("%[ \"a\" => 3 ];")!.a;"#);
    assert_eq!(v, crysirror_tjs2::runtime::Variant::Integer(3));
}

#[test]
fn super_member_of_a_multiply_inherited_class_comes_from_the_last_base() {
    // tjsInterCodeGen.cpp CreateExtendsExprCode: each `extends` base runs its
    // class body on the new object in order, so later bases replace earlier
    // members. KAGEX's AnimationLayerTransMove relies on this so
    // GraphicLayer's `super.beginTransition` reaches KAGTransMove.
    let value = run(r#"
        class A { function f() { return "A"; } }
        class B { function f() { return "B"; } }
        class AB extends A, B { function AB() { global.A.A(); global.B.B(); } }
        class C extends AB { function f() { return "C" + super.f(); } }
        var o = new C();
        var ab = new AB();
        return o.f() + ab.f();
    "#);
    assert_eq!(value, crysirror_tjs2::runtime::Variant::String("CBB".into()));
}
