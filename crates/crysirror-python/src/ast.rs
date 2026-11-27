// SPDX-License-Identifier: MPL-2.0
//! Python syntax tree. Mirrors CPython's `ast` module closely enough that the
//! compiler can follow CPython's symbol-table and code-generation rules. The
//! Python 2 `print`/`exec` statements and backtick repr are kept as distinct
//! nodes so the dialect difference stays visible.

pub type Ident = String;

#[derive(Clone, Debug)]
pub struct Loc {
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub loc: Loc,
    pub kind: StmtKind,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub loc: Loc,
    pub kind: ExprKind,
}

#[derive(Clone, Debug, Default)]
pub struct Arguments {
    pub posonly: Vec<Arg>,
    pub args: Vec<Arg>,
    pub vararg: Option<Arg>,
    pub kwonly: Vec<Arg>,
    pub kwarg: Option<Arg>,
    /// Defaults for the last `defaults.len()` of posonly+args.
    pub defaults: Vec<Expr>,
    /// One entry per kwonly argument.
    pub kw_defaults: Vec<Option<Expr>>,
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Ident,
    pub annotation: Option<Box<Expr>>,
    /// Python 2 tuple parameter `def f((a, b))`: unpacked on entry.
    pub unpack: Option<Box<Expr>>,
}

