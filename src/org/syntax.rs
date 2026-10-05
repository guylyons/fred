//! Org syntax on buffer lines: in-buffer settings (org-set-regexps-and-options),
//! headlines (org-complex-heading-regexp) and outline navigation.

use super::sexp::Sexp;
use std::collections::HashMap;
use std::ops::Range;

/// How a TODO state change is logged: `!` time, `@` note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Log {
    Time,
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keyword {
    pub name: String,
    pub key: Option<char>,
    pub enter: Option<Log>,
    pub leave: Option<Log>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoSeq {
    /// `type` sequences select by type; `sequence` ones cycle.
    pub is_type: bool,
    pub todo: Vec<Keyword>,
    pub done: Vec<Keyword>,
}

impl TodoSeq {
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.todo.iter().chain(&self.done).map(|k| k.name.as_str())
    }
}

/// An org-tag-alist entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagEntry {
    Tag(String, Option<char>),
    StartGroup,
    EndGroup,
    StartGroupTag,
    EndGroupTag,
    GroupTags,
    Newline,
}

/// What a buffer's keywords and the options say, for this buffer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub seqs: Vec<TodoSeq>,
    pub tags: Vec<TagEntry>,
    pub file_tags: Vec<String>,
    /// `#+PROPERTY` values (and CATEGORY), in order.
    pub properties: Vec<(String, String)>,
    pub archive: Option<String>,
    pub category: Option<String>,
    pub columns: Option<String>,
    pub constants: Vec<(String, String)>,
    pub links: Vec<(String, String)>,
    /// Highest, lowest, default priority values (characters as codes).
    pub priorities: (u32, u32, u32),
    /// Buffer-local values set by `#+STARTUP` and `#+OPTIONS` (^:).
    pub locals: HashMap<String, Sexp>,
    /// Every keyword line: (KEY upcased, value), in order.
    pub keywords: Vec<(String, String)>,
}

impl Settings {
    pub fn todo_names(&self) -> Vec<&str> {
        self.seqs.iter().flat_map(TodoSeq::names).collect()
    }

    pub fn is_todo(&self, w: &str) -> bool {
        self.seqs.iter().any(|s| s.names().any(|n| n == w))
    }

    pub fn is_done(&self, w: &str) -> bool {
        self.seqs.iter().any(|s| s.done.iter().any(|k| k.name == w))
    }

    pub fn done_names(&self) -> Vec<&str> {
        self.seqs
            .iter()
            .flat_map(|s| s.done.iter().map(|k| k.name.as_str()))
            .collect()
    }

    pub fn not_done_names(&self) -> Vec<&str> {
        self.seqs
            .iter()
            .flat_map(|s| s.todo.iter().map(|k| k.name.as_str()))
            .collect()
    }

    /// The sequence a keyword belongs to.
    pub fn seq_of(&self, w: &str) -> Option<&TodoSeq> {
        self.seqs.iter().find(|s| s.names().any(|n| n == w))
    }

    pub fn keyword(&self, w: &str) -> Option<&Keyword> {
        self.seqs
            .iter()
            .flat_map(|s| s.todo.iter().chain(&s.done))
            .find(|k| k.name == w)
    }

    /// An option for this buffer: `#+STARTUP` overrides, then the config.
    pub fn opt(&self, name: &str) -> Option<Sexp> {
        self.locals
            .get(name)
            .cloned()
            .or_else(|| super::sexp::option(name))
    }

    pub fn opt_bool(&self, name: &str, default: bool) -> bool {
        self.opt(name).map_or(default, |v| v.truthy())
    }

