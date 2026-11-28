// SPDX-License-Identifier: MPL-2.0
//! `renpy.lexersupport`, after Ren'Py 7.6/8.1 – 8.6 `lexersupport.pyx`
//! (MIT): the lexer's scanning primitives.
//!
//! Through 8.5, `match_logical_word(s, pos)` returns `(word, magic, end)`
//! (ASCII word characters; `magic` for a `__`-prefixed word of three or
//! more characters). 8.6 returns the end position or None, treats every
//! character from U+0080 as a word character, and adds
//! `match_whitespace`, `match_operator` and `match_string`.
use super::py;
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw};

const MODULE: &str = "renpy.lexersupport";

fn chars_pos(vm: &mut Vm, args: &[Value], n: usize) -> PyResult<(Vec<char>, Vec<i64>)> {
    let s = py::string(vm, &args[0])?;
    let mut pos = Vec::with_capacity(n);
    for a in &args[1..=n] {
        pos.push(py::int(vm, a)?);
    }
    Ok((s.chars().collect(), pos))
}

fn letterlike(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    letterlike(c) || c as u32 >= 128
}

fn opt_pos(i: usize, start: usize) -> Value {
    if i == start { Value::None } else { Value::Int(i as i64) }
}

/// Through 8.5: `(word, magic, end)`.
fn match_logical_word_old(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (s, p) = chars_pos(vm, args, 1)?;
    let start = usize::try_from(p[0]).ok().filter(|i| *i < s.len()).ok_or_else(|| vm.exc_msg("IndexError", "string index out of range"))?;
    let mut pos = start + 1;
    let c = s[start];
    if c == ' ' {
        while pos < s.len() && s[pos] == ' ' {
            pos += 1;
        }
    } else if letterlike(c) {
        while pos < s.len() && letterlike(s[pos]) {
            pos += 1;
        }
    }
    let word: String = s[start..pos].iter().collect();
    let magic = pos - start >= 3 && word.starts_with("__");
    let w = vm.types.new_string(word);
    Ok(vm.types.new_tuple(vec![w, Value::Bool(magic), Value::Int(pos as i64)]))
}

fn match_whitespace(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (s, p) = chars_pos(vm, args, 1)?;
    let start = p[0].max(0) as usize;
    let mut i = start;
    while i < s.len() && s[i] == ' ' {
        i += 1;
    }
    Ok(opt_pos(i, start))
}

fn match_logical_word(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (s, p) = chars_pos(vm, args, 1)?;
    let start = p[0].max(0) as usize;
    let mut i = start;
    while i < s.len() && is_ident_char(s[i]) {
        i += 1;
    }
    Ok(opt_pos(i, start))
}

fn match_operator(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (s, p) = chars_pos(vm, args, 1)?;
    let pos = p[0].max(0) as usize;
    let at = |k: usize| s.get(pos + k).copied().unwrap_or('\0');
    let (c1, c2, c3) = (at(0), at(1), at(2));
    let pair = |a: char, b: char| c1 == a && c2 == b;
    if (c1 == '.' && c2 == '.' && c3 == '.')
        || (c3 == '=' && (pair('/', '/') || pair('>', '>') || pair('<', '<') || pair('*', '*')))
    {
        return Ok(Value::Int(pos as i64 + 3));
    }
    if pair('/', '/')
        || pair('>', '>')
        || pair('<', '<')
        || pair('<', '>')
        || pair('*', '*')
        || pair('-', '>')
        || (c2 == '=' && "+-*/%@&|^:<>=!".contains(c1) && c1 != '\0')
    {
        return Ok(Value::Int(pos as i64 + 2));
    }
    if c1 != '\0' && "+-*/%@&|^,:!.;=~<>$?[]{}()".contains(c1) {
        return Ok(Value::Int(pos as i64 + 1));
    }
    Ok(Value::None)
}

/// The result of `match_string`.
enum Str {
    Unterminated,
    NotString,
    /// (end, need munge, newlines, position after the last newline)
    End(usize, bool, usize, Option<usize>),
}

