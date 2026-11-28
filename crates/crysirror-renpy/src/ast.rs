// SPDX-License-Identifier: MPL-2.0
//! Script node graph: a version-independent superset of the `renpy.ast`
//! statement classes (6.18 – 8.6). Nodes live in one arena; blocks are
//! chained through `next` exactly like `renpy.ast.chain_block`.

use crate::lexer::PyExpr;
use std::rc::Rc;

pub type NodeId = usize;

/// `Node.name`: labels are named by their label; other nodes by
/// (filename, version stamp, serial) as Ren'Py generates them. The
/// identity is part of original save files (rollback log, return stack).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NodeName {
    Label(Rc<str>),
    Generated {
        filename: Rc<str>,
        stamp: i64,
        serial: i64,
    },
    /// Any other tuple name, such as the `(..., "translate")` names of
    /// restructured translation nodes.
    Tuple(Rc<[NameAtom]>),
    /// Any other pickled name (kept verbatim as text for comparison).
    Other(Rc<str>),
}

/// One element of a tuple node name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NameAtom {
    Str(Rc<str>),
    Int(i64),
    /// A float element, by bit pattern.
    Float(u64),
}

impl NodeName {
    /// The elements of a generated or tuple name.
    pub fn atoms(&self) -> Option<Vec<NameAtom>> {
        match self {
            NodeName::Generated {
                filename,
                stamp,
                serial,
            } => Some(vec![
                NameAtom::Str(filename.clone()),
                NameAtom::Int(*stamp),
                NameAtom::Int(*serial),
            ]),
            NodeName::Tuple(t) => Some(t.to_vec()),
            _ => None,
        }
    }

    /// `name + (suffix,)`, as the translation restructurer names nodes.
    pub fn suffixed(&self, suffix: &str) -> NodeName {
        let mut a = self.atoms().unwrap_or_else(|| match self {
            NodeName::Label(l) | NodeName::Other(l) => vec![NameAtom::Str(l.clone())],
            _ => Vec::new(),
        });
        a.push(NameAtom::Str(suffix.into()));
        NodeName::Tuple(a.into())
    }

