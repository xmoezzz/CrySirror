// SPDX-License-Identifier: MPL-2.0
//! Read-only installed-data inspection. No game files are extracted.
use anyhow::{Context, Result, ensure};
use crysirror_capfs::{ambient_authority, fs::Dir};
use crysirror_core::{service::AssetReader, state::ClientId, vfs::VfsProvider};
use crysirror_naughty_dog::{Strings, levels};
use crysirror_resource::{ArchiveMount, DecodeContext, FormatRegistry, read_all, read_exact_at};
use std::{
    io::{Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
};

struct File(Mutex<crysirror_capfs::fs::File>, u64);
impl AssetReader for File {
    fn len(&self) -> crysirror_core::Result<u64> {
        Ok(self.1)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> crysirror_core::Result<usize> {
        let mut f = self
            .0
            .lock()
            .map_err(|_| crysirror_core::Error::Io("file lock poisoned".into()))?;
        f.seek(SeekFrom::Start(offset))
            .and_then(|_| f.read(out))
            .map_err(|e| crysirror_core::Error::Io(e.to_string()))
    }
}
fn file(dir: &Dir, path: &str) -> Result<Arc<dyn AssetReader>> {
    let f = dir.open(path)?;
    let len = f.metadata()?.len();
    Ok(Arc::new(File(Mutex::new(f), len)))
}
fn write_glb(
    out: &std::path::Path,
    meshes: &[crysirror_naughty_dog::mesh::Mesh],
    package: &dyn AssetReader,
    directory: &crysirror_naughty_dog::package::Directory,
) -> Result<()> {
    use crysirror_asset::interchange::glb::{Document, Material, Node, Primitive};
    let mut doc = Document { generator: "CrySirror Naughty Dog background inspection".into(), ..Default::default() };
    let mut textures = std::collections::HashMap::new();
    for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
        let desc = crysirror_naughty_dog::texture::VramDesc::read(package, item.payload_offset)?;
        textures.insert(desc.id, desc);
    }
    let mut images = std::collections::HashMap::new();
    for (m, mesh) in meshes.iter().enumerate() {
        let mut primitives = Vec::new();
        for group in &mesh.groups {
            let image = match group.material.base_color().and_then(|id| textures.get(&id)) {
                Some(desc) => match images.get(&desc.id) {
                    Some(&i) => Some(i),
                    None => {
                        let png = crysirror_image::encode_png(&desc.decode(package, directory.texture_base)?)?;
                        doc.images.push((png, "image/png"));
                        images.insert(desc.id, doc.images.len() - 1);
                        Some(doc.images.len() - 1)
                    }
                },
                None => None,
            };
            doc.materials.push(Material { name: group.material.name.clone(), base_color_texture: image, ..Default::default() });
            let material = doc.materials.len() - 1;
            for f in &mesh.fragments[group.fragments.clone()] {
                if f.positions.is_empty() {
                    continue;
                }
                primitives.push(Primitive {
                    positions: f.positions.clone(),
                    uvs: if f.uvs.len() == f.positions.len() { f.uvs.clone() } else { Vec::new() },
                    indices: f.triangles.iter().flatten().map(|&i| u32::from(i)).collect(),
                    material: Some(material),
                    ..Default::default()
                });
            }
        }
        doc.meshes.push((format!("prototype-{m}"), primitives));
        let mut node = Node::named(format!("prototype-{m}"));
        node.mesh = Some(m);
        doc.roots.push(doc.nodes.len());
        doc.nodes.push(node);
    }
    std::fs::write(out, doc.to_glb()?)?;
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .context("usage: level_resources GAME_ROOT [LEVEL_NAME]")?;
    let selected = args
        .next()
        .map(|s| {
            s.into_string()
                .map_err(|_| anyhow::anyhow!("level name encoding"))
        })
        .transpose()?
        .unwrap_or_else(|| "con-1-start".into());
    // Optional GLB of the level's background prototype meshes, written
    // where the user asks (never inside the repository).
    let glb_out = args.next().map(std::path::PathBuf::from);
    ensure!(args.next().is_none(), "unexpected argument");
    let disc = Dir::open_ambient_dir(root, ambient_authority())?;
    let dir = disc.open_dir("PS3_GAME/USRDIR/build/main")?;
    let text = read_all(file(&dir, "pak22.txt")?.as_ref(), 4 * 1024 * 1024)?;
    let levels = levels(std::str::from_utf8(&text)?)?;
    let level = levels
        .get(&selected)
        .with_context(|| format!("unknown level {selected}"))?;
    println!(
        "{} levels; {:?} ID {:?}, packages {:?}, {} actors, banks {:?}, caches {:?}",
        levels.len(),
        selected,
        level.id,
        level.packages,
        level.actors.len(),
        level.sound_banks,
        level.caches
    );
    let boot = read_all(file(&dir, "boot1/cont.eng.dds")?.as_ref(), 8 * 1024 * 1024)?;
    let surface = crysirror_image::decode_standard(&boot)?;
    let colours = surface
        .rgba
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect::<std::collections::BTreeSet<_>>();
    println!(
        "boot1/cont.eng.dds: {} x {}, {} distinct RGBA colours",
        surface.width,
        surface.height,
        colours.len()
    );
    let mut registry = FormatRegistry::default();
    crysirror_formats_arc::register(&mut registry)?;
    let mut context = DecodeContext::new(ClientId(1));
    context.limits.max_entry_size = 128 * 1024 * 1024;
    let mut total_files = 0;
    for name in [
        "pak22.psarc",
        "actor27.psarc",
        "dc1.psarc",
        "text1.psarc",
        "sound1/banks.psarc",
        "sound1/music.psarc",
        "sound1/streams.psarc",
        "sound1/speech/english.psarc",
    ] {
        let (_, archive) = registry.open(file(&dir, name)?, Some("sony/psarc"), &context)?;
        let count = archive.entries().len();
        total_files += count;
        let mount = ArchiveMount::new(archive, context.clone())?;
        println!("{name}: {count} resources");
        match name {
            "pak22.psarc" | "sound1/banks.psarc" | "actor27.psarc" => {
                let (resources, ext) = match name {
                    "pak22.psarc" => (&level.packages, "pak"),
                    "actor27.psarc" => (&level.actors, "pak"),
                    _ => (&level.sound_banks, "bnk"),
                };
                for entry in resources {
                    let p = format!("{entry}.{ext}");
                    if !mount.exists(&p) {
                        println!("  no exact archive name for {p}");
                        continue;
                    }
                    let handle = mount.open(&p)?;
                    if ext == "pak" {
                        let directory =
                            crysirror_naughty_dog::package::Directory::read(handle.as_ref())?;
                        let mut types = std::collections::BTreeMap::new();
                        for item in &directory.items {
                            *types.entry(item.kind.as_str()).or_insert(0) += 1;
                        }
                        println!(
                            "  {p}: {} pages, {} items {types:?}",
                            directory.page_count,
                            directory.items.len()
                        );
                        let (mut decoded, mut colours) = (0, std::collections::BTreeSet::new());
                        for item in directory.items.iter().filter(|i| i.kind == "VRAM_DESC") {
                            let desc = crysirror_naughty_dog::texture::VramDesc::read(handle.as_ref(), item.payload_offset)?;
                            let image = desc.decode(handle.as_ref(), directory.texture_base)?;
                            colours.extend(image.rgba.chunks_exact(4).take(4096).map(|c| u32::from_be_bytes(c.try_into().unwrap())));
                            decoded += 1;
                        }
                        println!("  {p}: decoded {decoded} VRAM textures, {} distinct sampled RGBA values", colours.len());
                        let collections: Vec<_> = directory.items.iter().filter(|i| i.kind == "PM_COLLECTION_2").collect();
                        if !collections.is_empty() {
                            let loaded = crysirror_naughty_dog::mesh::Loaded::read(handle.as_ref(), 256 << 20)?;
                            let decoded = collections.iter().try_fold(Vec::new(), |mut all, item| {
                                all.extend(crysirror_naughty_dog::mesh::prototype_meshes(&loaded, item.payload_offset as usize)?);
                                anyhow::Ok(all)
                            });
                            let meshes = match decoded {
                                Ok(m) => m,
                                Err(e) => {
                                    println!("  {p}: prototype meshes not decoded: {e:#}");
                                    continue;
                                }
                            };
                            let (fragments, vertices, triangles) = meshes.iter().flat_map(|m| &m.fragments).fold((0, 0, 0), |a, f| (a.0 + 1, a.1 + f.positions.len(), a.2 + f.triangles.len()));
                            println!("  {p}: {} prototype meshes, {fragments} fragments, {vertices} vertices, {triangles} triangles", meshes.len());
                            if let Some(out) = glb_out.as_ref().filter(|_| ext == "pak" && name == "pak22.psarc") {
                                write_glb(out, &meshes, handle.as_ref(), &directory)?;
                                println!("  wrote {}", out.display());
                            }
                        }
                    }
                    let digest = crysirror_resource::checksum::sha256_reader(
                        handle.as_ref(),
                        context.limits.max_entry_size,
                    )?;
                    let hex = digest
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>();
                    println!("  {p}: {} bytes, SHA-256 {hex}", handle.len()?);
                }
            }
            "text1.psarc" => {
                let mut strings = 0;
                for entry in mount.list()? {
                    let bytes = mount.read(&entry)?;
                    let dict = Strings::parse(&bytes).with_context(|| entry.clone())?;
                    strings += dict.entries().len();
                    if ["2.common", "11.common"].contains(&entry.as_str()) {
                        println!("  {entry}: ID 00049e15 = {:?}", dict.get(0x49e15));
                    }
                }
                println!("  {strings} localized string records");
            }
            _ => {
                let entries = mount.list()?;
                let path = entries.first().context("empty archive")?;
                let handle = mount.open(path)?;
                let mut header = [0; 4];
                read_exact_at(handle.as_ref(), 0, &mut header)?;
                println!("  {path}: magic {:02x?}", header);
            }
        }
    }
    println!("{total_files} mounted files");
    Ok(())
}

#[test]
#[ignore = "requires BCAS20097 01.00 via UNCHARTED2_ROOT"]
fn installed_movie_and_music_audio() -> Result<()> {
    let root = std::env::var_os("UNCHARTED2_ROOT").context("UNCHARTED2_ROOT")?;
    let dir = Dir::open_ambient_dir(root, ambient_authority())?.open_dir("PS3_GAME/USRDIR/build/main")?;
    let mut registry = FormatRegistry::default();
    crysirror_formats_arc::register(&mut registry)?;
    let mut context = DecodeContext::new(ClientId(1));
    context.limits.max_entry_size = 128 << 20;
    let mount = |source: &str| -> Result<ArchiveMount> {
        let (_, arc) = registry.open(file(&dir, source)?, Some("sony/psarc"), &context)?;
        ArchiveMount::new(arc, context.clone())
    };
    let english = mount("sound1/speech/english.psarc")?;
    let streams = mount("sound1/streams.psarc")?;
    let music = mount("sound1/music.psarc")?;
    let check = |archive: &ArchiveMount, name: &str| -> Result<()> {
        let mut audio = crysirror_audio::decoder::open_asset(archive.open(name)?, Some("xvag"))?;
        let mut buffer = [[0.; 2]; 4096];
        let mut frames = 0;
        while frames < 20 * audio.sample_rate() {
            let count = audio.read(&mut buffer)?;
            if buffer[..count].iter().flatten().any(|v| v.abs() > 0.0001) { return Ok(()); }
            if count == 0 { break; }
            frames += count as u32;
        }
        anyhow::bail!("{name} has no signal in {frames} frames")
    };
    let mut movies = 0;
    for item in dir.open_dir("movie1")?.entries()? {
        let filename = item?.file_name();
        let Some(name) = filename.to_str().and_then(|s| s.strip_suffix(".m2v")) else { continue };
        let archive = if name == "sce-ndi-logos" { &streams } else { &english };
        check(archive, &format!("{name}.xvag")).with_context(|| name.to_owned())?;
        movies += 1;
    }
    assert_eq!(movies, 47);
    let tracks = music.list()?;
    assert_eq!(tracks.len(), 446);
    for name in tracks { check(&music, &name).with_context(|| name)?; }
    Ok(())
}