fn scan_string(s: &[char], prefix_pos: usize, mut pos: usize) -> Str {
    let length = s.len();
    if pos >= length {
        return Str::NotString;
    }
    let mut c = s[pos];
    if !"\"'`".contains(c) {
        return Str::NotString;
    }
    let mut f_string = false;
    match pos.checked_sub(prefix_pos) {
        Some(0) => {}
        Some(1) => {
            let c1 = s[prefix_pos];
            if "fF".contains(c1) {
                f_string = true;
            } else if !"rRuUbB".contains(c1) {
                return Str::NotString;
            }
        }
        Some(2) => {
            let p: String = s[prefix_pos..pos].iter().collect::<String>().to_lowercase();
            if p == "rf" || p == "fr" {
                f_string = true;
            } else if p != "rb" && p != "br" {
                return Str::NotString;
            }
        }
        _ => return Str::NotString,
    }
    pos += 1;
    let quote = c;
    let mut quote_size = 1;
    if pos < length && s[pos] == quote {
        pos += 1;
        if pos < length && s[pos] == quote {
            quote_size = 3;
            pos += 1;
        } else {
            return Str::End(pos, false, 0, None);
        }
    }
    let (mut newlines, mut line_startpos) = (0usize, None::<usize>);
    let (mut brace_depth, mut bracket_depth, mut spec_depth) = (0i64, 0i64, -1i64);
    let mut end_quote_size = 0;
    let mut need_munge = false;
    let mut last_c;
    while end_quote_size != quote_size {
        if pos >= length {
            return Str::Unterminated;
        }
        last_c = c;
        c = s[pos];
        pos += 1;
        if c == '\\' {
            if pos >= length {
                return Str::Unterminated;
            }
            if s[pos] == '\n' {
                line_startpos = Some(pos + 1);
                newlines += 1;
            }
            end_quote_size = 0;
            pos += 1;
            continue;
        }
        if f_string && (c == '{' || c == '}') && pos < length && s[pos] == c {
            end_quote_size = 0;
            pos += 1;
            continue;
        }
        if f_string && c == '{' {
            end_quote_size = 0;
            brace_depth = 1;
            while brace_depth != 0 {
                if pos >= length {
                    return Str::Unterminated;
                }
                c = s[pos];
                if spec_depth != brace_depth {
                    let mut nested_prefix = pos;
                    while nested_prefix > 0 && pos - nested_prefix < 2 && "rRuUbBfF".contains(s[nested_prefix - 1]) {
                        nested_prefix -= 1;
                    }
                    if nested_prefix > 0 && is_ident_char(s[nested_prefix - 1]) {
                        nested_prefix = pos;
                    }
                    match scan_string(s, nested_prefix, pos) {
                        Str::Unterminated => return Str::Unterminated,
                        Str::End(end, munge, n, start) => {
                            pos = end;
                            need_munge |= munge;
                            newlines += n;
                            if start.is_some() {
                                line_startpos = start;
                            }
                            c = '\0';
                            continue;
                        }
                        Str::NotString => {}
                    }
                    if c == '(' || c == '[' {
                        bracket_depth += 1;
                    } else if c == ')' || c == ']' {
                        if bracket_depth != 0 {
                            bracket_depth -= 1;
                        }
                    } else if c == ':'
                        && bracket_depth == 0
                        && spec_depth < 0
                        && !(pos + 1 < length && s[pos + 1] == '=')
                    {
                        spec_depth = brace_depth;
                    } else if c == '#' {
                        while pos < length && s[pos] != '\n' {
                            pos += 1;
                        }
                        if pos >= length {
                            return Str::Unterminated;
                        }
                        c = '\n';
                    }
                }
                if c == '{' {
                    brace_depth += 1;
                } else if c == '}' {
                    brace_depth -= 1;
                    if spec_depth > brace_depth {
                        spec_depth = -1;
                    }
                } else if c == '\n' {
                    line_startpos = Some(pos);
                    newlines += 1;
                }
                pos += 1;
            }
            continue;
        }
        if c == '\n' {
            end_quote_size = 0;
            line_startpos = Some(pos);
            newlines += 1;
        } else if c == quote {
            end_quote_size += 1;
        } else {
            end_quote_size = 0;
        }
        if last_c == '_' && c == '_' {
            need_munge = true;
        }
    }
    Str::End(pos, need_munge, newlines, if newlines != 0 { line_startpos } else { None })
}

fn match_string(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let (s, p) = chars_pos(vm, args, 2)?;
    let (prefix_pos, pos) = (p[0].max(0) as usize, p[1].max(0) as usize);
    Ok(match scan_string(&s, prefix_pos, pos) {
        Str::Unterminated => Value::Int(-1),
        Str::NotString => Value::None,
        Str::End(end, munge, n, start) => vm.types.new_tuple(vec![
            Value::Int(end as i64),
            Value::Bool(munge),
            Value::Int(n as i64),
            start.map_or(Value::None, |s| Value::Int(s as i64)),
        ]),
    })
}

fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let p = &super::state::rt(vm).profile;
    if !p.since_pair(6, 1) {
        return Err(vm.exc_msg("ModuleNotFoundError", format!("No module named '{MODULE}'")));
    }
    if p.at_least(8, 6) {
        Ok(crysirror_python::modules::module_with(
            vm,
            MODULE,
            &[
                ("match_whitespace", match_whitespace),
                ("match_logical_word", match_logical_word),
                ("match_operator", match_operator),
                ("match_string", match_string),
            ],
        ))
    } else {
        Ok(crysirror_python::modules::module_with(vm, MODULE, &[("match_logical_word", match_logical_word_old)]))
    }
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(s: &str, pre: usize, pos: usize) -> Option<(usize, bool, usize)> {
        let c: Vec<char> = s.chars().collect();
        match scan_string(&c, pre, pos) {
            Str::End(e, m, n, _) => Some((e, m, n)),
            _ => None,
        }
    }

    #[test]
    fn strings_follow_upstream() {
        assert_eq!(end(r#""abc" x"#, 0, 0), Some((5, false, 0)));
        assert_eq!(end(r#"f"{a['x']}" y"#, 0, 1), Some((11, false, 0)));
        assert_eq!(end("'''a\nb''' z", 0, 0), Some((9, false, 1)));
        assert_eq!(end(r#""__x" "#, 0, 0), Some((5, true, 0)));
        assert_eq!(end(r#"x"a""#, 0, 1), None);
        let c: Vec<char> = "'abc".chars().collect();
        assert!(matches!(scan_string(&c, 0, 0), Str::Unterminated));
    }
}
