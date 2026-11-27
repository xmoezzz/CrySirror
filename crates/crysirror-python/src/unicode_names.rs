// SPDX-License-Identifier: MPL-2.0
//! `\N{NAME}` escapes. The full Unicode name table is not embedded; this
//! covers the names that occur in visual-novel scripts (punctuation, dashes,
//! spaces, arrows, hearts and music symbols). Unknown names are a syntax error.

const NAMES: &[(&str, char)] = &[
    ("NO-BREAK SPACE", '\u{a0}'),
    ("EM DASH", '\u{2014}'),
    ("EN DASH", '\u{2013}'),
    ("HORIZONTAL ELLIPSIS", '\u{2026}'),
    ("LEFT SINGLE QUOTATION MARK", '\u{2018}'),
    ("RIGHT SINGLE QUOTATION MARK", '\u{2019}'),
    ("LEFT DOUBLE QUOTATION MARK", '\u{201c}'),
    ("RIGHT DOUBLE QUOTATION MARK", '\u{201d}'),
    ("BULLET", '\u{2022}'),
    ("ZERO WIDTH SPACE", '\u{200b}'),
    ("ZERO WIDTH JOINER", '\u{200d}'),
    ("ZERO WIDTH NON-JOINER", '\u{200c}'),
    ("WORD JOINER", '\u{2060}'),
    ("IDEOGRAPHIC SPACE", '\u{3000}'),
    ("BLACK HEART SUIT", '\u{2665}'),
    ("WHITE HEART SUIT", '\u{2661}'),
    ("BLACK STAR", '\u{2605}'),
    ("WHITE STAR", '\u{2606}'),
    ("EIGHTH NOTE", '\u{266a}'),
    ("BEAMED EIGHTH NOTES", '\u{266b}'),
    ("LEFTWARDS ARROW", '\u{2190}'),
    ("UPWARDS ARROW", '\u{2191}'),
    ("RIGHTWARDS ARROW", '\u{2192}'),
    ("DOWNWARDS ARROW", '\u{2193}'),
    ("COPYRIGHT SIGN", '\u{a9}'),
    ("REGISTERED SIGN", '\u{ae}'),
    ("TRADE MARK SIGN", '\u{2122}'),
    ("DEGREE SIGN", '\u{b0}'),
    ("MULTIPLICATION SIGN", '\u{d7}'),
    ("DIVISION SIGN", '\u{f7}'),
    ("SECTION SIGN", '\u{a7}'),
    ("PILCROW SIGN", '\u{b6}'),
    ("MIDDLE DOT", '\u{b7}'),
    ("KATAKANA MIDDLE DOT", '\u{30fb}'),
    ("WAVE DASH", '\u{301c}'),
    ("FULLWIDTH TILDE", '\u{ff5e}'),
    ("LEFT CORNER BRACKET", '\u{300c}'),
    ("RIGHT CORNER BRACKET", '\u{300d}'),
    ("LEFT WHITE CORNER BRACKET", '\u{300e}'),
    ("RIGHT WHITE CORNER BRACKET", '\u{300f}'),
    ("IDEOGRAPHIC FULL STOP", '\u{3002}'),
    ("IDEOGRAPHIC COMMA", '\u{3001}'),
    ("REPLACEMENT CHARACTER", '\u{fffd}'),
    ("SPACE", ' '),
    ("LINE FEED", '\n'),
    ("CARRIAGE RETURN", '\r'),
    ("CHARACTER TABULATION", '\t'),
    ("NULL", '\0'),
    ("SOFT HYPHEN", '\u{ad}'),
    ("BOX DRAWINGS LIGHT HORIZONTAL", '\u{2500}'),
    ("FULL BLOCK", '\u{2588}'),
];

pub fn lookup(name: &str) -> Option<char> {
    let up = name.to_ascii_uppercase();
    if let Some(hex) = up.strip_prefix("U+") {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    if let Some(hex) = up.strip_prefix("CJK UNIFIED IDEOGRAPH-") {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    NAMES.iter().find(|(n, _)| *n == up).map(|(_, c)| *c)
}
