// SPDX-License-Identifier: MPL-2.0
use anyhow::{ensure, Result};
use crysirror_core::path::{Mount, VfsPath};
use serde::{Deserialize, Serialize};

fn strict() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Relative directory inside this client's asset capability.
    #[serde(default)]
    pub root: String,
    /// Client-relative font asset supplied by the project owner.
    pub font: String,
    #[serde(default)]
    pub font_face_index: u32,
    #[serde(default)]
    pub gbk: bool,
    /// Unknown native operations must fail visibly.
    #[serde(default = "strict")]
    pub strict: bool,
    /// Explicit diagnostic mode that discards original audio commands.
    #[serde(default)]
    pub silent: bool,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        let canonical = |value: &str| -> Result<()> {
            let path = VfsPath::new(Mount::Asset, value)
                .map_err(|e| anyhow::anyhow!("BGI asset path: {e}"))?;
            ensure!(path.relative() == value, "BGI asset path must be canonical");
            Ok(())
        };
        if !self.root.is_empty() {
            canonical(&self.root)?;
        }
        canonical(&self.font)?;
        ensure!(self.strict, "BGI requires strict native operation handling");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_strict_and_paths_are_capability_relative() {
        let value = serde_json::json!({"font":"font.ttf"});
        let config: Config = serde_json::from_value(value.clone()).unwrap();
        config.validate().unwrap();
        assert!(config.strict);
        assert!(!config.silent);
        for path in [
            "/font.ttf",
            "../font.ttf",
            "./font.ttf",
            "a//font.ttf",
            "a\\font.ttf",
        ] {
            let mut config = config.clone();
            config.font = path.into();
            assert!(config.validate().is_err(), "{path}");
        }
        let mut bad = value.clone();
        bad["strict"] = false.into();
        assert!(serde_json::from_value::<Config>(bad)
            .unwrap()
            .validate()
            .is_err());
        let mut bad = value;
        bad["unknown"] = true.into();
        assert!(serde_json::from_value::<Config>(bad).is_err());
    }
}
