// SPDX-License-Identifier: MPL-2.0
//! Ren'Py's OpenGL driver modules — `renpy.gl` (GL1/GLSL, 6.18 – 8.3),
//! `renpy.gl2` (model-based renderer, 7.4+), `renpy.uguu` / the 8.6
//! `renpy.gl2.uguu*` GL bindings, the Live2D modules and `renpy.gl2.assimp`
//! (8.4+) — with the module set and names of each release from
//! `gl_api.json` (see `gl_api.rs`).
//!
//! CrySirror renders through its own retained layer pipeline, so these
//! drivers never run: every class and function that needs an OpenGL
//! context, the Live2D Cubism Core or assimp raises `RuntimeError` naming
//! the module, the name and the missing host. Module data (GL enums, shader
//! sources, blacklists, attribute tables) is bound from the upstream
//! literals. Pure data the engine passes around is real:
//! `gl2mesh.AttributeLayout` and its layouts, the draw-mode constants
//! (shared with `renpy.display.render`) and the module-level instances
//! upstream creates at import (`assimp.loader`, `gl2draw.root_context`),
//! which exist without running their GL constructors.
use super::gl_api::{self, Ctx, NO_ASSIMP, NO_CUBISM, NO_GL};
use super::py;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};

// gl2mesh.AttributeLayout ----------------------------------------------------------------

fn al_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let offset = py::arg_or(&args[1..], kw, 0, "offset", Value::None);
    let stride = py::arg_or(&args[1..], kw, 1, "stride", Value::Int(0));
    let d = vm.types.new_dict();
    if !py::is_none(&offset) {
        vm.call_method(&d, "update", vec![offset])?;
    }
    py::set(vm, &args[0], "offset", d)?;
    let stride = py::int(vm, &stride)?;
    py::set(vm, &args[0], "stride", Value::Int(stride))?;
    Ok(Value::None)
}

fn al_add_attribute(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let name = py::arg_or(&args[1..], kw, 0, "name", Value::None);
    let length = py::arg_or(&args[1..], kw, 1, "length", Value::Int(0));
    let stride = py::get(vm, &args[0], "stride")?;
    let offset = py::get(vm, &args[0], "offset")?;
    ops::setitem(vm, &offset, name, stride.clone())?;
    let s = ops::binary_op(vm, &stride, &length, crysirror_python::ast::BinOp::Add, false, false)?;
    py::set(vm, &args[0], "stride", s)?;
    Ok(Value::None)
}

fn al_reduce(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = py::import_attr(vm, "renpy.gl2.gl2mesh", "AttributeLayout")?;
    let offset = py::get(vm, &args[0], "offset")?;
    let stride = py::get(vm, &args[0], "stride")?;
    let a = vm.types.new_tuple(vec![offset, stride]);
    Ok(vm.types.new_tuple(vec![cls, a]))
}

/// `AttributeLayout()` followed by `add_attribute` calls.
fn layout(vm: &mut Vm, ctx: &mut Ctx, attrs: &[(&str, i64)]) -> PyResult<Value> {
    let cls = gl_api::get(vm, ctx, "AttributeLayout")?;
    let l = vm.call(&cls, Vec::new(), Vec::new())?;
    for (n, len) in attrs {
        let n = vm.types.new_str(n);
        vm.call_method(&l, "add_attribute", vec![n, Value::Int(*len)])?;
    }
    Ok(l)
}

fn text_layout(ctx: &Ctx) -> Vec<(&'static str, i64)> {
    let mut v = vec![
        ("a_tex_coord", 2),
        ("a_text_center", 2),
        ("a_text_time", 1),
        ("a_text_min_time", 1),
        ("a_text_max_time", 1),
        ("a_text_index", 1),
        ("a_text_pos_rect", 4),
        ("a_text_ascent", 1),
        ("a_text_descent", 1),
    ];
    if ctx.at_least(8, 5) {
        v.push(("a_text_pseudo_glyph", 1));
    }
    v
}

// Providers --------------------------------------------------------------------------------

fn environ(vm: &mut Vm, name: &str, default: Value) -> PyResult<Value> {
    let env = py::import_attr(vm, "os", "environ")?;
    let k = vm.types.new_str(name);
    vm.call_method(&env, "get", vec![k, default])
}

/// An instance created at import without its GL constructor.
fn bare_instance(vm: &mut Vm, ctx: &mut Ctx, class: &str) -> PyResult<Value> {
    let cls = gl_api::get(vm, ctx, class)?;
    let new = py::get(vm, &cls, "__new__")?;
    vm.call(&new, vec![cls], Vec::new())
}

fn render_const(vm: &mut Vm, name: &str) -> PyResult<Value> {
    py::import_attr(vm, "renpy.display.render", name)
}

