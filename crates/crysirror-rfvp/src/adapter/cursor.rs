// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! RFVP slot policy over shared cursor decoding and UI resources. Reference:
//! rfvp 062fec82033b554b9eb3b5da069518c0c96a354e app.rs setup/update_cursor.
use super::*;

fn slot(name: &str) -> Option<u32> {
    let (stem, extension) = name.rsplit_once('.')?;
    if !extension.eq_ignore_ascii_case("ani") {
        return None;
    }
    let prefix = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    if prefix.is_empty()
        || prefix.len() == stem.len()
        || !prefix.bytes().all(|c| c.is_ascii_alphabetic() || c == b'_')
    {
        return None;
    }
    stem[prefix.len()..]
        .parse()
        .ok()
        .filter(|&index| index > 0 && index < 256)
}

impl RfvpVm {
    pub(super) fn load_cursors(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let storage = self
            .state
            .storage()
            .map_err(|e| anyhow::anyhow!("RFVP cursor storage: {e:?}"))?
            .clone();
        let names = storage.list(Mount::Asset, &self.state.root)?;
        let mut ops = Vec::new();
        for name in names {
            let Some(id) = slot(&name) else {
                continue;
            };
            ensure!(
                !self.cursor_slots.contains_key(&id),
                "ambiguous RFVP ANI cursor slot {id}"
            );
            let path = self
                .state
                .asset(&name)
                .map_err(|e| anyhow::anyhow!("cursor path: {e:?}"))?;
            let reader = storage.open_asset(&path)?;
            let size = reader.len()?;
            ensure!(size <= 16 * 1024 * 1024, "RFVP ANI cursor resource budget");
            let mut bytes = vec![0; size as usize];
            let mut offset = 0;
            while offset < bytes.len() {
                let n = reader.read_at(offset as u64, &mut bytes[offset..])?;
                ensure!(
                    n > 0 && n <= bytes.len() - offset,
                    "RFVP cursor resource changed or truncated"
                );
                offset += n;
            }
            let cursor = crysirror_image::cursor::decode_cursor(&bytes)
                .with_context(|| format!("RFVP ANI cursor {name}"))?;
            let frames: Vec<_> = cursor
                .frames
                .into_iter()
                .map(|frame| {
                    serde_json::json!({
                        "width":frame.image.width,"height":frame.image.height,
                        "hotspot":frame.hotspot,"rgba":frame.image.rgba
                    })
                })
                .collect();
            let steps: Vec<_> = cursor
                .steps
                .into_iter()
                .map(|(index, ms)| [index as u32, ms])
                .collect();
            ops.push(serde_json::json!({"op":"define_cursor","id":id,"cursor":{"frames":frames,"steps":steps}}));
            self.cursor_slots.insert(id, name);
        }
        if !ops.is_empty() {
            host.submit_engine(
                self.state.client,
                1,
                &serde_json::to_vec(&serde_json::json!({"ui":ops}))?,
            )?;
        }
        self.core
            .initialize_cursor(self.cursor_slots.contains_key(&1));
        Ok(())
    }

