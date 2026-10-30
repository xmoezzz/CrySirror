// SPDX-License-Identifier: MPL-2.0
use anyhow::{ensure, Context, Result};
use crysirror_core::path::{Mount, VfsPath};
use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Startup {
    pub script: String,
    pub canvas: [u32; 2],
    pub heap_bytes: u32,
}
/// Bounded native startup policy. XML is data; external entities and doctypes
/// cannot grant filesystem/network authority. See official startup reference.
pub fn parse_startup(bytes: &[u8]) -> Result<Startup> {
    ensure!(
        bytes.len() <= 1024 * 1024,
        "CatSystem2 startup XML byte budget"
    );
    let source = std::str::from_utf8(bytes).context("CatSystem2 startup XML must be UTF-8")?;
    let mut xml = Reader::from_str(source.trim_start_matches('\u{feff}'));
    xml.config_mut().trim_text(true);
    let mut stack = Vec::<String>::new();
    let mut fields = BTreeMap::<String, String>::new();
    let wanted = [
        "document/SCRIPT/start",
        "document/SCREEN/width",
        "document/SCREEN/height",
        "document/SCREEN/v_width",
        "document/SCREEN/v_height",
        "document/ETC/heap",
    ];
    let mut events = 0;
    loop {
        events += 1;
        ensure!(events <= 100_000, "CatSystem2 startup XML event budget");
        match xml.read_event()? {
            Event::Start(e) => {
                ensure!(stack.len() < 32, "CatSystem2 startup XML depth budget");
                stack.push(std::str::from_utf8(e.name().as_ref())?.to_owned());
            }
            Event::End(_) => {
                stack
                    .pop()
                    .context("CatSystem2 startup XML closing element")?;
            }
            Event::Text(e) => {
                let key = stack.join("/");
                if wanted.contains(&key.as_str()) {
                    let text = quick_xml::escape::unescape(&e.xml10_content()?)?.into_owned();
                    ensure!(
                        fields.insert(key, text).is_none(),
                        "duplicate CatSystem2 startup field"
                    );
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                anyhow::bail!("CatSystem2 startup external/entity declarations unsupported")
            }
            Event::Eof => break,
            _ => {}
        }
    }
    ensure!(stack.is_empty(), "CatSystem2 startup XML unclosed element");
    let script = fields
        .remove("document/SCRIPT/start")
        .context("CatSystem2 SCRIPT/start missing")?
        .replace('\\', "/");
    ensure!(
        VfsPath::new(Mount::Asset, &script)?.relative() == script,
        "CatSystem2 startup script escapes capability"
    );
    ensure!(
        script.to_ascii_lowercase().ends_with(".kcs"),
        "CatSystem2 native startup must name original KCS script"
    );
    let number = |name: &str, fallback: &str| -> Result<u32> {
        fields
            .get(name)
            .or_else(|| fields.get(fallback))
            .context("CatSystem2 SCREEN dimensions missing")?
            .parse()
            .context("CatSystem2 startup integer")
    };
    let canvas = [
        number("document/SCREEN/v_width", "document/SCREEN/width")?,
        number("document/SCREEN/v_height", "document/SCREEN/height")?,
    ];
    ensure!(
        canvas.iter().all(|n| (1..=8192).contains(n)),
        "CatSystem2 canvas budget"
    );
    let heap_bytes = fields
        .get("document/ETC/heap")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(1024 * 1024);
    ensure!(
        (4..=16 * 1024 * 1024).contains(&heap_bytes),
        "CatSystem2 heap budget"
    );
    Ok(Startup {
        script,
        canvas,
        heap_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_entry_and_virtual_canvas_are_preserved() {
        let xml = br#"<?xml version="1.0"?><document><SCREEN><width>640</width><height>360</height><v_width>1280</v_width><v_height>720</v_height></SCREEN><SCRIPT><start>kcs.int\main.kcs</start></SCRIPT><ETC><heap>4096</heap></ETC></document>"#;
        assert_eq!(
            parse_startup(xml).unwrap(),
            Startup {
                script: "kcs.int/main.kcs".into(),
                canvas: [1280, 720],
                heap_bytes: 4096
            }
        );
    }
    #[test]
    fn untrusted_startup_fails_at_its_actual_boundary() {
        for xml in ["<!DOCTYPE x [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><document/>",
            "<document><SCRIPT><start>../outside.kcs</start></SCRIPT></document>",
            "<document><SCRIPT><start>scene.int/01.cst</start></SCRIPT></document>",
            "<document><SCRIPT><start>main.kcs</start><start>second.kcs</start></SCRIPT></document>"] {
            assert!(parse_startup(xml.as_bytes()).is_err());
        }
    }
}
