//! Emacs regular expressions, translated to the regex crate's syntax.

/// Translate an Emacs regexp (as typed by a user: `\(` groups, `\|`,
/// `\<`, `\{n,m\}`, `[[:alpha:]]`, `\``, `\'`, `\s-`, `\w`) to Rust syntax.
pub fn translate(e: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = e.chars().collect();
    let mut i = 0;
    let mut in_class = false;
    while i < chars.len() {
        let c = chars[i];
        if in_class {
            match c {
                ']' if !out.ends_with('[') && !out.ends_with("[^") => {
                    in_class = false;
                    out.push(']');
                }
                '[' if chars.get(i + 1) == Some(&':') => {
                    // [:alpha:] passes through unchanged.
                    let end = chars[i..]
                        .iter()
                        .collect::<String>()
                        .find(":]")
                        .map(|j| i + j + 2)
                        .unwrap_or(i + 1);
                    out.extend(&chars[i..end]);
                    i = end;
                    continue;
                }
                '\\' => out.push_str("\\\\"),
                '[' => out.push_str("\\["),
                '&' | '~' => {
                    out.push('\\');
                    out.push(c);
                }
                c => out.push(c),
            }
            i += 1;
            continue;
        }
        match c {
            '\\' => {
                let n = chars.get(i + 1).copied();
                i += 2;
                match n {
                    Some('(') => {
                        if chars.get(i) == Some(&'?') {
                            // \(?: or \(?N:
                            let mut j = i + 1;
                            while j < chars.len() && chars[j].is_ascii_digit() {
                                j += 1;
                            }
                            if chars.get(j) == Some(&':') {
                                out.push_str("(?:");
                                i = j + 1;
                                continue;
                            }
                        }
                        out.push('(');
                    }
                    Some(')') => out.push(')'),
                    Some('|') => out.push('|'),
                    Some('{') => out.push('{'),
                    Some('}') => out.push('}'),
                    Some('<') | Some('>') => out.push_str("\\b"),
                    Some('b') => out.push_str("\\b"),
                    Some('B') => out.push_str("\\B"),
                    Some('`') => out.push_str("\\A"),
                    Some('\'') => out.push_str("\\z"),
                    Some('w') => out.push_str("\\w"),
                    Some('W') => out.push_str("\\W"),
                    Some('_') => {
                        // \_< \_> symbol boundaries.
                        i += 1;
                        out.push_str("\\b");
                    }
                    Some('s') | Some('S') => {
                        let class = chars.get(i).copied();
                        i += 1;
                        let neg = n == Some('S');
                        out.push_str(match (class, neg) {
                            (Some('-' | ' '), false) => "\\s",
                            (Some('-' | ' '), true) => "\\S",
                            (Some('w'), false) => "\\w",
                            (Some('w'), true) => "\\W",
                            (Some('.'), false) => "[[:punct:]]",
                            (_, false) => "\\s",
                            (_, true) => "\\S",
                        });
                    }
                    Some(d) if d.is_ascii_digit() => {
                        // Back-references are not supported: match anything.
                        out.push_str(".*?");
                    }
                    Some(c) => {
                        out.push('\\');
                        out.push(c);
                    }
                    None => out.push_str("\\\\"),
                }
                continue;
            }
            '[' => {
                in_class = true;
                out.push('[');
                if chars.get(i + 1) == Some(&'^') {
                    out.push('^');
                    i += 1;
                }
                // A leading ] is literal.
                if chars.get(i + 1) == Some(&']') {
                    out.push_str("\\]");
                    i += 1;
                }
            }
            '(' | ')' | '|' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
        i += 1;
    }
    out
}

/// Compile an Emacs regexp (case-insensitive when `fold`).
pub fn compile(e: &str, fold: bool) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(&translate(e))
        .case_insensitive(fold)
        .multi_line(true)
        .build()
        .map_err(|err| format!("Invalid regexp \"{e}\": {err}"))
}

/// regexp-quote.
pub fn quote(s: &str) -> String {
    regex::escape(s)
}

/// case-fold-search with search-upper-case: fold unless `s` has uppercase.
pub fn smart_fold(s: &str) -> bool {
    !s.chars().any(char::is_uppercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates() {
        assert_eq!(translate(r"\(foo\|bar\)+"), "(foo|bar)+");
        assert_eq!(translate(r"a{2}"), r"a\{2\}");
        assert_eq!(translate(r"x\{2,3\}"), "x{2,3}");
        assert_eq!(translate(r"\<\(?:Work\|Lab\)\>"), r"\b(?:Work|Lab)\b");
        assert_eq!(translate(r"[[:alpha:]_]+\s-*"), r"[[:alpha:]_]+\s*");
        assert_eq!(translate(r"[]a]"), r"[\]a]");
        assert!(
            compile(r"^\*+ \(TODO\)", false)
                .unwrap()
                .is_match("** TODO x")
        );
    }
}