    /// A symbol-valued option, as its name ("nil" for nil).
    pub fn opt_sym(&self, name: &str, default: &str) -> String {
        match self.opt(name) {
            Some(Sexp::Nil) => "nil".into(),
            Some(Sexp::T) => "t".into(),
            Some(v) => v.str().map_or_else(|| v.to_string(), str::to_owned),
            None => default.into(),
        }
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.keywords
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// All tags defined by `#+TAGS` or org-tag-alist, without group markers.
    pub fn tag_names(&self) -> Vec<String> {
        self.tags
            .iter()
            .filter_map(|t| match t {
                TagEntry::Tag(n, _) => Some(n.clone()),
                _ => None,
            })
            .collect()
    }
}

/// `"WAIT(w@/!)"` → keyword with key and logging.
pub fn parse_keyword(k: &str) -> Keyword {
    let (name, spec) = match k.find('(') {
        Some(i) if k.ends_with(')') => (&k[..i], &k[i + 1..k.len() - 1]),
        _ => (k, ""),
    };
    let mut chars = spec.chars().peekable();
    let key = chars.peek().copied().filter(|c| !"!@/".contains(*c));
    if key.is_some() {
        chars.next();
    }
    let rest: String = chars.collect();
    let log = |c: char| match c {
        '!' => Some(Log::Time),
        '@' => Some(Log::Note),
        _ => None,
    };
    let (enter, leave) = match rest.split_once('/') {
        Some((a, b)) => (
            a.chars().next().and_then(log),
            b.chars().next().and_then(log),
        ),
        None => (rest.chars().next().and_then(log), None),
    };
    Keyword {
        name: name.to_owned(),
        key,
        enter,
        leave,
    }
}

/// One sequence from its words (`TODO DOING | DONE`).
pub fn todo_seq(is_type: bool, words: &[String]) -> TodoSeq {
    let sep = words.iter().position(|w| w == "|");
    let kws: Vec<Keyword> = words
        .iter()
        .filter(|w| *w != "|")
        .map(|w| parse_keyword(w))
        .collect();
    let split = sep.unwrap_or(kws.len().saturating_sub(1));
    let (todo, done) = kws.split_at(split.min(kws.len()));
    TodoSeq {
        is_type,
        todo: todo.to_vec(),
        done: done.to_vec(),
    }
}

/// org-todo-keywords as sequences.
pub fn option_todo_seqs() -> Vec<TodoSeq> {
    let Some(v) = super::sexp::option("org-todo-keywords") else {
        return vec![todo_seq(false, &["TODO".into(), "DONE".into()])];
    };
    let Some(list) = v.list() else {
        return vec![todo_seq(false, &["TODO".into(), "DONE".into()])];
    };
    // A plain list of keywords: one sequence (org-todo-interpretation).
    if list.iter().all(|x| matches!(x, Sexp::Str(_))) {
        let words: Vec<String> = list
            .iter()
            .filter_map(|x| x.str().map(str::to_owned))
            .collect();
        return vec![todo_seq(false, &words)];
    }
    list.iter()
        .filter_map(|seq| {
            let items = seq.list()?;
            let (head, rest) = items.split_first()?;
            let is_type = head.str() == Some("type");
            let words: Vec<String> = rest
                .iter()
                .filter_map(|x| x.str().map(str::to_owned))
                .collect();
            Some(todo_seq(is_type, &words))
        })
        .collect()
}

/// org-tag-string-to-alist.
pub fn tag_string_to_alist(s: &str) -> Vec<TagEntry> {
    let mut out: Vec<TagEntry> = vec![];
    let mut group = false;
    for line in s.lines().filter(|l| !l.trim().is_empty()) {
        out.push(TagEntry::Newline);
        let tokens: Vec<&str> = line.split_whitespace().collect();
        for (i, tok) in tokens.iter().enumerate() {
            match *tok {
                "{" => {
                    out.push(TagEntry::StartGroup);
                    group = tokens.get(i + 2) == Some(&":");
                }
                "}" => {
                    out.push(TagEntry::EndGroup);
                    group = false;
                }
                "[" => {
                    out.push(TagEntry::StartGroupTag);
                    group = tokens.get(i + 2) == Some(&":");
                }
                "]" => {
                    out.push(TagEntry::EndGroupTag);
                    group = false;
                }
                ":" => out.push(TagEntry::GroupTags),
                tok => {
                    let (name, key) = match tok.find('(') {
                        Some(p) if tok.ends_with(')') && tok.len() == p + 3 => {
                            (&tok[..p], tok[p + 1..].chars().next())
                        }
                        _ => (tok, None),
                    };
                    let valid = name.starts_with('{') && name.ends_with('}')
                        || (!name.is_empty() && name.chars().all(is_tag_char));
                    if valid
                        && (group
                            || !out
                                .iter()
                                .any(|e| matches!(e, TagEntry::Tag(n, _) if n == name)))
                    {
                        out.push(TagEntry::Tag(name.to_owned(), key));
                    }
                }
            }
        }
    }
    if !out.is_empty() {
        out.remove(0);
    }
    out
}

/// org-tag-alist and org-tag-persistent-alist from the config.
fn option_tags(name: &str) -> Vec<TagEntry> {
    let Some(v) = super::sexp::option(name) else {
        return vec![];
    };
    let Some(list) = v.list() else {
        return match v.str() {
            Some(s) => tag_string_to_alist(s),
            None => vec![],
        };
    };
    list.iter()
        .filter_map(|e| {
            let head = e.car().unwrap_or(e);
            Some(match head.sym().or(head.str())? {
                ":startgroup" => TagEntry::StartGroup,
                ":endgroup" => TagEntry::EndGroup,
                ":startgrouptag" => TagEntry::StartGroupTag,
                ":endgrouptag" => TagEntry::EndGroupTag,
                ":grouptags" => TagEntry::GroupTags,
                ":newline" => TagEntry::Newline,
                tag => TagEntry::Tag(
                    tag.to_owned(),
                    e.cdr().int().and_then(|c| char::from_u32(c as u32)),
                ),
            })
        })
        .collect()
}

pub fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || "_@#%".contains(c)
}

