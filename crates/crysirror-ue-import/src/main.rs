// SPDX-License-Identifier: MPL-2.0
use anyhow::{Context, Result};
use crysirror_ue_import::iostore::{chunk_kind_name, Container};
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    if cmd == "schemas" {
        let bytes = std::fs::read(
            args.get(2)
                .context("schemas <shipping.exe> <output.usmap>")?,
        )?;
        let recovered = crysirror_ue_import::reflect::Reflection::read(&bytes)?;
        let encoded = recovered.usmap.encode()?;
        std::fs::write(args.get(3).context("output.usmap")?, &encoded)?;
        println!(
            "{}",
            serde_json::json!({"schemas":recovered.usmap.schemas.len(),"enums":recovered.usmap.enums.len(),"unknown_kinds":recovered.unknown_kinds,"bytes":encoded.len()})
        );
        return Ok(());
    }
    if cmd == "slots" {
        let usmap = crysirror_ue_import::usmap::Usmap::load(std::path::Path::new(
            args.get(2).context("usmap")?,
        ))?;
        let class = args.get(3).context("class")?;
        if class == "*" {
            let mut names: Vec<_> = usmap
                .schemas
                .values()
                .map(|s| {
                    format!(
                        "{} : {} ({} props)",
                        s.name,
                        s.super_name.as_deref().unwrap_or("-"),
                        s.props.len()
                    )
                })
                .collect();
            names.sort();
            println!("{} schemas, {} enums", names.len(), usmap.enums.len());
            for n in names {
                println!("{n}");
            }
            return Ok(());
        }
        for (i, s) in usmap.slots(class)?.iter().enumerate() {
            println!(
                "{i:4} {}",
                s.as_ref()
                    .map(|(n, t)| format!("{n} {t:?}"))
                    .unwrap_or_default()
                    .chars()
                    .take(110)
                    .collect::<String>()
            );
        }
        return Ok(());
    }
    if cmd == "tail" {
        // ue-import tail <utoc> <schema> <package suffix> <export name> [bytes]: native data after properties.
        let mut game = crysirror_ue_import::graph::Game::open(std::path::Path::new(&args[2]), std::path::Path::new(&args[3]))?;
        let p = game.package_by_path(&args[4])?;
        let i = p.pkg.exports.iter().position(|e| e.name == args[5]).context("export")?;
        let class = game.native_class(p.id, i)?;
        let data = &p.bytes[p.pkg.export_range(&p.pkg.exports[i])];
        let mut r = crysirror_ue_import::props::Reader { b: data, p: 0, names: &p.pkg.names };
        let v = crysirror_ue_import::props::properties(&mut r, &game.schema, &class)?;
        println!("{class} size {} props end {}\n{}", data.len(), r.p, serde_json::to_string(&v)?.chars().take(1500).collect::<String>());
        if let Some(out) = std::env::var_os("CRY_OUT") {
            std::fs::write(out, &data[r.p..])?;
        }
        let n: usize = args.get(6).and_then(|x| x.parse().ok()).unwrap_or(512);
        for (k, c) in data[r.p..].chunks(32).take(n / 32).enumerate() {
            println!("{:6x}: {}", r.p + k * 32, c.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
        }
        let bulk: Vec<_> = game.container.chunks.iter().filter(|k| k.id == p.id && k.kind != 1).map(|k| (k.kind, k.index)).collect();
        println!("other chunks for package: {bulk:?}");
        return Ok(());
    }
    if cmd == "level" {
        // ue-import level <utoc> <schema(.usmap|.exe)> <map suffix> <out.json>
        let mut game = crysirror_ue_import::graph::Game::open(std::path::Path::new(args.get(2).context("utoc")?), std::path::Path::new(args.get(3).context("schema")?))?;
        let v = crysirror_ue_import::level::extract(&mut game, args.get(4).context("map")?)?;
        std::fs::write(args.get(5).context("out")?, serde_json::to_vec_pretty(&v)?)?;
        println!("{} scene components, {} static meshes, {} lights, {} unresolved, errors {}", v["scene_components"], v["static_meshes"].as_array().map_or(0, |a| a.len()), v["lights"].as_array().map_or(0, |a| a.len()), v["unresolved_meshes"], v["errors"]);
        return Ok(());
    }
    let utoc = PathBuf::from(
        args.get(2)
            .context("usage: ue-import <info|list|extract> <container.utoc> [path] [out]")?,
    );
    let mut c = Container::open(&utoc)?;
    match cmd {
        "info" => {
            let mut kinds = std::collections::BTreeMap::new();
            for k in &c.chunks {
                *kinds.entry(chunk_kind_name(k.kind)).or_insert(0usize) += 1;
            }
            println!(
                "{}",
                serde_json::json!({"version": c.version, "flags": c.flags, "block_size": c.block_size,
                "methods": c.methods, "chunks": c.chunks.len(), "files": c.files.len(), "kinds": kinds})
            );
        }
        "list" => {
            for (p, e) in &c.files {
                println!(
                    "{e}\t{}\t{}\t{p}",
                    chunk_kind_name(c.chunks[*e].kind),
                    c.chunk_size(*e)
                );
            }
        }
        "extract-chunk" => {
            let kind: u8 = args.get(3).context("kind")?.parse()?;
            let e = c
                .chunks
                .iter()
                .position(|k| k.kind == kind)
                .context("no chunk of that kind")?;
            let data = c.read(e)?;
            std::fs::write(args.get(4).context("out")?, &data)?;
            println!("{} bytes", data.len());
        }
        "extract" => {
            let path = args.get(3).context("path")?;
            let out = PathBuf::from(args.get(4).context("out")?);
            let e = *c
                .files
                .get(path)
                .with_context(|| format!("{path} not in container"))?;
            let data = c.read(e)?;
            std::fs::write(&out, &data)?;
            println!("{} bytes -> {}", data.len(), out.display());
        }
        "classes" => {
            // Class histogram over every package, resolved via global.utoc ScriptObjects.
            let mut g = Container::open(&utoc.with_file_name("global.utoc"))?;
            let so = g
                .chunks
                .iter()
                .position(|k| k.kind == 5)
                .context("ScriptObjects chunk")?;
            let script = crysirror_ue_import::zen::parse_script_objects(&g.read(so)?)?;
            let mut hist = std::collections::BTreeMap::<String, usize>::new();
            let (mut ok, mut bad) = (0, 0);
            let pkgs: Vec<(String, usize)> = c
                .files
                .iter()
                .filter(|(p, _)| p.ends_with(".uasset") || p.ends_with(".umap"))
                .map(|(p, e)| (p.clone(), *e))
                .collect();
            for (p, e) in pkgs {
                match c
                    .read(e)
                    .and_then(|b| crysirror_ue_import::zen::parse_package(&b))
                {
                    Ok(pkg) => {
                        ok += 1;
                        for x in &pkg.exports {
                            let cls = if crysirror_ue_import::zen::index_kind(x.class) == 1 {
                                script
                                    .get(&x.class)
                                    .cloned()
                                    .unwrap_or_else(|| format!("script:{:x}", x.class))
                            } else {
                                "<package class>".into()
                            };
                            *hist.entry(cls).or_insert(0) += 1;
                        }
                    }
                    Err(err) => {
                        bad += 1;
                        if bad < 5 {
                            eprintln!("{p}: {err:#}");
                        }
                    }
                }
            }
            println!(
                "{}",
                serde_json::json!({"packages_ok": ok, "packages_failed": bad, "script_objects": script.len(), "classes": hist})
            );
        }
        "probe" => {
            // probe <utoc> <usmap> <Class> [max]: decode every export of an engine
            // class with a candidate schema and report how cleanly it parses.
            let usmap = crysirror_ue_import::usmap::Usmap::load(std::path::Path::new(
                args.get(3).context("usmap")?,
            ))?;
            let class = args.get(4).context("class")?.clone();
            let max: usize = args
                .get(5)
                .and_then(|x| x.parse().ok())
                .unwrap_or(usize::MAX);
            let mut g = Container::open(&utoc.with_file_name("global.utoc"))?;
            let so = g
                .chunks
                .iter()
                .position(|k| k.kind == 5)
                .context("ScriptObjects")?;
            let script = crysirror_ue_import::zen::parse_script_objects(&g.read(so)?)?;
            let want = format!("/Script/Engine.{class}");
            let (mut ok, mut fail, mut exact, mut seen) = (0usize, 0usize, 0usize, 0usize);
            let mut errors = std::collections::BTreeMap::<String, usize>::new();
            let mut sample = None;
            let pkgs: Vec<usize> = c
                .files
                .iter()
                .filter(|(p, _)| p.ends_with(".umap") || p.ends_with(".uasset"))
                .map(|(_, e)| *e)
                .collect();
            'outer: for e in pkgs {
                let bytes = c.read(e)?;
                let pkg = crysirror_ue_import::zen::parse_package(&bytes)?;
                for x in &pkg.exports {
                    if script.get(&x.class).map(String::as_str) != Some(want.as_str()) {
                        continue;
                    }
                    seen += 1;
                    if seen > max {
                        break 'outer;
                    }
                    let range = pkg.export_range(x);
                    if std::env::var_os("CRY_DUMP").is_some() && seen <= 6 {
                        let d = &bytes[range.clone()];
                        eprintln!(
                            "{} {} flags {:#x} size {}: {:02x?}",
                            pkg.name,
                            x.name,
                            x.flags,
                            d.len(),
                            &d[..d.len().min(48)]
                        );
                    }
                    let mut r = crysirror_ue_import::props::Reader {
                        b: &bytes[range.clone()],
                        p: 0,
                        names: &pkg.names,
                    };
                    match crysirror_ue_import::props::properties(&mut r, &usmap, &class) {
                        Ok(v) => {
                            ok += 1;
                            // UObject tail: bHasGuid (u32 0) for non-templates.
                            if r.b.len() - r.p >= 4 && r.b[r.p..r.p + 4] == [0, 0, 0, 0] {
                                exact += 1;
                            }
                            if sample.is_none()
                                && v.get("RelativeLocation").is_some_and(|l| !l.is_null())
                            {
                                sample = Some(v);
                            }
                        }
                        Err(err) => {
                            fail += 1;
                            *errors
                                .entry(format!("{err:#}").chars().take(140).collect())
                                .or_insert(0) += 1;
                        }
                    }
                }
            }
            let mut top: Vec<_> = errors.into_iter().collect();
            top.sort_by(|a, b| b.1.cmp(&a.1));
            top.truncate(6);
            println!(
                "{}",
                serde_json::json!({"class": class, "exports": seen.min(max), "ok": ok, "fail": fail, "guid_tail_ok": exact, "top_errors": top, "sample": sample})
            );
        }
        _ => anyhow::bail!("unknown command {cmd}"),
    }
    Ok(())
}
