// Copyright (c) 2026 CrySirror contributors. MIT; see LICENSE/SOURCE.json.
//! Native ENV/current-setting policy over an explicit portable client save file.
//! This is not a Windows registry importer or an original game-save codec.
use anyhow::{ensure, Context, Result};
use crysirror_core::{
    path::{Mount, VfsPath},
    service::ClientStorage,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub(crate) const SETTINGS_FILE: &str = "majiro-portable-settings-v1.json";
const MAX_SETTINGS_BYTES: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MajiroSettingsProfile {
    /// Client Save JSON retains native named REG_SZ values. Missing values
    /// follow reviewed native ENV defaults; no original registry is imported.
    #[default]
    PortableClient,
    /// Importing an original Windows registry is not implemented.
    StrictWindowsRegistry,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PortableRegistry {
    schema: u32,
    kind: String,
    reg_global: String,
    reg_local: String,
    #[serde(deserialize_with = "unique_values")]
    values: BTreeMap<String, String>,
}
fn unique_values<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    struct Values;
    impl<'de> serde::de::Visitor<'de> for Values {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("bounded unique native REG_SZ values")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut a: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((key, value)) = a.next_entry::<String, String>()? {
                if out.len() >= 4096
                    || key.len() > 256
                    || value.len() > 511
                    || out.insert(key, value).is_some()
                {
                    return Err(serde::de::Error::custom(
                        "duplicate or excessive native settings value",
                    ));
                }
            }
            Ok(out)
        }
    }
    d.deserialize_map(Values)
}
#[derive(Clone, Debug)]
pub(crate) struct NativeSettings {
    registry: PortableRegistry,
    pub effect_speed: i32,
    pub moji_speed: i32,
    pub auto_speed: i32,
    pub loaded_portable_file: bool,
    pub commits: u64,
}
fn numeric(text: &str) -> Result<i32> {
    // Portable profile is canonical signed decimal text written by our setter.
    // Unsupported imported/edited native atoi suffix/overflow profiles fail.
    text.trim()
        .parse()
        .context("unsupported native registry integer text")
}
impl NativeSettings {
    pub fn load(env: &[u8], storage: &dyn ClientStorage) -> Result<Self> {
        let env = crate::environment::parse(env)?;
        let global = env
            .get("REG_GLOBAL")
            .context("Majiro ENV REG_GLOBAL required for settings scope")?
            .clone();
        let local = env
            .get("REG_LOCAL")
            .context("Majiro ENV REG_LOCAL required for settings scope")?
            .clone();
        ensure!(
            global.len() <= 512 && local.len() <= 512,
            "Majiro native settings scope budget"
        );
        let path = VfsPath::new(Mount::Save, SETTINGS_FILE)?;
        let loaded = if let Some(metadata) = storage.metadata(Mount::Save, SETTINGS_FILE)? {
            ensure!(
                !metadata.is_directory
                    && metadata
                        .size
                        .is_some_and(|n| n <= MAX_SETTINGS_BYTES as u64),
                "Majiro portable settings file bounds"
            );
            let reader = storage.open_read(&path)?;
            Some(crysirror_resource::read_all(
                reader.as_ref(),
                MAX_SETTINGS_BYTES as u64,
            )?)
        } else {
            None
        };
        Self::from_bytes(&env, &global, &local, loaded.as_deref())
    }
    fn from_bytes(
        env: &BTreeMap<String, String>,
        global: &str,
        local: &str,
        bytes: Option<&[u8]>,
    ) -> Result<Self> {
        let mut registry = if let Some(bytes) = bytes {
            ensure!(
                bytes.len() <= MAX_SETTINGS_BYTES,
                "Majiro settings source budget"
            );
            let r: PortableRegistry = serde_json::from_slice(bytes)?;
            ensure!(
                r.schema == 1
                    && r.kind == "crysirror-majiro-portable-settings"
                    && r.reg_global == global
                    && r.reg_local == local,
                "unsupported or foreign Majiro portable settings profile"
            );
            r
        } else {
            PortableRegistry {
                schema: 1,
                kind: "crysirror-majiro-portable-settings".into(),
                reg_global: global.into(),
                reg_local: local.into(),
                values: BTreeMap::new(),
            }
        };
        let mut canonical = BTreeMap::new();
        for (key, value) in registry.values {
            ensure!(
                key.is_ascii()
                    && !key.is_empty()
                    && !key.chars().any(char::is_control)
                    && !value.chars().any(|c| c == '\0'),
                "unsupported native registry name/value"
            );
            ensure!(
                canonical.insert(key.to_ascii_lowercase(), value).is_none(),
                "case-colliding native registry values"
            );
        }
        registry.values = canonical;
        let read = |name: &str, env_name: &str| -> Result<i32> {
            let default = env
                .get(env_name)
                .map(|s| numeric(s))
                .transpose()?
                .unwrap_or(1000);
            registry
                .values
                .get(name)
                .map(|s| numeric(s))
                .unwrap_or(Ok(default))
        };
        let effect_speed = read("effectspeed", "EFFECTSPEED")?;
        let moji_speed = read("mojispeed", "MOJISPEED")?;
        let auto_speed = read("autospeed", "AUTOSPEED")?;
        Ok(Self {
            registry,
            effect_speed,
            moji_speed,
            auto_speed,
            loaded_portable_file: bytes.is_some(),
            commits: 0,
        })
    }
    pub fn registry_integer(&self, name: &str, default: i32) -> Result<i32> {
        validate_name(name)?;
        self.registry
            .values
            .get(&name.to_ascii_lowercase())
            .map(|s| numeric(s))
            .unwrap_or(Ok(default))
    }
    pub fn candidate_integer(&self, name: &str, value: i32, current: bool) -> Result<Self> {
        validate_name(name)?;
        let mut candidate = self.clone();
        ensure!(
            candidate.registry.values.len() < 4096
                || candidate
                    .registry
                    .values
                    .contains_key(&name.to_ascii_lowercase()),
            "Majiro settings entry budget"
        );
        candidate
            .registry
            .values
            .insert(name.to_ascii_lowercase(), value.to_string());
        if current {
            match name {
                "EffectSpeed" => candidate.effect_speed = value,
                "MojiSpeed" => candidate.moji_speed = value,
                "AutoSpeed" => candidate.auto_speed = value,
                _ => anyhow::bail!("unknown native current-setting field"),
            }
        }
        candidate.commits = candidate
            .commits
            .checked_add(1)
            .context("Majiro settings commit counter overflow")?;
        Ok(candidate)
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        let out = serde_json::to_vec(&self.registry)?;
        ensure!(
            out.len() <= MAX_SETTINGS_BYTES,
            "Majiro settings output budget"
        );
        Ok(out)
    }
    pub fn entries(&self) -> usize {
        self.registry.values.len()
    }
}
fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 256
            && name.is_ascii()
            && !name.chars().any(char::is_control),
        "unsupported native registry value name"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_env_defaults_and_published_portable_overrides_are_distinct() {
        let env = BTreeMap::from([
            ("EFFECTSPEED".into(), "725".into()),
            ("MOJISPEED".into(), "900".into()),
            ("AUTOSPEED".into(), "1250".into()),
        ]);
        let base = NativeSettings::from_bytes(&env, "maker", "game", None).unwrap();
        assert_eq!(base.effect_speed, 725);
        assert!(!base.loaded_portable_file);
        let generic = base.candidate_integer("EffectSpeed", 321, false).unwrap();
        assert_eq!(generic.effect_speed, 725);
        assert_eq!(generic.registry_integer("effectspeed", 7).unwrap(), 321);
        let restarted =
            NativeSettings::from_bytes(&env, "maker", "game", Some(&generic.bytes().unwrap()))
                .unwrap();
        assert_eq!(restarted.effect_speed, 321);
        assert!(restarted.loaded_portable_file);
        let current = base.candidate_integer("EffectSpeed", -20, true).unwrap();
        assert_eq!(current.effect_speed, -20);
        assert_eq!(base.effect_speed, 725);
        assert!(
            NativeSettings::from_bytes(&env, "other", "game", Some(&generic.bytes().unwrap()))
                .is_err()
        );
        let mut corrupt = generic.bytes().unwrap();
        corrupt[0] = b'x';
        assert!(NativeSettings::from_bytes(&env, "maker", "game", Some(&corrupt)).is_err());
        let duplicate=b"{\"schema\":1,\"kind\":\"crysirror-majiro-portable-settings\",\"reg_global\":\"maker\",\"reg_local\":\"game\",\"values\":{\"x\":\"1\",\"x\":\"2\"}}";
        assert!(NativeSettings::from_bytes(&env, "maker", "game", Some(duplicate)).is_err());
    }
}
