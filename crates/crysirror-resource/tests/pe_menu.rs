use crysirror_resource::pe_menu::MenuTemplate;
fn word(out: &mut Vec<u8>, n: u16) {
    out.extend(n.to_le_bytes());
}
fn text(out: &mut Vec<u8>, s: &str) {
    for c in s.encode_utf16() {
        word(out, c)
    }
    word(out, 0)
}
#[test]
fn standard_menu_preserves_command_state_popup_and_separators() {
    let mut b = vec![0, 0, 0, 0];
    word(&mut b, 0x90);
    text(&mut b, "System (&S)");
    word(&mut b, 3);
    word(&mut b, 119);
    text(&mut b, "Return to title\tCtrl+T");
    word(&mut b, 0);
    word(&mut b, 0);
    text(&mut b, "");
    word(&mut b, 0x88);
    word(&mut b, 42);
    text(&mut b, "Checked");
    let m = MenuTemplate::parse(&b).unwrap();
    assert_eq!(m.items.len(), 1);
    let root = &m.items[0];
    assert_eq!(root.id, None);
    assert_eq!(root.text, "System (&S)");
    assert_eq!(root.children.len(), 3);
    assert_eq!(root.children[0].id, Some(119));
    assert_eq!(root.children[0].state, 3);
    assert_eq!(root.children[0].text, "Return to title\tCtrl+T");
    assert_eq!(root.children[1].id, Some(0));
    assert!(root.children[1].text.is_empty());
    assert_eq!(root.children[2].state, 8);
}
#[test]
fn extended_truncated_unterminated_and_invalid_unicode_fail() {
    assert!(MenuTemplate::parse(&[1, 0, 4, 0, 0, 0, 0, 0])
        .unwrap_err()
        .to_string()
        .contains("version"));
    let mut good = vec![0, 0, 0, 0];
    word(&mut good, 0x80);
    word(&mut good, 1);
    text(&mut good, "A");
    for i in 0..good.len() {
        assert!(MenuTemplate::parse(&good[..i]).is_err());
    }
    let mut invalid = good.clone();
    invalid[8..10].copy_from_slice(&0xd800u16.to_le_bytes());
    assert!(MenuTemplate::parse(&invalid).is_err());
    good.extend([1, 2, 3, 4]);
    assert!(MenuTemplate::parse(&good).is_err());
}
#[test]
fn nesting_item_and_cumulative_caption_budgets_are_independent() {
    let mut deep = vec![0, 0, 0, 0];
    for _ in 0..18 {
        word(&mut deep, 0x90);
        text(&mut deep, "");
    }
    word(&mut deep, 0x80);
    word(&mut deep, 1);
    text(&mut deep, "");
    assert!(MenuTemplate::parse(&deep).is_err());
    let mut many = vec![0, 0, 0, 0];
    for _ in 0..1025 {
        word(&mut many, 0);
        word(&mut many, 1);
        text(&mut many, "");
    }
    assert!(MenuTemplate::parse(&many).is_err());
    let mut captions = vec![0, 0, 0, 0];
    for _ in 0..40 {
        word(&mut captions, 0);
        word(&mut captions, 1);
        text(&mut captions, &"X".repeat(2048));
    }
    assert!(MenuTemplate::parse(&captions).is_err());
}
