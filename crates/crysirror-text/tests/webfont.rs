//! WOFF/WOFF2 decoding against fonts named by `CRYSIRROR_WEBFONT_DIR`
//! (not distributed); every `.woff`/`.woff2` there must decode to an sfnt
//! that the shaping and outline backends accept.
use crysirror_text::webfont;

#[test]
fn web_fonts_decode_to_usable_sfnt() {
    let Some(dir) = std::env::var_os("CRYSIRROR_WEBFONT_DIR") else {
        eprintln!("CRYSIRROR_WEBFONT_DIR not set; skipped");
        return;
    };
    let mut seen = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "woff" && ext != "woff2" {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        let sfnt = webfont::decode(&bytes).unwrap().expect("a web font");
        let face = rustybuzz::Face::from_slice(&sfnt, 0).unwrap_or_else(|| panic!("{}: shaping face", p.display()));
        assert!(face.number_of_glyphs() > 0);
        ab_glyph::FontRef::try_from_slice(&sfnt).unwrap_or_else(|e| panic!("{}: outline face: {e}", p.display()));
        let mut buf = rustybuzz::UnicodeBuffer::new();
        buf.push_str("Hello");
        let shaped = rustybuzz::shape(&face, &[], buf);
        assert_eq!(shaped.len(), 5, "{}", p.display());
        seen += 1;
    }
    assert!(seen > 0, "no web fonts in CRYSIRROR_WEBFONT_DIR");
}
