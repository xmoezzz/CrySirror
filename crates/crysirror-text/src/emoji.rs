// SPDX-License-Identifier: MPL-2.0
//! Emoji sequence qualification (UTS #51) as a merged trie.
//!
//! `data/emoji-test-17.emt` is a compact form of Unicode 17 `emoji-test.txt`
//! (Unicode terms of use): each sequence keeps its codepoints, its status
//! (unqualified/component/minimally-qualified/fully-qualified) and the
//! emoji version that introduced it, in file order. [`EmojiTrie::build`]
//! selects the sequences of an emoji version and builds the trie the same
//! way as Ren'Py's `make_emoji_trie.py` (MIT): a node maps one character to
//! a child and `""` to the status of the sequence ending there; identical
//! subtrees are merged (the last one visited in preorder is kept) and nodes
//! are numbered in preorder from the root. Lookup is engine independent.
use std::{collections::HashMap, sync::OnceLock};

pub const NOT_EMOJI: u8 = 0;
pub const UNQUALIFIED: u8 = 1;
pub const COMPONENT: u8 = 2;
pub const MINIMALLY_QUALIFIED: u8 = 3;
pub const QUALIFIED: u8 = 4;

struct Sequence {
    cps: Vec<char>,
    level: u8,
    version: u16,
}

fn sequences() -> &'static [Sequence] {
    static S: OnceLock<Vec<Sequence>> = OnceLock::new();
    S.get_or_init(|| {
        let b: &[u8] = include_bytes!("../data/emoji-test-17.emt");
        assert_eq!(&b[..4], b"EMT1", "emoji data magic");
        let n = u32::from_le_bytes(b[4..8].try_into().unwrap());
        let mut p = 8;
        let mut out = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let k = b[p] as usize;
            p += 1;
            let mut cps = Vec::with_capacity(k);
            for _ in 0..k {
                let c = u32::from_le_bytes([b[p], b[p + 1], b[p + 2], 0]);
                p += 3;
                cps.push(char::from_u32(c).unwrap_or('\u{fffd}'));
            }
            out.push(Sequence {
                cps,
                level: b[p],
                version: b[p + 1] as u16,
            });
            p += 2;
        }
        out
    })
}

/// One merged trie node: its status and its children in sorted order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub level: u8,
    pub children: Vec<(char, usize)>,
}

/// A merged trie. Node 0 is the root; other nodes keep the generator's
/// preorder numbers (`e1`, `e2`, ...).
#[derive(Clone, Debug)]
pub struct EmojiTrie {
    pub nodes: Vec<Node>,
}