/// org-priority-to-value: a letter's code, or a number.
pub fn priority_value(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    let mut c = s.chars();
    let ch = c.next()?;
    c.next().is_none().then_some(ch as u32)
}

/// `#+KEY: value` on a line (affiliated or not): KEY upcased and value.
pub fn keyword_line(line: &str) -> Option<(String, &str)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("#+")?;
    let colon = rest.find(':')?;
    let key = &rest[..colon];
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    Some((key.to_uppercase(), rest[colon + 1..].trim()))
}

/// Settings from a buffer's lines (and SETUPFILE includes, read from disk
/// relative to `dir`).
pub fn settings<'a>(
    lines: impl Iterator<Item = &'a str>,
    dir: Option<&std::path::Path>,
) -> Settings {
    let mut kw: Vec<(String, String)> = vec![];
    let mut seen = vec![];
    collect_keywords(lines, dir, &mut kw, &mut seen);
    let mut st = Settings {
        keywords: kw.clone(),
        ..Settings::default()
    };
    fn all<'a>(kw: &'a [(String, String)], k: &'a str) -> impl Iterator<Item = &'a str> {
        kw.iter()
            .filter(move |(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    }
    let first = |k: &'static str| all(&kw, k).next();
    let all = |k: &'static str| all(&kw, k);
    // Startup options.
    for opt in all("STARTUP").flat_map(str::split_whitespace) {
        if let Some((var, val)) = startup_option(opt) {
            st.locals.insert(var.into(), val);
        }
    }
    for v in all("OPTIONS") {
        for o in v.split_whitespace() {
            if let Some(val) = o.strip_prefix("^:") {
                st.locals.insert(
                    "org-use-sub-superscripts".into(),
                    match val {
                        "t" => Sexp::T,
                        "nil" => Sexp::Nil,
                        _ => Sexp::Sym("{}".into()),
                    },
                );
            }
        }
    }
    st.file_tags = all("FILETAGS")
        .flat_map(|v| v.split_whitespace())
        .flat_map(|t| t.split(':'))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect();
    let tags: Vec<&str> = all("TAGS").collect();
    let mut tag_list = if tags.is_empty() {
        option_tags("org-tag-alist")
    } else {
        tag_string_to_alist(&tags.join("\n"))
    };
    if !st.locals.contains_key("org-tag-persistent-alist") {
        let persistent = option_tags("org-tag-persistent-alist");
        for e in persistent {
            if !matches!(&e, TagEntry::Tag(n, _) if tag_list.iter().any(|x| matches!(x, TagEntry::Tag(m, _) if m == n)))
            {
                tag_list.push(e);
            }
        }
    }
    st.tags = tag_list;
    for v in all("PROPERTY") {
        if let Some((k, val)) = v.split_once(char::is_whitespace) {
            let val = val.trim();
            // `NAME+` appends to a previous value.
            if let Some(base) = k.strip_suffix('+') {
                match st
                    .properties
                    .iter_mut()
                    .find(|(n, _)| n.eq_ignore_ascii_case(base))
                {
                    Some((_, old)) => {
                        old.push(' ');
                        old.push_str(val);
                    }
                    None => st.properties.push((base.into(), val.into())),
                }
            } else {
                st.properties.retain(|(n, _)| !n.eq_ignore_ascii_case(k));
                st.properties.push((k.into(), val.into()));
            }
        }
    }
    st.archive = first("ARCHIVE").map(str::to_owned);
    st.category = first("CATEGORY").map(str::to_owned);
    if let Some(c) = &st.category {
        st.properties.push(("CATEGORY".into(), c.clone()));
    }
    st.columns = first("COLUMNS").map(str::to_owned);
    for pair in all("CONSTANTS").flat_map(str::split_whitespace) {
        if let Some((n, v)) = pair.split_once('=') {
            st.constants.retain(|(m, _)| m != n);
            st.constants.push((n.into(), v.into()));
        }
    }
    for v in all("LINK") {
        let pair = if let Some(rest) = v.strip_prefix('"') {
            rest.find("\" ").map(|i| (&rest[..i], rest[i + 2..].trim()))
        } else {
            v.split_once(char::is_whitespace)
                .map(|(a, b)| (a, b.trim()))
        };
        if let Some((a, b)) = pair {
            st.links.push((a.into(), b.into()));
        }
    }
    let opt_char = |name: &str, d: char| {
        super::sexp::option(name)
            .and_then(|v| {
                v.int()
                    .or_else(|| v.str().and_then(|s| priority_value(s).map(i64::from)))
            })
            .map_or(d as u32, |v| v as u32)
    };
    st.priorities = (
        opt_char("org-priority-highest", 'A'),
        opt_char("org-priority-lowest", 'C'),
        opt_char("org-priority-default", 'B'),
    );
    if let Some(p) = first("PRIORITIES") {
        let v: Vec<&str> = p.split_whitespace().collect();
        if let [h, l, d, ..] = v.as_slice()
            && let (Some(h), Some(l), Some(d)) =
                (priority_value(h), priority_value(l), priority_value(d))
        {
            st.priorities = (h, l, d);
        }
    }
    let typ: Vec<TodoSeq> = all("TYP_TODO")
        .map(|v| {
            todo_seq(
                true,
                &v.split_whitespace().map(str::to_owned).collect::<Vec<_>>(),
            )
        })
        .collect();
    let seq: Vec<TodoSeq> = all("TODO")
        .chain(all("SEQ_TODO"))
        .map(|v| {
            todo_seq(
                false,
                &v.split_whitespace().map(str::to_owned).collect::<Vec<_>>(),
            )
        })
        .collect();
    st.seqs = typ.into_iter().chain(seq).collect();
    st.seqs.retain(|s| !s.todo.is_empty() || !s.done.is_empty());
    if st.seqs.is_empty() {
        st.seqs = option_todo_seqs();
    }
    st
}

fn collect_keywords<'a>(
    lines: impl Iterator<Item = &'a str>,
    dir: Option<&std::path::Path>,
    out: &mut Vec<(String, String)>,
    seen: &mut Vec<std::path::PathBuf>,
) {
    let mut block: Option<String> = None;
    for line in lines {
        let t = line.trim_start();
        if let Some(end) = &block {
            if t.to_ascii_lowercase().starts_with(end) {
                block = None;
            }
            continue;
        }
        let lower = t.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("#+begin_") {
            let name: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            block = Some(format!("#+end_{name}"));
            continue;
        }
        let Some((key, value)) = keyword_line(line) else {
            continue;
        };
        if key == "SETUPFILE" {
            let file = value.trim_matches('"');
            if file.is_empty() || file.contains("://") {
                continue;
            }
            let path = super::options::expand(file);
            let path = match dir {
                Some(d) if path.is_relative() => d.join(path),
                _ => path,
            };
            if seen.contains(&path) {
                continue;
            }
            seen.push(path.clone());
            if let Ok(text) = std::fs::read_to_string(&path) {
                collect_keywords(text.lines(), path.parent(), out, seen);
            }
            continue;
        }
        out.push((key, value.to_owned()));
    }
}

/// org-startup-options: the variable a `#+STARTUP` word sets.
pub fn startup_option(word: &str) -> Option<(&'static str, Sexp)> {
    let sym = |s: &str| Sexp::Sym(s.into());
    Some(match word.to_ascii_lowercase().as_str() {
        w @ ("fold" | "overview" | "nofold" | "showall" | "show2levels" | "show3levels"
        | "show4levels" | "show5levels" | "showeverything" | "content") => {
            ("org-startup-folded", sym(w))
        }
        "indent" => ("org-startup-indented", Sexp::T),
        "noindent" => ("org-startup-indented", Sexp::Nil),
        "num" => ("org-startup-numerated", Sexp::T),
        "nonum" => ("org-startup-numerated", Sexp::Nil),
        "hidestars" => ("org-hide-leading-stars", Sexp::T),
        "showstars" => ("org-hide-leading-stars", Sexp::Nil),
        "odd" => ("org-odd-levels-only", Sexp::T),
        "oddeven" => ("org-odd-levels-only", Sexp::Nil),
        "align" => ("org-startup-align-all-tables", Sexp::T),
        "noalign" => ("org-startup-align-all-tables", Sexp::Nil),
        "shrink" => ("org-startup-shrink-all-tables", Sexp::T),
        "descriptivelinks" => ("org-link-descriptive", Sexp::T),
        "literallinks" => ("org-link-descriptive", Sexp::Nil),
        "inlineimages" | "linkpreviews" => ("org-startup-with-link-previews", Sexp::T),
        "noinlineimages" | "nolinkpreviews" => ("org-startup-with-link-previews", Sexp::Nil),
        "latexpreview" => ("org-startup-with-latex-preview", Sexp::T),
        "nolatexpreview" => ("org-startup-with-latex-preview", Sexp::Nil),
        "customtime" => ("org-display-custom-times", Sexp::T),
        "logdone" => ("org-log-done", sym("time")),
        "lognotedone" => ("org-log-done", sym("note")),
        "nologdone" => ("org-log-done", Sexp::Nil),
        "lognoteclock-out" => ("org-log-note-clock-out", Sexp::T),
        "nolognoteclock-out" => ("org-log-note-clock-out", Sexp::Nil),
        "logrepeat" => ("org-log-repeat", sym("state")),
        "lognoterepeat" => ("org-log-repeat", sym("note")),
        "nologrepeat" => ("org-log-repeat", Sexp::Nil),
        "logdrawer" => ("org-log-into-drawer", Sexp::T),
        "nologdrawer" => ("org-log-into-drawer", Sexp::Nil),
        "logstatesreversed" => ("org-log-states-order-reversed", Sexp::T),
        "nologstatesreversed" => ("org-log-states-order-reversed", Sexp::Nil),
        "logreschedule" => ("org-log-reschedule", sym("time")),
        "lognotereschedule" => ("org-log-reschedule", sym("note")),
        "nologreschedule" => ("org-log-reschedule", Sexp::Nil),
        "logredeadline" => ("org-log-redeadline", sym("time")),
        "lognoteredeadline" => ("org-log-redeadline", sym("note")),
        "nologredeadline" => ("org-log-redeadline", Sexp::Nil),
        "logrefile" => ("org-log-refile", sym("time")),
        "lognoterefile" => ("org-log-refile", sym("note")),
        "nologrefile" => ("org-log-refile", Sexp::Nil),
        "fninline" => ("org-footnote-define-inline", Sexp::T),
        "nofninline" => ("org-footnote-define-inline", Sexp::Nil),
        "fnlocal" => ("org-footnote-section", Sexp::Nil),
        "fnauto" => ("org-footnote-auto-label", Sexp::T),
        "fnprompt" => ("org-footnote-auto-label", Sexp::Nil),
        "fnconfirm" => ("org-footnote-auto-label", sym("confirm")),
        "fnplain" => ("org-footnote-auto-label", sym("plain")),
        "fnadjust" => ("org-footnote-auto-adjust", Sexp::T),
        "nofnadjust" => ("org-footnote-auto-adjust", Sexp::Nil),
        "fnanon" => ("org-footnote-auto-label", sym("anonymous")),
        "constcgs" => ("constants-unit-system", sym("cgs")),
        "constsi" => ("constants-unit-system", sym("SI")),
        "noptag" => ("org-tag-persistent-alist", Sexp::Nil),
        "hideblocks" => ("org-hide-block-startup", Sexp::T),
        "nohideblocks" => ("org-hide-block-startup", Sexp::Nil),
        "hidedrawers" => ("org-hide-drawer-startup", Sexp::T),
        "nohidedrawers" => ("org-hide-drawer-startup", Sexp::Nil),
        "beamer" => ("org-startup-with-beamer-mode", Sexp::T),
        "entitiespretty" => ("org-pretty-entities", Sexp::T),
        "entitiesplain" => ("org-pretty-entities", Sexp::Nil),
        _ => return None,
    })
}

/// A parsed headline line; ranges are byte offsets in the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Headline {
    pub level: usize,
    pub todo: Option<String>,
    pub todo_range: Option<Range<usize>>,
    pub priority: Option<u32>,
    pub priority_range: Option<Range<usize>>,
    /// The title, after keyword, priority and COMMENT, before tags.
    pub title: Range<usize>,
    pub tags: Vec<String>,
    /// From the space before the tags to the end of the line.
    pub tags_range: Option<Range<usize>>,
    pub commented: bool,
}

