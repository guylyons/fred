//! Tags (org.el): inheritance, setting and aligning, fast tag selection,
//! groups, and the tags/property/TODO match language (org-make-tags-matcher)
//! with sparse trees.

use super::props::{Doc, Inherit};
use super::sexp::Sexp;
use super::syntax::{self, Lines, Settings, TagEntry};
use super::{Prefix, ctx, fold, structure};
use crate::editor::Editor;

/// org-use-tag-inheritance is non-nil.
pub fn use_inheritance() -> bool {
    super::sexp::option("org-use-tag-inheritance").is_none_or(|v| v.truthy())
}

/// org-tag-inherit-p.
pub fn inherit_p(tag: &str) -> bool {
    let excluded = super::options::strings("org-tags-exclude-from-inheritance", &[]);
    if excluded.iter().any(|t| t == tag) {
        return false;
    }
    match super::sexp::option("org-use-tag-inheritance") {
        None | Some(Sexp::T) => true,
        Some(Sexp::Nil) => false,
        Some(Sexp::Str(re)) => super::re::compile(&re, false).is_ok_and(|r| r.is_match(tag)),
        Some(v) => v.list().is_some_and(|l| l.iter().any(|x| x.str() == Some(tag))),
    }
}

/// org-make-tag-string.
pub fn make_tag_string(tags: &[String]) -> String {
    if tags.is_empty() { String::new() } else { format!(":{}:", tags.join(":")) }
}

/// Tag groups: (group tag, members) from the tag alist (org-tag-alist-to-groups).
pub fn groups(st: &Settings) -> Vec<(String, Vec<String>)> {
    let mut out = vec![];
    let mut cur: Option<(String, Vec<String>)> = None;
    let mut ingroup = false;
    let mut after_grouptags = false;
    let mut last_tag: Option<String> = None;
    for e in &st.tags {
        match e {
            TagEntry::StartGroup | TagEntry::StartGroupTag => {
                ingroup = true;
                after_grouptags = false;
                cur = None;
            }
            TagEntry::EndGroup | TagEntry::EndGroupTag => {
                if let Some(g) = cur.take() {
                    out.push(g);
                }
                ingroup = false;
                after_grouptags = false;
            }
            TagEntry::GroupTags => {
                after_grouptags = true;
                if let Some(t) = &last_tag {
                    cur = Some((t.clone(), vec![]));
                }
            }
            TagEntry::Tag(t, _) => {
                if ingroup && after_grouptags
                    && let Some((_, m)) = &mut cur
                {
                    m.push(t.clone());
                }
                last_tag = Some(t.clone());
            }
            TagEntry::Newline => {}
        }
    }
    out
}

/// Mutually exclusive groups `{ a b c }` (without a group tag), for
/// fast selection.
fn exclusive_groups(st: &Settings) -> Vec<Vec<String>> {
    let mut out = vec![];
    let mut cur: Option<Vec<String>> = None;
    for e in &st.tags {
        match e {
            TagEntry::StartGroup => cur = Some(vec![]),
            TagEntry::EndGroup => {
                if let Some(g) = cur.take() {
                    out.push(g);
                }
            }
            TagEntry::Tag(t, _) => {
                if let Some(g) = &mut cur {
                    g.push(t.clone());
                }
            }
            _ => {}
        }
    }
    out
}

/// org-set-tags on heading `h`: replace its local tags.
pub fn set_tags(ed: &mut Editor, h: usize, tags: &[String]) {
    let st = super::settings(ed);
    let line = ed.buf.line(h);
    let Some(hl) = syntax::headline(&line, &st) else { return };
    let mut tags: Vec<String> = tags.to_vec();
    if let Some(f) = super::sexp::option("org-tags-sort-function").filter(Sexp::truthy) {
        let desc = f.sym().is_some_and(|s| s.contains("string>") || s.contains("greaterp"));
        tags.sort();
        if desc {
            tags.reverse();
        }
    }
    if tags == hl.tags {
        return;
    }
    let base_end = hl.tags_range.as_ref().map_or(line.trim_end().len(), |r| r.start);
    let mut base = line[..base_end].trim_end_matches([' ', '\t']).to_owned();
    if syntax::level(&format!("{base} ")).is_some() && syntax::level(&base).is_none() {
        // Only stars left: keep the space that makes it a heading.
        base.push(' ');
    }
    let new = if tags.is_empty() {
        base
    } else if base.ends_with(' ') {
        format!("{base}{}", make_tag_string(&tags))
    } else {
        format!("{base} {}", make_tag_string(&tags))
    };
    let byte = ed.cur.byte;
    super::set_line(ed, h, &new);
    if !tags.is_empty() && super::options::bool("org-auto-align-tags", true) {
        align(ed, h);
    }
    if ed.cur.line == h {
        ed.cur.byte = byte.min(ed.buf.line(h).len());
    }
}