    /// The canonical form: 3-tuples of (str, int, int) are `Generated`.
    pub fn from_atoms(a: Vec<NameAtom>) -> NodeName {
        if let [NameAtom::Str(f), NameAtom::Int(s), NameAtom::Int(n)] = a.as_slice() {
            return NodeName::Generated {
                filename: f.clone(),
                stamp: *s,
                serial: *n,
            };
        }
        NodeName::Tuple(a.into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rollback {
    Normal,
    Never,
    Force,
}

#[derive(Clone, Debug, Default)]
pub struct Parameter {
    pub name: String,
    /// 0 positional-only, 1 positional-or-keyword, 2 *args, 3 keyword-only, 4 **kwargs.
    pub kind: u8,
    pub default: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Signature {
    pub parameters: Vec<Parameter>,
}

#[derive(Clone, Debug, Default)]
pub struct ArgumentInfo {
    /// (keyword name or None, expression text).
    pub arguments: Vec<(Option<String>, String)>,
    pub starred: Vec<usize>,
    pub doublestarred: Vec<usize>,
}

/// Image specifier of show/scene/hide.
#[derive(Clone, Debug, Default)]
pub struct ImSpec {
    pub name: Vec<String>,
    pub expression: Option<PyExpr>,
    pub tag: Option<String>,
    pub at_list: Vec<PyExpr>,
    pub layer: Option<String>,
    pub zorder: Option<PyExpr>,
    pub behind: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct MenuItem {
    pub label: String,
    pub condition: PyExpr,
    pub block: Option<Vec<NodeId>>,
    pub arguments: Option<ArgumentInfo>,
}

/// Python code embedded in a node; `mode` is exec/eval/hide.
#[derive(Clone, Debug)]
pub struct PyCode {
    pub source: PyExpr,
    pub mode: &'static str,
    /// Python 3 semantics requested by `rpy python 3` / Ren'Py 8.
    pub py3: bool,
}

#[derive(Clone, Debug)]
pub struct Style {
    pub name: String,
    pub parent: Option<String>,
    pub clear: bool,
    pub take: Option<String>,
    pub delattr: Vec<String>,
    pub variant: Option<PyExpr>,
    pub properties: Vec<(String, PyExpr)>,
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    Say {
        who: Option<PyExpr>,
        what: String,
        with_: Option<PyExpr>,
        interact: bool,
        attributes: Option<Vec<String>>,
        temporary_attributes: Option<Vec<String>>,
        arguments: Option<ArgumentInfo>,
        identifier: Option<String>,
        explicit_identifier: bool,
    },
    Init {
        block: Vec<NodeId>,
        priority: i32,
    },
    Label {
        name: Rc<str>,
        block: Vec<NodeId>,
        parameters: Option<Signature>,
        hide: bool,
    },
    Python {
        code: PyCode,
        hide: bool,
        store: String,
    },
    EarlyPython {
        code: PyCode,
        hide: bool,
        store: String,
    },
    Image {
        name: Vec<String>,
        code: Option<PyCode>,
        atl: Option<Rc<crate::atl::RawBlock>>,
    },
    Transform {
        store: String,
        name: String,
        atl: Rc<crate::atl::RawBlock>,
        parameters: Option<Signature>,
    },
    Show {
        imspec: ImSpec,
        atl: Option<Rc<crate::atl::RawBlock>>,
    },
    ShowLayer {
        layer: String,
        at_list: Vec<PyExpr>,
        atl: Option<Rc<crate::atl::RawBlock>>,
    },
    Camera {
        layer: String,
        at_list: Vec<PyExpr>,
        atl: Option<Rc<crate::atl::RawBlock>>,
    },
    Scene {
        imspec: Option<ImSpec>,
        layer: Option<String>,
        atl: Option<Rc<crate::atl::RawBlock>>,
    },
    Hide {
        imspec: ImSpec,
    },
    /// `expr` is the transition; `paired` marks the opening half of a
    /// `show ... with` pair (Ren'Py's `With(loc, None, expr)`).
    With {
        expr: Option<PyExpr>,
        paired: Option<PyExpr>,
    },
    Call {
        label: String,
        expression: bool,
        arguments: Option<ArgumentInfo>,
        global_label: String,
    },
    Return {
        expression: Option<PyExpr>,
    },
    Menu {
        items: Vec<MenuItem>,
        set: Option<PyExpr>,
        with_: Option<PyExpr>,
        has_caption: bool,
        arguments: Option<ArgumentInfo>,
    },
    Jump {
        target: String,
        expression: bool,
        global_label: String,
    },
    Pass,
    While {
        condition: PyExpr,
        block: Vec<NodeId>,
    },
    If {
        entries: Vec<(PyExpr, Vec<NodeId>)>,
    },
    /// Creator-defined statement; `parsed` is the registry's Python value.
    UserStatement {
        line: String,
        block: Rc<Vec<crate::lexer::GroupedLine>>,
        parsed: Option<crysirror_python::Value>,
        subparses: Vec<Vec<NodeId>>,
        code_block: Option<Vec<NodeId>>,
        init_priority: i32,
        translatable: bool,
        atl: Option<Rc<crate::atl::RawBlock>>,
    },
    PostUserStatement {
        parent: NodeId,
    },
    Define {
        store: String,
        name: String,
        index: Option<PyExpr>,
        operator: String,
        code: PyCode,
    },
    Default {
        store: String,
        name: String,
        code: PyCode,
    },
    Screen {
        screen: Rc<crate::sl2::Screen>,
    },
    Style(Box<Style>),
    Testcase {
        name: String,
    },
    Rpy {
        rest: (String, String),
    },
    Translate {
        identifier: String,
        language: Option<String>,
        block: Vec<NodeId>,
        alternate: Option<String>,
    },
    TranslateSay {
        identifier: String,
        language: Option<String>,
        say: Box<NodeKind>,
    },
    EndTranslate,
    TranslateString {
        language: Option<String>,
        old: String,
        new: String,
    },
    TranslatePython {
        language: Option<String>,
        code: PyCode,
    },
    TranslateBlock {
        language: Option<String>,
        block: Vec<NodeId>,
    },
    TranslateEarlyBlock {
        language: Option<String>,
        block: Vec<NodeId>,
    },
}

#[derive(Clone, Debug)]
pub struct Node {
    pub filename: Rc<str>,
    pub linenumber: u32,
    pub name: Option<NodeName>,
    pub next: Option<NodeId>,
    pub rollback: Rollback,
    /// First node of a multi-node statement (menu's label/say).
    pub statement_start: Option<NodeId>,
    pub kind: NodeKind,
}

impl Node {
    /// `Translate.after` / the successor of a node: a translate block's
    /// node continues into its block, so `after` is kept as `next` of the
    /// node chained by `chain_block`.
    pub fn next_after(&self) -> Option<NodeId> {
        self.next
    }
}

#[derive(Default)]
pub struct Arena {
    pub nodes: Vec<Node>,
}

impl NodeKind {
    /// ATL blocks owned directly by this node.
    pub fn atl_blocks(&self) -> Vec<&Rc<crate::atl::RawBlock>> {
        match self {
            NodeKind::Image { atl: Some(a), .. }
            | NodeKind::Show { atl: Some(a), .. }
            | NodeKind::ShowLayer { atl: Some(a), .. }
            | NodeKind::Camera { atl: Some(a), .. }
            | NodeKind::Scene { atl: Some(a), .. }
            | NodeKind::UserStatement { atl: Some(a), .. } => vec![a],
            NodeKind::Transform { atl, .. } => vec![atl],
            _ => Vec::new(),
        }
    }
}

impl Arena {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn add(&mut self, loc: &(Rc<str>, u32), kind: NodeKind) -> NodeId {
        self.nodes.push(Node {
            filename: loc.0.clone(),
            linenumber: loc.1,
            name: None,
            next: None,
            rollback: Rollback::Normal,
            statement_start: None,
            kind,
        });
        self.nodes.len() - 1
    }
    pub fn get(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }
    pub fn get_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    /// Child blocks of a node (for chaining and traversal).
    pub fn child_blocks(&self, id: NodeId) -> Vec<Vec<NodeId>> {
        match &self.nodes[id].kind {
            NodeKind::Init { block, .. }
            | NodeKind::Label { block, .. }
            | NodeKind::While { block, .. }
            | NodeKind::Translate { block, .. }
            | NodeKind::TranslateBlock { block, .. }
            | NodeKind::TranslateEarlyBlock { block, .. } => vec![block.clone()],
            NodeKind::If { entries } => entries.iter().map(|(_, b)| b.clone()).collect(),
            NodeKind::Menu { items, .. } => items.iter().filter_map(|i| i.block.clone()).collect(),
            NodeKind::UserStatement {
                subparses,
                code_block,
                ..
            } => {
                let mut v = subparses.clone();
                if let Some(c) = code_block {
                    v.push(c.clone());
                }
                v
            }
            _ => Vec::new(),
        }
    }

    /// `chain_block`: links each node to its successor; the last node of a
    /// block continues at `next`. Compound statements chain their children
    /// to their own successor (`Node.chain`).
    pub fn chain_block(&mut self, block: &[NodeId], next: Option<NodeId>) {
        for (i, &id) in block.iter().enumerate() {
            let succ = block.get(i + 1).copied().or(next);
            self.chain(id, succ);
        }
    }

    fn chain(&mut self, id: NodeId, next: Option<NodeId>) {
        // `Return.chain` and `Jump.chain` leave `next` unset.
        self.nodes[id].next = if matches!(
            self.nodes[id].kind,
            NodeKind::Return { .. } | NodeKind::Jump { .. }
        ) {
            None
        } else {
            next
        };
        match self.nodes[id].kind.clone() {
            // A label/init/translate block runs into what follows the node.
            NodeKind::Label { block, .. } => {
                let first = block.first().copied().or(next);
                self.nodes[id].next = first;
                self.chain_block(&block, next);
            }
            NodeKind::Init { block, .. }
            | NodeKind::TranslateBlock { block, .. }
            | NodeKind::TranslateEarlyBlock { block, .. } => self.chain_block(&block, None),
            NodeKind::Translate { block, .. } => self.chain_block(&block, next),
            NodeKind::While { block, .. } => self.chain_block(&block, Some(id)),
            NodeKind::If { entries } => {
                for (_, b) in entries {
                    self.chain_block(&b, next);
                }
            }
            NodeKind::Menu { items, .. } => {
                for it in items {
                    if let Some(b) = it.block {
                        self.chain_block(&b, next);
                    }
                }
            }
            NodeKind::UserStatement {
                subparses,
                code_block,
                ..
            } => {
                for b in subparses {
                    self.chain_block(&b, next);
                }
                if let Some(b) = code_block {
                    self.chain_block(&b, next);
                }
            }
            _ => {}
        }
    }
}

impl ArgumentInfo {
    /// `ArgumentInfo.get_code()`.
    pub fn get_code(&self) -> String {
        let mut l = Vec::new();
        for (i, (k, e)) in self.arguments.iter().enumerate() {
            if self.starred.contains(&i) {
                l.push(format!("*{e}"));
            } else if self.doublestarred.contains(&i) {
                l.push(format!("**{e}"));
            } else if let Some(k) = k {
                l.push(format!("{k}={e}"));
            } else {
                l.push(e.clone());
            }
        }
        format!("({})", l.join(", "))
    }
}
