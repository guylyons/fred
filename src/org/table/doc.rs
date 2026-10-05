//! The text model the table commands work on: buffer lines and a point,
//! with the Emacs primitives org-table.el is written against
//! (`forward-line`, `looking-at`, `skip-chars-*`, `insert`...), the table
//! predicates and geometry (org-at-table-p, org-table-begin/end, columns,
//! fields) and org-table-align.
//!
//! A command loads the buffer into a [`Doc`], runs, and the changed lines
//! are written back as one edit (see `mod.rs`).

use super::State;
use regex::{Captures, Regex};
use std::path::PathBuf;
use std::sync::LazyLock;
use unicode_width::UnicodeWidthStr;

/// A table row (org-table-to-lisp): `hline` or its fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Hline,
    Cells(Vec<String>),
}

impl Row {
    pub fn cells(&self) -> Option<&Vec<String>> {
        match self {
            Row::Cells(c) => Some(c),
            Row::Hline => None,
        }
    }
}

/// Buffer text and point.
#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub lines: Vec<String>,
    /// Point: line and byte offset.
    pub l: usize,
    pub c: usize,
    /// An Org buffer (else orgtbl-mode in another mode).
    pub org: bool,
    pub st: State,
    pub msg: Option<String>,
    /// Text for the kill ring: (text, whole lines).
    pub kill: Option<(String, bool)>,
    pub path: Option<PathBuf>,
    /// Emacs `last-command`.
    pub last_command: Option<String>,
    /// The command being run (`this-command`).
    pub this_command: String,
    /// The command was called interactively (called-interactively-p).
    pub interactive: bool,
    /// The active region, as (line, byte) of its start and end.
    pub region: Option<((usize, usize), (usize, usize))>,
    /// org-table-analyze's results for the table being computed.
    pub an: super::formula::Analysis,
}

pub fn re(s: &str) -> Regex {
    Regex::new(s).expect("valid regex")
}

macro_rules! lazy_re {
    ($name:ident, $re:expr) => {
        pub static $name: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| $crate::org::table::doc::re($re));
    };
}
pub(crate) use lazy_re;

/// org-table-line-regexp: `^[ \t]*|`.
pub fn is_table_line(s: &str) -> bool {
    s.trim_start_matches([' ', '\t']).starts_with('|')
}

/// org-table-hline-regexp: `^[ \t]*|-`.
pub fn is_hline(s: &str) -> bool {
    s.trim_start_matches([' ', '\t']).starts_with("|-")
}

/// org-table-dataline-regexp: `^[ \t]*|[^-]`.
pub fn is_dataline(s: &str) -> bool {
    let t = s.trim_start_matches([' ', '\t']);
    t.starts_with('|') && !t[1..].starts_with('-')
}

/// A table.el rule line (`+-`).
pub fn is_table_el_rule(s: &str) -> bool {
    let t = s.trim_start_matches([' ', '\t']);
    t.starts_with("+-") || t.starts_with("+=")
}

/// org-TBLFM-regexp, case-insensitive.
pub fn is_tblfm(s: &str) -> bool {
    let t = s.trim_start_matches([' ', '\t']);
    t.len() >= 8 && t[..8].eq_ignore_ascii_case("#+tblfm:")
}

/// The display width of `s` (org-string-width). Fred shows link markup,
/// so every character counts.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Emacs `org-trim`.
pub fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\n', '\r'])
}

/// org-string-nw-p: a string with a non-white character.
pub fn nw(s: &str) -> bool {
    s.chars().any(|c| !c.is_whitespace())
}

/// Blocks whose contents are not parsed as elements.
const RAW_BLOCKS: &[&str] = &["src", "example", "export", "comment", "verse"];

/// Inside a block whose contents are verbatim (a `|` line there is no
/// table).
pub fn in_raw_block(lines: &[String], l: usize) -> bool {
    for b in (0..l).rev() {
        let line = &lines[b];
        if super::super::syntax::level(line).is_some() {
            return false;
        }
        let t = line.trim_start().to_ascii_lowercase();
        if t.starts_with("#+end_") {
            return false;
        }
        if let Some(rest) = t.strip_prefix("#+begin_") {
            let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            if !RAW_BLOCKS.contains(&name.as_str()) {
                return false;
            }
            let close = format!("#+end_{name}");
            return lines[l..]
                .iter()
                .any(|x| x.trim().to_ascii_lowercase() == close);
        }
    }
    false
}

