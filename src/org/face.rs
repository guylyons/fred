//! Org faces (org-faces.el, org font-lock) as styles for Fred's renderer.

use super::sexp::Sexp;
use super::syntax::{self, Settings};
use crate::editor::Editor;
use crate::highlight::LineStyles;
use ratatui::style::{Color, Modifier, Style};
use std::cell::RefCell;

/// What kind of line each buffer line is, for context-dependent faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    BlockDelim,
    /// Inside a block; `true` for a src block.
    Block(bool),
    Drawer,
    Table,
}

#[derive(Default)]
pub struct Cache(RefCell<Option<(u64, Vec<Kind>)>>);

fn kinds(lines: &[String]) -> Vec<Kind> {
    let mut out = Vec::with_capacity(lines.len());
    let mut block: Option<(String, bool)> = None;
    let mut drawer = false;
    for l in lines {
        let t = l.trim_start();
        let lower = t.to_ascii_lowercase();
        if let Some((end, src)) = &block {
            if lower.starts_with(end.as_str()) {
                out.push(Kind::BlockDelim);
                block = None;
            } else {
                out.push(Kind::Block(*src));
            }
            continue;
        }
        if let Some(rest) = lower.strip_prefix("#+begin_") {
            let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            block = Some((format!("#+end_{name}"), name == "src"));
            out.push(Kind::BlockDelim);
            continue;
        }
        if lower.starts_with("#+begin:") {
            block = Some(("#+end:".into(), false));
            out.push(Kind::BlockDelim);
            continue;
        }
        if syntax::level(l).is_some() {
            drawer = false;
            out.push(Kind::Text);
            continue;
        }
        let tt = t.trim_end();
        if tt.len() > 2
            && tt.starts_with(':')
            && tt.ends_with(':')
            && !tt[1..tt.len() - 1].contains(' ')
        {
            drawer = !tt.eq_ignore_ascii_case(":END:");
            out.push(Kind::Drawer);
            continue;
        }
        if drawer {
            out.push(Kind::Drawer);
            continue;
        }
        if t.starts_with('|') || (t.starts_with("+-") && tt.ends_with('+')) {
            out.push(Kind::Table);
            continue;
        }
        out.push(Kind::Text);
    }
    out
}

fn kind_at(ed: &Editor, l: usize) -> Kind {
    let Some(o) = &ed.org else { return Kind::Text };
    let mut c = o.faces.0.borrow_mut();
    if c.as_ref().is_none_or(|(v, _)| *v != ed.buf.version) {
        let lines: Vec<String> = (0..ed.line_count()).map(|i| ed.buf.line(i)).collect();
        *c = Some((ed.buf.version, kinds(&lines)));
    }
    c.as_ref().unwrap().1.get(l).copied().unwrap_or(Kind::Text)
}

const LEVELS: [Color; 8] = [
    Color::Blue,
    Color::Yellow,
    Color::Magenta,
    Color::Cyan,
    Color::Green,
    Color::Red,
    Color::LightBlue,
    Color::LightMagenta,
];

fn hex(s: &str) -> Option<Color> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return named(s);
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn named(s: &str) -> Option<Color> {
    Some(match s.to_ascii_lowercase().replace(' ', "").as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" | "dimgray" => Color::DarkGray,
        "orange" => Color::Rgb(255, 165, 0),
        "pink" => Color::Rgb(255, 192, 203),
        "forestgreen" => Color::Rgb(34, 139, 34),
        "orangered" => Color::Rgb(255, 69, 0),
        _ => return None,
    })
}

