use crysirror_resource::xml::XmlDocument;
#[test]
fn standalone_document_preserves_attributes_text_entities_and_cdata() {
    let doc=XmlDocument::parse(br#"<?xml version="1.0" encoding="UTF-8"?><document><item mode="a&amp;b">A&amp;B&#x21;<![CDATA[<raw>]]></item><empty/></document>"#).unwrap();
    assert_eq!(doc.root().name, "document");
    assert_eq!(doc.node_count(), 3);
    let item = doc.child_path("item").unwrap().unwrap();
    assert_eq!(item.attributes["mode"], "a&b");
    assert_eq!(item.text, "A&B!<raw>");
    assert_eq!(doc.child_path("empty").unwrap().unwrap().text, "");
}
#[test]
fn child_paths_are_explicit_first_sibling_and_case_sensitive() {
    let d = XmlDocument::parse(b"<r><x><y>first</y><y>second</y></x></r>").unwrap();
    assert_eq!(d.child_path("/x//y/").unwrap().unwrap().text, "first");
    assert!(d.child_path("X/y").unwrap().is_none());
    assert!(d.child_path("x/y[1]").is_err());
    assert!(d.child_path("../x").is_err());
    let d = XmlDocument::parse(b"<r><x><z/></x><x><y>later</y></x></r>").unwrap();
    assert_eq!(d.child_path("x/y").unwrap().unwrap().text, "later");
}
#[test]
fn malformed_dtd_external_and_unknown_entity_documents_fail() {
    for bytes in [
        b"<r>".as_slice(),
        b"<r><x></r>",
        b"<a/><b/>",
        b"outside<r/>",
        b"<!DOCTYPE r [<!ENTITY x SYSTEM 'file:///tmp/private'>]><r>&x;</r>",
        b"<r>&unknown;</r>",
        b"<r a='x' a='y'/>",
        b"<1bad/>",
        b"<r 1bad='v'/>",
        b"<r>&#xFFFF;</r>",
        b"<r a='&#0;'/>",
        b"<?xml version='1.1'?><r/>",
    ] {
        assert!(
            XmlDocument::parse(bytes).is_err(),
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }
}
#[test]
fn depth_node_and_source_budgets_apply_without_a_vm() {
    let d = format!("{}{}", "<r>".repeat(65), "</r>".repeat(65));
    assert!(XmlDocument::parse(d.as_bytes()).is_err());
    let many = format!("<r>{}</r>", "<x/>".repeat(32768));
    assert!(XmlDocument::parse(many.as_bytes()).is_err());
    assert!(XmlDocument::parse(&vec![b' '; 8 * 1024 * 1024 + 1]).is_err());
}

#[test]
fn child_iteration_retains_source_order_and_rejects_foreign_nodes() {
    let a=crysirror_resource::xml::XmlDocument::parse(b"<document><row id='1'/><row id='2'/><tail/></document>").unwrap();
    let b=crysirror_resource::xml::XmlDocument::parse(b"<document><foreign/></document>").unwrap();
    let names=a.children(a.root()).unwrap().map(|n|n.name.as_str()).collect::<Vec<_>>();assert_eq!(names,["row","row","tail"]);
    assert!(a.children(b.root()).is_err());assert!(a.children(&a.root().clone()).is_err());
    let first=a.children(a.root()).unwrap().next().unwrap();assert_eq!(first.attributes.get("id").unwrap(),"1");assert_eq!(a.children(first).unwrap().count(),0);
}
