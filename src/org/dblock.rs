//! Dynamic blocks (org.el, "Dynamic blocks"): `#+BEGIN: NAME PARAMS`
//! ... `#+END:`, org-update-dblock and friends, the registered block
//! types (clocktable from org-clock.el, columnview from org-colview.el),
//! and the org-table-align the writers use for their tables.
//!
//! A writer gets the parameters and an [`Out`] standing for the emptied
//! block (one open line, point at its start, as org-prepare-dblock
//! leaves it) and inserts text there as the Lisp writers do.

use super::sexp::{self, Sexp};
use super::{Prefix, line};
use crate::editor::Editor;

/// A block's parameter plist, with :name, :indentation-column and
/// :content added as org-prepare-dblock does.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Params(pub Vec<(String, Sexp)>);

impl Params {
    /// Read `KEY VALUE ...` as `(read (concat "(" s ")"))`.
    pub fn parse(s: &str) -> Result<Params, String> {
        let v = sexp::read(&format!("({s})"))
            .map_err(|e| format!("Invalid dynamic block parameters: {e}"))?;
        Ok(Params::from_sexp(&v))
    }

    pub fn from_sexp(v: &Sexp) -> Params {
        let items = v.list().unwrap_or(&[]);
        let mut out = Params::default();
        let mut i = 0;
        while i < items.len() {
            if let Some(k) = items[i].sym() {
                out.0
                    .push((k.to_owned(), items.get(i + 1).cloned().unwrap_or(Sexp::Nil)));
            }
            i += 2;
        }
        out
    }

    /// plist-get.
    pub fn get(&self, k: &str) -> Option<&Sexp> {
        self.0.iter().find(|(x, _)| x == k).map(|(_, v)| v)
    }

    /// plist-member.
    pub fn has(&self, k: &str) -> bool {
        self.0.iter().any(|(x, _)| x == k)
    }

    /// plist-put.
    pub fn set(&mut self, k: &str, v: Sexp) {
        match self.0.iter_mut().find(|(x, _)| x == k) {
            Some(e) => e.1 = v,
            None => self.0.push((k.to_owned(), v)),
        }
    }

    /// The value is non-nil.
    pub fn on(&self, k: &str) -> bool {
        self.get(k).is_some_and(Sexp::truthy)
    }

    /// A string or a symbol's name (nil: None).
    pub fn str(&self, k: &str) -> Option<String> {
        match self.get(k)? {
            Sexp::Nil => None,
            Sexp::Int(i) => Some(i.to_string()),
            v => v
                .str()
                .map(str::to_owned)
                .or_else(|| v.sym().map(str::to_owned)),
        }
    }

    pub fn int(&self, k: &str) -> Option<i64> {
        match self.get(k)? {
            Sexp::Int(i) => Some(*i),
            Sexp::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    /// org-combine-plists: later values win, first-seen key order.
    pub fn combine(&self, over: &Params) -> Params {
        let mut out = self.clone();
        for (k, v) in &over.0 {
            out.set(k, v.clone());
        }
        out
    }

    /// The `#+BEGIN:` line's parameters as org-create-dblock prints them.
    pub fn print(&self) -> String {
        self.0
            .iter()
            .filter(|(k, _)| k != ":name")
            .map(|(k, v)| format!(" {k} {v}"))
            .collect()
    }
}

/// The emptied block a writer fills: `text` holds the open line's
/// newline; `pt` is point; `line` is the buffer line of the block body.
pub struct Out {
    pub text: String,
    pub pt: usize,
    pub line: usize,
}

impl Out {
    pub fn new(line: usize) -> Out {
        Out {
            text: "\n".into(),
            pt: 0,
            line,
        }
    }

    /// insert (-before-markers): point moves after the text.
    pub fn insert(&mut self, s: &str) {
        self.text.insert_str(self.pt, s);
        self.pt += s.len();
    }

    pub fn bolp(&self) -> bool {
        self.pt == 0 || self.text.as_bytes()[self.pt - 1] == b'\n'
    }

    /// (line-beginning-position 0): the start of the previous line.
    pub fn prev_bol(&self) -> usize {
        let bol = self.text[..self.pt].rfind('\n').map_or(0, |i| i + 1);
        if bol == 0 {
            0
        } else {
            self.text[..bol - 1].rfind('\n').map_or(0, |i| i + 1)
        }
    }

    /// The lines that end up between #+BEGIN: and #+END:.
    pub fn lines(&self) -> Vec<String> {
        self.text
            .strip_suffix('\n')
            .unwrap_or(&self.text)
            .split('\n')
            .map(str::to_owned)
            .collect()
    }
}

type Writer = fn(&mut Editor, &Params, &mut Out) -> Result<(), String>;

/// org-dynamic-block-alist (most recently defined first) and the
/// org-dblock-write: functions.
const TYPES: &[(&str, &str, Writer)] = &[
    (
        "columnview",
        "org-columns-insert-dblock",
        super::colview::dblock_write,
    ),
    (
        "clocktable",
        "org-clock-report",
        super::clock::table::dblock_write,
    ),
];

pub fn start_re() -> &'static regex::Regex {
    crate::org_re!(r"^[ \t]*#\+(?:BEGIN|begin):[ \t]+(\S+)(?:[ \t]+(.*))?")
}

pub fn end_re() -> &'static regex::Regex {
    crate::org_re!(r"^[ \t]*#\+(?:END|end)(?:[: \t]|$)")
}