/// A face spec (a plist, a color string or a face name) as a style.
pub fn face_style(face: &Sexp) -> Style {
    let mut st = Style::default();
    match face {
        Sexp::Str(s) => {
            if let Some(c) = hex(s) {
                st = st.fg(c);
            }
        }
        Sexp::Sym(name) => st = named_face(name),
        _ => {
            if let Some(c) = face
                .plist_get(":foreground")
                .and_then(|v| v.str())
                .and_then(hex)
            {
                st = st.fg(c);
            }
            if let Some(c) = face
                .plist_get(":background")
                .and_then(|v| v.str())
                .and_then(hex)
            {
                st = st.bg(c);
            }
            if face.plist_get(":weight").and_then(|v| v.sym()) == Some("bold") {
                st = st.add_modifier(Modifier::BOLD);
            }
            if face.plist_get(":slant").and_then(|v| v.sym()) == Some("italic") {
                st = st.add_modifier(Modifier::ITALIC);
            }
            if face.plist_get(":underline").is_some_and(Sexp::truthy) {
                st = st.add_modifier(Modifier::UNDERLINED);
            }
            if face.plist_get(":strike-through").is_some_and(Sexp::truthy) {
                st = st.add_modifier(Modifier::CROSSED_OUT);
            }
            if let Some(inherit) = face.plist_get(":inherit") {
                st = face_style(inherit).patch(st);
            }
        }
    }
    st
}

fn named_face(name: &str) -> Style {
    let s = Style::default();
    match name {
        "org-todo" | "org-warning" => s.fg(Color::Red).add_modifier(Modifier::BOLD),
        "org-done" => s.fg(Color::Green).add_modifier(Modifier::BOLD),
        "bold" => s.add_modifier(Modifier::BOLD),
        "italic" => s.add_modifier(Modifier::ITALIC),
        "underline" => s.add_modifier(Modifier::UNDERLINED),
        "shadow" | "org-archived" => s.fg(Color::DarkGray),
        _ => s,
    }
}

/// The face of a TODO keyword: org-todo-keyword-faces, else org-todo/org-done.
pub fn keyword_style(st: &Settings, kw: &str) -> Style {
    if let Some(list) = st.opt("org-todo-keyword-faces")
        && let Some(entries) = list.list()
        && let Some(e) = entries
            .iter()
            .find(|e| e.car().and_then(Sexp::str) == Some(kw))
    {
        return face_style(&e.cdr());
    }
    if st.is_done(kw) {
        named_face("org-done")
    } else {
        named_face("org-todo")
    }
}

fn priority_style(st: &Settings, p: u32) -> Style {
    if let Some(list) = st.opt("org-priority-faces")
        && let Some(entries) = list.list()
        && let Some(e) = entries
            .iter()
            .find(|e| e.car().and_then(Sexp::int) == Some(p as i64))
    {
        return face_style(&e.cdr());
    }
    Style::default().fg(Color::Magenta)
}

fn level_style(st: &Settings, lvl: usize) -> Style {
    let n = if st.opt_bool("org-odd-levels-only", false) {
        lvl.div_ceil(2)
    } else {
        lvl
    };
    let cycle = super::options::bool("org-cycle-level-faces", true);
    let i = if cycle {
        (n.max(1) - 1) % 8
    } else {
        (n.max(1) - 1).min(7)
    };
    Style::default().fg(LEVELS[i]).add_modifier(Modifier::BOLD)
}

fn push(out: &mut LineStyles, style: Style, r: std::ops::Range<usize>) {
    if r.start < r.end {
        out.push((style, r));
    }
}