fn provide(vm: &mut Vm, ctx: &mut Ctx, name: &str) -> PyResult<Option<Value>> {
    let v = match (ctx.name, name) {
        (_, "check_errors") => {
            let v = environ(vm, "RENPY_GL_CHECK_ERRORS", Value::Int(0))?;
            let int = py::import_attr(vm, "builtins", "int")?;
            vm.call(&int, vec![v], Vec::new())?
        }
        (_, "DISSOLVE" | "IMAGEDISSOLVE" | "PIXELLATE" | "FLATTEN" | "IDENTITY") => render_const(vm, name)?,
        // `egl_available()`: no EGL library is loaded.
        ("renpy.gl.gldraw", "EGL") => Value::Bool(false),
        ("renpy.gl.gltexture", "free_textures") => {
            let dd = py::import_attr(vm, "collections", "defaultdict")?;
            let list = py::import_attr(vm, "builtins", "list")?;
            vm.call(&dd, vec![list], Vec::new())?
        }
        ("renpy.gl2.assimp", "loader") => bare_instance(vm, ctx, "Loader")?,
        ("renpy.gl2.assimp", "loader_lock") => {
            let lock = py::import_attr(vm, "threading", "Lock")?;
            vm.call(&lock, Vec::new(), Vec::new())?
        }
        ("renpy.gl2.gl2draw", "debug") => {
            let e = vm.types.new_str("");
            environ(vm, "RENPY_GL_DEBUG", e)?
        }
        ("renpy.gl2.gl2draw", "default_position") => {
            let c = py::import_attr(vm, "pygame_sdl2", "WINDOWPOS_CENTERED")?;
            vm.types.new_tuple(vec![c.clone(), c])
        }
        ("renpy.gl2.gl2draw", "root_context") => bare_instance(vm, ctx, "GL2DrawingContext")?,
        ("renpy.gl2.gl2mesh", "AttributeLayout") => {
            let c = py::class(
                vm,
                "renpy.gl2.gl2mesh",
                "AttributeLayout",
                &[],
                &[("__init__", al_init), ("add_attribute", al_add_attribute), ("__reduce__", al_reduce)],
                &[],
            )?;
            Value::Obj(c)
        }
        ("renpy.gl2.gl2mesh", "SOLID_LAYOUT") => layout(vm, ctx, &[])?,
        ("renpy.gl2.gl2mesh", "TEXTURE_LAYOUT") => layout(vm, ctx, &[("a_tex_coord", 2)])?,
        ("renpy.gl2.gl2mesh", "MODEL_N_LAYOUT") => layout(vm, ctx, &[("a_tex_coord", 2), ("a_normal", 3)])?,
        ("renpy.gl2.gl2mesh", "MODEL_NT_LAYOUT") => layout(
            vm,
            ctx,
            &[("a_tex_coord", 2), ("a_normal", 3), ("a_tangent", 3), ("a_bitangent", 3)],
        )?,
        ("renpy.gl2.gl2mesh", "TEXT_LAYOUT") => {
            let attrs = text_layout(ctx);
            layout(vm, ctx, &attrs)?
        }
        // Shader-source parsing is pure: the native parser of
        // `gl2shadercache` (8.4+), and a real exception class.
        ("renpy.gl2.gl2shader", "Variable") => {
            let c = py::import_attr(vm, "renpy.gl2.gl2shadercache", "_Variable")?;
            let m = vm.types.new_str("renpy.gl2.gl2shader");
            py::set(vm, &c, "__module__", m)?;
            c
        }
        ("renpy.gl2.gl2shader", "ShaderError") => gl_api::exception_class(vm, "renpy.gl2.gl2shader", "ShaderError")?,
        ("renpy.gl2.gl2shader", "VARYING_TYPES") => {
            let set = py::import_attr(vm, "builtins", "set")?;
            let a = gl_api::get(vm, ctx, "ATTRIBUTE_TYPES")?;
            let u = gl_api::get(vm, ctx, "UNIFORM_TYPES")?;
            let a = vm.call(&set, vec![a], Vec::new())?;
            let u = vm.call(&set, vec![u], Vec::new())?;
            ops::binary_op(vm, &a, &u, crysirror_python::ast::BinOp::BitOr, false, false)?
        }
        _ => return Ok(None),
    };
    Ok(Some(v))
}

// Registration -----------------------------------------------------------------------------

fn reason(module: &str) -> &'static str {
    if module.starts_with("renpy.gl2.live2d") {
        NO_CUBISM
    } else if module == "renpy.gl2.assimp" {
        NO_ASSIMP
    } else {
        NO_GL
    }
}

macro_rules! gl_modules {
    ($($name:literal,)*) => {
        fn register(vm: &mut Vm) {
            $(
                vm.register_native_module($name, |vm| -> PyResult<ObjRef> {
                    gl_api::init(vm, $name, provide, &["*"], reason($name))
                });
            )*
        }
    };
}

gl_modules! {
    "renpy.gl.gl",
    "renpy.gl.gl1",
    "renpy.gl.glblacklist",
    "renpy.gl.gldraw",
    "renpy.gl.glenviron_fixed",
    "renpy.gl.glenviron_limited",
    "renpy.gl.glenviron_shader",
    "renpy.gl.glfunctions",
    "renpy.gl.glrtt_copy",
    "renpy.gl.glrtt_fbo",
    "renpy.gl.gltexture",
    "renpy.gl2.assimp",
    "renpy.gl2.gl2debug",
    "renpy.gl2.gl2draw",
    "renpy.gl2.gl2functions",
    "renpy.gl2.gl2geometry",
    "renpy.gl2.gl2mesh",
    "renpy.gl2.gl2mesh2",
    "renpy.gl2.gl2mesh3",
    "renpy.gl2.gl2model",
    "renpy.gl2.gl2physics",
    "renpy.gl2.gl2polygon",
    "renpy.gl2.gl2shader",
    "renpy.gl2.gl2statecache",
    "renpy.gl2.gl2texture",
    "renpy.gl2.gl2uniform",
    "renpy.gl2.live2dblend",
    "renpy.gl2.live2dmodel",
    "renpy.gl2.live2dmotion",
    "renpy.gl2.live2dphysics",
    "renpy.gl2.live2dpose",
    "renpy.gl2.uguu",
    "renpy.gl2.uguugl",
    "renpy.uguu.gl",
    "renpy.uguu.uguu",
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module("renpy.gl", |vm| gl_api::package(vm, "renpy.gl"));
    vm.register_native_module("renpy.uguu", |vm| gl_api::package(vm, "renpy.uguu"));
    register(vm);
}
