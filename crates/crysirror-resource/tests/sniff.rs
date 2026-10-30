// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! Self-authored headers and capability-contract tests; no game/OS magic files.
use crysirror_core::service::AssetReader;
use crysirror_resource::sniff::{FileDetector, MAX_FILE_PREFIX_BYTES};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";
const UNKNOWN: &[u8] = &[0, 253, 35, 135, 0, 254, 66, 0];

struct Reader {
    bytes: Vec<u8>,
    declared: u64,
    chunk: usize,
    zero: bool,
    overreport: bool,
    change_length: bool,
    lengths: AtomicUsize,
    reads: Mutex<Vec<(u64, usize)>>,
}
impl Reader {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            declared: bytes.len() as u64,
            chunk: usize::MAX,
            zero: false,
            overreport: false,
            change_length: false,
            lengths: AtomicUsize::new(0),
            reads: Mutex::new(vec![]),
        }
    }
}
impl AssetReader for Reader {
    fn len(&self) -> crysirror_core::Result<u64> {
        let previous = self.lengths.fetch_add(1, Ordering::SeqCst);
        Ok(self.declared + u64::from(self.change_length && previous > 0))
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> crysirror_core::Result<usize> {
        assert!(offset + output.len() as u64 <= MAX_FILE_PREFIX_BYTES as u64);
        self.reads.lock().unwrap().push((offset, output.len()));
        if self.zero {
            return Ok(0);
        }
        if self.overreport {
            return Ok(output.len() + 1);
        }
        let offset = usize::try_from(offset).unwrap();
        let Some(bytes) = self.bytes.get(offset..) else {
            return Ok(0);
        };
        let count = output.len().min(bytes.len()).min(self.chunk);
        output[..count].copy_from_slice(&bytes[..count]);
        Ok(count)
    }
}

#[test]
fn embedded_rules_identify_image_and_audio_content_without_a_system_database() {
    let detector = FileDetector::new().unwrap();
    assert!(detector.rule_count() >= 100);
    let png = detector.identify_bytes(PNG).unwrap().unwrap();
    assert_eq!(png.mime_type, "image/png");
    assert_eq!(png.bytes_examined as usize, PNG.len());
    assert!(!png.prefix_truncated);
    assert!(png.rule_source.is_some());
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&36_u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&8000_u32.to_le_bytes());
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data\0\0\0\0");
    let audio = detector.identify_bytes(&wav).unwrap().unwrap();
    assert!(audio.mime_type.starts_with("audio/"), "{audio:?}");
    assert!(
        audio.description.to_ascii_lowercase().contains("wave"),
        "{audio:?}"
    );
}

#[test]
fn empty_and_unrecognised_data_are_unknown_not_successful_format_support() {
    let detector = FileDetector::new().unwrap();
    assert_eq!(detector.identify_bytes(&[]).unwrap(), None);
    assert_eq!(detector.identify_bytes(UNKNOWN).unwrap(), None);
    let empty = Reader::new(&[]);
    assert_eq!(detector.identify(&empty).unwrap(), None);
    assert!(empty.reads.lock().unwrap().is_empty());
    assert_eq!(detector.identify_bytes(&PNG[..6]).unwrap(), None);
}

#[test]
fn partial_reads_preserve_content_and_prefix_limit_is_fixed_for_large_capabilities() {
    let detector = FileDetector::new().unwrap();
    let mut short = Reader::new(PNG);
    short.chunk = 3;
    assert_eq!(
        detector.identify(&short).unwrap(),
        detector.identify_bytes(PNG).unwrap()
    );
    assert!(short.reads.lock().unwrap().len() > 1);
    let mut large = Reader::new(PNG);
    large.bytes.resize(MAX_FILE_PREFIX_BYTES, 0);
    large.declared = u64::MAX;
    large.chunk = 4093;
    let identity = detector.identify(&large).unwrap().unwrap();
    assert_eq!(identity.mime_type, "image/png");
    assert_eq!(identity.bytes_examined as usize, MAX_FILE_PREFIX_BYTES);
    assert!(identity.prefix_truncated);
    {
        let reads = large.reads.lock().unwrap();
        let (offset, requested) = reads.last().unwrap();
        assert_eq!(offset + *requested as u64, MAX_FILE_PREFIX_BYTES as u64);
    }
    let mut bytes = PNG.to_vec();
    bytes.resize(MAX_FILE_PREFIX_BYTES + 256, 0);
    let identity = detector.identify_bytes(&bytes).unwrap().unwrap();
    assert!(identity.prefix_truncated);
    assert_eq!(identity.bytes_examined as usize, MAX_FILE_PREFIX_BYTES);
}

#[test]
fn premature_zero_truncation_overreport_and_length_changes_are_errors() {
    let detector = FileDetector::new().unwrap();
    let mut zero = Reader::new(PNG);
    zero.zero = true;
    assert!(detector.identify(&zero).is_err());
    let mut truncated = Reader::new(&PNG[..PNG.len() - 1]);
    truncated.declared = PNG.len() as u64;
    assert!(detector.identify(&truncated).is_err());
    let mut invalid = Reader::new(PNG);
    invalid.overreport = true;
    assert!(detector.identify(&invalid).is_err());
    let mut changed = Reader::new(PNG);
    changed.change_length = true;
    assert!(detector.identify(&changed).is_err());
}

#[test]
fn changing_the_extension_does_not_change_the_content_identity() {
    let detector = FileDetector::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("image.png");
    std::fs::write(&image, PNG).unwrap();
    let before = detector
        .identify(&*crysirror_resource::io::memory(
            std::fs::read(&image).unwrap(),
        ))
        .unwrap();
    for extension in ["tjs", "exe", "xp3", "unknown"] {
        let renamed = dir.path().join(format!("renamed.{extension}"));
        std::fs::rename(&image, &renamed).unwrap();
        let after = detector
            .identify(&*crysirror_resource::io::memory(
                std::fs::read(&renamed).unwrap(),
            ))
            .unwrap();
        assert_eq!(before, after);
        std::fs::rename(renamed, &image).unwrap();
    }
}

#[test]
fn immutable_detector_is_send_sync_and_shared_queries_are_deterministic() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<FileDetector>();
    let detector = FileDetector::new().unwrap();
    let expected = detector.identify_bytes(PNG).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let detector = detector.clone();
            let expected = expected.clone();
            scope.spawn(move || {
                for _ in 0..8 {
                    assert_eq!(detector.identify_bytes(PNG).unwrap(), expected);
                    assert_eq!(detector.identify_bytes(UNKNOWN).unwrap(), None);
                }
            });
        }
    });
}