/// Styles for line `l` of an Org buffer.
pub fn styles(ed: &Editor, l: usize, st: &Settings) -> LineStyles {
    let line = ed.buf.line(l);
    let mut out = LineStyles::new();
    let gray = Style::default().fg(Color::DarkGray);
    match kind_at(ed, l) {
        Kind::BlockDelim => {
            push(&mut out, gray, 0..line.len());
            return out;
        }
        Kind::Block(_) => {
            return out;
        }
        Kind::Drawer => {
            let t = line.trim_start();
            let off = line.len() - t.len();
            if let Some(rest) = t.strip_prefix(':') {
                let end = rest.find(':').map_or(t.len(), |i| i + 2);
                let drawer = t.trim_end().ends_with(':') && end == t.trim_end().len();
                let style = if drawer {
                    Style::default().fg(Color::Blue)
                } else {
                    Style::default().fg(Color::Cyan)
                };
                push(&mut out, style, off..off + end);
            }
            return out;
        }
        Kind::Table => {
            let style = Style::default().fg(Color::Blue);
            push(&mut out, style, 0..line.len());
            inline(&line, &mut out, st);
            return out;
        }
        Kind::Text => {}
    }
    if let Some(h) = syntax::headline(&line, st) {
        let base = level_style(st, h.level);
        let hide_stars = st.opt_bool("org-hide-leading-stars", false);
        if hide_stars && h.level > 1 {
            push(&mut out, Style::default().fg(Color::Black), 0..h.level - 1);
            push(&mut out, base, h.level - 1..line.len());
        } else {
            push(&mut out, base, 0..line.len());
        }
        let done = h.todo.as_deref().is_some_and(|k| st.is_done(k));
        if done && st.opt_bool("org-fontify-done-headline", true) {
            push(
                &mut out,
                Style::default().fg(Color::DarkGray),
                h.title.clone(),
            );
        }
        if let (Some(k), Some(r)) = (&h.todo, &h.todo_range) {
            push(&mut out, keyword_style(st, k), r.clone());
        }
        if let (Some(p), Some(r)) = (h.priority, &h.priority_range) {
            push(&mut out, priority_style(st, p), r.clone());
        }
        inline(&line[..h.title.end], &mut out, st);
        if let Some(r) = &h.tags_range {
            push(
                &mut out,
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Gray),
                r.clone(),
            );
        }
        return out;
    }
    let t = line.trim_start();
    let off = line.len() - t.len();
    if t.starts_with("#+") {
        if let Some((key, value)) = syntax::keyword_line(&line) {
            let vstart = line.len() - value.len();
            push(&mut out, gray, 0..vstart);
            let vstyle = match key.as_str() {
                "TITLE" => Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
                "AUTHOR" | "DATE" | "EMAIL" | "SUBTITLE" => Style::default().fg(Color::Blue),
                _ => gray,
            };
            push(&mut out, vstyle, vstart..line.len());
        } else {
            push(&mut out, gray, 0..line.len());
        }
        return out;
    }
    if t.starts_with("# ") || t == "#" {
        push(&mut out, gray, 0..line.len());
        return out;
    }
    if t.starts_with(": ") || t == ":" {
        push(&mut out, gray, off..line.len());
        return out;
    }
    // Planning keywords.
    for kw in ["SCHEDULED:", "DEADLINE:", "CLOSED:", "CLOCK:"] {
        let mut from = 0;
        while let Some(i) = line[from..].find(kw) {
            push(
                &mut out,
                Style::default().fg(Color::Cyan),
                from + i..from + i + kw.len(),
            );
            from += i + kw.len();
        }
    }
    // List bullets and checkboxes.
    if let Some(b) = bullet_len(t) {
        push(&mut out, Style::default().fg(Color::Yellow), off..off + b);
        let rest = &t[b..];
        if rest.starts_with("[ ] ") || rest.starts_with("[X] ") || rest.starts_with("[-] ") {
            push(
                &mut out,
                Style::default().add_modifier(Modifier::BOLD),
                off + b..off + b + 3,
            );
        }
        if let Some(i) = rest.find(" :: ") {
            push(
                &mut out,
                Style::default().add_modifier(Modifier::BOLD),
                off + b..off + b + i,
            );
        }
    }
    inline(&line, &mut out, st);
    out
}

/// A list bullet at the start of `t`: its length with the space.
pub fn bullet_len(t: &str) -> Option<usize> {
    if t.starts_with("- ") || t.starts_with("+ ") || (t.starts_with("* ") && false) {
        return Some(2);
    }
    if t == "-" || t == "+" {
        return Some(1);
    }
    let d = t.bytes().take_while(u8::is_ascii_digit).count();
    let a = if d == 0 && t.len() > 1 && t.as_bytes()[0].is_ascii_alphabetic() {
        1
    } else {
        d
    };
    if a > 0
        && matches!(t.as_bytes().get(a), Some(b'.' | b')'))
        && matches!(t.as_bytes().get(a + 1), Some(b' ') | None)
    {
        return Some((a + 2).min(t.len()));
    }
    None
}

