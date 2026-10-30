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
}
impl XmlDocument {
    /// Parse UTF-8 XML with bounded nodes/depth/attributes/cumulative strings.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_XML_BYTES, "XML source byte budget");
        let source = std::str::from_utf8(bytes).context("XML resource UTF-8 encoding")?;
        let mut reader = Reader::from_str(source.trim_start_matches('\u{feff}'));
        let mut document = Self {
            nodes: Vec::new(),
            root: 0,
            string_bytes: 0,
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
            !name.is_empty() && name.len() <= 1024,
            "XML element-name budget"
        );
        self.add_string(name.len())?;
        let mut attributes = BTreeMap::new();
        for attribute in e.attributes() {
            ensure!(attributes.len() < 128, "XML attribute count budget");
            let a = attribute?;
            let key = std::str::from_utf8(a.key.as_ref())?.to_owned();
            let value = a.decode_and_unescape_value(reader.decoder())?.into_owned();
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
            text.chars()
                .all(|c| c == '\t' || c == '\n' || c == '\r' || c >= ' '),
            "XML invalid control character"
        );
        if let Some(id) = stack.last() {
            self.add_string(text.len())?;
            self.nodes[*id].text.push_str(text);
        } else {
            ensure!(text.trim().is_empty(), "XML text outside root");
        }
        Ok(())
    }
    /// The original root element.
    pub fn root(&self) -> &XmlNode {
        &self.nodes[self.root]
    }
    /// Resolve slash-separated child names below the root; the first matching
    /// sibling is selected. Empty path refers to the root. No XPath guessing.
    pub fn child_path(&self, path: &str) -> Result<Option<&XmlNode>> {
        ensure!(path.len() <= 4096, "XML query path budget");
        let mut node = self.root;
        for name in path.split('/').filter(|s| !s.is_empty()) {
            ensure!(
                name.len() <= 1024
                    && !name.contains(['[', ']', '@', '*'])
                    && name != "."
                    && name != "..",
                "unsupported XML path syntax"
            );
            let Some(found) = self.nodes[node]
                .children
                .iter()
                .copied()
                .find(|i| self.nodes[*i].name == name)
            else {
                return Ok(None);
            };
            node = found;
        }
        Ok(Some(&self.nodes[node]))
    }
    /// Conservative retained-resource estimate for owner admission budgets.
    pub fn resident_bytes(&self) -> usize {
        self.string_bytes + self.nodes.len() * 256
    }
    /// Number of retained elements, independent of any VM.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}
