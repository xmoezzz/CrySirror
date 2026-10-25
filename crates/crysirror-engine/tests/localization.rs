// SPDX-License-Identifier: MPL-2.0
use crysirror_engine::{
    localization::{Catalog, Localizer},
    App, Value,
};
use crysirror_runtime::Runtime;
use serde_json::json;
use std::collections::BTreeMap;
fn catalog(value: serde_json::Value) -> Catalog {
    serde_json::from_value(value).unwrap()
}
fn count(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("count".into(), value.into())])
}

#[test]
fn locale_fallback_plural_operands_fonts_and_literal_parameters() {
    let mut l = Localizer::default();
    l.define("en",catalog(json!({"strings":{"coins":{"forms":{"one":"{count} coin","other":"{count} coins"}},"welcome":"Hello {name}! {{literal}}"},"fonts":["latin","fallback"]}))).unwrap();
    assert_eq!(l.translate("coins", &count("1")).unwrap(), "1 coin");
    assert_eq!(l.translate("coins", &count("1.0")).unwrap(), "1.0 coins");
    assert_eq!(l.translate("coins", &count("2")).unwrap(), "2 coins");
    l.set_locale("en-US").unwrap();
    assert_eq!(l.translate("coins", &count("2")).unwrap(), "2 coins");
    l.define("ja",catalog(json!({"strings":{"coins":{"forms":{"other":"{count}枚"}}},"fonts":["japanese","latin"]}))).unwrap();
    l.set_locale("ja-JP").unwrap();
    assert_eq!(l.translate("coins", &count("1")).unwrap(), "1枚");
    assert_eq!(l.fonts().unwrap(), ["japanese", "latin", "fallback"]);
    assert_eq!(
        l.translate(
            "welcome",
            &BTreeMap::from([("name".into(), "{count}".into())])
        )
        .unwrap(),
        "Hello {count}! {literal}"
    );
    let mut forms = serde_json::Map::new();
    for form in ["zero", "one", "two", "few", "many", "other"] {
        forms.insert(form.into(), json!(form));
    }
    l.define("ar", catalog(json!({"strings":{"n":{"forms":forms}}})))
        .unwrap();
    l.set_locale("ar-EG").unwrap();
    for (n, expected) in [
        ("0", "zero"),
        ("1", "one"),
        ("2", "two"),
        ("3", "few"),
        ("11", "many"),
        ("100", "other"),
    ] {
        assert_eq!(l.translate("n", &count(n)).unwrap(), expected);
    }
    assert!(l.translate("missing", &BTreeMap::new()).is_err());
    assert!(l.translate("n", &count("NaN")).is_err());
}

#[test]
fn reflective_catalogs_are_client_scoped_and_bad_profiles_roll_back() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("assets")).unwrap();
    let project = json!({"schema":2,"name":"localization","content_revision":"1","assets":"assets","canvas":[64,64],"clients":[{"name":"a","adapter":"crysirror-script-v1","game":"a"},{"name":"b","adapter":"crysirror-script-v1","game":"b"}]});
    std::fs::write(
        tmp.path().join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let mut app = App::new(
        Runtime::load(&tmp.path().join("project.json"), &tmp.path().join("state")).unwrap(),
        "a",
    )
    .unwrap();
    let cx = app.context();
    let loc = cx.singleton("Localization").unwrap();
    cx.call(
        loc,
        "define",
        &[
            Value::Str("en".into()),
            Value::from_json(json!({"strings":{"hello":"Hello"},"fonts":["latin"]})),
        ],
    )
    .unwrap();
    assert_eq!(
        cx.call(loc, "translate", &[Value::Str("hello".into())])
            .unwrap()
            .as_str()
            .unwrap(),
        "Hello"
    );
    let before = cx.call(loc, "profile", &[]).unwrap();
    let mut bad = before.to_json();
    bad["catalogs"]["en"]["strings"]["bad"] = json!({"forms":{"one":"missing other"}});
    assert!(cx
        .call(loc, "applyProfile", &[Value::from_json(bad)])
        .is_err());
    assert_eq!(
        cx.call(loc, "profile", &[]).unwrap().to_json(),
        before.to_json()
    );
    let b = cx.state().unwrap().runtime.client_id("b").unwrap();
    cx.state().unwrap().client = b;
    assert!(cx
        .call(loc, "translate", &[Value::Str("hello".into())])
        .is_err());
    cx.call(loc, "applyProfile", &[before]).unwrap();
    assert_eq!(cx.get(loc, "fonts").unwrap().to_json(), json!(["latin"]));
}