/// org-beginning-of-dblock: the #+BEGIN: line of the block around `l`.
pub fn begin_of(ed: &Editor, l: usize) -> Option<usize> {
    let b = (0..=l).rev().find(|&i| start_re().is_match(&line(ed, i)))?;
    let e = (b + 1..ed.line_count()).find(|&i| end_re().is_match(&line(ed, i)))?;
    (e >= l).then_some(b)
}

/// org-find-dblock: the first block named `name`.
pub fn find(ed: &Editor, name: &str) -> Option<usize> {
    let re = regex::Regex::new(&format!(
        r"(?i)^[ \t]*#\+BEGIN:[ \t]+{}\b",
        regex::escape(name)
    ))
    .ok()?;
    (0..ed.line_count()).find(|&i| re.is_match(&line(ed, i)))
}

/// org-create-dblock: a new block named `name` after the cursor line
/// (or on it when blank); the cursor goes to its #+BEGIN: line.
pub fn create(ed: &mut Editor, name: &str, params: &Params) -> usize {
    let l = ed.cur.line;
    let t = line(ed, l);
    let (at, col) = if t.trim().is_empty() {
        (l, ed.cur.byte.min(t.len()))
    } else {
        super::insert_lines(ed, l + 1, &[String::new()]);
        (l + 1, 0)
    };
    let t = line(ed, at);
    let lines = vec![
        format!("{}#+BEGIN: {name}{}", &t[..col], params.print()),
        String::new(),
        format!("{}#+END:", " ".repeat(col)),
        t[col..].to_owned(),
    ];
    super::splice(ed, at, 1, &lines);
    ed.set_cursor(at, col);
    at
}

/// org-update-dblock on the block starting at line `l`.
pub fn update(ed: &mut Editor, l: usize) -> Result<(), String> {
    let text = line(ed, l);
    let c = start_re().captures(&text).ok_or("Not at a dynamic block")?;
    let name = c[1].to_owned();
    let given = Params::parse(c.get(2).map_or("", |m| m.as_str()))?;
    let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
    let end = (l + 1..ed.line_count())
        .find(|&i| end_re().is_match(&line(ed, i)))
        .ok_or("Dynamic block not terminated")?;
    let content: String = (l + 1..end).map(|i| line(ed, i) + "\n").collect();
    let mut params = Params(vec![(":name".into(), Sexp::Str(name.clone()))]);
    params.0.extend(given.0);
    params.set(":indentation-column", Sexp::Int(indent as i64));
    params.set(":content", Sexp::Str(content));
    let writer = TYPES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, _, w)| *w)
        .ok_or_else(|| format!("Symbol’s function definition is void: org-dblock-write:{name}"))?;
    let (cl, cb) = (ed.cur.line, ed.cur.byte);
    super::splice(ed, l + 1, end - l - 1, &[String::new()]);
    let mut out = Out::new(l + 1);
    let r = writer(ed, &params, &mut out);
    let pad = " ".repeat(indent);
    let lines: Vec<String> = out
        .lines()
        .into_iter()
        .map(|x| format!("{pad}{x}"))
        .collect();
    let n = lines.len();
    super::splice(ed, l + 1, 1, &lines);
    if indent > 0 {
        let e = line(ed, l + 1 + n);
        super::set_line(
            ed,
            l + 1 + n,
            &format!("{pad}{}", e.trim_start_matches([' ', '\t'])),
        );
    }
    if cl == l {
        ed.set_cursor(l, cb.min(line(ed, l).len()));
    } else {
        ed.set_cursor(cl.min(ed.line_count() - 1), 0);
    }
    r?;
    ed.set_msg(format!(
        "Updating dynamic block `{name}' at line {}...done",
        l + 1
    ));
    Ok(())
}