impl Headline {
    pub fn title<'a>(&self, line: &'a str) -> &'a str {
        &line[self.title.clone()]
    }

    pub fn archived(&self) -> bool {
        self.tags.iter().any(|t| t == "ARCHIVE")
    }
}

/// Stars of a headline line (`* `), else None.
pub fn level(line: &str) -> Option<usize> {
    let n = line.bytes().take_while(|&b| b == b'*').count();
    (n > 0 && line.as_bytes().get(n) == Some(&b' ')).then_some(n)
}

/// org-complex-heading-regexp on one line.
pub fn headline(line: &str, st: &Settings) -> Option<Headline> {
    let lvl = level(line)?;
    let mut pos = lvl;
    let skip_spaces = |p: usize| p + line[p..].bytes().take_while(|&b| b == b' ').count();
    pos = skip_spaces(pos);
    let word_end = |p: usize| p + line[p..].find([' ', '\t']).unwrap_or(line.len() - p);
    let mut todo = None;
    let mut todo_range = None;
    let w = word_end(pos);
    if w > pos && st.is_todo(&line[pos..w]) && (w == line.len() || line.as_bytes()[w] == b' ') {
        todo = Some(line[pos..w].to_owned());
        todo_range = Some(pos..w);
        pos = skip_spaces(w);
    }
    let mut priority = None;
    let mut priority_range = None;
    if line[pos..].starts_with("[#")
        && let Some(close) = line[pos + 2..].find(']')
    {
        let v = &line[pos + 2..pos + 2 + close];
        let valid = (v.len() == 1 && v.as_bytes()[0].is_ascii_uppercase())
            || v.parse::<u32>().is_ok_and(|n| n <= 64);
        let end = pos + 3 + close;
        if valid && (end == line.len() || line.as_bytes()[end] == b' ') {
            priority = priority_value(v);
            priority_range = Some(pos..end);
            pos = skip_spaces(end);
        }
    }
    // Tags: `:a:b:` at the end, preceded by whitespace.
    let trimmed = line.trim_end_matches([' ', '\t']);
    let mut tags = vec![];
    let mut tags_range = None;
    let mut end = trimmed.len();
    if trimmed.ends_with(':') && trimmed.len() > lvl {
        let start = trimmed[lvl..].rfind([' ', '\t']).map(|i| i + lvl + 1);
        if let Some(s) = start {
            let group = &trimmed[s..];
            if group.len() >= 3
                && group.starts_with(':')
                && group[1..group.len() - 1]
                    .chars()
                    .all(|c| c == ':' || is_tag_char(c))
            {
                tags = group
                    .split(':')
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
                    .collect();
                let ws = trimmed[..s].trim_end_matches([' ', '\t']).len().max(lvl);
                tags_range = Some(ws..line.len());
                end = ws;
                pos = pos.min(end);
            }
        }
    }
    let end = end.max(pos);
    let mut title_start = pos;
    let comment = super::options::string("org-comment-string", "COMMENT");
    let commented = line[pos..end] == comment || line[pos..end].starts_with(&format!("{comment} "));
    if commented {
        title_start = skip_spaces((pos + comment.len()).min(end));
    }
    Some(Headline {
        level: lvl,
        todo,
        todo_range,
        priority,
        priority_range,
        title: title_start.min(end)..end,
        tags,
        tags_range,
        commented,
    })
}

