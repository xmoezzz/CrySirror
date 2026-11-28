// SPDX-License-Identifier: MPL-2.0
//! Single-paragraph logical-to-visual conversion in the manner of FriBidi's
//! `fribidi_log2vis` with its default and Arabic flags: UBA embedding
//! levels (UAX #9, `unicode-bidi`), Arabic presentation-form shaping and
//! lam-alef ligatures, glyph mirroring at odd levels, line reordering and
//! nonspacing marks kept after their base in right-to-left runs. The input
//! is one line; its length in characters is preserved (a ligature leaves
//! U+FEFF in the consumed alef's place).
use unicode_bidi::{Direction, Level, ParagraphBidiInfo, get_base_direction};

/// Paragraph direction request (FriBidi's `FriBidiParType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseDirection {
    /// Forced left-to-right.
    Ltr,
    /// Forced right-to-left.
    Rtl,
    /// First strong character, left-to-right when there is none.
    Neutral,
    /// Weak left-to-right: as Neutral.
    WeakLtr,
    /// Weak right-to-left: first strong character, else right-to-left.
    WeakRtl,
}

fn para_level(text: &str, dir: BaseDirection) -> Level {
    match dir {
        BaseDirection::Ltr => Level::ltr(),
        BaseDirection::Rtl => Level::rtl(),
        BaseDirection::Neutral | BaseDirection::WeakLtr | BaseDirection::WeakRtl => {
            match get_base_direction(text) {
                Direction::Ltr => Level::ltr(),
                Direction::Rtl => Level::rtl(),
                Direction::Mixed if dir == BaseDirection::WeakRtl => Level::rtl(),
                Direction::Mixed => Level::ltr(),
            }
        }
    }
}

/// Per-character embedding levels of the line and the resolved paragraph
/// direction (true for right-to-left).
pub fn embedding_levels(text: &str, dir: BaseDirection) -> (Vec<u8>, bool) {
    let level = para_level(text, dir);
    if text.is_empty() {
        return (Vec::new(), level.is_rtl());
    }
    let info = ParagraphBidiInfo::new(text, Some(level));
    let levels = info
        .reordered_levels_per_char(0..text.len())
        .into_iter()
        .map(|l| l.number())
        .collect();
    (levels, level.is_rtl())
}

/// Visual-order text of one line and the resolved paragraph direction.
pub fn log2vis(text: &str, dir: BaseDirection) -> (String, bool) {
    let (levels, rtl) = embedding_levels(text, dir);
    if text.is_empty() {
        return (String::new(), rtl);
    }
    let mut chars: Vec<char> = text.chars().collect();
    crate::arabic::shape_presentation_forms(&mut chars);
    for (c, l) in chars.iter_mut().zip(&levels) {
        if l % 2 == 1
            && let Some(m) = unicode_bidi_mirroring::get_mirrored(*c)
        {
            *c = m;
        }
    }
    let lv: Vec<Level> = levels.iter().map(|&l| Level::new(l).unwrap_or(Level::ltr())).collect();
    let mut order = ParagraphBidiInfo::reorder_visual(&lv);
    // Keep nonspacing marks after their base in right-to-left runs.
    use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};
    let is_mark = |i: usize| {
        matches!(
            chars[i].general_category(),
            GeneralCategory::NonspacingMark | GeneralCategory::EnclosingMark
        )
    };
    let mut i = 0;
    while i < order.len() {
        if levels[order[i]] % 2 == 1 && is_mark(order[i]) {
            let start = i;
            while i < order.len() && levels[order[i]] % 2 == 1 && is_mark(order[i]) {
                i += 1;
            }
            if i < order.len() && levels[order[i]] == levels[order[start]] {
                order[start..=i].rotate_right(1);
            }
        }
        i += 1;
    }
    (order.into_iter().map(|i| chars[i]).collect(), rtl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorders_hebrew_and_mirrors() {
        let (s, rtl) = log2vis("abc \u{5d0}\u{5d1}\u{5d2}", BaseDirection::Neutral);
        assert!(!rtl);
        assert_eq!(s, "abc \u{5d2}\u{5d1}\u{5d0}");
        let (s, rtl) = log2vis("\u{5d0}(\u{5d1})", BaseDirection::Neutral);
        assert!(rtl);
        assert_eq!(s, "(\u{5d1})\u{5d0}");
        let (l, rtl) = embedding_levels("123", BaseDirection::WeakRtl);
        assert!(rtl);
        assert_eq!(l.len(), 3);
        let (_, rtl) = log2vis("abc", BaseDirection::Rtl);
        assert!(rtl);
    }
}
