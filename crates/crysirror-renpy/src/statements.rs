// SPDX-License-Identifier: MPL-2.0
//! Ren'Py 6.99 creator-defined statements and their script lexer arguments.
use crate::{
    ast::{Arena, Node, NodeId, NodeKind},
    engine::state,
    lexer::{GroupedLine, Lexer, PResult},
};
use crysirror_python::{ObjRef, Payload, PyResult, Value, Vm, builtins::Kw, object::BaseKind, ops};
use std::rc::Rc;

#[derive(Clone)]
pub struct Definition {
    parse: Value,
    execute: Value,
    next: Value,
    block: bool,
    init: bool,
}

pub fn register(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let [name, parse, execute, next, block, init] = args else {
        return Err(vm.type_error("invalid statement registration arguments"));
    };
    let parts: Vec<_> = name
        .as_str()
        .ok_or_else(|| vm.type_error("statement name must be a string"))?
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let definition = Definition {
        parse: parse.clone(),
        execute: execute.clone(),
        next: next.clone(),
        block: ops::truthy(vm, block)?,
        init: ops::truthy(vm, init)?,
    };
    state(vm)
        .user_statements
        .borrow_mut()
        .insert(parts, definition);
    Ok(Value::None)
}

pub fn parse_source(
    arena: &mut Arena,
    definition: &Definition,
    lexer: &mut Lexer,
    loc: &(Rc<str>, u32),
) -> PResult<Vec<NodeId>> {
    if definition.block {
        lexer.expect_block("creator-defined statement")?;
    } else {
        lexer.expect_noblock("creator-defined statement")?;
    }
    let id = arena.add(
        loc,
        NodeKind::UserStatement {
            line: lexer.current_text(),
            block: lexer.subblock.clone(),
            parsed: None,
            subparses: Vec::new(),
            code_block: None,
            init_priority: 0,
            translatable: false,
            atl: None,
        },
    );
    let id = if definition.init && !lexer.init {
        arena.add(
            loc,
            NodeKind::Init {
                block: vec![id],
                priority: 0,
            },
        )
    } else {
        id
    };
    lexer.advance();
    Ok(vec![id])
}

pub fn parse(vm: &mut Vm, node: &Node, line: &str, block: Rc<Vec<GroupedLine>>) -> PyResult<Value> {
    let group = GroupedLine {
        filename: node.filename.clone(),
        number: node.linenumber,
        indent: 0,
        text: line.chars().collect::<Vec<_>>().into(),
        block,
    };
    let mut lexer = Lexer::new(Rc::new(vec![group]), false);
    lexer.advance();
    let st = state(vm);
    let (name, definition) = {
        let definitions = st.user_statements.borrow();
        definitions
            .iter()
            .filter(|(name, _)| {
                let mut probe = lexer.clone();
                name.iter().all(|word| probe.keyword(word))
            })
            .max_by_key(|(name, _)| name.len())
            .map(|(name, def)| (name.clone(), def.clone()))
    }
    .ok_or_else(|| {
        vm.exc_msg(
            "NotImplementedError",
            format!("unregistered statement {line}"),
        )
    })?;
    for word in &name {
        lexer.keyword(word);
    }
    let parsed = if definition.parse.is_none() {
        Value::None
    } else {
        let class = vm.ext_get::<LexerClass>().expect("lexer class").0.clone();
        let cursor = vm.types.new_native(&class, Box::new(lexer));
        vm.call1(&definition.parse, cursor)?
    };
    Ok(vm
        .types
        .new_tuple(vec![definition.execute, definition.next, parsed]))
}

struct LexerClass(ObjRef);

pub fn install(vm: &mut Vm) {
    use crysirror_python::object::NativeFn;
    let class = vm
        .types
        .new_builtin_type("Lexer", &vm.types.object, BaseKind::Object, true);
    let methods: &[(&str, NativeFn)] = &[
        ("eol", eol),
        ("keyword", keyword),
        ("name", name),
        ("simple_expression", simple_expression),
        ("rest", rest),
        ("expect_eol", expect_eol),
        ("error", error),
        ("advance", advance),
        ("require", require),
    ];
    for &(name, f) in methods {
        vm.types.add_method(&class, name, f);
    }
    vm.ext_set(LexerClass(class));
}

fn run<T>(vm: &mut Vm, args: &[Value], f: impl FnOnce(&mut Lexer) -> T) -> PyResult<T> {
    let Some(Payload::Native(data)) = args.first().and_then(Value::payload) else {
        return Err(vm.type_error("expected statement lexer"));
    };
    let mut data = data.borrow_mut();
    let lexer = data
        .downcast_mut::<Lexer>()
        .ok_or_else(|| vm.type_error("expected statement lexer"))?;
    Ok(f(lexer))
}

fn eol(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(run(vm, a, Lexer::eol)?))
}

fn keyword(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let word = a
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| vm.type_error("expected a keyword"))?;
    Ok(Value::Bool(run(vm, a, |lexer| lexer.keyword(word))?))
}

fn name(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    Ok(run(vm, a, Lexer::name)?.map_or(Value::None, |s| vm.types.new_str(&s)))
}

fn simple_expression(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let expression = run(vm, a, Lexer::simple_expression)?
        .map_err(|e| vm.exc_msg("SyntaxError", e.to_string()))?;
    Ok(expression.map_or(Value::None, |e| vm.types.new_str(&e.text)))
}

fn rest(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let rest = run(vm, a, Lexer::raw_rest)?;
    Ok(vm.types.new_str(&rest))
}

fn expect_eol(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    run(vm, a, Lexer::expect_eol)?.map_err(|e| vm.exc_msg("SyntaxError", e.to_string()))?;
    Ok(Value::None)
}

fn advance(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(run(vm, a, |l| {
        l.advance();
        !l.eob
    })?))
}

fn error(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let message = a
        .get(1)
        .and_then(Value::as_str)
        .unwrap_or("invalid statement");
    let parsed = run(vm, a, |l| l.error::<()>(message))?;
    Err(vm.exc_msg("SyntaxError", parsed.unwrap_err().to_string()))
}

fn require(vm: &mut Vm, a: &[Value], _: Kw) -> PyResult<Value> {
    let Some(call) = a.get(1) else {
        return Err(vm.type_error("require expects a lexer method"));
    };
    let value = vm.call0(call)?;
    if value.is_none() {
        let msg = a
            .get(2)
            .and_then(Value::as_str)
            .unwrap_or("required token not found");
        return error(vm, &[a[0].clone(), vm.types.new_str(msg)], &[]);
    }
    Ok(value)
}
