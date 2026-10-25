// SPDX-License-Identifier: MPL-2.0
//! Behaviour tree definitions and per-entity AI state (ADR 0014). The
//! executor lives in crysirror-engine (it needs physics, navigation and the
//! gameplay system); everything here is saved data.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn half() -> f32 {
    0.5
}
fn one() -> f32 {
    1.
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BbValue {
    Bool(bool),
    Number(f32),
    Vec3([f32; 3]),
    Text(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compare {
    Set,
    NotSet,
    Greater,
    Less,
    Equal,
    True,
    False,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BtNode {
    /// First child that does not fail.
    Selector {
        children: Vec<BtNode>,
    },
    /// Children in order until one fails or runs.
    Sequence {
        children: Vec<BtNode>,
    },
    Invert {
        child: Box<BtNode>,
    },
    /// After the child succeeds, fail for `seconds`.
    Cooldown {
        seconds: f32,
        child: Box<BtNode>,
    },
    /// Blackboard condition.
    Check {
        key: String,
        op: Compare,
        #[serde(default)]
        value: f32,
    },
    /// Owner (or blackboard entity `target`) owns a gameplay tag.
    HasTag {
        tag: String,
        #[serde(default)]
        target: Option<String>,
    },
    /// Distance from owner to blackboard target (entity or vec3) < `less_than`.
    Near {
        target: String,
        less_than: f32,
    },
    Wait {
        seconds: f32,
    },
    /// Path-find and walk to a blackboard target with the character controller.
    MoveTo {
        target: String,
        #[serde(default = "speed")]
        speed: f32,
        #[serde(default = "half")]
        acceptance: f32,
    },
    /// Turn toward a blackboard target.
    Face {
        target: String,
        #[serde(default = "turn")]
        degrees_per_second: f32,
    },
    /// Try to activate a gameplay ability (success if activated).
    Ability {
        name: String,
    },
    /// Write a blackboard key.
    Set {
        key: String,
        value: BbValue,
    },
    /// Nearest entity owning `tag` within `radius` → blackboard `into` (entity id).
    Perceive {
        tag: String,
        radius: f32,
        into: String,
        #[serde(default = "one")]
        min_interval: f32,
    },
    Succeed,
    Fail,
}
fn speed() -> f32 {
    2.
}
fn turn() -> f32 {
    360.
}
impl BtNode {
    pub fn count(&self) -> usize {
        1 + match self {
            BtNode::Selector { children } | BtNode::Sequence { children } => {
                children.iter().map(BtNode::count).sum()
            }
            BtNode::Invert { child } | BtNode::Cooldown { child, .. } => child.count(),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiDefs {
    #[serde(default)]
    pub trees: BTreeMap<String, BtNode>,
}
impl AiDefs {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.trees.len() <= 1024, "behaviour tree limit");
        for (n, t) in &self.trees {
            ensure!(t.count() <= 4096, "tree '{n}' too large");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BtStatus {
    Success,
    Failure,
    Running,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiState {
    pub tree: String,
    #[serde(default)]
    pub blackboard: BTreeMap<String, BbValue>,
    /// Per-node timers (node index in depth-first order → seconds).
    #[serde(default)]
    pub memory: BTreeMap<u32, f32>,
    /// Last root status, for inspection.
    #[serde(default)]
    pub status: Option<BtStatus>,
    /// Depth-first index of the leaf that ran last (debugging).
    #[serde(default)]
    pub active_leaf: Option<u32>,
}
impl AiState {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.blackboard.len() <= 256 && self.memory.len() <= 4096,
            "AI state limits"
        );
        Ok(())
    }
}