/// The column of byte `b` in `s` (current-column).
pub fn col_of(s: &str, b: usize) -> usize {
    width(&s[..b.min(s.len())])
}

/// The byte where column `col` starts in `s` (move-to-column).
pub fn byte_of_col(s: &str, col: usize) -> usize {
    let mut w = 0;
    for (i, ch) in s.char_indices() {
        if w >= col {
            return i;
        }
        w += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    s.len()
}

impl Doc {
    /// A Doc from `text`; `<point>` marks point (else the start).
    pub fn new(text: &str) -> Doc {
        let (text, pt) = match text.find("<point>") {
            Some(i) => (format!("{}{}", &text[..i], &text[i + 7..]), Some(i)),
            None => (text.to_owned(), None),
        };
        let body = text.strip_suffix('\n').unwrap_or(&text);
        let lines: Vec<String> = body.split('\n').map(String::from).collect();
        let (mut l, mut c) = (0, 0);
        if let Some(p) = pt {
            l = text[..p].matches('\n').count();
            c = p - text[..p].rfind('\n').map_or(0, |i| i + 1);
        }
        Doc {
            lines,
            l,
            c,
            org: true,
            interactive: true,
            ..Doc::default()
        }
    }

    /// The text, lines joined by newlines.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn line(&self) -> &str {
        &self.lines[self.l]
    }

    pub fn n(&self) -> usize {
        self.lines.len()
    }

    pub fn rest(&self) -> &str {
        &self.lines[self.l][self.c..]
    }

    pub fn before(&self) -> &str {
        &self.lines[self.l][..self.c]
    }

    pub fn eolp(&self) -> bool {
        self.c >= self.lines[self.l].len()
    }

    pub fn bolp(&self) -> bool {
        self.c == 0
    }

    pub fn eobp(&self) -> bool {
        self.l + 1 >= self.lines.len() && self.eolp()
    }

    pub fn bobp(&self) -> bool {
        self.l == 0 && self.c == 0
    }

    pub fn goto(&mut self, l: usize, c: usize) {
        self.l = l.min(self.lines.len().saturating_sub(1));
        let line = &self.lines[self.l];
        let mut c = c.min(line.len());
        while !line.is_char_boundary(c) {
            c -= 1;
        }
        self.c = c;
    }

    pub fn bol(&mut self) {
        self.c = 0;
    }

    pub fn eol(&mut self) {
        self.c = self.lines[self.l].len();
    }

    /// forward-line: to the start of the line `n` lines away. Returns the
    /// count of lines it could not move (point ends at the buffer's edge).
    pub fn forward_line(&mut self, n: isize) -> isize {
        if n >= 0 {
            let max = self.lines.len() - 1 - self.l;
            if n as usize > max {
                self.l += max;
                self.eol();
                // Emacs: moving onto the end of an unterminated last line
                // counts as one line moved.
                return n - max as isize - if self.lines[self.l].is_empty() { 0 } else { 1 };
            }
            self.l += n as usize;
            self.c = 0;
            0
        } else {
            let k = (-n) as usize;
            if k > self.l {
                let short = k - self.l;
                self.l = 0;
                self.c = 0;
                return -(short as isize);
            }
            self.l -= k;
            self.c = 0;
            0
        }
    }

    pub fn current_col(&self) -> usize {
        col_of(self.line(), self.c)
    }

    pub fn move_to_column(&mut self, col: usize) {
        self.c = byte_of_col(self.line(), col);
    }

    pub fn char_after(&self) -> Option<char> {
        self.rest().chars().next()
    }

    pub fn char_before(&self) -> Option<char> {
        self.before().chars().next_back()
    }

    pub fn forward_char(&mut self, n: usize) {
        for _ in 0..n {
            match self.char_after() {
                Some(ch) => self.c += ch.len_utf8(),
                None => break,
            }
        }
    }

    pub fn backward_char(&mut self, n: usize) {
        for _ in 0..n {
            match self.char_before() {
                Some(ch) => self.c -= ch.len_utf8(),
                None => break,
            }
        }
    }

    /// skip-chars-forward within the line while `f` holds.
    pub fn skip_fwd(&mut self, f: impl Fn(char) -> bool) -> usize {
        let start = self.c;
        while let Some(ch) = self.char_after() {
            if !f(ch) {
                break;
            }
            self.c += ch.len_utf8();
        }
        self.c - start
    }

    /// skip-chars-backward within the line while `f` holds.
    pub fn skip_back(&mut self, f: impl Fn(char) -> bool) -> usize {
        let start = self.c;
        while let Some(ch) = self.char_before() {
            if !f(ch) {
                break;
            }
            self.c -= ch.len_utf8();
        }
        start - self.c
    }

    /// looking-at on the rest of the line (`re` should start with `^`).
    pub fn looking_at<'a>(&'a self, r: &Regex) -> Option<Captures<'a>> {
        r.captures(self.rest())
    }

    /// Insert `s` (may hold newlines) at point; point ends after it.
    pub fn insert(&mut self, s: &str) {
        let line = std::mem::take(&mut self.lines[self.l]);
        let (a, b) = line.split_at(self.c);
        let joined = format!("{a}{s}");
        let mut parts: Vec<String> = joined.split('\n').map(String::from).collect();
        let last = parts.len() - 1;
        let new_c = parts[last].len();
        parts[last].push_str(b);
        let nl = parts.len();
        self.lines.splice(self.l..=self.l, parts);
        self.l += nl - 1;
        self.c = new_c;
    }

    /// Insert at point without moving it (save-excursion insert).
    pub fn insert_keep(&mut self, s: &str) {
        let (l, c) = (self.l, self.c);
        self.insert(s);
        self.l = l;
        self.c = c;
    }

    /// Delete from (l1, c1) to (l2, c2); point goes to the start.
    pub fn delete(&mut self, from: (usize, usize), to: (usize, usize)) -> String {
        let (a, b) = if from <= to { (from, to) } else { (to, from) };
        let mut out = String::new();
        if a.0 == b.0 {
            out.push_str(&self.lines[a.0][a.1..b.1]);
            self.lines[a.0].replace_range(a.1..b.1, "");
        } else {
            out.push_str(&self.lines[a.0][a.1..]);
            for l in a.0 + 1..b.0 {
                out.push('\n');
                out.push_str(&self.lines[l]);
            }
            out.push('\n');
            out.push_str(&self.lines[b.0][..b.1]);
            let tail = self.lines[b.0][b.1..].to_owned();
            self.lines[a.0].truncate(a.1);
            self.lines[a.0].push_str(&tail);
            self.lines.drain(a.0 + 1..=b.0);
        }
        self.l = a.0;
        self.c = a.1;
        out
    }

    /// Delete whole line `l` (with its newline).
    pub fn delete_line(&mut self, l: usize) -> String {
        let s = self.lines.remove(l);
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        if self.l > l || self.l >= self.lines.len() {
            self.l = self.l.saturating_sub(1).min(self.lines.len() - 1);
        }
        if self.l == l {
            self.c = 0;
        }
        self.c = self.c.min(self.lines[self.l].len());
        s
    }

    /// Insert whole lines before line `at`.
    pub fn insert_lines(&mut self, at: usize, new: &[String]) {
        let at = at.min(self.lines.len());
        self.lines.splice(at..at, new.iter().cloned());
        if self.l >= at {
            self.l += new.len();
        }
    }

    /// Replace bytes `r` of the current line.
    pub fn replace_in_line(&mut self, r: std::ops::Range<usize>, with: &str) {
        self.lines[self.l].replace_range(r, with);
    }

    pub fn message(&mut self, m: impl Into<String>) {
        self.msg = Some(m.into());
    }

    // ---- table predicates and geometry ----

    /// A line belongs to a table.el table (its run starts with `+-`).
    fn table_el_at(&self, l: usize) -> bool {
        let any = |s: &str| {
            let t = s.trim_start_matches([' ', '\t']);
            t.starts_with('|') || t.starts_with('+')
        };
        let mut b = l;
        while b > 0 && any(&self.lines[b - 1]) {
            b -= 1;
        }
        is_table_el_rule(&self.lines[b])
    }

    /// org-at-table-p on line `l`.
    pub fn table_line_at(&self, l: usize) -> bool {
        l < self.lines.len()
            && is_table_line(&self.lines[l])
            && (!self.org || (!in_raw_block(&self.lines, l) && !self.table_el_at(l)))
    }

    pub fn at_table(&self) -> bool {
        self.table_line_at(self.l)
    }

    /// org-at-table.el-p.
    pub fn at_table_el(&self) -> bool {
        let t = self.line().trim_start_matches([' ', '\t']);
        (t.starts_with('|') || t.starts_with('+'))
            && self.table_el_at(self.l)
            && !in_raw_block(&self.lines, self.l)
    }

    /// org-at-table-hline-p.
    pub fn at_hline(&self) -> bool {
        is_hline(self.line())
    }

    /// org-at-TBLFM-p: a #+TBLFM line belonging to a table.
    pub fn at_tblfm_line(&self, l: usize) -> bool {
        if !is_tblfm(&self.lines[l]) {
            return false;
        }
        let mut b = l;
        while b > 0 && is_tblfm(&self.lines[b - 1]) {
            b -= 1;
        }
        b > 0 && self.table_line_at(b - 1)
    }

    pub fn at_tblfm(&self) -> bool {
        self.at_tblfm_line(self.l)
    }

    /// org-table-begin: first line of the table around line `l`.
    pub fn begin_of(&self, l: usize) -> usize {
        let mut b = l;
        while b > 0 && is_table_line(&self.lines[b - 1]) {
            b -= 1;
        }
        b
    }

    /// org-table-end: the line after the table around line `l`.
    pub fn end_of(&self, l: usize) -> usize {
        let mut e = l;
        while e < self.lines.len() && is_table_line(&self.lines[e]) {
            e += 1;
        }
        e
    }

    pub fn table_begin(&self) -> usize {
        self.begin_of(self.l)
    }

    pub fn table_end(&self) -> usize {
        self.end_of(self.l)
    }

    /// The beginning and end of a table.el table around point.
    pub fn table_el_bounds(&self) -> (usize, usize) {
        let any = |s: &str| {
            let t = s.trim_start_matches([' ', '\t']);
            t.starts_with('|') || t.starts_with('+')
        };
        let mut b = self.l;
        while b > 0 && any(&self.lines[b - 1]) {
            b -= 1;
        }
        let mut e = self.l;
        while e < self.lines.len() && any(&self.lines[e]) {
            e += 1;
        }
        (b, e)
    }

    // ---- columns and fields ----

    /// org-table-current-column.
    pub fn current_column(&self) -> usize {
        let before = self.before();
        let Some(first) = before.find('|') else {
            return 0;
        };
        let hline = self.at_hline();
        1 + before[first + 1..]
            .chars()
            .filter(|&c| c == '|' || (hline && c == '+'))
            .count()
    }

    /// org-table-goto-column.
    pub fn goto_column(&mut self, n: usize, on_delim: bool, force: bool) {
        self.bol();
        if n == 0 {
            return;
        }
        let mut n = n as isize;
        loop {
            n -= 1;
            if n <= -1 {
                break;
            }
            if let Some(i) = self.rest().find('|') {
                self.c += i + 1;
            } else if force {
                self.eol();
                self.skip_back(|c| c != '|');
                self.insert(" | ");
            } else {
                break;
            }
        }
        if force && !self.rest().contains('|') {
            let (l, c) = (self.l, self.c);
            self.eol();
            self.insert(" | ");
            self.goto(l, c);
        }
        if on_delim {
            self.backward_char(1);
        } else if self.rest().starts_with(' ') {
            self.forward_char(1);
        }
    }

    /// org-table-get-field: the field in column `n` (or the current one),
    /// replaced by `replace` if given. Returns the old value.
    pub fn get_field(&mut self, n: Option<usize>, replace: Option<&str>) -> String {
        if let Some(n) = n {
            self.goto_column(n, false, false);
        }
        self.skip_back(|c| c != '|');
        if self.bolp() || self.rest().chars().all(|c| c == ' ' || c == '\t') {
            return String::new();
        }
        let pos = self.c;
        let end = self.rest().find('|').map_or(self.line().len(), |i| pos + i);
        let val = self.line()[pos..end].to_owned();
        if let Some(r) = replace {
            let r = if r.is_empty() { " " } else { r };
            self.replace_in_line(pos..end, r);
        }
        let eol = self.line().len();
        let target = (pos + 1).min(eol);
        self.goto(self.l, target);
        val
    }

    /// org-table-check-inside-data-field.
    pub fn check_inside_data_field(
        &self,
        noerror: bool,
        assume_table: bool,
    ) -> Result<bool, String> {
        let inside = (assume_table || self.at_table())
            && !self.before().chars().all(|c| c == ' ' || c == '\t')
            && !self.at_hline()
            && !self.rest().chars().all(|c| c == ' ' || c == '\t');
        if inside {
            Ok(true)
        } else if noerror {
            Ok(false)
        } else {
            Err("Not in table data field".into())
        }
    }

    /// org-table-current-dline: data lines from the table start to point.
    pub fn current_dline(&self) -> usize {
        let b = self.table_begin();
        (b..=self.l)
            .filter(|&l| is_dataline(&self.lines[l]))
            .count()
    }

    /// org-table-current-line: data lines whose end is before point, plus one.
    pub fn current_line_index(&self) -> usize {
        let b = self.table_begin();
        let e = self.table_end();
        let mut cnt = 0;
        for l in b..e {
            if is_dataline(&self.lines[l]) {
                cnt += 1;
                if l >= self.l {
                    break;
                }
            }
        }
        cnt
    }

    /// org-table-goto-line: the Nth data line; false if it does not exist.
    pub fn goto_line(&mut self, n: usize) -> bool {
        let b = self.table_begin();
        let e = self.table_end();
        let mut cnt = 0;
        self.goto(b, 0);
        for l in b..e {
            if is_dataline(&self.lines[l]) {
                cnt += 1;
                // Point after the match `^[ \t]*|[^-]`.
                let i = self.lines[l].find('|').unwrap_or(0) + 1;
                let c = self.lines[l][i..]
                    .chars()
                    .next()
                    .map_or(i, |ch| i + ch.len_utf8());
                self.goto(l, c);
                if cnt == n {
                    return true;
                }
            }
        }
        cnt == n
    }

    /// org-table-get.
    pub fn table_get(&mut self, line: Option<usize>, column: Option<usize>) -> Option<String> {
        let column = column.unwrap_or_else(|| self.current_column());
        let (l, c) = (self.l, self.c);
        let r = if line.is_none_or(|n| self.goto_line(n)) {
            Some(trim(&self.get_field(Some(column), None)).to_owned())
        } else {
            None
        };
        self.goto(l, c);
        r
    }

    /// org-table-put.
    pub fn table_put(
        &mut self,
        line: Option<usize>,
        column: Option<usize>,
        value: &str,
        align: bool,
    ) -> bool {
        let column = column.unwrap_or_else(|| self.current_column());
        let (l, c) = (self.l, self.c);
        let ok = if line.is_none_or(|n| self.goto_line(n)) {
            self.goto_column(column, false, true);
            self.get_field(Some(column), Some(value));
            true
        } else {
            false
        };
        self.goto(l, c);
        if align {
            self.align();
        }
        ok
    }

    /// org-table-to-lisp from the table at line `b`.
    pub fn to_lisp_at(&self, b: usize) -> Vec<Row> {
        let mut out = vec![];
        let mut l = self.begin_of(b);
        while l < self.lines.len() {
            let line = &self.lines[l];
            let t = line.trim_start_matches([' ', '\t']);
            let Some(rest) = t.strip_prefix('|') else {
                break;
            };
            out.push(row_of(rest));
            l += 1;
        }
        out
    }

    pub fn to_lisp(&self) -> Vec<Row> {
        self.to_lisp_at(self.l)
    }

    /// The shrunk columns of the table at point.
    pub fn shrunk(&self) -> Vec<usize> {
        self.st.shrunk_in(self.table_begin(), self.table_end())
    }

    pub fn set_shrunk(&mut self, cols: Vec<usize>) {
        let (b, e) = (self.table_begin(), self.table_end());
        self.st.set_shrunk(b, e, cols);
    }

    /// org-table-align.
    pub fn align(&mut self) {
        let beg = self.table_begin();
        let end = self.table_end();
        let (line0, col0) = (self.l, self.current_column());
        let table = self.to_lisp_at(beg);
        let rows: Vec<&Vec<String>> = table.iter().filter_map(Row::cells).collect();
        let (widths, aligns) = if rows.is_empty() {
            // Only rules: count columns on this line.
            let n = 1 + self.lines[self.l].matches('+').count();
            (vec![1; n], vec!['l'; n])
        } else {
            column_layout(&rows)
        };
        let rule = widths
            .iter()
            .map(|w| "-".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("+");
        let first = &self.lines[beg];
        let indent = first[..first.find('|').unwrap_or(0) + 1].to_owned();
        for (i, row) in table.iter().enumerate() {
            let new = match row {
                Row::Hline => format!("{indent}{rule}|"),
                Row::Cells(cells) => {
                    let mut s = indent.clone();
                    for (j, w) in widths.iter().enumerate() {
                        let f = cells.get(j).map_or("", String::as_str);
                        s.push_str(&align_field(f, *w, aligns[j]));
                        s.push('|');
                    }
                    s
                }
            };
            if self.lines[beg + i] != new {
                self.lines[beg + i] = new;
            }
        }
        self.st.last_alignment = aligns;
        self.st.last_widths = widths;
        self.st.aligned = Some((beg, end));
        self.st.may_need_update = false;
        // org-table-save-field.
        self.goto(line0, 0);
        self.goto_column(col0, false, false);
    }
}

