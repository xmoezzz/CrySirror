// SPDX-License-Identifier: MPL-2.0
use anyhow::{ensure, Context, Result};
use crysirror_core::path::{Mount, VfsPath};
use crysirror_resource::xml::XmlDocument;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Original startup configuration projected onto the native VM boundary.
pub struct Startup {
    /// Capability-relative original SCRIPT/start path.
    pub script: String,
    /// Original virtual width and height, falling back to physical dimensions.
    pub canvas: [u32; 2],
    /// Original ETC/heap size in bytes.
    pub heap_bytes: u32,
}
/// Bounded native startup policy. XML is data; external entities and doctypes
/// cannot grant filesystem/network authority. See official startup reference.
pub fn parse_startup(bytes: &[u8]) -> Result<Startup> {
    ensure!(
        bytes.len() <= 1024 * 1024,
        "CatSystem2 startup XML byte budget"
    );
    let document = XmlDocument::parse(bytes)?;
    ensure!(
        document.root().name == "document",
        "CatSystem2 startup root must be document"
    );
    let mut fields = BTreeMap::<String, String>::new();
    for path in [
        "SCRIPT/start",
        "SCREEN/width",
        "SCREEN/height",
        "SCREEN/v_width",
        "SCREEN/v_height",
        "ETC/heap",
    ] {
        if let Some(node) = document.unique_child_path(path)? {
            fields.insert(format!("document/{path}"), node.text.trim().to_owned());
        }
    }
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