    pub(super) fn publish_cursor(&mut self, host: &mut dyn HostServices) -> Result<()> {
        let cursor = self.core.take_cursor_state();
        let mut ops = Vec::new();
        if self.cursor_applied != Some(cursor.index) {
            // Slot zero explicitly requests the platform default. An absent ANI
            // slot uses the original desktop's standard-arrow fallback.
            let select = if self.cursor_slots.contains_key(&cursor.index) {
                serde_json::json!({"kind":"custom","id":cursor.index})
            } else {
                serde_json::json!({"kind":"standard","shape":"default"})
            };
            ops.push(serde_json::json!({"op":"set_cursor","cursor":select}));
        }
        if let Some(visible) = cursor.visible {
            ops.push(serde_json::json!({"op":"set_cursor_visible","visible":visible}));
        }
        if let Some(position) = cursor.position {
            ops.push(serde_json::json!({"op":"warp_pointer","position":position}));
        }
        if !ops.is_empty() {
            host.submit_engine(
                self.state.client,
                1,
                &serde_json::to_vec(&serde_json::json!({"ui":ops}))?,
            )?;
        }
        self.cursor_applied = Some(cursor.index);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored_ani() -> Vec<u8> {
        let mut dib = Vec::new();
        for v in [40u32, 2, 4] {
            dib.extend(v.to_le_bytes());
        }
        dib.extend(1u16.to_le_bytes());
        dib.extend(1u16.to_le_bytes());
        dib.extend([0u8; 24]);
        dib.extend([0, 0, 0, 0, 255, 255, 255, 0]);
        dib.extend([128, 0, 0, 0, 192, 0, 0, 0]);
        dib.extend([0, 0, 0, 0, 128, 0, 0, 0]);
        let mut cur = vec![0, 0, 2, 0, 1, 0, 2, 2, 0, 0, 1, 0, 0, 0];
        cur.extend((dib.len() as u32).to_le_bytes());
        cur.extend(22u32.to_le_bytes());
        cur.extend(dib);
        let mut frames = b"fram".to_vec();
        for _ in 0..2 {
            frames.extend(b"icon");
            frames.extend((cur.len() as u32).to_le_bytes());
            frames.extend(&cur);
        }
        let header: Vec<_> = [36u32, 2, 2, 0, 0, 0, 0, 6, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let mut body = b"ACON".to_vec();
        for (name, bytes) in [(b"anih", header), (b"LIST", frames)] {
            body.extend(name);
            body.extend((bytes.len() as u32).to_le_bytes());
            body.extend(bytes);
        }
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((body.len() as u32).to_le_bytes());
        bytes.extend(body);
        bytes
    }
    #[test]
    fn native_named_cursor_slots_are_bounded_and_default_is_not_a_resource() {
        assert_eq!(slot("cursor1.ani"), Some(1));
        assert_eq!(slot("mouse_003.ANI"), Some(3));
        for name in [
            "cursor0.ani",
            "1.ani",
            "x-2.ani",
            "cursor256.ani",
            "cursor.ani",
            "cursor1.png",
        ] {
            assert_eq!(slot(name), None, "{name}");
        }
    }

    #[test]
    fn installed_native_cursor_defines_shared_animated_resource_and_preserves_explicit_default() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("assets")).unwrap();
        std::fs::write(root.path().join("assets/cursor1.ani"), authored_ani()).unwrap();
        let project = root.path().join("project.json");
        let data = serde_json::json!({"schema":2,"name":"cursor","content_revision":"1","assets":"assets",
            "canvas":[64,32],"clients":[{"name":"fixture","game":"cursor","adapter":"crysirror-script-v1"}]});
        std::fs::write(&project, serde_json::to_vec(&data).unwrap()).unwrap();
        let mut rt = crysirror_runtime::Runtime::with_registry(
            crysirror_runtime::Project::load(&project).unwrap(),
            &project,
            &root.path().join("state"),
            AdapterRegistry::default(),
        )
        .unwrap();
        let mut vm = RfvpVm::new(
            ClientId(1),
            Config {
                root: String::new(),
                nls: Nls::Sjis,
                default_font: "unused.ttf".into(),
                screen: Some([64, 32]),
            },
        )
        .unwrap();
        vm.state.storage = Some(rt.host.storage(ClientId(1)).unwrap());
        vm.load_cursors(&mut rt.host).unwrap();
        vm.publish_cursor(&mut rt.host).unwrap();
        let ui = &rt.host.engine.ui;
        let cursor = ui.cursors.get(1, 1).unwrap();
        assert_eq!(cursor.frames[0].hotspot, [1, 0]);
        assert_eq!(cursor.steps, [[0, 100], [1, 100]]);
        assert_eq!(
            serde_json::to_value(ui.cursors.selected).unwrap(),
            serde_json::json!([1,{"kind":"custom","id":1}])
        );
        vm.core.initialize_cursor(false);
        vm.publish_cursor(&mut rt.host).unwrap();
        assert_eq!(
            serde_json::to_value(rt.host.engine.ui.cursors.selected).unwrap(),
            serde_json::json!([1,{"kind":"standard","shape":"default"}])
        );
        assert!(
            rt.host.engine.ui.cursors.get(1, 1).is_some(),
            "selecting the OS default retains reusable resources"
        );
    }
}
