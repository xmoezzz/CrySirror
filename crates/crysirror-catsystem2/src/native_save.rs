// SPDX-License-Identifier: MPL-2.0
//! Original CSS save-container layout and native handle policy. Capability I/O
//! and compression belong to shared Storage and Resource transforms.
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::{ClientStorage, MemoryAssetReader},
    state::ClientId,
    Error,
};
use crysirror_resource::{DecodeContext, TransformSpec};
use std::sync::Arc;
const LIMIT: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug)]
pub(crate) struct Container {
    pub tag: u32,
    pub bytes: Vec<u8>,
    pub records: Vec<(u32, std::ops::Range<usize>)>,
}
impl Container {
    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"tag":self.tag,"bytes":self.bytes.len(),"records":self.records.len()})
    }
}
///78BB10: missing file or wrong signature returns null; stricter bounded
/// validation rejects unsupported damaged extents rather than fake success.
pub(crate) fn load(
    storage: &dyn ClientStorage,
    client: ClientId,
    name: &str,
    cancel: crysirror_tasks::Cancellation,
) -> Result<Option<Container>> {
    cancel.check()?;
    let path = VfsPath::new(Mount::Save, name)?;
    let reader = match storage.open_read(&path) {
        Ok(r) => r,
        Err(Error::NotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let bytes = crysirror_resource::read_all(reader.as_ref(), LIMIT as u64)?;
    parse(bytes, client, cancel)
}
fn parse(
    bytes: Vec<u8>,
    client: ClientId,
    cancel: crysirror_tasks::Cancellation,
) -> Result<Option<Container>> {
    cancel.check()?;
    if bytes.get(..4) != Some(b"CSS\0") {
        return Ok(None);
    }
    ensure!(bytes.len() >= 16, "CatSystem2 CSS truncated header");
    let word = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let packed = word(4) as usize;
    let size = word(8) as usize;
    let tag = word(12);
    ensure!(
        size <= LIMIT && packed <= LIMIT && packed.checked_add(16) == Some(bytes.len()),
        "CatSystem2 CSS extent budget/layout"
    );
    let context = DecodeContext::new(client).with_cancellation(cancel.clone());
    let mut spec = TransformSpec::new("zlib");
    spec.expected_size = Some(size as u64);
    let stream = context.transforms.apply(
        &spec,
        Arc::new(MemoryAssetReader(Arc::from(bytes[16..].to_vec()))),
        &serde_json::Value::Null,
        &context,
    )?;
    let bytes = crysirror_resource::read_all(stream.as_ref(), LIMIT as u64)?;
    let mut records = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        cancel.check()?;
        ensure!(records.len() < 65536, "CatSystem2 CSS record budget");
        let header = bytes
            .get(offset..offset + 8)
            .context("CatSystem2 CSS truncated record")?;
        let id = u32::from_le_bytes(header[..4].try_into()?);
        let len = u32::from_le_bytes(header[4..].try_into()?) as usize;
        let begin = offset + 8;
        let end = begin
            .checked_add(len)
            .context("CatSystem2 CSS record overflow")?;
        ensure!(end <= bytes.len(), "CatSystem2 CSS record extent");
        records.push((id, begin..end));
        offset = end;
    }
    cancel.check()?;
    Ok(Some(Container {
        tag,
        bytes,
        records,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn css(body: &[u8]) -> Vec<u8> {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(body).unwrap();
        let z = z.finish().unwrap();
        let mut b = b"CSS\0".to_vec();
        for x in [z.len() as u32, body.len() as u32, 42] {
            b.extend(x.to_le_bytes());
        }
        b.extend(z);
        b
    }
    #[test]
    fn actual_header_tag_and_duplicate_record_order_are_preserved() {
        let c = ClientId(1);
        let token = crysirror_tasks::Cancellation::default();
        let mut body = Vec::new();
        body.extend(7u32.to_le_bytes());
        body.extend(3u32.to_le_bytes());
        body.extend(b"abc");
        body.extend(7u32.to_le_bytes());
        body.extend(0u32.to_le_bytes());
        let parsed = parse(css(&body), c, token.clone()).unwrap().unwrap();
        assert_eq!(parsed.tag, 42);
        assert_eq!(parsed.records, vec![(7, 8..11), (7, 19..19)]);
        assert_eq!(parsed.bytes, body);
        assert!(parse(b"not CSS".to_vec(), c, token.clone())
            .unwrap()
            .is_none());
        assert!(parse(css(&[1, 2]), c, token).is_err());
    }
}