/// The fields of a table line after its first `|`.
pub fn row_of(rest: &str) -> Row {
    if rest.starts_with('-') {
        return Row::Hline;
    }
    let mut cells = vec![];
    let mut s = rest;
    loop {
        s = s.trim_start_matches([' ', '\t']);
        if s.is_empty() {
            break;
        }
        let end = s.find('|').unwrap_or(s.len());
        cells.push(s[..end].trim_end_matches([' ', '\t']).to_owned());
        s = if end < s.len() { &s[end + 1..] } else { "" };
    }
    Row::Cells(cells)
}

/// org-table-to-lisp on text.
pub fn to_lisp_text(text: &str) -> Vec<Row> {
    let mut out = vec![];
    for line in text.lines() {
        let t = line.trim_start_matches([' ', '\t']);
        let Some(rest) = t.strip_prefix('|') else {
            break;
        };
        out.push(row_of(rest));
    }
    out
}

/// The default org-table-number-regexp.
pub const NUMBER_RE: &str = r"^([<>]?[-+^.0-9]*[0-9][-+^.0-9eEdDx()%:]*|[<>]?[-+]?0[xX][[:xdigit:].]+|[<>]?[-+]?[0-9]+#[0-9a-zA-Z.]+|nan|[-+u]?inf)$";

