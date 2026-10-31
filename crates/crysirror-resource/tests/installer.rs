use crysirror_core::{
    service::{AssetReader, MemoryAssetReader},
    state::ClientId,
};
use crysirror_resource::{installer::InstallerCatalog, read_all, DecodeContext};
use std::{
    io::{Cursor, Write},
    sync::Arc,
};
fn fixture(name: &str) -> Vec<u8> {
    let mut cab = cab::CabinetBuilder::new();
    cab.add_folder(cab::CompressionType::MsZip).add_file("f1");
    let mut writer = cab.build(Cursor::new(Vec::new())).unwrap();
    writer
        .next_file()
        .unwrap()
        .unwrap()
        .write_all(b"native script bytes")
        .unwrap();
    let cabinet = writer.finish().unwrap().into_inner();
    let mut package =
        msi::Package::create(msi::PackageType::Installer, Cursor::new(Vec::new())).unwrap();
    use msi::{Column as C, Insert, Value as V};
    package
        .create_table(
            "Directory",
            vec![
                C::build("Directory").primary_key().string(72),
                C::build("Directory_Parent").nullable().string(72),
                C::build("DefaultDir").string(255),
            ],
        )
        .unwrap();
    package
        .insert_rows(
            Insert::into("Directory")
                .row(vec![V::from("ROOT"), V::Null, V::from("SourceDir")])
                .row(vec![
                    V::from("DATA"),
                    V::from("ROOT"),
                    V::from("DATA~1|data"),
                ]),
        )
        .unwrap();
    package
        .create_table(
            "Component",
            vec![
                C::build("Component").primary_key().string(72),
                C::build("Directory_").string(72),
            ],
        )
        .unwrap();
    package
        .insert_rows(Insert::into("Component").row(vec![V::from("cmp"), V::from("DATA")]))
        .unwrap();
    package
        .create_table(
            "Media",
            vec![
                C::build("DiskId").primary_key().int16(),
                C::build("LastSequence").int32(),
                C::build("Cabinet").string(255),
            ],
        )
        .unwrap();
    package
        .insert_rows(Insert::into("Media").row(vec![V::Int(1), V::Int(1), V::from("#pack")]))
        .unwrap();
    package
        .create_table(
            "File",
            vec![
                C::build("File").primary_key().string(72),
                C::build("Component_").string(72),
                C::build("FileName").string(255),
                C::build("FileSize").int32(),
                C::build("Sequence").int32(),
            ],
        )
        .unwrap();
    package
        .insert_rows(Insert::into("File").row(vec![
            V::from("f1"),
            V::from("cmp"),
            V::from(name),
            V::Int(19),
            V::Int(1),
        ]))
        .unwrap();
    package
        .write_stream("pack")
        .unwrap()
        .write_all(&cabinet)
        .unwrap();
    package.into_inner().unwrap().into_inner()
}
#[test]
fn shared_installed_view_reads_member_without_installing_and_retains_budget() {
    let cx = DecodeContext::new(ClientId(3));
    let source: Arc<dyn AssetReader> =
        Arc::new(MemoryAssetReader(fixture("SCRIPT~1|script.spt").into()));
    let catalog = InstallerCatalog::open(source, &cx).unwrap();
    assert_eq!(catalog.files()[0].path, "data/script.spt");
    let reader = catalog.read(0, &cx).unwrap();
    assert_eq!(
        read_all(reader.as_ref(), 64).unwrap(),
        b"native script bytes"
    );
    assert_eq!(cx.resident_bytes(), 19);
    drop(reader);
    assert_eq!(cx.resident_bytes(), 0);
    assert!(catalog.read(1, &cx).is_err());
    let mut small = DecodeContext::new(ClientId(4));
    small.limits.max_entry_size = 18;
    let source = Arc::new(MemoryAssetReader(fixture("script.spt").into()));
    assert!(InstallerCatalog::open(source, &small).is_err());
}
#[test]
fn installed_paths_reject_traversal() {
    let cx = DecodeContext::new(ClientId(3));
    let source = Arc::new(MemoryAssetReader(fixture("../escape.spt").into()));
    assert!(InstallerCatalog::open(source, &cx).is_err());
}
#[test]
fn installed_directory_view_remaps_paths_without_losing_original_record_identity() {
    use crysirror_resource::{ArchiveFormat,installer::MsiArchiveFormat};
    let source:Arc<dyn AssetReader>=Arc::new(MemoryAssetReader(fixture("SCRIPT~1|script.spt").into()));
    let mut cx=DecodeContext::new(ClientId(3));
    cx.parameters.insert("windows/msi".into(),serde_json::json!({"root":"data"}));
    let archive=MsiArchiveFormat.open(source.clone(),&cx).unwrap();
    assert_eq!(archive.entries().len(),1);
    assert_eq!(archive.entries()[0].path,"script.spt");
    assert_eq!(archive.entries()[0].metadata["installed_path"],"data/script.spt");
    assert_eq!(read_all(archive.open(archive.entries()[0].id,&cx).unwrap().as_ref(),64).unwrap(),b"native script bytes");
    for root in ["../data","/data","data/","absent"] {
        cx.parameters.insert("windows/msi".into(),serde_json::json!({"root":root}));
        assert!(MsiArchiveFormat.open(source.clone(),&cx).is_err(),"accepted root {root}");
    }
}
#[test]
#[ignore = "reads owner MSI media at runtime; never distributes installer or game data"]
fn owner_system_nnn_installed_catalog() {
    struct File(std::sync::Mutex<std::fs::File>, u64);
    impl AssetReader for File {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(self.1)
        }
        fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = self.0.lock().unwrap();
            f.seek(SeekFrom::Start(offset))
                .and_then(|_| f.read(out))
                .map_err(|e| crysirror_core::Error::Io(e.to_string()))
        }
    }
    let f = std::fs::File::open(std::env::var("CRYSIRROR_TEST_MSI").unwrap()).unwrap();
    let len = f.metadata().unwrap().len();
    let mut cx = DecodeContext::new(ClientId(1));
    cx.limits.max_entry_size = 1024 * 1024 * 1024;
    let catalog =
        InstallerCatalog::open(Arc::new(File(std::sync::Mutex::new(f), len)), &cx).unwrap();
    println!("installed_files={}", catalog.files().len());
    for (i, f) in catalog.files().iter().enumerate() {
        println!("{} {} {}", i, f.size, f.path);
        if f.path.to_ascii_lowercase().ends_with(".spt") {
            let r = catalog.read(i, &cx).unwrap();
            let mut prefix = [0; 32];
            r.read_at(0, &mut prefix).unwrap();
            println!("script_prefix={prefix:02x?}");
            break;
        }
    }
}
