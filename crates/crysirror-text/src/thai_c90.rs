// SPDX-License-Identifier: MPL-2.0
//! Thai C90 re-encoding: base consonants, upper/lower vowels and tone marks
//! are mapped to the private-use positional variants (U+F700..U+F71A) that
//! C90-encoded Thai fonts provide, for renderers without OpenType Thai
//! mark positioning. Two editions, after the Ren'Py `extras.thaic90`
//! implementations (MIT; originally the renpy-thai-font-adjuster by
//! SahabandhSthabara and Saamkhaih Kyakya, MIT): [`legacy`] (Ren'Py 7.6 -
//! 8.5) and [`reencode`] (8.6), kept separately because their output
//! differs for some sequences.

fn is_base(c: char) -> bool {
    ('\u{0E01}'..='\u{0E30}').contains(&c) || c == '\u{0E40}' || c == '\u{0E41}'
}
fn is_base_asc(c: char) -> bool {
    matches!(c, '\u{0E1B}' | '\u{0E1D}' | '\u{0E1F}' | '\u{0E2C}')
}
fn is_base_desc(c: char) -> bool {
    matches!(c, '\u{0E0E}' | '\u{0E0F}')
}
fn is_top(c: char) -> bool {
    ('\u{0E48}'..='\u{0E4C}').contains(&c)
}
fn is_lower(c: char) -> bool {
    ('\u{0E38}'..='\u{0E3A}').contains(&c)
}
fn is_upper(c: char) -> bool {
    matches!(
        c,
        '\u{0E31}' | '\u{0E34}' | '\u{0E35}' | '\u{0E36}' | '\u{0E37}' | '\u{0E47}' | '\u{0E4D}'
    )
}
fn top_left(c: char) -> char {
    char::from_u32(0xF713 + (c as u32 - 0x0E48)).unwrap_or(c)
}
fn top_lowleft(c: char) -> char {
    char::from_u32(0xF705 + (c as u32 - 0x0E48)).unwrap_or(c)
}
fn top_low(c: char) -> char {
    char::from_u32(0xF70A + (c as u32 - 0x0E48)).unwrap_or(c)
}
fn upper_left(c: char) -> char {
    match c {
        '\u{0E31}' => '\u{F710}',
        '\u{0E34}' => '\u{F701}',
        '\u{0E35}' => '\u{F702}',
        '\u{0E36}' => '\u{F703}',
        '\u{0E37}' => '\u{F704}',
        '\u{0E4D}' => '\u{F711}',
        '\u{0E47}' => '\u{F712}',
        c => c,
    }
}
fn lower_desc(c: char) -> char {
    char::from_u32(0xF718 + (c as u32 - 0x0E38)).unwrap_or(c)
}

/// Ren'Py 7.6 - 8.5 `thaic90`. Indexing before the start wraps to the end
/// of the string, as the original's negative Python index does.
pub fn legacy(s: &str) -> String {
    let s = s
        .replace('\u{0E33}', "\u{0E4D}\u{0E32}")
        .replace("\u{0E48}\u{0E4D}", "\u{0E4D}\u{0E48}")
        .replace("\u{0E49}\u{0E4D}", "\u{0E4D}\u{0E49}")
        .replace("\u{0E4A}\u{0E4D}", "\u{0E4D}\u{0E4A}")
        .replace("\u{0E4B}\u{0E4D}", "\u{0E4D}\u{0E4B}")
        .replace("\u{0E4C}\u{0E4D}", "\u{0E4D}\u{0E4C}");
    let s: Vec<char> = s.chars().collect();
    let n = s.len();
    let at = |i: isize| -> char { s[i.rem_euclid(n as isize) as usize] };
    let mut rv = String::with_capacity(n * 3);
    for z in 0..n {
        let mut c = s[z];
        let zi = z as isize;
        if is_top(c) && z > 0 {
            let mut b = s[z - 1];
            if is_lower(b) {
                b = at(zi - 2);
            }
            if is_base(b) {
                let nikhahit = z < n - 1 && (s[z + 1] == '\u{0E33}' || s[z + 1] == '\u{0E4D}');
                if is_base_asc(b) {
                    c = if nikhahit { top_left(c) } else { top_lowleft(c) };
                } else if !nikhahit {
                    c = top_low(c);
                }
            }
            if z > 1 && is_upper(s[z - 1]) && is_base_asc(s[z - 2]) {
                // The original maps the already-mapped character, so a
                // second mapping yields its 'error' fallback.
                if !is_top(c) {
                    rv.push_str("error");
                    continue;
                }
                c = top_left(c);
            }
        } else if is_upper(c) && z > 0 && is_base_asc(s[z - 1]) {
            c = upper_left(c);
        } else if is_lower(c) && z > 0 && is_base_desc(s[z - 1]) {
            c = lower_desc(c);
        } else if c == '\u{0E0D}' && z + 1 < n && is_lower(s[z + 1]) {
            c = '\u{F70F}';
        } else if c == '\u{0E10}' && z + 1 < n && is_lower(s[z + 1]) {
            c = '\u{F700}';
        }
        rv.push(c);
    }
    rv
}

