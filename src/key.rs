//! Key representation, independent of the terminal library.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    Esc,
    Enter,
    Backspace,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
    /// Shift on a non-character key (S-Left); a shifted character is
    /// already its uppercase form.
    pub shift: bool,
}

impl Key {
    pub fn new(code: KeyCode) -> Key {
        Key {
            code,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    pub fn ch(c: char) -> Key {
        Key::new(KeyCode::Char(c))
    }

    pub fn ctrl(c: char) -> Key {
        Key {
            code: KeyCode::Char(c),
            ctrl: true,
            alt: false,
            shift: false,
        }
    }

    /// The plain character typed, if this is an unmodified char key.
    pub fn char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.ctrl && !self.alt => Some(c),
            _ => None,
        }
    }

    pub fn is(&self, code: KeyCode) -> bool {
        self.code == code && !self.ctrl && !self.alt && !self.shift
    }

    /// This key with modifiers: `"C-M-S-"` letters in any order.
    pub fn with(code: KeyCode, mods: &str) -> Key {
        Key {
            code,
            ctrl: mods.contains('C'),
            alt: mods.contains('M'),
            shift: mods.contains('S'),
        }
    }
}

/// Parse vim-style key notation: `abc<Esc><C-r><Enter><BS><Tab><S-Tab><lt>`.
pub fn parse_keys(s: &str) -> Vec<Key> {
    let mut out = vec![];
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
            && let Some(k) = named(&rest[1..end])
        {
            out.push(k);
            rest = &rest[end + 1..];
            continue;
        }
        out.push(Key::ch(c));
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn named(name: &str) -> Option<Key> {
    // Modifier prefixes C- M- S- in any order, then a key name or character.
    let (mut ctrl, mut alt, mut shift, mut rest) = (false, false, false, name);
    loop {
        if let Some(r) = rest.strip_prefix("C-").filter(|r| !r.is_empty()) {
            ctrl = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("M-").filter(|r| !r.is_empty()) {
            alt = true;
            rest = r;
        } else if let Some(r) = rest
            .strip_prefix("S-")
            .filter(|r| *r != "Tab" && !r.is_empty())
        {
            shift = true;
            rest = r;
        } else {
            break;
        }
    }
    let code = match rest {
        "Esc" => KeyCode::Esc,
        "Enter" | "CR" => KeyCode::Enter,
        "BS" => KeyCode::Backspace,
        "Tab" => KeyCode::Tab,
        "S-Tab" => KeyCode::BackTab,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "Del" => KeyCode::Delete,
        "lt" => KeyCode::Char('<'),
        _ => {
            let mut it = rest.chars();
            let ch = it.next()?;
            if it.next().is_some() || !(ctrl || alt) {
                return None;
            }
            KeyCode::Char(if ctrl { ch.to_ascii_lowercase() } else { ch })
        }
    };
    Some(Key {
        code,
        ctrl,
        alt,
        shift: shift && !matches!(code, KeyCode::Char(_)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notation() {
        let k = parse_keys("a<Esc><C-r><Enter><lt>x<BS><S-Tab>");
        assert_eq!(
            k,
            vec![
                Key::ch('a'),
                Key::new(KeyCode::Esc),
                Key::ctrl('r'),
                Key::new(KeyCode::Enter),
                Key::ch('<'),
                Key::ch('x'),
                Key::new(KeyCode::Backspace),
                Key::new(KeyCode::BackTab),
            ]
        );
        assert_eq!(parse_keys("<"), vec![Key::ch('<')]);
        assert_eq!(
            parse_keys("<M-S-Left><C-c><M-Enter>"),
            vec![
                Key::with(KeyCode::Left, "MS"),
                Key::ctrl('c'),
                Key::with(KeyCode::Enter, "M"),
            ]
        );
    }
}
