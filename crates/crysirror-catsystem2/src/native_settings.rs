// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Original reviewed ABI's logical configuration byte records, not host state.
//! Constructor/source order is recorded in SOURCE.json. Original system/global
//! game saves are separate and unsupported. XML parsing and storage are shared.
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
    Error,
};
use crysirror_resource::xml::XmlDocument;
const BASE: usize = 73128;
const EXTENT: usize = 5744;
pub(crate) struct NativeSettings {
    bytes: Vec<u8>,
    pub flag_count: u32,
    pub gamedef: bool,
    pub setup: bool,
}
impl NativeSettings {
    fn put(&mut self, offset: usize, value: u32) {
        let at = offset - BASE;
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn get(&self, offset: usize) -> u32 {
        let at = offset - BASE;
        u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap())
    }
    fn string(&mut self, offset: usize, extent: usize, value: &str) -> Result<()> {
        ensure!(
            value.is_ascii() && value.len() < extent,
            "CatSystem2 native settings string profile"
        );
        let at = offset - BASE;
        self.bytes[at..at + extent].fill(0);
        self.bytes[at..at + value.len()].copy_from_slice(value.as_bytes());
        Ok(())
    }
    fn constructor() -> Self {
        // 71C5A0 zeroes the actual81100-byte native manager-data buffer before
        // 71D070 applies these fields. This is initialization, never fallback.
        let mut s = Self {
            bytes: vec![0; EXTENT],
            flag_count: 1024,
            gamedef: false,
            setup: false,
        };
        s.put(73144, 1);
        for row in 0..48 {
            s.put(73148 + row * 116, 1);
            s.put(73184 + row * 116, 100);
            s.put(73228 + row * 116, 100);
        }
        for offset in [78716, 78720, 78724, 78728, 78732] {
            s.put(offset, 100);
        }
        for (offset, value) in [
            (78756, 4),
            (78760, 4),
            (78768, 15),
            (78772, 5),
            (78776, 1),
            (78784, 1),
            (78792, 100),
            (78800, u32::MAX),
            (78836, 1),
            (78840, 1),
            (78844, 1),
            (78848, 5),
            (78860, u32::MAX),
            (78864, 1),
            (78868, 1),
        ] {
            s.put(offset, value);
        }
        // 71D070 deliberately overwrites78780 from1 to0 near its return.
        s
    }
    fn startup(&mut self, d: &XmlDocument) -> Result<()> {
        self.flag_count = number(d, "GAME/flag/max", 1024)?;
        for row in 0..48 {
            let base = 73148 + row * 116;
            let path = format!("VOICE/v{row:02}");
            for (name, off, width) in [("voice", 4, 32), ("head", 40, 40), ("int", 84, 32)] {
                if let Some(v) = value(d, &format!("{path}/{name}"))? {
                    let v = if name == "int" {
                        v.to_ascii_lowercase()
                    } else {
                        v.to_owned()
                    };
                    self.string(base + off, width, &v)?;
                }
            }
            for (name, off) in [("mastervol", 36), ("group", 80)] {
                let old = self.get(base + off);
                self.put(base + off, number(d, &format!("{path}/{name}"), old)?);
            }
        }
        for (offset, path) in [
            (78716, "VOICE/vol/master"),
            (78720, "VOICE/vol/bgm"),
            (78724, "VOICE/vol/se"),
            (78728, "VOICE/vol/voice"),
            (78732, "VOICE/vol/play"),
            (78796, "GAME/read/enable"),
            (78800, "GAME/read/color"),
            (78804, "GAME/skipspeed"),
            (78792, "GAME/log/limit"),
            (78812, "GAME/string/wipe"),
            (78816, "GAME/ctrlmode"),
            (78820, "GAME/autoleft"),
            (78824, "GAME/readsel/enable"),
        ] {
            let old = self.get(offset);
            self.put(offset, number(d, path, old)?);
        }
        // 71D430 selects the current auto preset after reading all32 presets.
        let preset = self.get(78760);
        ensure!(preset < 32, "CatSystem2 native auto preset index");
        for (name, offset) in [("base", 78768), ("char", 78772)] {
            let old = self.get(offset);
            self.put(
                offset,
                number(d, &format!("GAME/autopriset/p{preset:02}/{name}"), old)?,
            );
        }
        let rgba = [
            number(d, "GAME/wndcolor/r", 255)?,
            number(d, "GAME/wndcolor/g", 255)?,
            number(d, "GAME/wndcolor/b", 255)?,
            number(d, "GAME/wndcolor/a", 255)?,
        ];
        ensure!(
            rgba.iter().all(|v| *v <= 255),
            "CatSystem2 native wndcolor profile"
        );
        let alpha = if number(d, "CONFIG_B/wnd_alpha", 1)? != 0 {
            100 * rgba[3] / 255
        } else {
            rgba[3]
        };
        self.put(
            78860,
            (alpha << 24) | (rgba[0] << 16) | (rgba[1] << 8) | rgba[2],
        );
        // SYSTEMVOICE names/INT/play arrays start78872, beyond359's four spans.
        Ok(())
    }
    fn apply_persisted(&mut self, d: &XmlDocument) -> Result<()> {
        // Complete720610 projection for fields inside the four359 spans.
        for (offset, path) in [
            (73144, "VOICE/cont_voice"),
            (78716, "VOICE/vol_master"),
            (78720, "VOICE/vol_bgm"),
            (78724, "VOICE/vol_se"),
            (78728, "VOICE/vol_voice"),
            (78732, "VOICE/vol_play"),
            (78736, "VOICE/mute_master"),
            (78740, "VOICE/mute_bgm"),
            (78744, "VOICE/mute_se"),
            (78748, "VOICE/mute_voice"),
            (78752, "VOICE/mute_play"),
            (78756, "MESSAGE/speed"),
            (78760, "MESSAGE/auto_p"),
            (78768, "MESSAGE/base"),
            (78772, "MESSAGE/CHAR"),
            (78764, "MESSAGE/ind"),
            (78780, "MESSAGE/cont"),
            (78776, "MESSAGE/still"),
            (78784, "MESSAGE/waitpcm"),
            (78788, "MESSAGE/skip_full"),
            (78792, "MESSAGE/log_limit"),
            (78828, "EFFECT/speed"),
            (78836, "EFFECT/help"),
            (78840, "EFFECT/auto_c"),
            (78844, "EFFECT/auto_cd"),
            (78848, "EFFECT/auto_cds"),
            (78852, "EFFECT/dramatic"),
            (78832, "EFFECT/skipdraw"),
            (78856, "EFFECT/skipmot"),
            (78860, "GAME/wndcolor"),
            (78864, "GAME/enabledrag"),
            (78868, "GAME/drawwnd"),
            (78796, "MESSAGE/enable_read"),
            (78800, "MESSAGE/read_color"),
            (78804, "MESSAGE/skipspeed"),
            (78808, "MESSAGE/contskip"),
            (78812, "MESSAGE/wipe"),
            (78816, "MESSAGE/ctrlmode"),
            (78820, "MESSAGE/autoleft"),
            (78824, "MESSAGE/enable_read_sel"),
        ] {
            let old = self.get(offset);
            self.put(offset, number(d, path, old)?);
        }
        for row in 0..48 {
            let at = 73148 + row * 116;
            let old = self.get(at);
            self.put(at, number(d, &format!("VOICE/play{row:02}"), old)?);
        }
        Ok(())
    }
    fn finish(&mut self) {
        for (offset, source) in [
            (73128, 78720),
            (73132, 78724),
            (73136, 78728),
            (73140, 78732),
        ] {
            self.put(offset, self.get(78716).wrapping_mul(self.get(source)) / 100);
        }
    }
    pub(crate) fn writes(&self, addresses: &[u32]) -> Result<Vec<(u32, Vec<u8>)>> {
        ensure!(addresses.len() == 4, "CatSystem2 native359 arguments");
        let mut at = 0;
        let mut writes = vec![];
        for (address, length) in addresses.iter().zip([5628, 72, 32, 12]) {
            writes.push((*address, self.bytes[at..at + length].to_vec()));
            at += length;
        }
        Ok(writes)
    }
}
fn value<'a>(d: &'a XmlDocument, path: &str) -> Result<Option<&'a str>> {
    Ok(d.unique_child_path(path)?.map(|n| n.text.as_str()))
}
fn number(d: &XmlDocument, path: &str, default: u32) -> Result<u32> {
    Ok(value(d, path)?
        .map(crate::native_values::integer)
        .unwrap_or(default))
}
pub(crate) fn optional_asset(storage: &dyn ClientStorage, path: &str) -> Result<Option<Vec<u8>>> {
    let path = VfsPath::new(Mount::Asset, path)?;
    let r = match storage.open_asset(&path) {
        Ok(r) => r,
        Err(Error::NotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    Ok(Some(crysirror_resource::read_all(r.as_ref(), 1024 * 1024)?))
}
// Native5D4A00(type19)→5D59E0 mode3 tries the configured loose INT
// directory first, then the archive member. This is native name policy, not
// another filesystem implementation; both candidates remain scoped Storage.
pub(crate) fn optional_native_asset(
    storage: &dyn ClientStorage,
    startup: &XmlDocument,
    path: &str,
) -> Result<Option<Vec<u8>>> {
    let (archive, member) = path
        .split_once('/')
        .context("CatSystem2 configured resource path")?;
    if let Some(table) = startup.unique_child_path("INT")? {
        for row in startup.children(table)? {
            let fields = startup.children(row)?.collect::<Vec<_>>();
            let Some(name) = fields.iter().find(|n| n.name == "int") else {
                continue;
            };
            if !name.text.eq_ignore_ascii_case(archive) {
                continue;
            }
            if let Some(dir) = fields
                .iter()
                .find(|n| n.name == "dir")
                .filter(|n| !n.text.is_empty())
            {
                let candidate = format!("{}/{}", dir.text.replace('\\', "/"), member);
                if let Some(bytes) = optional_asset(storage, &candidate)? {
                    return Ok(Some(bytes));
                }
            }
            break;
        }
    }
    optional_asset(storage, path)
}
pub(crate) fn load(
    storage: &dyn ClientStorage,
    check: &dyn Fn() -> Result<()>,
) -> Result<NativeSettings> {
    check()?;
    let startup = optional_asset(storage, "config/startup.xml")?
        .context("CatSystem2 original startup missing")?;
    let mut result = NativeSettings::constructor();
    result.startup(&XmlDocument::parse(&startup)?)?;
    // WinMain52C190: creator(0,0,1),7126A0(gamedef) then7126D0(setup).
    if let Some(bytes) = optional_native_asset(
        storage,
        &XmlDocument::parse(&startup)?,
        "config.int/gamedef.xml",
    )? {
        check()?;
        result.apply_persisted(&XmlDocument::parse(&bytes)?)?;
        result.gamedef = true;
    }
    check()?;
    let path = VfsPath::new(Mount::Save, "setup.xml")?;
    let setup = match storage.open_read(&path) {
        Ok(reader) => Some(crysirror_resource::read_all(reader.as_ref(), 1024 * 1024)?),
        Err(Error::NotFound) => None,
        Err(e) => return Err(e.into()),
    };
    if let Some(bytes) = setup {
        ensure!(bytes.len() <= 1024 * 1024, "CatSystem2 setup XML budget");
        result.apply_persisted(&XmlDocument::parse(&bytes)?)?;
        result.setup = true;
    }
    result.finish();
    check()?;
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reviewed_constructor_and_startup_are_real_nonzero_records() {
        let mut s = NativeSettings::constructor();
        s.startup(&XmlDocument::parse(br#"<document><CONFIG_B><wnd_alpha>0</wnd_alpha></CONFIG_B><GAME><wndcolor><r>1</r><g>2</g><b>3</b><a>80</a></wndcolor><flag><max>2048</max></flag><read><enable>0</enable><color>0xff0000ff</color></read></GAME></document>"#).unwrap()).unwrap();
        s.finish();
        let w = s.writes(&[0, 6000, 7000, 8000]).unwrap();
        assert_eq!(
            w.iter().map(|(_, b)| b.len()).collect::<Vec<_>>(),
            [5628, 72, 32, 12]
        );
        assert_eq!(s.flag_count, 2048);
        assert_eq!(s.get(73148), 1);
        assert_eq!(s.get(73184), 100);
        assert_eq!(s.get(73228), 100);
        assert_eq!(s.get(78860), 0x50010203);
        assert_eq!(s.get(78800), 0xff0000ff);
        assert_eq!(s.get(73128), 100);
    }
    #[test]
    fn source_overlay_order_and_effective_volume_use_original_math() {
        let mut s = NativeSettings::constructor();
        s.apply_persisted(&XmlDocument::parse(b"<document><VOICE><vol_master>80</vol_master><vol_bgm>60</vol_bgm></VOICE><MESSAGE><speed>7</speed><CHAR>5</CHAR><cont_voice>0</cont_voice></MESSAGE></document>").unwrap()).unwrap();
        s.apply_persisted(&XmlDocument::parse(b"<document><VOICE><vol_master>50</vol_master></VOICE><MESSAGE><speed>3</speed></MESSAGE></document>").unwrap()).unwrap();
        s.finish();
        assert_eq!(s.get(73128), 30);
        assert_eq!(s.get(78756), 3);
        assert_eq!(s.get(78772), 5);
        assert_eq!(s.get(73144), 1);
    }
    struct Storage {
        assets: std::collections::BTreeMap<String, Vec<u8>>,
        setup: Vec<u8>,
    }
    struct Reader(Vec<u8>);
    impl crysirror_core::service::AssetReader for Reader {
        fn len(&self) -> crysirror_core::Result<u64> {
            Ok(self.0.len() as u64)
        }
        fn read_at(&self, offset: u64, output: &mut [u8]) -> crysirror_core::Result<usize> {
            let at = usize::try_from(offset).unwrap_or(usize::MAX);
            let Some(bytes) = self.0.get(at..) else {
                return Ok(0);
            };
            let n = bytes.len().min(output.len());
            output[..n].copy_from_slice(&bytes[..n]);
            Ok(n)
        }
    }
    impl ClientStorage for Storage {
        fn read(&self, path: &VfsPath) -> crysirror_core::Result<Vec<u8>> {
            let r = self.open_read(path)?;
            crysirror_resource::read_all(r.as_ref(), 1024 * 1024)
                .map_err(|e| Error::Io(e.to_string()))
        }
        fn open_asset(
            &self,
            path: &VfsPath,
        ) -> crysirror_core::Result<std::sync::Arc<dyn crysirror_core::service::AssetReader>>
        {
            let bytes = self.assets.get(path.relative()).ok_or(Error::NotFound)?;
            Ok(std::sync::Arc::new(Reader(bytes.clone())))
        }
        fn open_read(
            &self,
            path: &VfsPath,
        ) -> crysirror_core::Result<std::sync::Arc<dyn crysirror_core::service::AssetReader>>
        {
            if path.mount() == Mount::Asset {
                return self.open_asset(path);
            }
            if path.mount() != Mount::Save || path.relative() != "setup.xml" {
                return Err(Error::NotFound);
            }
            Ok(std::sync::Arc::new(Reader(self.setup.clone())))
        }
        fn write_atomic(&self, _: &VfsPath, _: &[u8]) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn remove(&self, _: &VfsPath) -> crysirror_core::Result<()> {
            Err(Error::ReadOnly)
        }
        fn list(&self, _: Mount, _: &str) -> crysirror_core::Result<Vec<String>> {
            Err(Error::Unsupported("test listing"))
        }
    }
    #[test]
    fn source_loading_prefers_native_loose_default_then_client_setup_and_rejects_unknown_xml() {
        let mut files = std::collections::BTreeMap::new();
        files.insert("config/startup.xml".into(),b"<document><INT><int13><int>config.int</int><dir>config</dir></int13></INT><GAME><flag><max>2048</max></flag></GAME></document>".to_vec());
        files.insert(
            "config.int/gamedef.xml".into(),
            b"<document><VOICE><vol_master>90</vol_master></VOICE></document>".to_vec(),
        );
        files.insert("config/gamedef.xml".into(),b"<document><VOICE><vol_master>80</vol_master><vol_bgm>60</vol_bgm></VOICE><MESSAGE><speed>7</speed></MESSAGE></document>".to_vec());
        let mut storage=Storage{assets:files,setup:b"<document><VOICE><vol_master>50</vol_master></VOICE><MESSAGE><speed>3</speed></MESSAGE></document>".to_vec()};
        let result = load(&storage, &|| Ok(())).unwrap();
        assert!(result.gamedef && result.setup);
        assert_eq!(result.flag_count, 2048);
        assert_eq!(result.get(73128), 30);
        assert_eq!(result.get(78756), 3);
        storage.setup = b"<!DOCTYPE unsupported><document/>".to_vec();
        assert!(load(&storage, &|| Ok(())).is_err());
        assert!(load(&storage, &|| anyhow::bail!("cancelled")).is_err());
    }
    #[test]
    fn sound_found_state_keeps_real_shared_declarations_and_mode3_precedence() {
        let mut storage=Storage{assets:std::collections::BTreeMap::from([
            ("config/startup.xml".into(),b"<document><INT><int13><name>config</name><int>config.int</int><dir>config</dir></int13></INT><SOUNDCONFIG>soundconf.xml</SOUNDCONFIG></document>".to_vec()),
            ("config/soundconf.xml".into(),b"<document><SOUNDGROUP><a><label>loose</label></a></SOUNDGROUP><PART><voice><volume>75</volume></voice></PART></document>".to_vec()),
            ("config.int/soundconf.xml".into(),b"<document><SOUNDGROUP><a><label>packed</label></a></SOUNDGROUP></document>".to_vec()),
        ]),setup:Vec::new()};
        let read=|storage:&Storage|crate::sound_definition::load(storage,&||Ok(())).unwrap();
        let result=read(&storage);assert_eq!(result.found(),1);
        assert_eq!(result.document.as_ref().unwrap().unique_child_path("SOUNDGROUP/a/label").unwrap().unwrap().text,"loose");
        assert_eq!(result.document.as_ref().unwrap().unique_child_path("PART/voice/volume").unwrap().unwrap().text,"75");
        storage.assets.remove("config/soundconf.xml");
        assert_eq!(read(&storage).document.as_ref().unwrap().unique_child_path("SOUNDGROUP/a/label").unwrap().unwrap().text,"packed");
        storage.assets.remove("config.int/soundconf.xml");assert_eq!(read(&storage).found(),0);
        storage.assets.insert("config/soundconf.xml".into(),b"<!DOCTYPE unsupported><document/>".to_vec());
        assert!(crate::sound_definition::load(&storage,&||Ok(())).is_err());
        assert!(crate::sound_definition::load(&storage,&||anyhow::bail!("cancelled")).is_err());
    }
}