/// org-table-number-regexp (the user's value is an Emacs regexp).
pub fn number_re() -> Regex {
    match super::super::options::get("org-table-number-regexp") {
        Some(toml::Value::String(s)) => super::emacs_regex(&s).unwrap_or_else(|_| re(NUMBER_RE)),
        _ => {
            static R: LazyLock<Regex> = LazyLock::new(|| re(NUMBER_RE));
            R.clone()
        }
    }
}

lazy_re!(COOKIE_ALIGN, r"^<([lrc])[0-9]*>$");

/// Widths and alignments of the columns of `rows` (org-table-align).
pub fn column_layout(rows: &[&Vec<String>]) -> (Vec<usize>, Vec<char>) {
    let ncol = rows.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
    let fraction = match super::super::options::get("org-table-number-fraction") {
        Some(toml::Value::Float(f)) => f,
        Some(toml::Value::Integer(i)) => i as f64,
        _ => 0.5,
    };
    let num = number_re();
    let mut widths = vec![];
    let mut aligns = vec![];
    for i in 0..ncol {
        let mut max_width = 1;
        let mut fixed = None;
        let mut numbers = 0;
        let mut non_empty = 0;
        for row in rows {
            let cell = row.get(i).map_or("", String::as_str);
            max_width = max_width.max(width(cell));
            if fixed.is_some() || cell.is_empty() {
                continue;
            }
            if let Some(c) = COOKIE_ALIGN.captures(cell) {
                fixed = c[1].chars().next();
            } else {
                non_empty += 1;
                if num.is_match(cell) {
                    numbers += 1;
                }
            }
        }
        widths.push(max_width);
        aligns.push(
            fixed.unwrap_or(if numbers as f64 >= fraction * non_empty as f64 {
                'r'
            } else {
                'l'
            }),
        );
    }
    (widths, aligns)
}

