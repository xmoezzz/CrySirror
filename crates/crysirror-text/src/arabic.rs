// SPDX-License-Identifier: MPL-2.0
//! Arabic presentation forms: contextual shaping to Unicode presentation
//! forms (as FriBidi's `SHAPE_ARAB_PRES | SHAPE_ARAB_LIGA` does for
//! visual-order text), and the reverse mapping to base letters.
//!
//! The table pairs each base letter with its isolated, final, initial and
//! medial presentation forms in Arabic Presentation Forms-A/B, and the
//! lam-alef ligatures; it is the table Ren'Py 7.8/8.3 `extras.py` (MIT)
//! publishes as `arabic_reverse_mappings`. Joining classes follow from the
//! table: letters with initial/medial forms join on both sides, letters
//! with a final form on the right only. Letters without presentation forms
//! do not join (a simplification of the `ArabicShaping.txt` classes FriBidi
//! consults); nonspacing marks are transparent; ZWJ and tatweel cause
//! joining.
use std::{collections::HashMap, sync::OnceLock};
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// Rows of (isolated, final, initial, medial) forms, 0 for absent, and the
/// base text. Single-element rows are isolated-only; the ligature rows'
/// base text is in visual order (alef, then lam), as upstream lists it.
/// The U+0649 row with initial/medial forms keeps upstream's U+FBE4 medial.
pub const PRESENTATION_FORMS: &[(&[u32], &str)] = &[
    (&[0xFB50, 0xFB51], "\u{0671}"),
    (&[0xFB52, 0xFB53, 0xFB54, 0xFB55], "\u{067B}"),
    (&[0xFB56, 0xFB57, 0xFB58, 0xFB59], "\u{067E}"),
    (&[0xFB5A, 0xFB5B, 0xFB5C, 0xFB5D], "\u{0680}"),
    (&[0xFB5E, 0xFB5F, 0xFB60, 0xFB61], "\u{067A}"),
    (&[0xFB62, 0xFB63, 0xFB64, 0xFB65], "\u{067F}"),
    (&[0xFB66, 0xFB67, 0xFB68, 0xFB69], "\u{0679}"),
    (&[0xFB6A, 0xFB6B, 0xFB6C, 0xFB6D], "\u{06A4}"),
    (&[0xFB6E, 0xFB6F, 0xFB70, 0xFB71], "\u{06A6}"),
    (&[0xFB72, 0xFB73, 0xFB74, 0xFB75], "\u{0684}"),
    (&[0xFB76, 0xFB77, 0xFB78, 0xFB79], "\u{0683}"),
    (&[0xFB7A, 0xFB7B, 0xFB7C, 0xFB7D], "\u{0686}"),
    (&[0xFB7E, 0xFB7F, 0xFB80, 0xFB81], "\u{0687}"),
    (&[0xFB82, 0xFB83], "\u{068D}"),
    (&[0xFB84, 0xFB85], "\u{068C}"),
    (&[0xFB86, 0xFB87], "\u{068E}"),
    (&[0xFB88, 0xFB89], "\u{0688}"),
    (&[0xFB8A, 0xFB8B], "\u{0698}"),
    (&[0xFB8C, 0xFB8D], "\u{0691}"),
    (&[0xFB8E, 0xFB8F, 0xFB90, 0xFB91], "\u{06A9}"),
    (&[0xFB92, 0xFB93, 0xFB94, 0xFB95], "\u{06AF}"),
    (&[0xFB96, 0xFB97, 0xFB98, 0xFB99], "\u{06B3}"),
    (&[0xFB9A, 0xFB9B, 0xFB9C, 0xFB9D], "\u{06B1}"),
    (&[0xFB9E, 0xFB9F], "\u{06BA}"),
    (&[0xFBA0, 0xFBA1, 0xFBA2, 0xFBA3], "\u{06BB}"),
    (&[0xFBA4, 0xFBA5], "\u{06C0}"),
    (&[0xFBA6, 0xFBA7, 0xFBA8, 0xFBA9], "\u{06C1}"),
    (&[0xFBAA, 0xFBAB, 0xFBAC, 0xFBAD], "\u{06BE}"),
    (&[0xFBAE, 0xFBAF], "\u{06D2}"),
    (&[0xFBB0, 0xFBB1], "\u{06D3}"),
    (&[0xFBD3, 0xFBD4, 0xFBD5, 0xFBD6], "\u{06AD}"),
    (&[0xFBD7, 0xFBD8], "\u{06C7}"),
    (&[0xFBD9, 0xFBDA], "\u{06C6}"),
    (&[0xFBDB, 0xFBDC], "\u{06C8}"),
    (&[0xFBDD], "\u{0677}"),
    (&[0xFBDE, 0xFBDF], "\u{06CB}"),
    (&[0xFBE0, 0xFBE1], "\u{06C5}"),
    (&[0xFBE2, 0xFBE3], "\u{06C9}"),
    (&[0xFBE4, 0xFBE5, 0xFBE6, 0xFBE7], "\u{06D0}"),
    (&[0, 0, 0xFBE8, 0xFBE4], "\u{0649}"),
    (&[0xFBFC, 0xFBFD, 0xFBFE, 0xFBFF], "\u{06CC}"),
    (&[0xFE80], "\u{0621}"),
    (&[0xFE81, 0xFE82], "\u{0622}"),
    (&[0xFE83, 0xFE84], "\u{0623}"),
    (&[0xFE85, 0xFE86], "\u{0624}"),
    (&[0xFE87, 0xFE88], "\u{0625}"),
    (&[0xFE89, 0xFE8A, 0xFE8B, 0xFE8C], "\u{0626}"),
    (&[0xFE8D, 0xFE8E], "\u{0627}"),
    (&[0xFE8F, 0xFE90, 0xFE91, 0xFE92], "\u{0628}"),
    (&[0xFE93, 0xFE94], "\u{0629}"),
    (&[0xFE95, 0xFE96, 0xFE97, 0xFE98], "\u{062A}"),
    (&[0xFE99, 0xFE9A, 0xFE9B, 0xFE9C], "\u{062B}"),
    (&[0xFE9D, 0xFE9E, 0xFE9F, 0xFEA0], "\u{062C}"),
    (&[0xFEA1, 0xFEA2, 0xFEA3, 0xFEA4], "\u{062D}"),
    (&[0xFEA5, 0xFEA6, 0xFEA7, 0xFEA8], "\u{062E}"),
    (&[0xFEA9, 0xFEAA], "\u{062F}"),
    (&[0xFEAB, 0xFEAC], "\u{0630}"),
    (&[0xFEAD, 0xFEAE], "\u{0631}"),
    (&[0xFEAF, 0xFEB0], "\u{0632}"),
    (&[0xFEB1, 0xFEB2, 0xFEB3, 0xFEB4], "\u{0633}"),
    (&[0xFEB5, 0xFEB6, 0xFEB7, 0xFEB8], "\u{0634}"),
    (&[0xFEB9, 0xFEBA, 0xFEBB, 0xFEBC], "\u{0635}"),
    (&[0xFEBD, 0xFEBE, 0xFEBF, 0xFEC0], "\u{0636}"),
    (&[0xFEC1, 0xFEC2, 0xFEC3, 0xFEC4], "\u{0637}"),
    (&[0xFEC5, 0xFEC6, 0xFEC7, 0xFEC8], "\u{0638}"),
    (&[0xFEC9, 0xFECA, 0xFECB, 0xFECC], "\u{0639}"),
    (&[0xFECD, 0xFECE, 0xFECF, 0xFED0], "\u{063A}"),
    (&[0xFED1, 0xFED2, 0xFED3, 0xFED4], "\u{0641}"),
    (&[0xFED5, 0xFED6, 0xFED7, 0xFED8], "\u{0642}"),
    (&[0xFED9, 0xFEDA, 0xFEDB, 0xFEDC], "\u{0643}"),
    (&[0xFEDD, 0xFEDE, 0xFEDF, 0xFEE0], "\u{0644}"),
    (&[0xFEE1, 0xFEE2, 0xFEE3, 0xFEE4], "\u{0645}"),
    (&[0xFEE5, 0xFEE6, 0xFEE7, 0xFEE8], "\u{0646}"),
    (&[0xFEE9, 0xFEEA, 0xFEEB, 0xFEEC], "\u{0647}"),
    (&[0xFEED, 0xFEEE], "\u{0648}"),
    (&[0xFEEF, 0xFEF0], "\u{0649}"),
    (&[0xFEF1, 0xFEF2, 0xFEF3, 0xFEF4], "\u{064A}"),
    (&[0xFEF5, 0xFEF6], "\u{0622}\u{0644}"),
    (&[0xFEF7, 0xFEF8], "\u{0623}\u{0644}"),
    (&[0xFEF9, 0xFEFA], "\u{0625}\u{0644}"),
    (&[0xFEFB, 0xFEFC], "\u{0627}\u{0644}"),
];