/// Lines of a buffer, for navigation helpers.
pub trait Lines {
    fn n_lines(&self) -> usize;
    fn line_text(&self, i: usize) -> String;
}

impl Lines for crate::buffer::Buffer {
    fn n_lines(&self) -> usize {
        self.len_lines()
    }
    fn line_text(&self, i: usize) -> String {
        self.line(i)
    }
}

impl Lines for [String] {
    fn n_lines(&self) -> usize {
        self.len()
    }
    fn line_text(&self, i: usize) -> String {
        self[i].clone()
    }
}

impl Lines for Vec<String> {
    fn n_lines(&self) -> usize {
        self.len()
    }
    fn line_text(&self, i: usize) -> String {
        self[i].clone()
    }
}

/// The headline at or before line `l` (org-back-to-heading).
pub fn heading_at_or_before<L: Lines + ?Sized>(b: &L, l: usize) -> Option<usize> {
    (0..=l.min(b.n_lines().saturating_sub(1)))
        .rev()
        .find(|&i| level(&b.line_text(i)).is_some())
}

/// The next headline after `l` with level <= `lvl` (or any, with usize::MAX).
pub fn next_heading<L: Lines + ?Sized>(b: &L, l: usize, max_level: usize) -> Option<usize> {
    (l + 1..b.n_lines()).find(|&i| level(&b.line_text(i)).is_some_and(|n| n <= max_level))
}

