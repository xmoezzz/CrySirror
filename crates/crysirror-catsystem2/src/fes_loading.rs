// SPDX-License-Identifier: MPL-2.0
//! Native FES naming/load policy. Shared Storage/XML/Resource transforms own
//! capability I/O, XML parsing and zlib; this module has no decompressor.
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{ClientStorage, MemoryAssetReader},
    state::ClientId,
};
use crysirror_resource::{xml::XmlDocument, DecodeContext, TransformSpec};
use std::sync::Arc;
pub(crate) struct Prepared {
    pub path: String,
    pub program: Arc<crate::fes_program::Program>,
}
pub(crate) fn load(
    storage: &dyn ClientStorage,
    client: ClientId,
    name: &str,
    cancel: crysirror_tasks::Cancellation,
) -> Result<Prepared> {
    let check = || cancel.check();
    check()?;
    ensure!(
        name.is_ascii() && name.len() < 260,
        "CatSystem2 unsupported FES filename profile"
    );
    let name = name.replace('\\', "/");
    VfsPath::new(Mount::Asset, &name)?;
    let startup = crate::native_settings::optional_asset(storage, "config/startup.xml")?
        .context("CatSystem2 FES startup missing")?;
    let xml = XmlDocument::parse(&startup)?;
    let table = xml
        .unique_child_path("INT")?
        .context("CatSystem2 FES INT table missing")?;
    let mut row = None;
    for node in xml.children(table)? {
        if let Some(n) = xml.children(node)?.find(|n| n.name == "name") {
            if n.text.eq_ignore_ascii_case("fes") {
                ensure!(row.is_none(), "CatSystem2 duplicate FES INT declaration");
                row = Some(node);
            }
        }
    }
    let row = row.context("CatSystem2 FES resource type missing")?;
    let field = |key: &str| -> Result<Option<String>> {
        let mut found = xml.children(row)?.filter(|n| n.name == key);
        let v = found.next();
        ensure!(found.next().is_none(), "CatSystem2 duplicate FES INT field");
        Ok(v.map(|n| n.text.clone()))
    };
    let archive = field("int")?.context("CatSystem2 FES archive missing")?;
    let ext = field("ext")?.context("CatSystem2 FES extension missing")?;
    ensure!(
        !ext.is_empty() && !ext.contains(['.', '/', '\\', ':']),
        "CatSystem2 FES extension profile"
    );
    let mut candidates = Vec::new();
    if name.contains('/') {
        candidates.push(name);
    } else {
        //6A7510 treats any dot after the last separator as an extension.
        let file = if name.contains('.') {
            name
        } else {
            format!("{name}.{ext}")
        };
        if let Some(dir) = field("dir")?.filter(|v| !v.is_empty()) {
            candidates.push(format!("{dir}/{file}"));
        }
        candidates.push(format!("{archive}/{file}"));
    }
    for path in candidates {
        check()?;
        VfsPath::new(Mount::Asset, &path)?;
        let Some(bytes) = crate::native_settings::optional_asset(storage, &path)? else {
            continue;
        };
        ensure!(
            bytes.len() >= 16 && bytes.len() <= 16 * 1024 * 1024 && &bytes[..4] == b"FES\0",
            "CatSystem2 unsupported FES container"
        );
        let word = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let packed = word(4);
        let size = word(8);
        ensure!(
            packed > 0
                && packed.checked_add(16) == Some(bytes.len())
                && size > 0
                && size <= 16 * 1024 * 1024
                && word(12) == 0,
            "CatSystem2 unsupported plain FES/preprocess or container extent"
        );
        let context = DecodeContext::new(client).with_cancellation(cancel.clone());
        let mut spec = TransformSpec::new("zlib");
        spec.expected_size = Some(size as u64);
        let reader = context.transforms.apply(
            &spec,
            Arc::new(MemoryAssetReader(Arc::from(bytes[16..].to_vec()))),
            &serde_json::Value::Null,
            &context,
        )?;
        let text = crysirror_resource::read_all(reader.as_ref(), 16 * 1024 * 1024)?;
        check()?;
        let program = Arc::new(crate::fes_program::Program::parse(&text, &check)?);
        return Ok(Prepared { path, program });
    }
    anyhow::bail!("CatSystem2 unsupported FES missing-load result profile")
}