/// Objects inside a line: links, timestamps, emphasis, footnotes, targets,
/// macros and statistics cookies.
fn inline(line: &str, out: &mut LineStyles, _st: &Settings) {
    let link = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::UNDERLINED);
    let date = Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::UNDERLINED);
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let rest = &line[i..];
        if rest.starts_with("[[")
            && let Some(e) = rest.find("]]")
        {
            push(out, link, i..i + e + 2);
            i += e + 2;
            continue;
        }
        if rest.starts_with("[fn:")
            && let Some(e) = rest.find(']')
        {
            push(
                out,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
                i..i + e + 1,
            );
            i += e + 1;
            continue;
        }
        if (rest.starts_with('<') || rest.starts_with('['))
            && let Some(len) = timestamp_len(rest)
        {
            push(out, date, i..i + len);
            i += len;
            continue;
        }
        if rest.starts_with("<<")
            && let Some(e) = rest.find(">>")
        {
            push(
                out,
                Style::default().add_modifier(Modifier::UNDERLINED),
                i..i + e + 2,
            );
            i += e + 2;
            continue;
        }
        if rest.starts_with("{{{")
            && let Some(e) = rest.find("}}}")
        {
            push(out, Style::default().fg(Color::DarkGray), i..i + e + 3);
            i += e + 3;
            continue;
        }
        if rest.starts_with("http://")
            || rest.starts_with("https://")
            || rest.starts_with("file:")
            || rest.starts_with("mailto:")
        {
            let e = rest
                .find(|c: char| c.is_whitespace() || c == ']' || c == '>')
                .unwrap_or(rest.len());
            push(out, link, i..i + e);
            i += e;
            continue;
        }
        if rest.starts_with('[')
            && let Some(e) = rest.find(']')
            && e > 1
            && rest[1..e]
                .chars()
                .all(|c| c.is_ascii_digit() || c == '/' || c == '%')
            && (rest[1..e].contains('/') || rest[1..e].ends_with('%'))
        {
            push(
                out,
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                i..i + e + 1,
            );
            i += e + 1;
            continue;
        }
        let c = rest.chars().next().unwrap();
        if "*/_=~+".contains(c)
            && let Some(len) = emphasis_len(line, i)
        {
            let style = match c {
                '*' => Style::default().add_modifier(Modifier::BOLD),
                '/' => Style::default().add_modifier(Modifier::ITALIC),
                '_' => Style::default().add_modifier(Modifier::UNDERLINED),
                '+' => Style::default().add_modifier(Modifier::CROSSED_OUT),
                _ => Style::default().fg(Color::DarkGray),
            };
            push(out, style, i..i + len);
            i += len;
            continue;
        }
        i += c.len_utf8();
    }
}

/// A timestamp (or range) at the start of `s`: its byte length.
pub fn timestamp_len(s: &str) -> Option<usize> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^(?:<\d{4}-\d{2}-\d{2}(?: [^>\n]*)?>(?:--<\d{4}-\d{2}-\d{2}(?: [^>\n]*)?>)?|\[\d{4}-\d{2}-\d{2}(?: [^\]\n]*)?\](?:--\[\d{4}-\d{2}-\d{2}(?: [^\]\n]*)?\])?|<%%\([^>\n]*\)>)")
            .unwrap()
    });
    re.find(s).map(|m| m.end())
}

/// org-emphasis-regexp-components: emphasis starting at byte `i`.
pub fn emphasis_len(line: &str, i: usize) -> Option<usize> {
    let marker = line[i..].chars().next()?;
    let pre_ok = i == 0 || {
        let p = line[..i].chars().next_back().unwrap();
        p.is_whitespace() || "-('\"{".contains(p)
    };
    if !pre_ok {
        return None;
    }
    let body_start = i + 1;
    let first = line[body_start..].chars().next()?;
    if first.is_whitespace() {
        return None;
    }
    let mut j = body_start + first.len_utf8();
    let mut prev = first;
    while j <= line.len() {
        let rest = &line[j..];
        let Some(c) = rest.chars().next() else { break };
        if c == marker && !prev.is_whitespace() && j > body_start {
            let after = rest[1..].chars().next();
            if after.is_none_or(|a| a.is_whitespace() || "-.,:!?;'\")}\\[".contains(a)) {
                return Some(j + 1 - i);
            }
        }
        prev = c;
        j += c.len_utf8();
    }
    None
}