/// org-align-tags on one heading.
pub fn align(ed: &mut Editor, h: usize) {
    let text = ed.buf.line(h);
    let mut b = super::buf::EBuf::new(&text, if ed.cur.line == h { ed.cur.byte.min(text.len()) } else { 0 });
    structure::align_tags_here(&mut b, structure::tags_column());
    if b.s != text {
        super::set_line(ed, h, &b.s);
        if ed.cur.line == h {
            ed.cur.byte = b.pt;
        }
    }
}

/// org-align-tags with ALL.
pub fn align_all(ed: &mut Editor) {
    for h in 0..ed.line_count() {
        if ctx::at_heading(ed, h) {
            align(ed, h);
        }
    }
}

/// org-toggle-tag: returns true when the tag is now on.
pub fn toggle_tag(ed: &mut Editor, h: usize, tag: &str, onoff: Option<bool>) -> bool {
    let st = super::settings(ed);
    let mut current = syntax::headline(&ed.buf.line(h), &st).map(|x| x.tags).unwrap_or_default();
    let on = match onoff {
        Some(false) => false,
        Some(true) => true,
        None => !current.iter().any(|t| t == tag),
    };
    current.retain(|t| t != tag);
    if on {
        current.push(tag.to_owned());
    }
    set_tags(ed, h, &current);
    on
}

