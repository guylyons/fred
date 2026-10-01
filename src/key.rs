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
}

impl Key {
    pub fn new(code: KeyCode) -> Key {
        Key { code, ctrl: false, alt: false }
    }

    pub fn ch(c: char) -> Key {
        Key::new(KeyCode::Char(c))
    }

    pub fn ctrl(c: char) -> Key {
        Key { code: KeyCode::Char(c), ctrl: true, alt: false }
    }

    /// The plain character typed, if this is an unmodified char key.
    pub fn char(&self) -> Option<char> {
        match self.code {
            KeyCode::Char(c) if !self.ctrl && !self.alt => Some(c),
            _ => None,
        }
    }

    pub fn is(&self, code: KeyCode) -> bool {
        self.code == code && !self.ctrl && !self.alt
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
    if let Some(c) = name.strip_prefix("C-") {
        let mut it = c.chars();
        let ch = it.next()?;
        return it.next().is_none().then(|| Key::ctrl(ch.to_ascii_lowercase()));
    }
    Some(Key::new(match name {
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
        _ => return None,
    }))
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
    }
}