/// Hide link brackets and targets (org-link-descriptive), emphasis
/// markers (org-hide-emphasis-markers) and macro braces
/// (org-hide-macro-markers) on a line that does not hold the cursor.
pub fn conceal(
    line: String,
    st: Option<LineStyles>,
    settings: &Settings,
) -> (String, Option<LineStyles>) {
    let mut hide: Vec<std::ops::Range<usize>> = vec![];
    if settings.opt_bool("org-link-descriptive", true) && line.contains("[[") {
        for lk in super::links::links_in(&line, settings) {
            let r = lk.range.clone();
            if !line[r.clone()].starts_with("[[") {
                continue;
            }
            match &lk.desc {
                Some(d) => {
                    let dstart = r.end - 2 - d.len();
                    hide.push(r.start..dstart);
                    hide.push(r.end - 2..r.end);
                }
                None => {
                    hide.push(r.start..r.start + 2);
                    hide.push(r.end - 2..r.end);
                }
            }
        }
    }
    if settings.opt_bool("org-hide-emphasis-markers", false) {
        let mut i = 0;
        while i < line.len() {
            let c = line[i..].chars().next().unwrap();
            if "*/_=~+".contains(c)
                && !(i == 0 && c == '*' && syntax::level(&line).is_some())
                && let Some(len) = emphasis_len(&line, i)
                && !hide.iter().any(|h| h.contains(&i))
            {
                hide.push(i..i + 1);
                hide.push(i + len - 1..i + len);
                i += len;
                continue;
            }
            i += c.len_utf8();
        }
    }
    if settings.opt_bool("org-hide-macro-markers", false) {
        let mut from = 0;
        while let Some(s) = line[from..].find("{{{").map(|x| x + from) {
            let Some(e) = line[s..].find("}}}").map(|x| x + s) else {
                break;
            };
            hide.push(s..s + 3);
            hide.push(e..e + 3);
            from = e + 3;
        }
    }
    if hide.is_empty() {
        return (line, st);
    }
    hide.sort_by_key(|r| r.start);
    let mut out = String::with_capacity(line.len());
    // New position of each old byte offset.
    let mut map: Vec<usize> = Vec::with_capacity(line.len() + 1);
    let mut h = 0;
    for (i, c) in line.char_indices() {
        while h < hide.len() && hide[h].end <= i {
            h += 1;
        }
        let hidden = h < hide.len() && hide[h].contains(&i);
        for _ in 0..c.len_utf8() {
            map.push(out.len());
        }
        if !hidden {
            out.push(c);
        }
    }
    map.push(out.len());
    let st = st.map(|v| {
        v.into_iter()
            .map(|(s, r)| (s, map[r.start.min(line.len())]..map[r.end.min(line.len())]))
            .filter(|(_, r)| r.start < r.end)
            .collect()
    });
    (out, st)
}

#[cfg(test)]
mod conceal_tests {
    use super::*;

    #[test]
    fn hides_link_targets_and_markers() {
        let st = syntax::settings("".lines(), None);
        let (l, _) = conceal(
            "see [[https://x.org][the site]] and [[t]]".into(),
            None,
            &st,
        );
        assert_eq!(l, "see the site and t");
        let st = syntax::settings("#+STARTUP: literallinks".lines(), None);
        let (l, _) = conceal("[[a][b]]".into(), None, &st);
        assert_eq!(l, "[[a][b]]");
        let st = syntax::settings("".lines(), None);
        super::super::options::put("org-hide-emphasis-markers", toml::Value::Boolean(true));
        let (l, s) = conceal(
            "a *bold* b".into(),
            Some(vec![(Style::default(), 2..8)]),
            &st,
        );
        assert_eq!(l, "a bold b");
        assert_eq!(s.unwrap()[0].1, 2..6);
        super::super::options::put("org-hide-emphasis-markers", toml::Value::Boolean(false));
    }
}