/// org-get-buffer-tags (with file tags), for completion.
pub fn buffer_tags(ed: &Editor) -> Vec<String> {
    let st = super::settings(ed);
    let mut out: Vec<String> = vec![];
    for l in 0..ed.line_count() {
        if let Some(h) = syntax::headline(&ed.buf.line(l), &st) {
            for t in h.tags {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    for t in &st.file_tags {
        if !out.contains(t) {
            out.push(t.clone());
        }
    }
    out
}

/// The table offered by org-set-tags-command: defined tags, else buffer tags.
fn tag_table(ed: &Editor, st: &Settings) -> Vec<TagEntry> {
    if st.tags.iter().any(|e| matches!(e, TagEntry::Tag(..))) {
        let mut t = st.tags.clone();
        for b in buffer_tags(ed) {
            if !t.iter().any(|e| matches!(e, TagEntry::Tag(n, _) if *n == b)) {
                t.push(TagEntry::Tag(b, None));
            }
        }
        t
    } else {
        buffer_tags(ed).into_iter().map(|t| TagEntry::Tag(t, None)).collect()
    }
}

const FAST_KEYS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ{|}~";

/// Keys for each tag in a table (explicit, else derived from the name).
fn assign_keys(table: &[TagEntry]) -> Vec<(String, char)> {
    let explicit: Vec<char> = table.iter().filter_map(|e| if let TagEntry::Tag(_, Some(k)) = e { Some(*k) } else { None }).collect();
    let mut out: Vec<(String, char)> = vec![];
    let mut pool = FAST_KEYS.chars();
    for e in table {
        let TagEntry::Tag(t, k) = e else { continue };
        if out.iter().any(|(n, _)| n == t) {
            continue;
        }
        let key = match k {
            Some(k) => *k,
            None => {
                let auto = t.trim_start_matches('@').chars().next().map(|c| c.to_ascii_lowercase()).unwrap_or(' ');
                if !explicit.contains(&auto) && !out.iter().any(|(_, c)| *c == auto) {
                    auto
                } else {
                    let mut c = ' ';
                    for x in pool.by_ref() {
                        if !explicit.contains(&x) && !out.iter().any(|(_, y)| *y == x) {
                            c = x;
                            break;
                        }
                    }
                    c
                }
            }
        };
        out.push((t.clone(), key));
    }
    out
}

/// State of a running fast tag selection.
#[derive(Clone)]
struct Fast {
    h: usize,
    current: Vec<String>,
    inherited: Vec<String>,
    keys: Vec<(String, char)>,
    todo_keys: Vec<(String, char)>,
    groups: Vec<Vec<String>>,
    single: bool,
}

/// org-fast-tag-selection as a key menu that stays open until RET.
fn fast_selection(ed: &mut Editor, f: Fast) {
    let mut entries: Vec<(String, String)> = vec![];
    entries.push((String::new(), format!("Inherited: {}", f.inherited.join(" "))));
    entries.push((String::new(), format!("Current: {}", f.current.join(" "))));
    entries.push((String::new(), if f.single { "Next change exits".into() } else { String::new() }));
    for (t, k) in &f.keys {
        let mark = if f.current.contains(t) {
            " ✓"
        } else if f.inherited.contains(t) {
            " (inherited)"
        } else {
            ""
        };
        entries.push((k.to_string(), format!("{t}{mark}")));
    }
    for (t, k) in &f.todo_keys {
        entries.push((k.to_string(), format!("TODO: {t}")));
    }
    entries.push(("RET".into(), "accept".into()));
    entries.push((" ".into(), "clear".into()));
    entries.push(("TAB".into(), "edit (completion)".into()));
    entries.push(("C-c".into(), if f.single { "multi".into() } else { "single".into() }));
    let state = f.clone();
    ed.org_menu_enter = Some("RET".into());
    super::menu(ed, "Tags [a-z..]:toggle [SPC]:clear [RET]:accept [TAB]:edit", entries, move |ed, k| {
        let mut f = state;
        let mut exit_now = false;
        match k.as_str() {
            "RET" => return finish_fast(ed, f),
            " " => {
                f.current.clear();
                exit_now = f.single;
            }
            "C-c" => f.single = !f.single,
            "TAB" => {
                let all = buffer_tags(ed);
                let f2 = f.clone();
                super::complete(ed, "Tag: ", all, false, move |ed, t| {
                    let mut f = f2;
                    if !t.trim().is_empty() {
                        add_or_remove(&mut f, t.trim());
                    }
                    if f.single {
                        finish_fast(ed, f);
                    } else {
                        fast_selection(ed, f);
                    }
                });
                return;
            }
            key => {
                let c = key.chars().next().unwrap_or(' ');
                if let Some((t, _)) = f.todo_keys.iter().find(|(_, k)| *k == c).cloned() {
                    let save = ed.cur;
                    ed.set_cursor(f.h, 0);
                    let _ = super::todo::todo_to(ed, Some(t));
                    ed.cur = save;
                    exit_now = f.single;
                } else if let Some((t, _)) = f.keys.iter().find(|(_, k)| *k == c).cloned() {
                    add_or_remove(&mut f, &t);
                    exit_now = f.single;
                }
            }
        }
        if exit_now {
            finish_fast(ed, f);
        } else {
            fast_selection(ed, f);
        }
    });
}

fn add_or_remove(f: &mut Fast, tag: &str) {
    if f.current.iter().any(|t| t == tag) {
        f.current.retain(|t| t != tag);
    } else {
        for g in &f.groups {
            if g.iter().any(|x| x == tag) {
                f.current.retain(|t| !g.contains(t));
            }
        }
        f.current.push(tag.to_owned());
    }
    // Sort in display order.
    let order: Vec<&String> = f.keys.iter().map(|(t, _)| t).collect();
    f.current.sort_by_key(|t| order.iter().position(|o| *o == t).unwrap_or(usize::MAX));
}

fn finish_fast(ed: &mut Editor, f: Fast) {
    ed.org_menu_enter = None;
    ed.undo.begin(ed.cur.pos());
    set_tags(ed, f.h, &f.current);
    ed.undo.end(ed.cur.pos());
}

/// org-set-tags-command.
pub fn set_tags_command(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    if arg == Prefix::U(1) {
        align_all(ed);
        return Ok(());
    }
    if let Some((lo, hi)) = ed.org_region
        && super::options::bool("org-loop-over-headlines-in-active-region", true)
    {
        let heads: Vec<usize> = (lo..=hi).filter(|&l| ctx::at_heading(ed, l)).collect();
        let st = super::settings(ed);
        let table = tag_table(ed, &st);
        let names: Vec<String> = assign_keys(&table).into_iter().map(|(t, _)| t).collect();
        super::read(ed, "Tags: ", "", move |ed, s| {
            let tags: Vec<String> = s.split([':', ' ']).filter(|t| !t.is_empty()).map(str::to_owned).collect();
            ed.undo.begin(ed.cur.pos());
            for &h in &heads {
                set_tags(ed, h, &tags);
            }
            ed.undo.end(ed.cur.pos());
            let _ = names;
        });
        return Ok(());
    }
    let l = ed.cur.line;
    if ctx::before_first_heading(ed, l) {
        return Err("Setting file tags is not supported yet".into());
    }
    let h = fold::back_to_heading(ed, l).unwrap();
    let st = super::settings(ed);
    let path = ed.path.clone();
    let doc = Doc::new(&ed.buf, &st, path.as_deref());
    let current = doc.local_tags(h);
    let all = doc.tags(Some(h), false);
    let inherited: Vec<String> = all.into_iter().filter(|t| !current.contains(t)).collect();
    let table = tag_table(ed, &st);
    let use_fast = arg != Prefix::U(2)
        && match super::sexp::option("org-use-fast-tag-selection") {
            Some(Sexp::T) => true,
            Some(Sexp::Nil) => false,
            _ => table.iter().any(|e| matches!(e, TagEntry::Tag(_, Some(_)))),
        };
    if use_fast {
        let include_todo = super::options::bool("org-fast-tag-selection-include-todo", false);
        let todo_keys = if include_todo {
            st.seqs.iter().flat_map(|s| s.todo.iter().chain(&s.done)).filter_map(|k| k.key.map(|c| (k.name.clone(), c))).collect()
        } else {
            vec![]
        };
        let single = super::sexp::option("org-fast-tag-selection-single-key").is_some_and(|v| v.truthy());
        fast_selection(
            ed,
            Fast {
                h,
                current,
                inherited,
                keys: assign_keys(&table),
                todo_keys,
                groups: exclusive_groups(&st),
                single,
            },
        );
        return Ok(());
    }
    let candidates: Vec<String> = assign_keys(&table).into_iter().map(|(t, _)| t).collect();
    let initial = make_tag_string(&current);
    let _ = candidates;
    super::read(ed, "Tags: ", &initial, move |ed, s| {
        let tags: Vec<String> = s
            .split(|c: char| !(syntax::is_tag_char(c)))
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect();
        ed.undo.begin(ed.cur.pos());
        set_tags(ed, h, &tags);
        ed.undo.end(ed.cur.pos());
    });
    Ok(())
}

// ---- the match language ----

#[derive(Clone, Debug, PartialEq)]
enum Operand {
    Regexp(String),
    Str(String),
    Time(i64),
    Num(f64),
}

#[derive(Clone, Debug, PartialEq)]
enum Term {
    /// Exact tag.
    Tag(String),
    /// `{regexp}` on tags.
    TagRe(String),
    /// PROP op value, with `*` (must exist).
    Prop { name: String, op: String, value: Operand, star: bool },
}

/// A compiled matcher (org-make-tags-matcher).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Matcher {
    pub source: String,
    /// OR of ANDs of (negated?, term).
    tags: Vec<Vec<(bool, Term)>>,
    todo: Vec<Vec<(bool, Term)>>,
    /// `/!`: only not-done TODO entries.
    pub todo_only: bool,
}

/// Values a matcher needs from an entry.
pub struct EntryInfo<'a> {
    pub todo: Option<&'a str>,
    pub tags: &'a [String],
    pub level: usize,
    pub prop: &'a dyn Fn(&str) -> Option<String>,
    pub not_done: &'a dyn Fn(&str) -> bool,
}

/// One query term at the start of `s`: (+/-/:, term text, rest).
fn next_term(s: &str) -> Option<(Option<char>, String, &str)> {
    let s = s.strip_prefix('&').unwrap_or(s);
    let mut chars = s.char_indices().peekable();
    let sign = match s.chars().next() {
        Some(c @ ('+' | '-' | ':')) => {
            chars.next();
            Some(c)
        }
        _ => None,
    };
    let start = sign.map_or(0, |c| c.len_utf8());
    let rest = &s[start..];
    if rest.starts_with('{') {
        let e = rest.find('}')?;
        return Some((sign, rest[..=e].to_owned(), &rest[e + 1..]));
    }
    // Property name with operator?
    let name_len = {
        let mut i = 0;
        let b = rest.as_bytes();
        while i < b.len() {
            if b[i] == b'\\' && i + 1 < b.len() {
                i += 2;
            } else if (b[i] as char).is_alphanumeric() || b[i] == b'_' || b[i] >= 0x80 {
                i += 1;
            } else {
                break;
            }
        }
        i
    };
    let after = &rest[name_len..];
    for op in ["<=", ">=", "=<", "=>", "<>", "!=", "/=", "==", "<", ">", "="] {
        if name_len > 0 && after.starts_with(op) {
            let mut v = &after[op.len()..];
            let star = v.starts_with('*');
            if star {
                v = &v[1..];
            }
            let vlen = if v.starts_with('{') {
                v.find('}')? + 1
            } else if v.starts_with('"') {
                v[1..].find('"')? + 2
            } else {
                v.bytes().take_while(|c| c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'e' | b'E' | b'+')).count()
            };
            if vlen == 0 {
                return None;
            }
            let term = format!("{}{op}{}{}", &rest[..name_len], if star { "*" } else { "" }, &v[..vlen]);
            return Some((sign, term, &v[vlen..]));
        }
    }
    let tag_len = rest.chars().take_while(|c| syntax::is_tag_char(*c)).map(char::len_utf8).sum::<usize>();
    if tag_len == 0 {
        return None;
    }
    Some((sign, rest[..tag_len].to_owned(), &rest[tag_len..]))
}