/// Presentation form (or ligature) back to base text; later rows win, so
/// U+FBE4 maps to U+0649 as upstream's dictionary does.
pub fn reverse_mappings() -> &'static HashMap<char, &'static str> {
    static M: OnceLock<HashMap<char, &'static str>> = OnceLock::new();
    M.get_or_init(|| {
        let mut m = HashMap::new();
        for (forms, base) in PRESENTATION_FORMS {
            for &f in forms.iter().filter(|f| **f != 0) {
                if let Some(c) = char::from_u32(f) {
                    m.insert(c, *base);
                }
            }
        }
        m
    })
}

/// Replaces presentation forms in `s` by their base letters.
pub fn unmap_presentation_forms(s: &str) -> String {
    let m = reverse_mappings();
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match m.get(&c) {
            Some(b) => out.push_str(b),
            None => out.push(c),
        }
    }
    out
}

#[derive(Clone, Copy, Default)]
struct Forms {
    iso: u32,
    fin: u32,
    ini: u32,
    med: u32,
}

fn forms() -> &'static HashMap<char, Forms> {
    static F: OnceLock<HashMap<char, Forms>> = OnceLock::new();
    F.get_or_init(|| {
        let mut m: HashMap<char, Forms> = HashMap::new();
        for (row, base) in PRESENTATION_FORMS {
            let mut it = base.chars();
            let (Some(b), None) = (it.next(), it.next()) else {
                continue;
            };
            let e = m.entry(b).or_default();
            let g = |i: usize| row.get(i).copied().unwrap_or(0);
            let set = |slot: &mut u32, v: u32| {
                if v != 0 {
                    *slot = v;
                }
            };
            set(&mut e.iso, g(0));
            set(&mut e.fin, g(1));
            set(&mut e.ini, g(2));
            // U+0649's medial form is U+FBE9 (upstream's row says U+FBE4).
            set(&mut e.med, if b == '\u{0649}' && g(3) != 0 { 0xFBE9 } else { g(3) });
        }
        m
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Join {
    None,
    Right,
    Dual,
    Causing,
    Transparent,
}

fn joining(c: char) -> Join {
    if c == '\u{200d}' || c == '\u{0640}' {
        return Join::Causing;
    }
    if let Some(f) = forms().get(&c) {
        return if f.ini != 0 || f.med != 0 {
            Join::Dual
        } else if f.fin != 0 {
            Join::Right
        } else {
            Join::None
        };
    }
    match c.general_category() {
        GeneralCategory::NonspacingMark | GeneralCategory::EnclosingMark => Join::Transparent,
        GeneralCategory::Format if c != '\u{200c}' => Join::Transparent,
        _ => Join::None,
    }
}

/// Lam-alef ligature (isolated, final) for an alef variant.
fn lam_alef(alef: char) -> Option<(u32, u32)> {
    match alef {
        '\u{0622}' => Some((0xFEF5, 0xFEF6)),
        '\u{0623}' => Some((0xFEF7, 0xFEF8)),
        '\u{0625}' => Some((0xFEF9, 0xFEFA)),
        '\u{0627}' => Some((0xFEFB, 0xFEFC)),
        _ => None,
    }
}

/// Shapes logical-order text in place to presentation forms. A lam-alef
/// pair becomes the ligature in the lam's place and U+FEFF fills the
/// alef's, so the length (and any per-character arrays) are kept.
pub fn shape_presentation_forms(text: &mut [char]) {
    let n = text.len();
    let kinds: Vec<Join> = text.iter().map(|&c| joining(c)).collect();
    let prev_joins = |i: usize| -> bool {
        let mut j = i;
        while j > 0 {
            j -= 1;
            match kinds[j] {
                Join::Transparent => continue,
                Join::Dual | Join::Causing => return true,
                _ => return false,
            }
        }
        false
    };
    let next_joins = |i: usize| -> bool {
        let mut j = i + 1;
        while j < n {
            match kinds[j] {
                Join::Transparent => j += 1,
                Join::Dual | Join::Right | Join::Causing => return true,
                _ => return false,
            }
        }
        false
    };
    let mut out = text.to_vec();
    let mut i = 0;
    while i < n {
        let c = text[i];
        let k = kinds[i];
        if k != Join::Dual && k != Join::Right {
            i += 1;
            continue;
        }
        let before = prev_joins(i);
        if c == '\u{0644}' {
            let mut j = i + 1;
            while j < n && kinds[j] == Join::Transparent {
                j += 1;
            }
            if let Some((iso, fin)) = text.get(j).and_then(|&a| lam_alef(a)) {
                out[i] = char::from_u32(if before { fin } else { iso }).unwrap_or(c);
                out[j] = '\u{feff}';
                i = j + 1;
                continue;
            }
        }
        let after = k == Join::Dual && next_joins(i);
        let f = forms()[&c];
        let pick = match (before, after) {
            (true, true) => f.med,
            (false, true) => f.ini,
            (true, false) => f.fin,
            (false, false) => f.iso,
        };
        if pick != 0 {
            out[i] = char::from_u32(pick).unwrap_or(c);
        }
        i += 1;
    }
    text.copy_from_slice(&out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_and_unmaps() {
        let mut t: Vec<char> = "\u{0628}\u{0628}\u{0628}".chars().collect();
        shape_presentation_forms(&mut t);
        assert_eq!(t, ['\u{fe91}', '\u{fe92}', '\u{fe90}']);
        let mut t: Vec<char> = "\u{0644}\u{0627}".chars().collect();
        shape_presentation_forms(&mut t);
        assert_eq!(t, ['\u{fefb}', '\u{feff}']);
        assert_eq!(unmap_presentation_forms("\u{fe91}x"), "\u{0628}x");
        assert_eq!(reverse_mappings()[&'\u{fbe4}'], "\u{0649}");
        assert_eq!(reverse_mappings()[&'\u{fe94}'], "\u{0629}");
        assert_eq!(reverse_mappings()[&'\u{fefb}'], "\u{0627}\u{0644}");
    }
}
