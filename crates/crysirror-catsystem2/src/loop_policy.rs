// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Original LoopPoint text, sound-prefix and resource-name policy only.
//! Shared engine services own the catalog and all decoding/playback.
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{AudioLoopMetadata, AudioLoopRange, AudioLoopTimebase},
};
use crysirror_resource::xml::{XmlDocument, XmlNode};
use std::collections::BTreeMap;
pub(crate) const MAX_LOOP_FILE: usize = 1024 * 1024;
#[derive(Clone)]
struct Archive {
    path: String,
    ext: String,
}
pub(crate) struct SoundPaths {
    archives: BTreeMap<String, Archive>,
    rules: Vec<(String, String)>,
}
fn text<'a>(doc: &'a XmlDocument, node: &'a XmlNode, name: &str) -> Result<Option<&'a str>> {
    let mut nodes = doc.children(node)?.filter(|n| n.name == name);
    let value = nodes.next();
    ensure!(
        nodes.next().is_none(),
        "duplicate CatSystem2 sound config field {name}"
    );
    Ok(value.map(|n| n.text.trim()))
}
fn walk_rules(
    doc: &XmlDocument,
    node: &XmlNode,
    rules: &mut Vec<(String, String)>,
    depth: usize,
) -> Result<()> {
    ensure!(
        depth < 16 && rules.len() < 256,
        "CatSystem2 sound-prefix budget"
    );
    if let Some(head) = text(doc, node, "head")?.filter(|v| !v.is_empty()) {
        ensure!(
            head.is_ascii() && head.len() <= 260,
            "CatSystem2 sound prefix"
        );
        rules.push((
            head.to_ascii_lowercase(),
            text(doc, node, "int")?.unwrap_or("").to_ascii_lowercase(),
        ));
    }
    if let Some(part) = doc.children(node)?.find(|n| n.name == "part") {
        for child in doc.children(part)? {
            walk_rules(doc, child, rules, depth + 1)?;
        }
    }
    Ok(())
}
impl SoundPaths {
    pub(crate) fn config_name(startup: &[u8]) -> Result<String> {
        let doc = XmlDocument::parse(startup)?;
        let name = doc
            .unique_child_path("SOUNDCONFIG")?
            .context("CatSystem2 legacy sound-prefix profile unsupported")?
            .text
            .trim()
            .replace('\\', "/");
        let path = if name.contains('/') {
            name
        } else {
            format!("config/{name}")
        };
        ensure!(
            VfsPath::new(Mount::Asset, &path)?.relative() == path,
            "CatSystem2 sound config path"
        );
        Ok(path)
    }
    pub(crate) fn parse(startup: &[u8], sound: &[u8]) -> Result<Self> {
        let doc = XmlDocument::parse(startup)?;
        let table = doc
            .unique_child_path("INT")?
            .context("CatSystem2 INT resource table")?;
        let mut archives = BTreeMap::new();
        for row in doc.children(table)? {
            if !row.name.starts_with("int") {
                continue;
            }
            let Some(name) = text(&doc, row, "name")? else {
                continue;
            };
            let path = text(&doc, row, "int")?
                .context("CatSystem2 sound archive path")?
                .replace('\\', "/");
            let ext = text(&doc, row, "ext")?.unwrap_or("").to_owned();
            ensure!(
                VfsPath::new(Mount::Asset, &path)?.relative() == path,
                "CatSystem2 sound archive path"
            );
            ensure!(
                !ext.contains(['/', '\\', ':', '.']) && ext.len() <= 32,
                "CatSystem2 sound extension"
            );
            ensure!(
                archives
                    .insert(name.to_ascii_lowercase(), Archive { path, ext })
                    .is_none(),
                "duplicate CatSystem2 archive type"
            );
        }
        let sound = XmlDocument::parse(sound)?;
        let parts = sound
            .unique_child_path("PART")?
            .context("CatSystem2 PART sound config")?;
        let mut rules = vec![];
        for part in sound.children(parts)? {
            walk_rules(&sound, part, &mut rules, 0)?;
        }
        ensure!(!rules.is_empty(), "CatSystem2 sound-prefix rules absent");
        Ok(Self { archives, rules })
    }
    pub(crate) fn resolve(&self, name: &str) -> Result<String> {
        ensure!(
            !name.is_empty() && name.is_ascii() && name.len() < 260,
            "CatSystem2 sound name profile"
        );
        let name = name.replace('\\', "/");
        if name.contains('/') {
            ensure!(
                VfsPath::new(Mount::Asset, &name)?.relative() == name,
                "CatSystem2 explicit sound path"
            );
            return Ok(name);
        }
        let lower = name.to_ascii_lowercase();
        let (_, kind) = self
            .rules
            .iter()
            .find(|(head, _)| lower.starts_with(head))
            .context("CatSystem2 sound prefix has no configured type")?;
        let archive = self
            .archives
            .get(kind)
            .context("CatSystem2 sound type has no archive")?;
        let suffix = if name.contains('.') || archive.ext.is_empty() {
            name
        } else {
            format!("{name}.{}", archive.ext)
        };
        let path = format!("{}/{suffix}", archive.path);
        ensure!(
            VfsPath::new(Mount::Asset, &path)?.relative() == path,
            "CatSystem2 resolved sound path"
        );
        Ok(path)
    }
}
/// Original wrapping signed64 decimal/hex parser, with suffix stop and native
/// decimal-fraction zero behavior. Checked by EDX:EAX at6A47B0, not decompiler int.
fn integer64(text: &str) -> i64 {
    let mut s = text.as_bytes();
    let negative = s.first() == Some(&b'-');
    if matches!(s.first(), Some(b'+' | b'-')) {
        s = &s[1..];
    }
    if !s.first().is_some_and(u8::is_ascii_digit) {
        return 0;
    }
    let hex = s.starts_with(b"0x") || s.starts_with(b"0X");
    if hex {
        s = &s[2..];
    }
    let mut value = 0u64;
    let mut count = 0;
    for b in s {
        let d = match b {
            b'0'..=b'9' => u64::from(b - b'0'),
            b'a'..=b'f' if hex => u64::from(b - b'a' + 10),
            b'A'..=b'F' if hex => u64::from(b - b'A' + 10),
            _ => break,
        };
        value = value
            .wrapping_mul(if hex { 16 } else { 10 })
            .wrapping_add(d);
        count += 1;
    }
    if !hex && s.get(count) == Some(&b'.') {
        return 0;
    }
    if negative {
        value.wrapping_neg() as i64
    } else {
        value as i64
    }
}
#[cfg(test)]
fn parse(bytes: &[u8], paths: &SoundPaths) -> Result<Vec<AudioLoopMetadata>> {
    parse_checked(bytes, paths, &|| Ok(()))
}
pub(crate) fn parse_checked(
    bytes: &[u8],
    paths: &SoundPaths,
    check: &dyn Fn() -> Result<()>,
) -> Result<Vec<AudioLoopMetadata>> {
    ensure!(
        bytes.len() <= MAX_LOOP_FILE,
        "CatSystem2 LoopPoint byte budget"
    );
    let input = std::str::from_utf8(bytes).context("CatSystem2 LoopPoint UTF8 profile")?;
    ensure!(
        !input.contains('\0'),
        "CatSystem2 embedded LoopPoint terminator"
    );
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let b = normalized.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut block = false;
    let mut i = 0;
    while i < b.len() {
        if i % 4096 == 0 {
            check()?;
        }
        if block {
            if b.get(i..i + 2) == Some(b"*/") {
                block = false;
                i += 2;
            } else {
                if b[i] == b'\n' {
                    out.push(b'\n');
                }
                i += 1;
            }
        } else if b.get(i..i + 2) == Some(b"/*") {
            block = true;
            i += 2;
        } else if b.get(i..i + 2) == Some(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    let out = String::from_utf8(out)?;
    let mut entries = BTreeMap::<String, AudioLoopMetadata>::new();
    for line in out.lines() {
        check()?;
        let line = line.replace('\t', " ");
        let line = line.trim_matches(' ').replace('"', "");
        let mut words = line.split(' ').filter(|w| !w.is_empty());
        let Some(name) = words.next() else {
            continue;
        };
        let asset = paths.resolve(name)?.to_ascii_lowercase();
        // Native mapper's descriptor has32 bytes before the two64-bit ranges.
        ensure!(
            asset.len() < 32,
            "CatSystem2 native loop descriptor name budget"
        );
        let start = words.next().map(integer64).unwrap_or(0);
        let end = words.next().map(integer64).unwrap_or(-1);
        ensure!(
            words.next().is_none(),
            "unsupported CatSystem2 LoopPoint extra fields"
        );
        ensure!(
            start >= 0 && (end == -1 || end > start),
            "unsupported CatSystem2 LoopPoint range"
        );
        let key = asset.to_ascii_lowercase();
        let end_frame = entries
            .get(&key)
            .map(|old| old.range.end_frame)
            .unwrap_or(if end == -1 { None } else { Some(end as u64) });
        // Native5EA0F0 replaces only start64 for an existing case-insensitive key.
        entries.insert(
            key,
            AudioLoopMetadata {
                asset,
                range: AudioLoopRange {
                    timebase: AudioLoopTimebase::ReferenceFrames { sample_rate: 44100 },
                    start_frame: start as u64,
                    end_frame,
                },
            },
        );
    }
    ensure!(entries.len() <= 4096, "CatSystem2 LoopPoint entry budget");
    Ok(entries.into_values().collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn paths() -> SoundPaths {
        SoundPaths::parse(br#"<document><INT><int19><name>bgm</name><int>bgm.int</int><ext>ogg</ext></int19></INT></document>"#,br#"<document><PART><bgm><head>BGM</head><int>bgm</int></bgm></PART></document>"#).unwrap()
    }
    #[test]
    fn native_comments_quotes_frame_timebase_and_duplicate_end_policy() {
        let m = parse(
            b"// heading\r\n\"bgm_01.ogg\"\t10\t30\r/* x */bgm_01.ogg 12 40\nbgm_02 0 -1\n",
            &paths(),
        )
        .unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].asset, "bgm.int/bgm_01.ogg");
        assert_eq!(m[0].range.start_frame, 12);
        assert_eq!(m[0].range.end_frame, Some(30));
        assert_eq!(m[1].range.end_frame, None);
        assert_eq!(
            m[0].range.timebase,
            AudioLoopTimebase::ReferenceFrames { sample_rate: 44100 }
        );
    }
    #[test]
    fn original_signed64_and_visible_unsupported_policy() {
        assert_eq!(integer64("4294967297"), 4294967297);
        assert_eq!(integer64("0xFFFFFFFFFFFFFFFF"), -1);
        assert_eq!(integer64("-0x10"), -16);
        assert_eq!(integer64("12.4"), 0);
        assert_eq!(integer64(" 12"), 0);
        assert_eq!(integer64("18446744073709551616"), 0);
        for text in [
            "unknown 0 10",
            "../x 0 10",
            "bgm_01 20 10",
            "bgm_01 -1 10",
            "bgm_01 1 2 3",
        ] {
            assert!(parse(text.as_bytes(), &paths()).is_err(), "{text}");
        }
    }
}
