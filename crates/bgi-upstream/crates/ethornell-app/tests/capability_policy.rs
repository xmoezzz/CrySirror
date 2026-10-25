// Production policy modules, compiled as an isolated integration harness.
#[path = "../src/file_policy.rs"]
mod file_policy;

fn windows_wildcard_matches(pattern: &str, value: &str) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Token {
        Char(char),
        Star,
        DosStar,
        DosQm,
        DosDot,
    }
    let upper = |c: char| {
        let mut u = c.to_uppercase();
        match (u.next(), u.next()) {
            (Some(u), None) => u,
            _ => c,
        }
    };
    let chars = pattern.chars().collect::<Vec<_>>();
    let tokens = chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let next = chars.get(i + 1).copied();
            match c {
                '?' => Token::DosQm,
                '*' if next == Some('.') => Token::DosStar,
                '*' => Token::Star,
                '.' if matches!(next, None | Some('?') | Some('*')) => Token::DosDot,
                c => Token::Char(upper(c)),
            }
        })
        .collect::<Vec<_>>();
    let name = value.chars().map(upper).collect::<Vec<_>>();
    let last_dot = name.iter().rposition(|&c| c == '.');
    // Breadth-first NFA over (token index) for each name position.
    let mut states = vec![false; tokens.len() + 1];
    let close = |states: &mut Vec<bool>, at_end: bool, position: usize| {
        // Epsilon moves: stars match zero characters; DOS_DOT matches zero
        // characters at the end; DOS_QM skips at a period or the end.
        let mut index = 0;
        while index < tokens.len() {
            if states[index] {
                let skip = match tokens[index] {
                    Token::Star => true,
                    Token::DosStar => last_dot.is_none_or(|dot| position <= dot),
                    Token::DosDot => at_end,
                    Token::DosQm => at_end || name.get(position) == Some(&'.'),
                    Token::Char(_) => false,
                };
                if skip && !states[index + 1] {
                    states[index + 1] = true;
                }
            }
            index += 1;
        }
    };
    states[0] = true;
    close(&mut states, name.is_empty(), 0);
    for (position, &c) in name.iter().enumerate() {
        let mut next = vec![false; tokens.len() + 1];
        for (index, token) in tokens.iter().enumerate() {
            if !states[index] {
                continue;
            }
            match *token {
                Token::Char(expected) => {
                    if expected == c {
                        next[index + 1] = true;
                    }
                }
                Token::Star => next[index] = true,
                // Up to (not including) the final period, then the
                // pattern's own '.' must take it.
                Token::DosStar => {
                    if last_dot.is_none_or(|dot| position < dot) {
                        next[index] = true;
                    }
                }
                Token::DosQm => {
                    if c != '.' {
                        next[index + 1] = true;
                    }
                }
                Token::DosDot => {
                    if c == '.' {
                        next[index + 1] = true;
                    }
                }
            }
        }
        states = next;
        close(&mut states, position + 1 == name.len(), position + 1);
    }
    states[tokens.len()]
}

