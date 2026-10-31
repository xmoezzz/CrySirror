// SPDX-License-Identifier: MPL-2.0
//! Bounded immutable UTF-8 XML resources, reusable without a native VM.
//! This is an element/attribute document reader, not XPath or an external
//! entity loader. DTDs and unknown entities fail before publication.
use anyhow::{ensure, Context, Result};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use std::collections::BTreeMap;
/// Maximum original XML input bytes.
pub const MAX_XML_BYTES: usize = 8 * 1024 * 1024;
/// Maximum retained XML nodes.
pub const MAX_XML_NODES: usize = 32768;
/// Immutable element in a shared XML resource.
#[derive(Clone, Debug)]
pub struct XmlNode {
    /// Original qualified element name; namespaces are preserved as text.
    pub name: String,
    /// Concatenated character data, with XML 1.0 newline normalization.
    pub text: String,
    /// Original qualified attribute names and decoded values.
    pub attributes: BTreeMap<String, String>,
    children: Vec<usize>,
}
/// A validated CPU document. It contains no paths, device pointers or callbacks.
#[derive(Clone, Debug)]
pub struct XmlDocument {
    nodes: Vec<XmlNode>,
    root: usize,
    string_bytes: usize,
    attribute_count: usize,
}
impl XmlDocument {
    /// Parse UTF-8 XML with bounded nodes/depth/attributes/cumulative strings.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_XML_BYTES, "XML source byte budget");
        let source = std::str::from_utf8(bytes).context("XML resource UTF-8 encoding")?;
        let mut reader = Reader::from_str(source.strip_prefix('\u{feff}').unwrap_or(source));
        reader.config_mut().check_comments = true;
        let mut declared = false;
        let mut document = Self {
            nodes: Vec::new(),
            root: 0,
            string_bytes: 0,
            attribute_count: 0,
        };
        let mut stack = Vec::<usize>::new();
        let mut root = None;
        let mut events = 0usize;
        loop {
            events += 1;
            ensure!(events <= MAX_XML_NODES * 16, "XML event budget");
            let event = reader.read_event()?;
            let empty = matches!(&event, Event::Empty(_));
            match event {
                Event::Start(e) | Event::Empty(e) => {
                    let node = document.node_from(&e, &reader)?;
                    ensure!(
                        stack.len() < 64 && document.nodes.len() < MAX_XML_NODES,
                        "XML node/depth budget"
                    );
                    let id = document.nodes.len();
                    document.nodes.push(node);
                    if let Some(parent) = stack.last().copied() {
                        document.nodes[parent].children.push(id);
                    } else {
                        ensure!(root.replace(id).is_none(), "XML multiple root elements");
                    }
                    if !empty {
                        stack.push(id);
                    }
                }
                Event::End(_) => {
                    stack.pop().context("XML unexpected close element")?;
                }
                Event::Text(e) => {
                    let text = e.xml10_content()?;
                    document.append_text(&stack, &text)?;
                }
                Event::CData(e) => {
                    let text = e.xml10_content()?;
                    document.append_text(&stack, &text)?;
                }
                Event::GeneralRef(e) => {
                    let text = if let Some(c) = e.resolve_char_ref()? {
                        c.to_string()
                    } else {
                        match e.decode()?.as_ref() {
                            "amp" => "&",
                            "lt" => "<",
                            "gt" => ">",
                            "apos" => "'",
                            "quot" => "\"",
                            _ => anyhow::bail!("XML unknown/external entity"),
                        }
                        .to_owned()
                    };
                    document.append_text(&stack, &text)?;
                }
                Event::DocType(_) => anyhow::bail!("XML DTD/external entities are unsupported"),
                Event::Decl(e) => {
                    ensure!(!declared && root.is_none(), "XML repeated/late declaration");
                    declared = true;
                    ensure!(
                        e.version()?.as_ref() == b"1.0",
                        "unsupported XML declaration version"
                    );
                    if let Some(encoding) = e.encoding() {
                        ensure!(
                            encoding?.eq_ignore_ascii_case(b"utf-8"),
                            "XML declaration is not UTF-8"
                        );
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        ensure!(stack.is_empty(), "XML unclosed element");
        document.root = root.context("XML root missing")?;
        Ok(document)
    }
    fn node_from(&mut self, e: &BytesStart<'_>, reader: &Reader<&[u8]>) -> Result<XmlNode> {
        let name = std::str::from_utf8(e.name().as_ref())?.to_owned();
        ensure!(
            name.len() <= 1024 && valid_name(&name),
            "XML element-name budget"
        );
        self.add_string(name.len())?;
        let mut attributes = BTreeMap::new();
        for attribute in e.attributes() {
            ensure!(attributes.len() < 128, "XML attribute count budget");
            self.attribute_count += 1;
            ensure!(
                self.attribute_count <= 65536,
                "XML cumulative attribute budget"
            );
            let a = attribute?;
            let key = std::str::from_utf8(a.key.as_ref())?.to_owned();
            ensure!(valid_name(&key), "XML invalid attribute name");
            let value = a
                .decoded_and_normalized_value(quick_xml::XmlVersion::Explicit1_0, reader.decoder())?
                .into_owned();
            ensure!(
                key.len() <= 1024 && value.chars().all(|c| valid_char(c)),
                "XML attribute name/control character"
            );
            self.add_string(key.len() + value.len())?;
            ensure!(
                attributes.insert(key, value).is_none(),
                "XML duplicate attribute"
            );
        }
        Ok(XmlNode {
            name,
            text: String::new(),
            attributes,
            children: Vec::new(),
        })
    }
    fn add_string(&mut self, bytes: usize) -> Result<()> {
        self.string_bytes = self
            .string_bytes
            .checked_add(bytes)
            .context("XML retained string overflow")?;
        ensure!(
            self.string_bytes <= 16 * 1024 * 1024,
            "XML cumulative string budget"
        );
        Ok(())
    }
    fn append_text(&mut self, stack: &[usize], text: &str) -> Result<()> {
        ensure!(
            text.chars().all(|c| valid_char(c)),
            "XML invalid control character"
        );
        if let Some(id) = stack.last() {
            self.add_string(text.len())?;
            self.nodes[*id].text.push_str(text);
        } else {
            ensure!(
                text.bytes()
                    .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n')),
                "XML text outside root"
            );
        }
        Ok(())
    }
    /// The original root element.
    pub fn root(&self) -> &XmlNode {
        &self.nodes[self.root]
    }
    /// Iterate immediate children in source order. The node must be a borrowed
    /// member of this immutable document; foreign or cloned nodes are rejected.
    pub fn children<'a>(
        &'a self,
        node: &XmlNode,
    ) -> Result<impl Iterator<Item = &'a XmlNode> + 'a> {
        let base = self.nodes.as_ptr() as usize;
        let address = node as *const XmlNode as usize;
        let offset = address
            .checked_sub(base)
            .context("XML node belongs to another document")?;
        let size = std::mem::size_of::<XmlNode>();
        let index = offset / size;
        ensure!(
            offset % size == 0
                && index < self.nodes.len()
                && std::ptr::eq(&self.nodes[index], node),
            "XML node belongs to another document"
        );
        Ok(self.nodes[index].children.iter().map(|id| &self.nodes[*id]))
    }

    /// Resolve slash-separated child names below the root; the first complete
    /// matching sibling path is selected. Empty path refers to the root.
    /// No XPath guessing.
    pub fn child_path(&self, path: &str) -> Result<Option<&XmlNode>> {
        Ok(self.path_indices(path, 1)?.first().map(|i| &self.nodes[*i]))
    }
    /// Resolve a configuration field, rejecting ambiguous duplicate paths.
    pub fn unique_child_path(&self, path: &str) -> Result<Option<&XmlNode>> {
        let indices = self.path_indices(path, 2)?;
        ensure!(indices.len() < 2, "XML duplicate configuration path {path}");
        Ok(indices.first().map(|i| &self.nodes[*i]))
    }
    fn path_indices(&self, path: &str, limit: usize) -> Result<Vec<usize>> {
        ensure!(path.len() <= 4096, "XML query path budget");
        let names = path
            .split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        for name in &names {
            ensure!(
                name.len() <= 1024
                    && !name.contains(['[', ']', '@', '*'])
                    && *name != "."
                    && *name != "..",
                "unsupported XML path syntax"
            );
        }
        ensure!(names.len() <= 64, "XML query depth budget");
        fn find(
            doc: &XmlDocument,
            node: usize,
            names: &[&str],
            limit: usize,
            out: &mut Vec<usize>,
        ) {
            if out.len() >= limit {
                return;
            }
            if names.is_empty() {
                out.push(node);
                return;
            }
            for i in doc.nodes[node]
                .children
                .iter()
                .copied()
                .filter(|i| doc.nodes[*i].name == names[0])
            {
                find(doc, i, &names[1..], limit, out);
                if out.len() >= limit {
                    return;
                }
            }
        }
        let mut indices = Vec::new();
        find(self, self.root, &names, limit, &mut indices);
        Ok(indices)
    }
    /// Conservative retained-resource estimate for owner admission budgets.
    pub fn resident_bytes(&self) -> usize {
        self.string_bytes * 2 + self.nodes.len() * 256 + self.attribute_count * 128
    }
    /// Number of retained elements, independent of any VM.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

fn valid_char(c: char) -> bool {
    matches!(c, '\u{9}'|'\u{a}'|'\u{d}'|'\u{20}'..='\u{d7ff}'|'\u{e000}'..='\u{fffd}'|'\u{10000}'..='\u{10ffff}')
}

// XML 1.0 Fifth Edition productions 4/4a/5; independent grammar predicate.
// https://www.w3.org/TR/REC-xml/#NT-NameStartChar
fn name_start(c: char) -> bool {
    matches!(c, ':'|'A'..='Z'|'_'|'a'..='z'|'\u{c0}'..='\u{d6}'|'\u{d8}'..='\u{f6}'|'\u{f8}'..='\u{2ff}'|'\u{370}'..='\u{37d}'|'\u{37f}'..='\u{1fff}'|'\u{200c}'..='\u{200d}'|'\u{2070}'..='\u{218f}'|'\u{2c00}'..='\u{2fef}'|'\u{3001}'..='\u{d7ff}'|'\u{f900}'..='\u{fdcf}'|'\u{fdf0}'..='\u{fffd}'|'\u{10000}'..='\u{effff}')
}
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(name_start) && chars.all(|c| {
        name_start(c)
            || matches!(c,'-'|'.'|'0'..='9'|'\u{b7}'|'\u{300}'..='\u{36f}'|'\u{203f}'..='\u{2040}')
    })
}