/// org-table--align-field: ` field ` padded to `w`.
pub fn align_field(field: &str, w: usize, align: char) -> String {
    let spaces = w.saturating_sub(width(field));
    let pre = match align {
        'r' => spaces,
        'c' => spaces / 2,
        _ => 0,
    };
    format!(" {}{}{} ", " ".repeat(pre), field, " ".repeat(spaces - pre))
}

/// org-table-clean-line: fields blanked to spaces, `|` kept.
pub fn clean_line(s: &str) -> String {
    if is_hline(s) {
        return s
            .chars()
            .map(|c| if c == '|' || c == '+' { '|' } else { ' ' })
            .collect();
    }
    // Replace each non-blank field by as many spaces as its width.
    let mut out = String::new();
    let mut rest = s;
    let Some(first) = rest.find('|') else {
        return s.to_owned();
    };
    out.push_str(&rest[..=first]);
    rest = &rest[first + 1..];
    while let Some(i) = rest.find('|') {
        let field = &rest[..i];
        if field.chars().any(|c| !matches!(c, ' ' | '\t' | '\r')) {
            out.push_str(&" ".repeat(width(field)));
        } else {
            out.push_str(field);
        }
        out.push('|');
        rest = &rest[i + 1..];
    }
    out.push_str(rest);
    out
}