/// org-update-all-dblocks (org-map-dblocks with org-update-dblock).
pub fn update_all(ed: &mut Editor) {
    let mut l = 0;
    while l < ed.line_count() {
        if start_re().is_match(&line(ed, l)) {
            if update(ed, l).is_err() {
                ed.set_msg("Error during update of dynamic block");
            }
            match (l + 1..ed.line_count()).find(|&i| end_re().is_match(&line(ed, i))) {
                Some(e) => l = e,
                None => {
                    ed.set_err("Dynamic block not terminated");
                    return;
                }
            }
        }
        l += 1;
    }
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    let l = ed.cur.line;
    Some(match name {
        "org-update-dblock" => update(ed, l),
        "org-dblock-update" => {
            if !arg.is_none() {
                update_all(ed);
                Ok(())
            } else {
                match begin_of(ed, l) {
                    Some(b) => update(ed, b),
                    None => Err("Not in a dynamic block".into()),
                }
            }
        }
        "org-update-all-dblocks" => {
            update_all(ed);
            Ok(())
        }
        "org-dynamic-block-insert-dblock" => {
            let given = super::take_call_arg();
            let go = |ed: &mut Editor, t: String| {
                ed.undo.begin(ed.cur.pos());
                let r = match TYPES.iter().find(|(n, _, _)| *n == t) {
                    Some((_, cmd, _)) => super::call(ed, cmd, Prefix::None),
                    None => Err(format!("No such dynamic block: \"{t}\"")),
                };
                ed.undo.end(ed.cur.pos());
                if let Err(e) = r {
                    ed.set_err(e);
                }
            };
            match given {
                Some(t) => go(ed, t),
                None => {
                    let types = TYPES.iter().map(|(n, _, _)| (*n).to_owned()).collect();
                    super::complete(ed, "Dynamic block: ", types, false, go);
                }
            }
            Ok(())
        }
        _ => return None,
    })
}

// ---- org-table-align, for the tables writers produce ----

/// org-string-width: links count as their description.
pub fn string_width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(super::links::display_format(s).as_str())
}

fn number_re() -> regex::Regex {
    let d = r"^([<>]?[-+^.0-9]*[0-9][-+^.0-9eEdDx()%:]*|[<>]?[-+]?0[xX][[:xdigit:].]+|[<>]?[-+]?[0-9]+#[0-9a-zA-Z.]+|nan|[-+u]?inf)$";
    super::options::get("org-table-number-regexp")
        .and_then(|v| v.as_str().and_then(|r| super::re::compile(r, false).ok()))
        .unwrap_or_else(|| regex::Regex::new(d).unwrap())
}

/// A table line's fields (org-table-to-lisp), None for a rule.
pub fn parse_row(s: &str) -> Option<Vec<String>> {
    let t = s.trim_start_matches([' ', '\t']).strip_prefix('|')?;
    if t.starts_with('-') {
        return None;
    }
    let mut out = vec![];
    let mut rest = t;
    loop {
        rest = rest.trim_start_matches([' ', '\t']);
        if rest.is_empty() {
            break;
        }
        let e = rest.find('|').unwrap_or(rest.len());
        out.push(rest[..e].trim_end_matches([' ', '\t']).to_owned());
        rest = if e < rest.len() { &rest[e + 1..] } else { "" };
    }
    Some(out)
}

/// org-table-align on the table `rows` (lines starting with `|`).
pub fn align(rows: &[String]) -> Vec<String> {
    let Some(first) = rows.first() else {
        return vec![];
    };
    let indent = &first[..first.len() - first.trim_start_matches([' ', '\t']).len()];
    let table: Vec<Option<Vec<String>>> = rows.iter().map(|r| parse_row(r)).collect();
    let data: Vec<&Vec<String>> = table.iter().flatten().collect();
    let ncols = data.iter().map(|r| r.len()).max().unwrap_or(1).max(1);
    let fraction = super::options::get("org-table-number-fraction")
        .and_then(|v| v.as_float().or(v.as_integer().map(|i| i as f64)))
        .unwrap_or(0.5);
    let num = number_re();
    let fixed = crate::org_re!(r"^<([lrc])[0-9]*>$");
    let mut widths = vec![1; ncols];
    let mut aligns = vec!['l'; ncols];
    for i in 0..ncols {
        let (mut numbers, mut non_empty, mut fix) = (0usize, 0usize, None);
        for r in &data {
            let cell = r.get(i).map_or("", String::as_str);
            widths[i] = widths[i].max(string_width(cell));
            if fix.is_some() || cell.is_empty() {
                continue;
            }
            if let Some(c) = fixed.captures(cell) {
                fix = c[1].chars().next();
                continue;
            }
            non_empty += 1;
            if num.is_match(cell) {
                numbers += 1;
            }
        }
        aligns[i] = fix.unwrap_or(if data.is_empty() {
            'l'
        } else if numbers as f64 >= fraction * non_empty as f64 {
            'r'
        } else {
            'l'
        });
    }
    let rule = widths
        .iter()
        .map(|w| "-".repeat(w + 2))
        .collect::<Vec<_>>()
        .join("+");
    table
        .iter()
        .map(|r| match r {
            None => format!("{indent}|{rule}|"),
            Some(cells) => {
                let fields: Vec<String> = (0..ncols)
                    .map(|i| {
                        let f = cells.get(i).map_or("", String::as_str);
                        let spaces = widths[i].saturating_sub(string_width(f));
                        let pre = match aligns[i] {
                            'r' => spaces,
                            'c' => spaces / 2,
                            _ => 0,
                        };
                        format!(" {}{f}{} ", " ".repeat(pre), " ".repeat(spaces - pre))
                    })
                    .collect();
                format!("{indent}|{}|", fields.join("|"))
            }
        })
        .collect()
}

