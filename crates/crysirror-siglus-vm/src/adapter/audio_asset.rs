// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Native sound directory/extension precedence over shared capability lookup.
use super::*;
impl SiglusVm {
    pub(super) fn audio_asset(
        &self,
        host: &mut dyn HostServices,
        name: &str,
        directory: &str,
    ) -> Result<String> {
        if let Some(alias) = self.state.config.resources.get(name) {
            return Ok(alias.clone());
        }
        ensure!(
            self.state
                .vm
                .as_ref()
                .unwrap()
                .ctx
                .globals
                .append_dir
                .is_empty(),
            "native sound append search needs an explicit resource alias"
        );
        let name = name.replace('\\', "/");
        VfsPath::new(Mount::Asset, &name)?;
        ensure!(
            matches!(directory, "wav" | "bgm"),
            "unsupported native sound resource directory"
        );
        let storage = host.storage(ClientId(self.state.client))?;
        let find = |candidate: &str| -> Result<Option<String>> {
            Ok(
                crysirror_resource::capability_lookup::resolve_ascii_case_insensitive(
                    storage.as_ref(),
                    &VfsPath::new(Mount::Asset, candidate)?,
                )?
                .map(|p| p.relative().to_owned()),
            )
        };
        if name.contains('/') {
            if let Some(path) = find(&name)? {
                return Ok(path);
            }
        }
        let extension = std::path::Path::new(&name).extension();
        if let Some(extension) = extension {
            ensure!(
                matches!(
                    extension
                        .to_str()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str(),
                    "wav" | "nwa" | "ogg" | "owp" | "ovk"
                ),
                "unsupported native sound extension"
            );
            if let Some(path) = find(&format!("{directory}/{name}"))? {
                return Ok(path);
            }
        } else {
            // Fixed pinned Siglus WAV_ORDER; never a codec-availability fallback.
            for extension in ["wav", "nwa", "ogg", "owp", "ovk"] {
                if let Some(path) = find(&format!("{directory}/{name}.{extension}"))? {
                    return Ok(path);
                }
            }
        }
        anyhow::bail!("native sound resource not found: {name}")
    }
}