/// The line after a subtree starting at heading `h` (org-end-of-subtree).
pub fn subtree_end<L: Lines + ?Sized>(b: &L, h: usize) -> usize {
    let lvl = level(&b.line_text(h)).unwrap_or(0);
    next_heading(b, h, lvl).unwrap_or(b.n_lines())
}

/// The line after heading `h`'s own entry: the next heading of any level.
pub fn entry_end<L: Lines + ?Sized>(b: &L, h: usize) -> usize {
    next_heading(b, h, usize::MAX).unwrap_or(b.n_lines())
}

/// Parent heading of the heading at `h`.
pub fn parent<L: Lines + ?Sized>(b: &L, h: usize) -> Option<usize> {
    let lvl = level(&b.line_text(h))?;
    (0..h)
        .rev()
        .find(|&i| level(&b.line_text(i)).is_some_and(|n| n < lvl))
}

/// Direct children headings of `h`.
pub fn children<L: Lines + ?Sized>(b: &L, h: usize) -> Vec<usize> {
    let end = subtree_end(b, h);
    let mut out = vec![];
    let mut min: Option<usize> = None;
    for i in h + 1..end {
        if let Some(n) = level(&b.line_text(i)) {
            let m = *min.get_or_insert(n);
            if n <= m {
                min = Some(n);
                out.retain(|&c| level(&b.line_text(c)).is_some_and(|x| x <= n));
                out.push(i);
            }
        }
    }
    out
}