/// org-table-sort-lines on aligned `rows`: the lines between the rules
/// around data line `n` (1-based, rules not counted), by column `col`
/// (1-based) and `kind` (a n t, upper case reversed).
pub fn sort_rows(rows: &mut [String], n: usize, col: usize, kind: char) {
    let data: Vec<usize> = (0..rows.len())
        .filter(|&i| parse_row(&rows[i]).is_some())
        .collect();
    let Some(&at) = data.get(n.saturating_sub(1)) else {
        return;
    };
    let lo = (0..at)
        .rev()
        .find(|&i| parse_row(&rows[i]).is_none())
        .map_or(0, |i| i + 1);
    let hi = (at..rows.len())
        .find(|&i| parse_row(&rows[i]).is_none())
        .unwrap_or(rows.len());
    let key = |r: &String| -> String {
        let c = parse_row(r)
            .and_then(|c| c.get(col.saturating_sub(1)).cloned())
            .unwrap_or_default();
        super::links::display_format(&c)
    };
    let num = |s: &str| -> f64 {
        let t = s.trim_matches(['*', '/', '_', '=', '~', '+']);
        if let Ok(m) = super::time::duration::to_minutes(t, false)
            && t.contains(':')
        {
            return m;
        }
        t.parse().unwrap_or(0.0)
    };
    let lower = kind.to_ascii_lowercase();
    let slice = &mut rows[lo..hi];
    slice.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        let o = match lower {
            'n' => num(&ka)
                .partial_cmp(&num(&kb))
                .unwrap_or(std::cmp::Ordering::Equal),
            't' => {
                let t = |s: &str| {
                    super::time::stamp::time_string_to_seconds(s)
                        .map_or_else(|_| num(s), |x| x as f64)
                };
                t(&ka)
                    .partial_cmp(&t(&kb))
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
            _ => ka.to_lowercase().cmp(&kb.to_lowercase()),
        };
        if kind.is_ascii_uppercase() {
            o.reverse()
        } else {
            o
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn align_numbers_right_when_most_are_numbers() {
        let t = align(&v(&[
            "|Headline|Time||",
            "|-",
            "|*Total time*| *1:00*||",
            "|-",
            "|H1|1:30||",
            "|\\_  Sub||0:30|",
        ]));
        assert_eq!(
            t,
            v(&[
                "| Headline     | Time   |      |",
                "|--------------+--------+------|",
                "| *Total time* | *1:00* |      |",
                "|--------------+--------+------|",
                "| H1           | 1:30   |      |",
                "| \\_  Sub      |        | 0:30 |",
            ])
        );
        let t = align(&v(&["| a | b |", "| [[x][H]] | 1 |", "| y | 22 |"]));
        assert_eq!(t[1], "| [[x][H]] |  1 |");
    }

    #[test]
    fn params_read_and_print() {
        let p = Params::parse(
            ":scope file :maxlevel 2 :block 2017-W40 :sort (2 . ?a) :tstart \"<-2d>\"",
        )
        .unwrap();
        assert_eq!(p.str(":scope").as_deref(), Some("file"));
        assert_eq!(p.int(":maxlevel"), Some(2));
        assert_eq!(p.str(":block").as_deref(), Some("2017-W40"));
        assert_eq!(
            p.print(),
            " :scope file :maxlevel 2 :block 2017-W40 :sort (2 . 97) :tstart \"<-2d>\""
        );
    }

    #[test]
    fn create_and_update_keep_indentation() {
        let mut e = super::super::tests::org("* H\n", "");
        let p = Params::parse(":scope subtree :maxlevel 2").unwrap();
        create(&mut e, "clocktable", &p);
        assert_eq!(
            e.buf.text(),
            "* H\n#+BEGIN: clocktable :scope subtree :maxlevel 2\n\n#+END:\n"
        );
        assert_eq!(e.cur.line, 1);
        let mut e = super::super::tests::org("* H\n  #+BEGIN: nosuch\n  old\n  #+END:\n", "j");
        assert!(update(&mut e, 1).is_err());
        assert!(begin_of(&e, 2) == Some(1) && begin_of(&e, 0).is_none());
    }
}