/// Ren'Py 8.6 `thaic90`.
pub fn reencode(s: &str) -> String {
    let s: Vec<char> = s.chars().collect();
    let n = s.len();
    let mut out = String::with_capacity(n * 3);
    let mut z = 0;
    while z < n {
        let c = s[z];
        if c == '\u{0E33}' {
            let base = if z >= 2 && is_lower(s[z - 1]) {
                Some(s[z - 2])
            } else if z >= 1 {
                Some(s[z - 1])
            } else {
                None
            };
            out.push(if base.is_some_and(is_base_asc) { '\u{F711}' } else { '\u{0E4D}' });
            out.push('\u{0E32}');
            z += 1;
            continue;
        }
        if is_top(c) && z > 0 {
            let prev = s[z - 1];
            let prev2 = if z > 1 { Some(s[z - 2]) } else { None };
            let base = if z >= 2 && is_lower(prev) { s[z - 2] } else { prev };
            if z + 1 < n {
                let next = s[z + 1];
                if next == '\u{0E4D}' || next == '\u{0E33}' {
                    if is_base_asc(base) {
                        out.push('\u{F711}');
                        out.push(top_left(c));
                    } else {
                        out.push('\u{0E4D}');
                        out.push(c);
                    }
                    if next == '\u{0E33}' {
                        out.push('\u{0E32}');
                    }
                    z += 2;
                    continue;
                }
            }
            if is_upper(prev) {
                out.push(if prev2.is_some_and(is_base_asc) { top_left(c) } else { c });
            } else if is_base_asc(base) {
                out.push(top_lowleft(c));
            } else {
                out.push(top_low(c));
            }
            z += 1;
            continue;
        } else if is_upper(c) && z > 0 && is_base_asc(s[z - 1]) {
            out.push(upper_left(c));
        } else if is_lower(c) && z > 0 && is_base_desc(s[z - 1]) {
            out.push(lower_desc(c));
        } else if c == '\u{0E0D}' && z + 1 < n && is_lower(s[z + 1]) {
            out.push('\u{F70F}');
        } else if c == '\u{0E10}' && z + 1 < n && is_lower(s[z + 1]) {
            out.push('\u{F700}');
        } else {
            out.push(c);
        }
        z += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_marks_move() {
        // po pla + mai ek: lowered-left tone mark.
        assert_eq!(reencode("\u{0E1B}\u{0E48}"), "\u{0E1B}\u{F705}");
        assert_eq!(legacy("\u{0E1B}\u{0E48}"), "\u{0E1B}\u{F705}");
        // ko kai + mai ek: low tone mark.
        assert_eq!(reencode("\u{0E01}\u{0E48}"), "\u{0E01}\u{F70A}");
        assert_eq!(legacy("\u{0E01}\u{0E48}"), "\u{0E01}\u{F70A}");
        // sara am after a tall consonant.
        assert_eq!(reencode("\u{0E1B}\u{0E33}"), "\u{0E1B}\u{F711}\u{0E32}");
    }
}
