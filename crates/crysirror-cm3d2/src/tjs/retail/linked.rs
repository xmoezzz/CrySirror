// SPDX-License-Identifier: MPL-2.0
//! COM3D2 1.0.4's separate FileSystemOld namespace.
use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct Mounts {
    archives: Vec<String>,
    paths_dat: String,
    plugin: String,
}

impl State {
    pub(crate) fn attach_linked(&mut self, mounts: &Mounts, primary: &[String]) -> Result<()> {
        ensure!(
            !mounts.archives.is_empty() && mounts.archives.len() <= 256,
            "linked CM3D2 archive list"
        );
        let mut seen = BTreeSet::new();
        for path in &mounts.archives {
            VfsPath::new(Mount::Asset, path)?;
            ensure!(
                !primary.iter().any(|other| path.eq_ignore_ascii_case(other))
                    && seen.insert(path.to_ascii_lowercase()),
                "linked CM3D2 requires separate archive mounts"
            );
        }
        let source = self.source()?;
        let path = VfsPath::new(Mount::Asset, &mounts.paths_dat)?;
        let data = source.storage.read(&path)?;
        let mut r = crate::binio::Reader::new(&data);
        r.expect_str("CM3D2_PATHS")?;
        let _version = r.i32()?;
        let n = r.count()?;
        ensure!(n <= 4096, "linked CM3D2 path list limit");
        let suffixes = (0..n)
            .map(|_| {
                let suffix = r.str()?;
                ensure!(
                    !suffix.contains(['/', '\\', '\0']),
                    "invalid linked CM3D2 archive suffix"
                );
                Ok(suffix)
            })
            .collect::<Result<Vec<_>>>()?;
        let plugin = VfsPath::new(Mount::Asset, &mounts.plugin)?;
        let key = NativeKeys::from_dll(&source.storage.read(&plugin)?)?.table_key;
        let files = GameFiles::index(source.storage.as_ref(), &mounts.archives)?;
        ensure!(
            files.exists("yotogi_stage_setting.nei"),
            "missing linked CM3D2 stage settings"
        );
        self.legacy_files = files;
        self.legacy_key = Some(key);
        self.legacy_suffixes = suffixes;
        Ok(())
    }

    pub(crate) fn legacy_table(&self, name: &str) -> Result<Table> {
        let storage = &self.source()?.storage;
        let key = self
            .legacy_key
            .as_ref()
            .context("linked CM3D2 archives are not mounted")?;
        let bytes = self.legacy_files.read(storage.as_ref(), name)?;
        Table::open(&bytes, key).with_context(|| format!("linked CM3D2 table {name}"))
    }

    pub(crate) fn linked_files(&self) -> &GameFiles {
        &self.legacy_files
    }

    pub(super) fn legacy_ids(&self, base: &str) -> Result<BTreeSet<i32>> {
        let mut ids = BTreeSet::new();
        let names = std::iter::once(format!("{base}.nei")).chain(
            self.legacy_suffixes.iter().map(|suffix| format!("{base}_{suffix}.nei"))
        );
        for name in names.filter(|name| self.legacy_files.exists(name)) {
            let table = self.legacy_table(&name)?;
            ids.extend((1..table.rows).filter(|&r| table.exists(0, r)).map(|r| table.int(0, r)));
        }
        Ok(ids)
    }
}

pub(crate) struct Stage {
    pub index: usize,
    pub sort: i32,
    pub key: String,
    pub name: String,
    pub backdrop: String,
    pub thumbnail: String,
    pub music: String,
    pub camera: String,
    pub enabled: bool,
}

pub(crate) fn stages(s: &TjsState) -> Result<Vec<Stage>> {
    require_transferred_character(s)?;
    let state = &s.retail;
    let names = std::iter::once("yotogi_stage_list.nei".to_owned()).chain(
        state
            .legacy_suffixes
            .iter()
            .map(|suffix| format!("yotogi_stage_list_{suffix}.nei")),
    );
    let mut allowed = BTreeMap::new();
    for name in names.filter(|name| state.legacy_files.exists(name)) {
        let table = state.legacy_table(&name)?;
        for row in 1..table.rows {
            if table.exists(0, row) {
                allowed
                    .entry(table.string(0, row))
                    .or_insert_with(|| table.string(1, row));
            }
        }
    }
    let table = state.legacy_table("yotogi_stage_setting.nei")?;
    let mut rows = Vec::new();
    for row in 1..table.rows {
        let key = table.string(3, row);
        let Some(backdrop) = allowed.get(&key) else {
            continue;
        };
        let grade = table.string(0, row);
        let index = crate::yotogi_data::STAGES
            .iter()
            .copied()
            .chain([
                "地下室",
                "ロッカールーム",
                "トイレ",
                "電車",
                "教室",
                "ハネムーンルーム",
                "南国",
                "和風旅館",
                "コスプレ会場",
                "プライベートルーム",
                "海",
                "畑",
                "ヴィラ",
                "ベットルーム",
                "露天風呂",
            ])
            .position(|name| name == key)
            .with_context(|| format!("unknown linked stage {key}"))?;
        rows.push(Stage {
            index,
            sort: table.int(1, row),
            key,
            name: table.string(2, row),
            backdrop: backdrop.clone(),
            thumbnail: table.string(4, row),
            music: format!("{}.ogg", table.string(5, row)),
            camera: table.string(6, row),
            // GetPlayableStageList(99, null) accepts symbolic requirements.
            enabled: grade
                .trim()
                .parse::<i32>()
                .map_or(true, |grade| grade <= 99),
        });
    }
    rows.sort_by_key(|stage| (stage.sort, stage.index));
    Ok(rows)
}
