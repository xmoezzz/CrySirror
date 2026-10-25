// SPDX-License-Identifier: MPL-2.0
//! Client-owned string catalogs and font-alias selection. No device/font bytes.
use crate::{
    object::{Class, ClassInfo, MethodInfo, ObjectId, PropertyInfo, TypeHint as T},
    EngineContext, Value,
};
use anyhow::{bail, ensure, Context, Result};
use intl_pluralrules::{PluralCategory, PluralRuleType, PluralRules};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};
use unic_langid::LanguageIdentifier;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Text(String),
    Plural(PluralMessage),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluralMessage {
    pub forms: BTreeMap<String, String>,
    #[serde(default)]
    pub ordinal: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub strings: BTreeMap<String, Message>,
    #[serde(default)]
    pub fonts: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Localizer {
    pub locale: String,
    pub fallback_locale: String,
    pub catalogs: BTreeMap<String, Catalog>,
}
impl Default for Localizer {
    fn default() -> Self {
        Self {
            locale: "en".into(),
            fallback_locale: "en".into(),
            catalogs: BTreeMap::new(),
        }
    }
}
fn tag(locale: &str) -> Result<String> {
    ensure!(locale.len() <= 64, "locale tag length");
    Ok(locale
        .parse::<LanguageIdentifier>()
        .map_err(|_| anyhow::anyhow!("invalid BCP-47 language identifier"))?
        .to_string())
}
fn candidates(locale: &str) -> Result<Vec<String>> {
    let id: LanguageIdentifier = locale
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid locale"))?;
    let mut values = vec![id.to_string()];
    let mut simple = id.clone();
    simple.clear_variants();
    if simple.to_string() != values[0] {
        values.push(simple.to_string());
    }
    if simple.region.take().is_some() {
        values.push(simple.to_string());
    }
    if simple.script.take().is_some() {
        values.push(simple.to_string());
    }
    Ok(values)
}
fn category(locale: &str, number: &str, ordinal: bool) -> Result<&'static str> {
    for candidate in candidates(locale)? {
        if let Ok(rules) = PluralRules::create(
            candidate.parse::<LanguageIdentifier>().unwrap(),
            if ordinal {
                PluralRuleType::ORDINAL
            } else {
                PluralRuleType::CARDINAL
            },
        ) {
            return Ok(match rules.select(number).map_err(anyhow::Error::msg)? {
                PluralCategory::ZERO => "zero",
                PluralCategory::ONE => "one",
                PluralCategory::TWO => "two",
                PluralCategory::FEW => "few",
                PluralCategory::MANY => "many",
                PluralCategory::OTHER => "other",
            });
        }
    }
    bail!("no plural rules for locale {locale}")
}
impl Catalog {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.strings.len() <= 65536 && self.fonts.len() <= 16,
            "catalog entry/font limit"
        );
        let mut bytes = 0usize;
        for (key, value) in &self.strings {
            ensure!(
                !key.is_empty() && key.len() <= 256 && !key.chars().any(char::is_control),
                "localization key"
            );
            bytes += key.len();
            match value {
                Message::Text(text) => bytes += text.len(),
                Message::Plural(message) => {
                    ensure!(
                        message.forms.contains_key("other") && message.forms.len() <= 6,
                        "plural needs other form"
                    );
                    for (form, text) in &message.forms {
                        ensure!(
                            ["zero", "one", "two", "few", "many", "other"].contains(&form.as_str()),
                            "unknown plural category"
                        );
                        bytes += text.len();
                    }
                }
            }
            ensure!(bytes <= 4 * 1024 * 1024, "catalog byte limit");
        }
        for font in &self.fonts {
            ensure!(
                !font.is_empty() && font.len() <= 256 && !font.chars().any(char::is_control),
                "font alias"
            );
        }
        Ok(())
    }
}
impl Localizer {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            tag(&self.locale)? == self.locale
                && tag(&self.fallback_locale)? == self.fallback_locale,
            "non-canonical locale"
        );
        ensure!(self.catalogs.len() <= 64, "catalog locale limit");
        for (locale, catalog) in &self.catalogs {
            ensure!(tag(locale)? == *locale, "non-canonical catalog locale");
            catalog.validate()?;
        }
        self.fonts()?;
        Ok(())
    }
    pub fn define(&mut self, locale: &str, catalog: Catalog) -> Result<()> {
        let locale = tag(locale)?;
        catalog.validate()?;
        ensure!(
            self.catalogs.contains_key(&locale) || self.catalogs.len() < 64,
            "catalog locale limit"
        );
        let mut staged = self.clone();
        staged.catalogs.insert(locale, catalog);
        staged.fonts()?;
        *self = staged;
        Ok(())
    }
    pub fn set_locale(&mut self, locale: &str) -> Result<()> {
        let mut staged = self.clone();
        staged.locale = tag(locale)?;
        staged.fonts()?;
        *self = staged;
        Ok(())
    }
    pub fn set_fallback_locale(&mut self, locale: &str) -> Result<()> {
        let mut staged = self.clone();
        staged.fallback_locale = tag(locale)?;
        staged.fonts()?;
        *self = staged;
        Ok(())
    }
    fn order(&self) -> Result<Vec<String>> {
        let mut values = candidates(&self.locale)?;
        for value in candidates(&self.fallback_locale)? {
            if !values.contains(&value) {
                values.push(value);
            }
        }
        Ok(values)
    }
    /// Alias order only. Caller resolves installed fonts via the text service.
    pub fn fonts(&self) -> Result<Vec<String>> {
        let mut fonts = Vec::new();
        for locale in self.order()? {
            if let Some(catalog) = self.catalogs.get(&locale) {
                for font in &catalog.fonts {
                    if !fonts.contains(font) {
                        fonts.push(font.clone());
                    }
                }
            }
        }
        ensure!(
            fonts.len() <= 16,
            "resolved font fallback exceeds text's 16-alias limit"
        );
        Ok(fonts)
    }
    pub fn translate(&self, key: &str, params: &BTreeMap<String, String>) -> Result<String> {
        for locale in self.order()? {
            if let Some(message) = self
                .catalogs
                .get(&locale)
                .and_then(|catalog| catalog.strings.get(key))
            {
                let template = match message {
                    Message::Text(text) => text,
                    Message::Plural(message) => {
                        let count = params
                            .get("count")
                            .context("plural requires count parameter")?;
                        ensure!(
                            count.len() <= 128
                                && count.parse::<f64>().is_ok_and(|n| n.is_finite() && n >= 0.),
                            "invalid plural count"
                        );
                        let form = category(&locale, count, message.ordinal)?;
                        message
                            .forms
                            .get(form)
                            .or_else(|| message.forms.get("other"))
                            .context("missing other plural form")?
                    }
                };
                return interpolate(template, params);
            }
        }
        bail!("missing localization key {key:?} for {}", self.locale)
    }
}
fn interpolate(template: &str, params: &BTreeMap<String, String>) -> Result<String> {
    ensure!(
        params.len() <= 128 && params.values().all(|v| v.len() <= 65536),
        "localization parameter limits"
    );
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    ensure!(
                        next.is_ascii_alphanumeric() || next == '_',
                        "invalid message placeholder"
                    );
                    name.push(next);
                    ensure!(name.len() <= 128, "message placeholder limit");
                }
                ensure!(
                    closed && !name.is_empty(),
                    "unclosed/empty message placeholder"
                );
                out.push_str(
                    params
                        .get(&name)
                        .with_context(|| format!("missing localization parameter {name}"))?,
                );
            }
            '}' => bail!("unescaped closing brace"),
            _ => out.push(ch),
        }
        ensure!(out.len() <= 1024 * 1024, "localized result exceeds 1 MiB");
    }
    Ok(out)
}
#[derive(Default)]
pub struct LocalizationRuntime {
    clients: BTreeMap<u32, Localizer>,
}
impl LocalizationRuntime {
    pub fn client(&mut self, client: u32) -> &mut Localizer {
        self.clients.entry(client).or_default()
    }
}
pub struct LocalizationClass;
fn info() -> &'static ClassInfo {
    static INFO: OnceLock<ClassInfo> = OnceLock::new();
    INFO.get_or_init(||{
        fn method(name:&'static str,params:Vec<(&'static str,T)>,returns:T,doc:&'static str)->MethodInfo {MethodInfo{name,params,returns,doc}}
        ClassInfo{name:"Localization",doc:"Client-scoped string catalogs, CLDR plurals and locale font aliases; no filesystem or GPU ownership.",properties:vec![
            PropertyInfo{name:"locale",ty:T::String,writable:false,doc:"Current canonical language identifier."},
            PropertyInfo{name:"fonts",ty:T::List(Box::new(T::String)),writable:false,doc:"Resolved fallback font aliases for current and fallback locale."},
        ],methods:vec![
            method("define",vec![("locale",T::String),("catalog",T::Map)],T::Void,"Define {strings:{key:string|{forms:{one?,other,...},ordinal?}},fonts?:string[]}."),
            method("setLocale",vec![("locale",T::String)],T::Void,"Select locale, falling back by variant/region/script/language then fallback locale."),
            method("setFallbackLocale",vec![("locale",T::String)],T::Void,"Choose final fallback language."),
            method("translate",vec![("key",T::String),("params",T::Map)],T::String,"Substitute {name}; plural messages select by count. Decimal strings preserve visible fraction digits."),
            method("profile",vec![],T::Map,"Export locale/catalog configuration for explicit persistence."),
            method("applyProfile",vec![("profile",T::Map)],T::Void,"Validate and atomically restore exported configuration."),
        ]}
    })
}
impl Class for LocalizationClass {
    fn info(&self) -> &ClassInfo {
        info()
    }
    fn get(&self, cx: &mut EngineContext, _: ObjectId, property: &str) -> Result<Value> {
        let state = cx.state()?;
        let localizer = state.localization.client(state.client.0);
        Ok(match property {
            "locale" => Value::Str(localizer.locale.clone()),
            "fonts" => Value::List(localizer.fonts()?.into_iter().map(Value::Str).collect()),
            _ => bail!("unknown Localization property"),
        })
    }
    fn call(
        &self,
        cx: &mut EngineContext,
        _: ObjectId,
        method: &str,
        args: &[Value],
    ) -> Result<Value> {
        let arg = |i: usize| args.get(i).context("missing localization argument");
        let state = cx.state()?;
        let localizer = state.localization.client(state.client.0);
        match method {
            "define" => localizer.define(
                arg(0)?.as_str()?,
                serde_json::from_value(arg(1)?.to_json())?,
            )?,
            "setLocale" => localizer.set_locale(arg(0)?.as_str()?)?,
            "setFallbackLocale" => localizer.set_fallback_locale(arg(0)?.as_str()?)?,
            "translate" => {
                let mut params = BTreeMap::new();
                if let Some(value) = args.get(1) {
                    let Value::Map(values) = value else {
                        bail!("localization params must be a map")
                    };
                    for (key, value) in values {
                        let text = match value {
                            Value::Str(s) => s.clone(),
                            Value::Int(n) => n.to_string(),
                            Value::Float(n) if n.is_finite() => serde_json::to_string(n)?,
                            Value::Bool(b) => b.to_string(),
                            _ => bail!("localization params must be scalar"),
                        };
                        params.insert(key.clone(), text);
                    }
                }
                return Ok(Value::Str(localizer.translate(arg(0)?.as_str()?, &params)?));
            }
            "profile" => return Ok(Value::from_json(serde_json::to_value(localizer)?)),
            "applyProfile" => {
                let replacement: Localizer = serde_json::from_value(arg(0)?.to_json())?;
                replacement.validate()?;
                *localizer = replacement;
            }
            _ => bail!("unknown Localization method"),
        }
        Ok(Value::Null)
    }
}