fn parse_term(t: &str, now: i64) -> Term {
    if let Some(re) = t.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
        return Term::TagRe(re.to_owned());
    }
    for op in ["<=", ">=", "=<", "=>", "<>", "!=", "/=", "==", "<", ">", "="] {
        if let Some(i) = t.find(op)
            && t[..i].chars().all(|c| c.is_alphanumeric() || c == '_' || c == '\\' || c == '-')
            && i > 0
        {
            let name = t[..i].replace('\\', "").to_uppercase();
            let mut v = &t[i + op.len()..];
            let star = v.starts_with('*');
            if star {
                v = &v[1..];
            }
            let value = if let Some(re) = v.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
                Operand::Regexp(re.to_owned())
            } else if let Some(s) = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
                match matcher_time(s, now) {
                    Some(ts) => Operand::Time(ts),
                    None => Operand::Str(s.to_owned()),
                }
            } else {
                Operand::Num(v.parse().unwrap_or(0.0))
            };
            return Term::Prop { name, op: op.to_owned(), value, star };
        }
    }
    Term::Tag(t.to_owned())
}

/// org-matcher-time: `<now>`, `<today>`, `<-3d>`, `<2026-01-01>`.
fn matcher_time(s: &str, now: i64) -> Option<i64> {
    let inner = s.strip_prefix(['<', '['])?.strip_suffix(['>', ']'])?;
    let day = 86400;
    let midnight = now - now.rem_euclid(day);
    Some(match inner {
        "now" => now,
        "today" => midnight,
        "tomorrow" => midnight + day,
        "yesterday" => midnight - day,
        r if r.starts_with(['+', '-']) => {
            let (num, unit) = r.split_at(r.len() - 1);
            let n: i64 = num.parse().ok()?;
            midnight
                + n * match unit {
                    "d" => day,
                    "w" => 7 * day,
                    "m" => 30 * day,
                    "y" => 365 * day,
                    _ => return None,
                }
        }
        r => {
            let d = r.get(..10)?;
            let (y, rest) = d.split_once('-')?;
            let (m, dd) = rest.split_once('-')?;
            days_from_civil(y.parse().ok()?, m.parse().ok()?, dd.parse().ok()?) * day
        }
    })
}