#[derive(Clone, Debug)]
pub struct Keyword {
    /// None for `**mapping`.
    pub arg: Option<Ident>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub struct Alias {
    pub name: Ident,
    pub asname: Option<Ident>,
}

#[derive(Clone, Debug)]
pub struct WithItem {
    pub context: Expr,
    pub vars: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct Handler {
    pub loc: Loc,
    pub typ: Option<Expr>,
    pub name: Option<Expr>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub struct Comprehension {
    pub target: Expr,
    pub iter: Expr,
    pub ifs: Vec<Expr>,
}

#[derive(Clone, Debug)]
pub struct MatchCase {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub enum Pattern {
    Value(Expr),
    Singleton(Constant),
    Sequence(Vec<Pattern>),
    Mapping { keys: Vec<Expr>, patterns: Vec<Pattern>, rest: Option<Ident> },
    Class { cls: Expr, patterns: Vec<Pattern>, kwd_attrs: Vec<Ident>, kwd_patterns: Vec<Pattern> },
    Star(Option<Ident>),
    As { pattern: Option<Box<Pattern>>, name: Option<Ident> },
    Or(Vec<Pattern>),
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    FunctionDef {
        name: Ident,
        args: Box<Arguments>,
        body: Vec<Stmt>,
        decorators: Vec<Expr>,
        returns: Option<Box<Expr>>,
        is_async: bool,
    },
    ClassDef {
        name: Ident,
        bases: Vec<Expr>,
        keywords: Vec<Keyword>,
        body: Vec<Stmt>,
        decorators: Vec<Expr>,
    },
    Return(Option<Expr>),
    Delete(Vec<Expr>),
    Assign { targets: Vec<Expr>, value: Expr },
    AugAssign { target: Expr, op: BinOp, value: Expr },
    AnnAssign { target: Expr, annotation: Expr, value: Option<Expr>, simple: bool },
    For { target: Expr, iter: Expr, body: Vec<Stmt>, orelse: Vec<Stmt>, is_async: bool },
    While { test: Expr, body: Vec<Stmt>, orelse: Vec<Stmt> },
    If { test: Expr, body: Vec<Stmt>, orelse: Vec<Stmt> },
    With { items: Vec<WithItem>, body: Vec<Stmt>, is_async: bool },
    Match { subject: Expr, cases: Vec<MatchCase> },
    Raise { exc: Option<Expr>, cause: Option<Expr>, /* Python 2 `raise E, V, T` */ py2_value: Option<Expr>, py2_tb: Option<Expr> },
    Try { body: Vec<Stmt>, handlers: Vec<Handler>, orelse: Vec<Stmt>, finalbody: Vec<Stmt> },
    Assert { test: Expr, msg: Option<Expr> },
    Import(Vec<Alias>),
    ImportFrom { module: Option<Ident>, names: Vec<Alias>, level: u32 },
    Global(Vec<Ident>),
    Nonlocal(Vec<Ident>),
    Expr(Expr),
    Pass,
    Break,
    Continue,
    /// Python 2 `print [>>dest,] values [,]`.
    Print { dest: Option<Expr>, values: Vec<Expr>, nl: bool },
    /// Python 2 `exec code [in globals[, locals]]`.
    Exec { body: Expr, globals: Option<Expr>, locals: Option<Expr> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    None,
    Bool(bool),
    Int(num_bigint::BigInt),
    Float(f64),
    Complex(f64, f64),
    Str(String),
    Bytes(Vec<u8>),
    Ellipsis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mult,
    MatMult,
    Div,
    Mod,
    Pow,
    LShift,
    RShift,
    BitOr,
    BitXor,
    BitAnd,
    FloorDiv,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Invert,
    Not,
    UAdd,
    USub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
    Is,
    IsNot,
    In,
    NotIn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctx {
    Load,
    Store,
    Del,
}

#[derive(Clone, Debug)]
pub enum FPart {
    Lit(String),
    Field { value: Box<Expr>, conversion: Option<char>, spec: Option<Vec<FPart>>, debug_text: Option<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompKind {
    List,
    Set,
    Gen,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    BoolOp { op: BoolOp, values: Vec<Expr> },
    NamedExpr { target: Box<Expr>, value: Box<Expr> },
    BinOp { left: Box<Expr>, op: BinOp, right: Box<Expr> },
    UnaryOp { op: UnaryOp, operand: Box<Expr> },
    Lambda { args: Box<Arguments>, body: Box<Expr> },
    IfExp { test: Box<Expr>, body: Box<Expr>, orelse: Box<Expr> },
    Dict { keys: Vec<Option<Expr>>, values: Vec<Expr> },
    Set(Vec<Expr>),
    Comp { kind: CompKind, elt: Box<Expr>, generators: Vec<Comprehension> },
    DictComp { key: Box<Expr>, value: Box<Expr>, generators: Vec<Comprehension> },
    Await(Box<Expr>),
    Yield(Option<Box<Expr>>),
    YieldFrom(Box<Expr>),
    Compare { left: Box<Expr>, ops: Vec<CmpOp>, comparators: Vec<Expr> },
    Call { func: Box<Expr>, args: Vec<Expr>, keywords: Vec<Keyword> },
    FString(Vec<FPart>),
    Constant(Constant),
    Attribute { value: Box<Expr>, attr: Ident, ctx: Ctx },
    Subscript { value: Box<Expr>, slice: Box<Expr>, ctx: Ctx },
    Starred { value: Box<Expr>, ctx: Ctx },
    Name { id: Ident, ctx: Ctx },
    List { elts: Vec<Expr>, ctx: Ctx },
    Tuple { elts: Vec<Expr>, ctx: Ctx },
    Slice { lower: Option<Box<Expr>>, upper: Option<Box<Expr>>, step: Option<Box<Expr>> },
    /// Python 2 backtick repr.
    Repr(Box<Expr>),
}

impl Expr {
    pub fn new(loc: Loc, kind: ExprKind) -> Expr {
        Expr { loc, kind }
    }
    pub fn with_ctx(mut self, ctx: Ctx) -> Expr {
        match &mut self.kind {
            ExprKind::Attribute { ctx: c, .. }
            | ExprKind::Subscript { ctx: c, .. }
            | ExprKind::Name { ctx: c, .. } => *c = ctx,
            ExprKind::Starred { value, ctx: c } => {
                *c = ctx;
                let v = core::mem::replace(value.as_mut(), Expr::new(Loc { line: 0, col: 0 }, ExprKind::Constant(Constant::None)));
                **value = v.with_ctx(ctx);
            }
            ExprKind::List { elts, ctx: c } | ExprKind::Tuple { elts, ctx: c } => {
                *c = ctx;
                for e in elts.iter_mut() {
                    let v = core::mem::replace(e, Expr::new(Loc { line: 0, col: 0 }, ExprKind::Constant(Constant::None)));
                    *e = v.with_ctx(ctx);
                }
            }
            _ => {}
        }
        self
    }
}

#[derive(Clone, Debug)]
pub enum Module {
    Exec(Vec<Stmt>),
    Eval(Expr),
}
