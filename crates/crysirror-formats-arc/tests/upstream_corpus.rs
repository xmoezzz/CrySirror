// SPDX-License-Identifier: GPL-3.0-or-later
//! External upstream self-authored fixture comparison. Binary fixtures remain in
//! the ignored reference checkout; expected ASCII outputs come from its C++ tests.
use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use crysirror_formats_arc::{BgiArc, MajiroArc, Nscripter, Xp3};
use crysirror_resource::{read_all, ArchiveFormat, DecodeContext, PipelineRules, TransformSpec};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
fn compare(
    root: &Path,
    relative: &str,
    format: &dyn ArchiveFormat,
    expected: &[(&str, &[u8])],
    context: &DecodeContext,
) {
    let path = root.join(relative).canonicalize().unwrap();
    assert!(path.starts_with(root));
    let source: Arc<dyn AssetReader> =
        Arc::new(MemoryAssetReader(Arc::from(std::fs::read(path).unwrap())));
    assert!(
        format.probe(source.as_ref()).unwrap().confidence > 0,
        "probe {relative}"
    );
    let archive = format
        .open(source, context)
        .unwrap_or_else(|e| panic!("{relative}: {e:#}"));
    let actual: BTreeMap<_, _> = archive
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.path.clone(),
                read_all(
                    archive
                        .open(entry.id, context)
                        .unwrap_or_else(|e| panic!("{relative}:{}: {e:#}", entry.path))
                        .as_ref(),
                    context.limits.max_entry_size,
                )
                .unwrap(),
            )
        })
        .collect();
    let expected: BTreeMap<_, _> = expected
        .iter()
        .map(|(name, bytes)| (name.to_string(), bytes.to_vec()))
        .collect();
    assert_eq!(actual, expected, "{relative}");
}
#[test]
#[ignore = "requires complete pinned arc_unpacker checkout via ARC_UNPACKER_SOURCE; no fixture binaries are distributed"]
fn original_cpp_declared_fixture_outputs_match_the_rust_archive_plugins() {
    let root = PathBuf::from(
        std::env::var_os("ARC_UNPACKER_SOURCE").expect("ARC_UNPACKER_SOURCE required"),
    )
    .canonicalize()
    .unwrap();
    let revision = std::process::Command::new("git")
        .args(["-C", root.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(revision.status.success());
    assert_eq!(
        std::str::from_utf8(&revision.stdout).unwrap().trim(),
        "456834ecf2e5686813802c37efd829310485c57d"
    );
    let status = std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap(),
            "diff",
            "--quiet",
            "HEAD",
            "--",
            "src",
            "tests/dec",
        ])
        .status()
        .unwrap();
    assert!(status.success(), "reference source/fixtures changed");
    let mut context = DecodeContext::new(ClientId(1));
    let mut rules = PipelineRules::default();
    rules.rules.insert(
        ("kirikiri/xp3".into(), None),
        vec![TransformSpec::new("identity")],
    );
    context.pipelines = Arc::new(rules);
    for file in [
        "xp3-v1.xp3",
        "xp3-v2.xp3",
        "xp3-compressed-table.xp3",
        "xp3-compressed-files.xp3",
        "xp3-multiple-segm.xp3",
        "xp3-shuffled.xp3",
        "xp3-time.xp3",
    ] {
        compare(
            &root,
            &format!("tests/dec/kirikiri/files/xp3/{file}"),
            &Xp3,
            &[
                ("123.txt", b"1234567890"),
                ("abc.xyz", b"abcdefghijklmnopqrstuvwxyz"),
            ],
            &context,
        );
    }
    for file in ["v2.arc", "v3.arc"] {
        compare(
            &root,
            &format!("tests/dec/majiro/files/arc/{file}"),
            &MajiroArc,
            &[
                ("123.txt", b"1234567890"),
                ("abc.txt", b"abcdefghijklmnopqrstuvwxyz"),
            ],
            &context,
        );
    }
    for file in ["test_packfile.arc", "test_buriko_arc20.arc"] {
        compare(
            &root,
            &format!("tests/dec/bgi/files/arc/{file}"),
            &BgiArc,
            &[("abc.txt", b"123"), ("another.txt", b"abcdefghij")],
            &context,
        );
    }
    compare(
        &root,
        "tests/dec/nscripter/files/sar/test.sar",
        &Nscripter::sar(),
        &[
            ("abc.txt", b"123"),
            ("dir/another.txt", b"AAAAAAAAAAAAAAAA"),
        ],
        &context,
    );
    compare(
        &root,
        "tests/dec/nscripter/files/nsa/uncompressed.nsa",
        &Nscripter::nsa(),
        &[("abc.txt", b"123"), ("another.txt", b"abcdefghij")],
        &context,
    );
    // The original test defines its small generated bitmap as a zlib hexadecimal
    // literal, not an installed-game image. Read that expected value in place.
    let test =
        std::fs::read_to_string(root.join("tests/dec/nscripter/nsa_archive_decoder_test.cc"))
            .unwrap();
    let tail = &test[test.find("\"test.bmp\"").unwrap()..];
    let packed: Vec<u8> = tail
        .as_bytes()
        .windows(4)
        .filter(|p| p[0] == b'\\' && p[1] == b'x')
        .map(|p| u8::from_str_radix(std::str::from_utf8(&p[2..4]).unwrap(), 16).unwrap())
        .collect();
    let mut bitmap = Vec::new();
    flate2::read::ZlibDecoder::new(&packed[..])
        .read_to_end(&mut bitmap)
        .unwrap();
    compare(
        &root,
        "tests/dec/nscripter/files/nsa/lzss.nsa",
        &Nscripter::nsa(),
        &[("test.bmp", &bitmap)],
        &context,
    );
    println!("verified 14 external source fixtures against the original C++ tests' declared outputs; no original C++ executable was built");
}