/// Days since 1970-01-01 of a civil date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Expand group tags into regexps (org-tags-expand).
pub fn expand_groups(m: &str, st: &Settings) -> String {
    if !super::options::bool("org-group-tags", true) {
        return m.to_owned();
    }
    let gs = groups(st);
    if gs.is_empty() {
        return m.to_owned();
    }
    let expand = |tag: &str| -> Vec<String> {
        let mut out = vec![];
        let mut todo = std::collections::VecDeque::from([tag.to_owned()]);
        while let Some(t) = todo.pop_front() {
            if out.contains(&t) {
                continue;
            }
            out.push(t.clone());
            if let Some((_, members)) = gs.iter().find(|(g, _)| g.eq_ignore_ascii_case(&t)) {
                todo.extend(members.iter().cloned());
            }
        }
        out
    };
    let mut res = String::new();
    let mut rest = m;
    while !rest.is_empty() {
        if rest.starts_with('{')
            && let Some(e) = rest.find('}')
        {
            res.push_str(&rest[..=e]);
            rest = &rest[e + 1..];
            continue;
        }
        let w: usize = rest.chars().take_while(|c| syntax::is_tag_char(*c)).map(char::len_utf8).sum();
        if w == 0 {
            let c = rest.chars().next().unwrap();
            res.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let word = &rest[..w];
        let is_prop = rest[w..].starts_with(['<', '>', '=', '!', '/']) && !rest[w..].starts_with("/");
        if !is_prop && gs.iter().any(|(g, _)| g.eq_ignore_ascii_case(word)) {
            let tags = expand(word);
            let (re_tags, plain): (Vec<String>, Vec<String>) = tags.into_iter().partition(|t| t.starts_with('{'));
            let regular = plain.iter().map(|t| regex::escape(t)).collect::<Vec<_>>().join("\\|");
            let regexps = re_tags.iter().map(|t| t.trim_start_matches('{').trim_end_matches('}').to_owned()).collect::<Vec<_>>().join("\\|");
            res.push_str(&if plain.is_empty() {
                format!("{{{regexps}}}")
            } else if re_tags.is_empty() {
                format!("{{\\<\\(?:{regular}\\)\\>}}")
            } else {
                format!("{{\\<\\(?:{regular}\\)\\>\\|{regexps}}}")
            });
        } else {
            res.push_str(word);
        }
        rest = &rest[w..];
    }
    res
}

fn parse_or(s: &str, now: i64) -> Vec<Vec<(bool, Term)>> {
    let mut out = vec![];
    for alt in split_or(s) {
        let mut and = vec![];
        let mut rest = alt.as_str();
        while let Some((sign, term, r)) = next_term(rest) {
            and.push((sign == Some('-'), parse_term(&term, now)));
            rest = r;
        }
        out.push(and);
    }
    out
}

/// Split on `|` outside `{...}` and `"..."`.
fn split_or(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let (mut brace, mut quote) = (false, false);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if !quote => brace = true,
            '}' if !quote => brace = false,
            '"' if !brace => quote = !quote,
            '\\' if chars.peek() == Some(&'|') => {
                cur.push_str("\\|");
                chars.next();
                continue;
            }
            '|' if !brace && !quote => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter().filter(|s| !s.trim().is_empty()).collect()
}

impl Matcher {
    /// org-make-tags-matcher.
    pub fn new(m: &str, st: &Settings, now: i64) -> Matcher {
        let expanded = expand_groups(m, st);
        let mut todo_only = false;
        // The TODO part follows the last slash outside quotes.
        let mut split = None;
        let mut quote = false;
        let mut brace = false;
        for (i, c) in expanded.char_indices() {
            match c {
                '"' if !brace => quote = !quote,
                '{' if !quote => brace = true,
                '}' if !quote => brace = false,
                '/' if !quote && !brace && !expanded[..i].ends_with(['<', '>', '=', '!']) && !expanded[i + 1..].starts_with('=') => split = Some(i),
                _ => {}
            }
        }
        let (tagsm, todom) = match split {
            Some(i) => {
                let mut t = expanded[i + 1..].trim_start_matches('/').to_owned();
                if let Some(r) = t.strip_prefix('!') {
                    todo_only = true;
                    t = r.to_owned();
                }
                (expanded[..i].to_owned(), t)
            }
            None => (expanded.clone(), String::new()),
        };
        Matcher {
            source: m.to_owned(),
            tags: if tagsm.trim().is_empty() { vec![] } else { parse_or(&tagsm, now) },
            todo: if todom.trim().is_empty() { vec![] } else { parse_or(&todom, now) },
            todo_only,
        }
    }

    fn term(t: &Term, e: &EntryInfo, todo_part: bool) -> bool {
        match t {
            Term::Tag(tag) if todo_part => e.todo == Some(tag.as_str()),
            Term::TagRe(re) if todo_part => super::re::compile(re, false).is_ok_and(|r| e.todo.is_some_and(|t| r.is_match(t))),
            Term::Tag(tag) => e.tags.iter().any(|t| t == tag),
            Term::TagRe(re) => super::re::compile(re, false).is_ok_and(|r| e.tags.iter().any(|t| r.is_match(t))),
            Term::Prop { name, op, value, star } => {
                let v = match name.as_str() {
                    "LEVEL" => Some(e.level.to_string()),
                    "TODO" => e.todo.map(str::to_owned),
                    _ => (e.prop)(name),
                };
                if *star && v.is_none() {
                    return false;
                }
                let v = v.unwrap_or_default();
                let cmp = |o: std::cmp::Ordering| match op.as_str() {
                    "<" => o.is_lt(),
                    ">" => o.is_gt(),
                    "<=" | "=<" => o.is_le(),
                    ">=" | "=>" => o.is_ge(),
                    "=" | "==" => o.is_eq(),
                    _ => o.is_ne(),
                };
                match value {
                    Operand::Regexp(re) => {
                        let m = super::re::compile(re, false).is_ok_and(|r| r.is_match(&v));
                        if matches!(op.as_str(), "<>" | "!=" | "/=") { !m } else { m }
                    }
                    Operand::Str(s) => cmp(v.as_str().cmp(s.as_str())),
                    Operand::Time(t) => match matcher_time(&format!("<{}>", v.trim_matches(['<', '>', '[', ']'])), 0) {
                        Some(x) => cmp(x.cmp(t)),
                        None => false,
                    },
                    Operand::Num(n) => {
                        let x: f64 = v.trim().parse().unwrap_or(0.0);
                        cmp(x.partial_cmp(n).unwrap_or(std::cmp::Ordering::Equal))
                    }
                }
            }
        }
    }

    pub fn matches(&self, e: &EntryInfo) -> bool {
        if self.todo_only && !e.todo.is_some_and(|t| (e.not_done)(t)) {
            return false;
        }
        let eval = |or: &[Vec<(bool, Term)>], todo: bool| or.is_empty() || or.iter().any(|and| and.iter().all(|(neg, t)| Self::term(t, e, todo) != *neg));
        eval(&self.tags, false) && eval(&self.todo, true)
    }
}

/// Entries of `b` matching `m`: heading lines (org-scan-tags).
pub fn scan<L: Lines + ?Sized>(b: &L, st: &Settings, file: Option<&std::path::Path>, m: &Matcher) -> Vec<usize> {
    let doc = Doc::new(b, st, file);
    let mut out = vec![];
    let sublevels = super::sexp::option("org-tags-match-list-sublevels").is_none_or(|v| v.truthy());
    let mut skip_until = 0;
    for l in 0..b.n_lines() {
        let line = b.line_text(l);
        let Some(h) = syntax::headline(&line, st) else { continue };
        if l < skip_until {
            continue;
        }
        let tags = doc.tags(Some(l), false);
        let prop = |p: &str| doc.get(Some(l), p, Inherit::Selective, false);
        let not_done = |t: &str| !st.is_done(t);
        let info = EntryInfo {
            todo: h.todo.as_deref(),
            tags: &tags,
            level: h.level,
            prop: &prop,
            not_done: &not_done,
        };
        if m.matches(&info) {
            out.push(l);
            if !sublevels {
                skip_until = syntax::subtree_end(b, l);
            }
        }
    }
    out
}

/// Show matching entries, hide the rest (org-occur's display).
pub fn sparse_show(ed: &mut Editor, hits: &[usize], key: &str) {
    fold::overview(ed);
    // Fold everything below the top level, then reveal each match.
    let n = ed.line_count();
    for l in 0..n {
        if ctx::at_heading(ed, l) {
            fold::fold_subtree(ed, l, true);
        }
    }
    for &h in hits {
        fold::show_context_for(ed, h, key);
        fold::show_heading(ed, h);
    }
}

/// org-match-sparse-tree.
pub fn match_sparse_tree(ed: &mut Editor, todo_only: bool, m: &str) {
    let st = super::settings(ed);
    let now = super::now();
    let mut matcher = Matcher::new(m, &st, now);
    matcher.todo_only |= todo_only;
    let path = ed.path.clone();
    let hits = scan(&ed.buf, &st, path.as_deref(), &matcher);
    sparse_show(ed, &hits, "tags-tree");
    ed.set_msg(format!("{} matches for \"{m}\"", hits.len()));
    if let Some(o) = &mut ed.org {
        o.sparse_hits = hits;
    }
}

/// org-change-tag-in-region.
fn change_tag_in_region(ed: &mut Editor) -> Result<(), String> {
    let (lo, hi) = ed.org_region.ok_or("No active region")?;
    let tags = buffer_tags(ed);
    super::complete(ed, "Tag: ", tags, false, move |ed, tag| {
        super::menu(ed, "[s]et or [r]emove?", vec![("s".into(), "set".into()), ("r".into(), "remove".into())], move |ed, k| {
            let off = k == "r";
            ed.undo.begin(ed.cur.pos());
            let mut cnt = 0;
            for l in lo..=hi {
                if ctx::at_heading(ed, l) {
                    toggle_tag(ed, l, &tag, Some(!off));
                    cnt += 1;
                }
            }
            ed.undo.end(ed.cur.pos());
            ed.set_msg(format!("Tag :{tag}: {} in {cnt} headings", if off { "removed" } else { "set" }));
        });
    });
    Ok(())
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-set-tags-command" => set_tags_command(ed, arg),
        "org-change-tag-in-region" => change_tag_in_region(ed),
        "org-toggle-tags-groups" => {
            let on = !super::options::bool("org-group-tags", true);
            super::options::put("org-group-tags", toml::Value::Boolean(on));
            ed.set_msg(format!("Groups tags support has been turned {}", if on { "on" } else { "off" }));
            Ok(())
        }
        "org-match-sparse-tree" | "org-tags-sparse-tree" => {
            let todo_only = !arg.is_none();
            let tags = buffer_tags(ed);
            super::complete(ed, "Match: ", tags, false, move |ed, m| match_sparse_tree(ed, todo_only, &m));
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{org, shown};
    use super::*;

    fn eval(m: &str, todo: Option<&str>, tags: &[&str], level: usize, props: &[(&str, &str)]) -> bool {
        let st = syntax::settings("#+TODO: TODO NEXT | DONE".lines(), None);
        let tags: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
        let prop = |p: &str| props.iter().find(|(k, _)| k.eq_ignore_ascii_case(p)).map(|(_, v)| v.to_string());
        let not_done = |t: &str| t != "DONE";
        Matcher::new(m, &st, 0).matches(&EntryInfo { todo, tags: &tags, level, prop: &prop, not_done: &not_done })
    }

    #[test]
    fn matcher_language() {
        assert!(eval("work", None, &["work"], 1, &[]));
        assert!(!eval("work-boss", None, &["work", "boss"], 1, &[]));
        assert!(eval("work|home", None, &["home"], 1, &[]));
        assert!(eval("+work+urgent", None, &["work", "urgent"], 1, &[]));
        assert!(eval("{^w}", None, &["work"], 1, &[]));
        assert!(eval("work/TODO", Some("TODO"), &["work"], 1, &[]));
        assert!(!eval("work/TODO", Some("DONE"), &["work"], 1, &[]));
        assert!(eval("/!", Some("NEXT"), &[], 1, &[]));
        assert!(!eval("/!", Some("DONE"), &[], 1, &[]));
        assert!(eval("Effort<2", None, &[], 1, &[("Effort", "1")]));
        assert!(eval("LEVEL=2", None, &[], 2, &[]));
        assert!(eval("WITH={Sarah}", None, &[], 1, &[("WITH", "Sarah, Bob")]));
        assert!(eval("CAT=\"x\"", None, &[], 1, &[("CAT", "x")]));
        assert!(!eval("PROP<>*\"x\"", None, &[], 1, &[]));
        assert!(eval("work-PROP=\"y\"", None, &["work"], 1, &[("PROP", "z")]));
        assert!(eval("TODO=\"NEXT\"", Some("NEXT"), &[], 1, &[]));
        assert_eq!(days_from_civil(1970, 1, 2), 1);
    }

    #[test]
    fn group_tags_expand() {
        let st = syntax::settings("#+TAGS: [ Work : Lab Conf ]".lines(), None);
        let x = expand_groups("Work", &st);
        assert!(x.starts_with("{\\<\\(?:") && x.contains("Lab") && x.contains("Conf"), "{x}");
        let tags = vec!["Lab".to_string()];
        let prop = |_: &str| None;
        let nd = |_: &str| true;
        assert!(Matcher::new("Work", &st, 0).matches(&EntryInfo { todo: None, tags: &tags, level: 1, prop: &prop, not_done: &nd }));
    }

    #[test]
    fn set_and_inherit_tags() {
        let mut e = org("#+FILETAGS: :f:\n* A :a:\n** B :b:", "");
        set_tags(&mut e, 2, &["b".into(), "c".into()]);
        assert!(e.buf.line(2).ends_with(":b:c:"));
        assert_eq!(e.buf.line(2).len(), 77);
        let st = super::super::settings(&e);
        let doc = Doc::new(&e.buf, &st, None);
        assert_eq!(doc.tags(Some(2), false), vec!["f", "a", "b", "c"]);
        set_tags(&mut e, 1, &[]);
        assert_eq!(e.buf.line(1), "* A");
        // Sparse tree through C-c / m is in todo.rs; the matcher here.
        let mut e = org("* A :x:\n** a1\n* B\n** b1 :x:", "");
        match_sparse_tree(&mut e, false, "x");
        assert!(!e.folds.hidden(1));
        assert!(!e.folds.hidden(3));
    }

    #[test]
    fn fast_tag_selection_menu() {
        let e = org("#+TAGS: work(w) home(h)\n* A", "j<C-c><C-q>w<Enter>");
        assert!(e.buf.line(1).ends_with(":work:"), "{}", e.buf.line(1));
        let e = org("#+TAGS: { work(w) home(h) }\n* A :work:", "j<C-c><C-q>h<Enter>");
        assert!(e.buf.line(1).ends_with(":home:"), "{}", e.buf.line(1));
        let e = org("* A", "<C-c><C-q>x:y<Enter>");
        assert!(e.buf.line(0).ends_with(":x:y:"), "{}", e.buf.line(0));
    }
}