/// Next sibling heading (same level, same parent).
pub fn next_sibling<L: Lines + ?Sized>(b: &L, h: usize) -> Option<usize> {
    let lvl = level(&b.line_text(h))?;
    let e = subtree_end(b, h);
    (e < b.n_lines() && level(&b.line_text(e)) == Some(lvl)).then_some(e)
}

/// Previous sibling heading.
pub fn prev_sibling<L: Lines + ?Sized>(b: &L, h: usize) -> Option<usize> {
    let lvl = level(&b.line_text(h))?;
    for i in (0..h).rev() {
        if let Some(n) = level(&b.line_text(i)) {
            if n < lvl {
                return None;
            }
            if n == lvl {
                return Some(i);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_and_settings() {
        let text = "#+TODO: TODO(t) WAIT(w@/!) | DONE(d!) CANCELED(c@)\n#+STARTUP: content logdone\n#+TAGS: { @work(w) @home(h) } laptop\n#+PRIORITIES: 1 5 3\n#+begin_src org\n#+TODO: NOPE\n#+end_src\n#+PROPERTY: Effort_ALL 0 1\n#+PROPERTY: Effort_ALL+ 2\n";
        let st = settings(text.lines(), None);
        assert_eq!(st.todo_names(), vec!["TODO", "WAIT", "DONE", "CANCELED"]);
        assert_eq!(st.done_names(), vec!["DONE", "CANCELED"]);
        let wait = st.keyword("WAIT").unwrap();
        assert_eq!(
            (wait.key, wait.enter, wait.leave),
            (Some('w'), Some(Log::Note), Some(Log::Time))
        );
        assert_eq!(
            st.opt_sym("org-startup-folded", "showeverything"),
            "content"
        );
        assert_eq!(st.opt_sym("org-log-done", "nil"), "time");
        assert_eq!(st.tag_names(), vec!["@work", "@home", "laptop"]);
        assert_eq!(st.priorities, (1, 5, 3));
        assert_eq!(st.properties, vec![("Effort_ALL".into(), "0 1 2".into())]);
        // Without #+TODO: the default sequence.
        let st = settings("* a".lines(), None);
        assert_eq!(st.todo_names(), vec!["TODO", "DONE"]);
        assert_eq!(
            todo_seq(false, &["A".into(), "B".into(), "C".into()])
                .done
                .len(),
            1
        );
    }

    #[test]
    fn headlines() {
        let st = settings("".lines(), None);
        let l = "** TODO [#A] COMMENT Write it   :work:x@y:";
        let h = headline(l, &st).unwrap();
        assert_eq!(h.level, 2);
        assert_eq!(h.todo.as_deref(), Some("TODO"));
        assert_eq!(h.priority, Some('A' as u32));
        assert!(h.commented);
        assert_eq!(h.title(l), "Write it");
        assert_eq!(h.tags, vec!["work", "x@y"]);
        assert_eq!(&l[h.tags_range.clone().unwrap()], "   :work:x@y:");
        let l = "* TODOs are fun";
        let h = headline(l, &st).unwrap();
        assert_eq!((h.todo.clone(), h.title(l)), (None, "TODOs are fun"));
        assert!(headline("*bold*", &st).is_none());
        let l = "* a:b: c";
        assert!(headline(l, &st).unwrap().tags.is_empty());
        let l = "* :tag:";
        let h = headline(l, &st).unwrap();
        assert_eq!((h.title(l), h.tags.len()), ("", 1));
    }

    #[test]
    fn navigation() {
        let b: Vec<String> = "* a\nx\n** b\n*** c\n** d\n* e"
            .lines()
            .map(String::from)
            .collect();
        assert_eq!(subtree_end(&b, 0), 5);
        assert_eq!(subtree_end(&b, 2), 4);
        assert_eq!(children(&b, 0), vec![2, 4]);
        assert_eq!(parent(&b, 3), Some(2));
        assert_eq!(next_sibling(&b, 2), Some(4));
        assert_eq!(prev_sibling(&b, 4), Some(2));
        assert_eq!(prev_sibling(&b, 2), None);
        assert_eq!(heading_at_or_before(&b, 1), Some(0));
    }
}