impl EmojiTrie {
    /// The trie of every sequence introduced up to `version_tenths`
    /// (151 = Emoji 15.1, 170 = Emoji 17.0).
    pub fn build(version_tenths: u16) -> EmojiTrie {
        // Unmerged trie in insertion order (dict order in the generator).
        struct Raw {
            level: u8,
            children: Vec<(char, usize)>,
        }
        let mut raw = vec![Raw {
            level: 0,
            children: Vec::new(),
        }];
        for s in sequences().iter().filter(|s| s.version <= version_tenths) {
            let mut d = 0usize;
            for &c in &s.cps {
                d = match raw[d].children.iter().find(|(k, _)| *k == c) {
                    Some(&(_, i)) => i,
                    None => {
                        raw.push(Raw {
                            level: 0,
                            children: Vec::new(),
                        });
                        let i = raw.len() - 1;
                        raw[d].children.push((c, i));
                        i
                    }
                };
            }
            raw[d].level = s.level;
        }
        raw[0].level = 0;
        // Structural identity, bottom-up.
        let mut ids: HashMap<(u8, Vec<(char, usize)>), usize> = HashMap::new();
        let mut class = vec![usize::MAX; raw.len()];
        fn classify(
            i: usize,
            raw: &[Raw],
            ids: &mut HashMap<(u8, Vec<(char, usize)>), usize>,
            class: &mut [usize],
        ) -> usize {
            if class[i] != usize::MAX {
                return class[i];
            }
            let mut key: Vec<(char, usize)> = raw[i]
                .children
                .iter()
                .map(|&(c, j)| (c, classify(j, raw, ids, class)))
                .collect();
            key.sort();
            let n = ids.len();
            let id = *ids.entry((raw[i].level, key)).or_insert(n);
            class[i] = id;
            id
        }
        classify(0, &raw, &mut ids, &mut class);
        // pass1: the last node of each class in preorder is canonical.
        let mut canon = vec![usize::MAX; ids.len()];
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            canon[class[i]] = i;
            for &(_, j) in raw[i].children.iter().rev() {
                stack.push(j);
            }
        }
        // Names: preorder over canonical nodes, children in their order.
        let mut name = vec![usize::MAX; raw.len()];
        let mut order = Vec::new();
        let mut stack = vec![canon[class[0]]];
        while let Some(i) = stack.pop() {
            if name[i] != usize::MAX {
                continue;
            }
            name[i] = order.len();
            order.push(i);
            for &(_, j) in raw[i].children.iter().rev() {
                let cj = canon[class[j]];
                if name[cj] == usize::MAX {
                    stack.push(cj);
                }
            }
        }
        let nodes = order
            .iter()
            .map(|&i| {
                let mut children: Vec<(char, usize)> = raw[i]
                    .children
                    .iter()
                    .map(|&(c, j)| (c, name[canon[class[j]]]))
                    .collect();
                children.sort();
                Node {
                    level: raw[i].level,
                    children,
                }
            })
            .collect();
        EmojiTrie { nodes }
    }

    pub fn child(&self, node: usize, c: char) -> Option<usize> {
        let n = &self.nodes[node];
        n.children
            .binary_search_by(|(k, _)| k.cmp(&c))
            .ok()
            .map(|i| n.children[i].1)
    }

    /// Longest emoji sequence at the start of `s`: (characters, status).
    pub fn match_prefix(&self, s: &[char]) -> (usize, u8) {
        let mut node = 0;
        let mut best = (0, NOT_EMOJI);
        for (i, &c) in s.iter().enumerate() {
            match self.child(node, c) {
                Some(n) => {
                    node = n;
                    if self.nodes[n].level != NOT_EMOJI {
                        best = (i + 1, self.nodes[n].level);
                    }
                }
                None => break,
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_counts_match_generated_releases() {
        // Ren'Py 7.8/8.2-8.4 ship e1..e262, 8.5+ e1..e272.
        assert_eq!(EmojiTrie::build(151).nodes.len(), 263);
        assert_eq!(EmojiTrie::build(160).nodes.len(), 263);
        assert_eq!(EmojiTrie::build(170).nodes.len(), 273);
    }

    /// `EMOJI_TRIE_PY=<emoji_trie.py> EMOJI_VERSION=<tenths>`: the trie
    /// matches a generated Ren'Py module node for node, names included.
    #[test]
    #[ignore]
    fn matches_generated_module() {
        let (Ok(path), Ok(v)) = (std::env::var("EMOJI_TRIE_PY"), std::env::var("EMOJI_VERSION"))
        else {
            return;
        };
        let t = EmojiTrie::build(v.parse().unwrap());
        let mut ours: Vec<String> = t
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let name = if i == 0 { "emoji".to_string() } else { format!("e{i}") };
                let mut l = vec![format!("{name} = {{"), format!("'': {},", n.level)];
                for (c, j) in &n.children {
                    l.push(format!("'{c}': e{j},"));
                }
                l.push("}".into());
                l.join(" ")
            })
            .collect();
        // Releases format the module differently (one line per node, or
        // black's multi-line dicts); compare without layout.
        let norm = |s: &str| {
            s.chars()
                .filter(|c| !c.is_whitespace())
                .map(|c| if c == '"' { '\'' } else { c })
                .collect::<String>()
                .replace(",}", "}")
        };
        let text = std::fs::read_to_string(path).unwrap();
        let mut theirs = Vec::new();
        let mut block = String::new();
        for l in text.lines() {
            if block.is_empty() && !((l.starts_with('e')) && l.contains(" = {")) {
                continue;
            }
            block.push_str(l);
            if l.trim_end().ends_with('}') {
                theirs.push(norm(&block));
                block.clear();
            }
        }
        let mut ours: Vec<String> = ours.iter().map(|s| norm(s)).collect();
        ours.sort();
        theirs.sort();
        assert_eq!(ours.len(), theirs.len());
        for (a, b) in ours.iter().zip(&theirs) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn matches_sequences() {
        let t = EmojiTrie::build(170);
        assert_eq!(t.match_prefix(&['😀', 'a']), (1, QUALIFIED));
        assert_eq!(t.match_prefix(&['a']), (0, NOT_EMOJI));
        let family: Vec<char> = "👨\u{200d}👩\u{200d}👧".chars().collect();
        assert_eq!(t.match_prefix(&family), (5, QUALIFIED));
    }
}
