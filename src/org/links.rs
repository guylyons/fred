//! Links (ol.el and the link parts of org.el): parsing at point, storing,
//! inserting, opening (files, URLs, internal targets, IDs, shell, elisp),
//! the mark ring and link navigation.

use super::sexp::Sexp;
use super::syntax::{self, Settings};
use super::{Prefix, ctx, fold, props};
use crate::editor::Editor;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    /// org-stored-links: (link, description), newest first.
    static STORED: RefCell<Vec<(String, Option<String>)>> = const { RefCell::new(vec![]) };
    /// org-mark-ring: (file, line), newest first, and the last goto index.
    static RING: RefCell<(Vec<(Option<PathBuf>, usize)>, Option<usize>)> = const { RefCell::new((vec![], None)) };
    /// org-link--insert-history.
    static HISTORY: RefCell<Vec<String>> = const { RefCell::new(vec![]) };
}

pub fn stored_links() -> Vec<(String, Option<String>)> {
    STORED.with(|s| s.borrow().clone())
}

/// Built-in link types (org-link-parameters), plus configured ones.
pub fn link_types() -> Vec<String> {
    let mut v: Vec<String> = [
        "attachment", "bbdb", "bibtex", "docview", "doi", "elisp", "eshell", "eww", "file", "file+emacs", "file+sys",
        "ftp", "gnus", "help", "http", "https", "id", "info", "irc", "mailto", "man", "mhe", "news", "rmail", "shell",
        "shortdoc", "w3m",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(p) = super::sexp::option("org-link-parameters").and_then(|v| v.list().map(<[Sexp]>::to_vec)) {
        for e in p {
            if let Some(t) = e.car().and_then(Sexp::str)
                && !v.iter().any(|x| x == t)
            {
                v.push(t.to_owned());
            }
        }
    }
    v
}

/// A link as org-element parses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// file, fuzzy, custom-id, coderef, radio, http, id, ...
    pub kind: String,
    pub path: String,
    pub search: Option<String>,
    /// `file+sys` / `file+emacs` application.
    pub application: Option<String>,
    pub desc: Option<String>,
    /// The raw link text (`[[...]]` contents or the plain link).
    pub raw: String,
    pub range: std::ops::Range<usize>,
}

/// org-link-unescape.
pub fn unescape(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            let mut j = i;
            while j < chars.len() && chars[j] == '\\' {
                j += 1;
            }
            let n = j - i;
            if j == chars.len() || chars[j] == '[' || chars[j] == ']' {
                out.extend(std::iter::repeat_n('\\', n / 2));
            } else {
                out.extend(std::iter::repeat_n('\\', n));
            }
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// org-link-escape.
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i <= chars.len() {
        let mut j = i;
        while j < chars.len() && chars[j] == '\\' {
            j += 1;
        }
        let n = j - i;
        let at_end = j == chars.len();
        let bracket = !at_end && (chars[j] == '[' || chars[j] == ']');
        if at_end || bracket {
            out.extend(std::iter::repeat_n('\\', if n > 0 || bracket { 2 * n } else { 0 }));
            if bracket {
                out.push('\\');
                out.push(chars[j]);
            }
        } else {
            out.extend(std::iter::repeat_n('\\', n));
            out.push(chars[j]);
        }
        if at_end {
            break;
        }
        i = j + 1;
    }
    out
}

/// org-link-make-string.
pub fn make_string(link: &str, desc: Option<&str>) -> Result<String, String> {
    let desc = desc.map(str::trim).filter(|d| !d.is_empty()).map(|d| {
        let zw = '\u{200B}';
        let d = if d.ends_with(']') { format!("{d}{zw}") } else { d.to_owned() };
        d.replace("]]", &format!("]{zw}]"))
    });
    if link.trim().is_empty() {
        return desc.ok_or_else(|| "Empty link".to_owned());
    }
    Ok(match desc {
        Some(d) => format!("[[{}][{d}]]", escape(link)),
        None => format!("[[{}]]", escape(link)),
    })
}

/// org-link-expand-abbrev with the buffer's #+LINK and org-link-abbrev-alist.
pub fn expand_abbrev(link: &str, st: &Settings) -> String {
    let (key, tag) = match link.split_once(':') {
        Some((k, rest)) => (k, Some(rest.strip_prefix(':').unwrap_or(rest))),
        None => (link, None),
    };
    let global: Vec<(String, String)> = super::sexp::option("org-link-abbrev-alist")
        .and_then(|v| v.list().map(|l| l.iter().filter_map(|e| Some((e.car()?.str()?.to_owned(), e.cdr().str()?.to_owned()))).collect()))
        .unwrap_or_default();
    let Some((_, rpl)) = st.links.iter().find(|(k, _)| k == key).or_else(|| global.iter().find(|(k, _)| k == key)) else {
        return link.to_owned();
    };
    let tag = tag.unwrap_or("");
    if rpl.contains("%s") {
        rpl.replacen("%s", tag, 1)
    } else if rpl.contains("%h") {
        rpl.replacen("%h", &hexify(tag), 1)
    } else {
        format!("{rpl}{tag}")
    }
}

/// url-hexify-string.
fn hexify(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// Classify a raw link path the way org-element-link-parser does.
pub fn classify(raw: &str, st: &Settings) -> (String, String, Option<String>, Option<String>) {
    let raw = expand_abbrev(raw, st);
    let raw = raw.trim().to_owned();
    if let Some(rest) = raw.strip_prefix('#') {
        return ("custom-id".into(), rest.to_owned(), None, None);
    }
    if raw.starts_with('(') && raw.ends_with(')') {
        return ("coderef".into(), raw[1..raw.len() - 1].to_owned(), None, None);
    }
    if raw.starts_with('/') || raw.starts_with("./") || raw.starts_with("../") || raw.starts_with("~/") || raw == "~" {
        let (p, s) = split_search(&raw);
        return ("file".into(), p, s, None);
    }
    if let Some((t, rest)) = raw.split_once(':') {
        if link_types().iter().any(|x| x == t) {
            if t == "file" || t == "file+sys" || t == "file+emacs" || t == "docview" || t == "attachment" {
                let app = t.strip_prefix("file+").map(str::to_owned);
                let (p, s) = split_search(rest);
                let kind = if t.starts_with("file") { "file" } else { t };
                return (kind.into(), p, s, app);
            }
            let path = if matches!(t, "http" | "https" | "ftp" | "mailto" | "news") { rest.to_owned() } else { rest.to_owned() };
            return (t.into(), path, None, None);
        }
    }
    ("fuzzy".into(), raw, None, None)
}

fn split_search(s: &str) -> (String, Option<String>) {
    match s.find("::") {
        Some(i) => (s[..i].to_owned(), Some(s[i + 2..].to_owned())),
        None => (s.to_owned(), None),
    }
}

/// Links in a line of text.
pub fn links_in(line: &str, st: &Settings) -> Vec<Link> {
    let mut out = vec![];
    let mut i = 0;
    let types = link_types();
    while i < line.len() {
        let rest = &line[i..];
        if rest.starts_with("[[") {
            // A bracket link: [[path]] or [[path][desc]], backslash escapes.
            let b = rest.as_bytes();
            let mut j = 2;
            while j < b.len() && !(b[j] == b']' && (j == 0 || b[j - 1] != b'\\' || (j >= 2 && b[j - 2] == b'\\'))) {
                if b[j] == b'[' && b[j - 1] != b'\\' {
                    break;
                }
                j += 1;
            }
            if j < b.len() && b[j] == b']' {
                let raw = unescape(&rest[2..j]);
                let (desc, end) = if rest[j..].starts_with("][") {
                    match rest[j + 2..].find("]]") {
                        Some(e) => (Some(rest[j + 2..j + 2 + e].to_owned()), j + 2 + e + 2),
                        None => {
                            i += 2;
                            continue;
                        }
                    }
                } else if rest[j..].starts_with("]]") {
                    (None, j + 2)
                } else {
                    i += 2;
                    continue;
                };
                let (kind, path, search, application) = classify(&raw, st);
                out.push(Link { kind, path, search, application, desc, raw, range: i..i + end });
                i += end;
                continue;
            }
        }
        if rest.starts_with('<')
            && let Some(e) = rest.find('>')
            && let Some((t, _)) = rest[1..e].split_once(':')
            && types.iter().any(|x| x == t)
        {
            let raw = rest[1..e].to_owned();
            let (kind, path, search, application) = classify(&raw, st);
            out.push(Link { kind, path, search, application, desc: None, raw, range: i..i + e + 1 });
            i += e + 1;
            continue;
        }
        // Plain links: TYPE:PATH at a word boundary.
        let boundary = i == 0 || !line[..i].ends_with(|c: char| c.is_alphanumeric());
        if boundary
            && let Some(colon) = rest.find(':')
            && colon > 0
            && types.iter().any(|t| *t == rest[..colon])
            && rest.len() > colon + 1
            && !rest[colon + 1..].starts_with(char::is_whitespace)
        {
            let mut end = colon + 1 + rest[colon + 1..].find(|c: char| c.is_whitespace() || "()<>[]\"".contains(c)).unwrap_or(rest.len() - colon - 1);
            // Trailing punctuation is not part of a plain link.
            while end > colon + 1 && rest[..end].ends_with(['.', ',', ';', ':', '!', '?', '\'']) {
                end -= 1;
            }
            let raw = rest[..end].to_owned();
            let (kind, path, search, application) = classify(&raw, st);
            out.push(Link { kind, path, search, application, desc: None, raw, range: i..i + end });
            i += end;
            continue;
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// The link at the cursor.
pub fn link_at(ed: &Editor) -> Option<Link> {
    let st = super::settings(ed);
    let line = ed.buf.line(ed.cur.line);
    links_in(&line, &st).into_iter().find(|l| l.range.contains(&ed.cur.byte))
}

// ---- storing ----

fn add_stored(ed: &mut Editor, link: String, desc: Option<String>) {
    let entry = (link.clone(), desc.clone());
    let msg = STORED.with(|s| {
        let mut s = s.borrow_mut();
        if s.first() == Some(&entry) {
            "This link has already been stored".to_owned()
        } else if let Some(i) = s.iter().position(|e| *e == entry) {
            s.remove(i);
            s.insert(0, entry);
            format!("Link moved to front: {}", desc.clone().unwrap_or(link.clone()))
        } else {
            s.insert(0, entry);
            format!("Stored: {}", desc.clone().unwrap_or(link.clone()))
        }
    });
    ed.set_msg(msg);
}

/// org-link-normalize-string with statistics cookies (and search syntax).
pub fn normalize(s: &str, search_syntax: bool, pipes: bool) -> String {
    let re = crate::org_re!(r"\[\d*(?:%|/\d*)\]");
    let mut s = re.replace_all(s.trim(), " ").into_owned();
    if pipes {
        s = s.replace('|', " ");
    }
    if search_syntax {
        loop {
            let t = s.trim();
            if t.starts_with('(') && t.ends_with(')') {
                s = t[1..t.len() - 1].trim().to_owned();
            } else if t.starts_with(['#', '*']) {
                s = t.trim_start_matches(['#', '*']).trim_start().to_owned();
            } else {
                break;
            }
        }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The heading text without keyword, priority, COMMENT, tags.
pub fn heading_text(line: &str, st: &Settings) -> String {
    syntax::headline(line, st).map(|h| h.title(line).to_owned()).unwrap_or_default()
}

/// org-link-precise-link-target: (search, description).
fn precise_target(ed: &Editor) -> Option<(String, Option<String>)> {
    let st = super::settings(ed);
    let l = ed.cur.line;
    if let Some((lo, hi)) = ed.org_region {
        let max = super::options::int("org-link-context-for-files", 1).max(1) as usize;
        let text = (lo..=hi.min(lo + max - 1)).map(|i| ed.buf.line(i)).collect::<Vec<_>>().join("\n");
        return Some((normalize(&text, true, false), None)).filter(|(s, _)| !s.is_empty());
    }
    let line = ed.buf.line(l);
    if ed.org.is_some() {
        // A <<target>> at point.
        let mut from = 0;
        while let Some(s) = line[from..].find("<<").map(|i| i + from) {
            let Some(e) = line[s..].find(">>").map(|i| s + i) else { break };
            if (s..e + 2).contains(&ed.cur.byte) && !line[s..].starts_with("<<<") {
                let v = line[s + 2..e].to_owned();
                return Some((v.clone(), Some(v)));
            }
            from = e + 2;
        }
        // A named element.
        let mut i = l;
        loop {
            if let Some((k, v)) = syntax::keyword_line(&ed.buf.line(i))
                && k == "NAME"
            {
                return Some((v.to_owned(), Some(v.to_owned())));
            }
            if i == 0 || syntax::keyword_line(&ed.buf.line(i)).is_none() && i != l {
                break;
            }
            i -= 1;
        }
        match fold::back_to_heading(ed, l) {
            None => {
                let s = normalize(&line, true, false);
                return (!s.is_empty()).then_some((s, None));
            }
            Some(h) => {
                let text = heading_text(&ed.buf.line(h), &st);
                let custom = props::get(ed, Some(h), "CUSTOM_ID", props::Inherit::No);
                let search = match custom {
                    Some(c) => format!("#{c}"),
                    None => format!("*{}", normalize(&text, false, false)),
                };
                return Some((search, Some(normalize(&text, false, false))));
            }
        }
    }
    let s = normalize(&line, true, false);
    (!s.is_empty()).then_some((s, None))
}

fn abbreviate(p: &Path) -> String {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let home = std::env::var("HOME").unwrap_or_default();
    let s = abs.display().to_string();
    if !home.is_empty() && s.starts_with(&home) { format!("~{}", &s[home.len()..]) } else { s }
}

/// org-link--file-link-description.
fn file_desc(path: &str) -> Option<String> {
    match super::sexp::option("org-link-default-file-link-description") {
        Some(Sexp::Sym(s)) if s == "filename" => Path::new(path).file_name().map(|f| f.to_string_lossy().into_owned()),
        Some(Sexp::Sym(s)) if s == "file-path" => Some(path.to_owned()),
        _ => None,
    }
}

/// org-store-link: returns (link, description).
pub fn store(ed: &mut Editor, arg: Prefix) -> Result<(String, Option<String>), String> {
    let context = super::options::bool("org-link-context-for-files", true) != (arg == Prefix::U(1));
    let (link, desc) = if let Some(d) = &ed.dired {
        let file = crate::dired::selection(ed).into_iter().next().unwrap_or_else(|| d.dir.clone());
        let f = abbreviate(&file);
        (format!("file:{f}"), file_desc(&f))
    } else if ed.org_view.is_some() {
        return super::call(ed, "org-agenda-store-link", arg).map(|()| stored_links().first().cloned().unwrap_or_default());
    } else if let Some(p) = ed.path.clone() {
        let f = abbreviate(&p);
        let base = format!("file:{f}");
        if context && let Some((search, sdesc)) = precise_target(ed) {
            (format!("{base}::{search}"), sdesc.or_else(|| file_desc(&f)))
        } else {
            (base, file_desc(&f))
        }
    } else {
        return Err("No method for storing a link from this buffer".into());
    };
    // ID links when org-id-link-to-org-use-id says so.
    let (link, desc) = match super::options::string("org-id-link-to-org-use-id", "nil").as_str() {
        "t" | "create-if-interactive" | "create-if-interactive-and-no-custom-id" if ed.org.is_some() && !ctx::before_first_heading(ed, ed.cur.line) => {
            let h = fold::back_to_heading(ed, ed.cur.line).unwrap();
            let has_custom = props::get(ed, Some(h), "CUSTOM_ID", props::Inherit::No).is_some();
            let mode = super::options::string("org-id-link-to-org-use-id", "nil");
            if mode == "create-if-interactive-and-no-custom-id" && has_custom {
                (link, desc)
            } else {
                let id = match props::get(ed, Some(h), "ID", props::Inherit::No) {
                    Some(id) => id,
                    None => {
                        let id = new_id();
                        props::put(ed, Some(h), "ID", &id)?;
                        id
                    }
                };
                let st = super::settings(ed);
                (format!("id:{id}"), Some(heading_text(&ed.buf.line(h), &st)))
            }
        }
        "use-existing" if ed.org.is_some() && !ctx::before_first_heading(ed, ed.cur.line) => {
            let h = fold::back_to_heading(ed, ed.cur.line).unwrap();
            match props::get(ed, Some(h), "ID", props::Inherit::No) {
                Some(id) => {
                    let st = super::settings(ed);
                    (format!("id:{id}"), Some(heading_text(&ed.buf.line(h), &st)))
                }
                None => (link, desc),
            }
        }
        _ => (link, desc),
    };
    let desc = desc.filter(|d| d != "NONE").map(|d| display_format(&d));
    Ok((link, desc))
}

/// A new UUID (org-id-new with the uuid method).
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut bytes);
    } else {
        let t = super::now() as u64 ^ (std::process::id() as u64) << 32;
        bytes[..8].copy_from_slice(&t.to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// org-link-display-format: links replaced by their descriptions.
pub fn display_format(s: &str) -> String {
    let re = crate::org_re!(r"\[\[((?:[^\]\[\\]|\\.)+)\](?:\[([^\]]+)\])?\]");
    re.replace_all(s, |c: &regex::Captures| c.get(2).or(c.get(1)).unwrap().as_str().to_owned()).into_owned()
}

// ---- inserting ----

/// org-link--normalize-filename for org-link-file-path-type.
fn normalize_filename(path: &str, method: &str, cur_dir: &Path) -> String {
    let expanded = super::options::expand(path);
    let abs = if expanded.is_absolute() { expanded } else { cur_dir.join(expanded) };
    match method {
        "absolute" => abbreviate(&abs),
        "noabbrev" => abs.display().to_string(),
        "relative" => pathdiff(&abs, cur_dir),
        _ => match abs.strip_prefix(cur_dir) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => abbreviate(&abs),
        },
    }
}

fn pathdiff(path: &Path, base: &Path) -> String {
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(a, c)| a == c).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    out.display().to_string()
}

/// org-link-make-string-for-buffer.
fn make_for_buffer(ed: &Editor, link: &str, desc: Option<String>, abs: bool) -> Result<String, String> {
    let mut link = link.to_owned();
    if link.starts_with('<') && link.ends_with('>') && !link[1..].starts_with(|c: char| c.is_ascii_digit()) {
        link = link[1..link.len() - 1].to_owned();
    }
    let me = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok());
    if let Some(rest) = link.strip_prefix("file:")
        && let Some(i) = rest.find("::")
    {
        let target = std::path::absolute(super::options::expand(&rest[..i])).ok();
        if me.is_some() && target == me {
            link = rest[i + 2..].to_owned();
        }
    }
    let mut desc = desc;
    for t in ["file:", "docview:"] {
        if let Some(rest) = link.strip_prefix(t) {
            let (path, search) = split_search(rest);
            let path = if path.is_empty() { ed.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default() } else { path };
            let dir = me.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let method = if abs { "absolute".to_owned() } else { super::options::string("org-link-file-path-type", "adaptive") };
            let np = normalize_filename(&path, &method, &dir);
            if desc.as_deref() == Some(&path) {
                desc = Some(np.clone());
            }
            link = format!("{t}{np}{}", search.map(|s| format!("::{s}")).unwrap_or_default());
            break;
        }
    }
    make_string(&link, desc.as_deref())
}

/// org-insert-link.
pub fn insert_link(ed: &mut Editor, arg: Prefix, location: Option<String>, description: Option<String>) -> Result<(), String> {
    let st = super::settings(ed);
    let l = ed.cur.line;
    let line = ed.buf.line(l);
    // Editing the link at point.
    if location.is_none()
        && let Some(lk) = links_in(&line, &st).into_iter().find(|x| x.range.contains(&ed.cur.byte))
    {
        let range = lk.range.clone();
        let bracket = line[range.clone()].starts_with("[[");
        let initial = if bracket { lk.raw.clone() } else { lk.raw.trim_start_matches('<').trim_end_matches('>').to_owned() };
        let desc = lk.desc.clone();
        super::read(ed, "Link: ", &initial, move |ed, link| {
            finish_insert(ed, link, desc, Some((l, range)), false, true);
        });
        return Ok(());
    }
    let region = ed.org_region;
    let region_text = region.map(|(lo, hi)| (lo..=hi).map(|i| ed.buf.line(i)).collect::<Vec<_>>().join("\n"));
    let remove = region.map(|(lo, hi)| (lo, hi));
    if let Some(loc) = location {
        finish_insert(ed, loc, description.or(region_text), None, false, false);
        return Ok(());
    }
    if matches!(arg, Prefix::U(1) | Prefix::U(2)) {
        let abs = arg == Prefix::U(2);
        let dir = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok()).and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let files = list_files(&dir);
        super::complete(ed, "File: ", files, false, move |ed, f| {
            finish_insert(ed, format!("file:{f}"), None, None, abs, true);
        });
        return Ok(());
    }
    let stored = stored_links();
    // The default (most recent stored link) first, as vertico shows it.
    let mut candidates: Vec<String> = stored.iter().map(|(l, _)| l.clone()).collect();
    candidates.extend(stored.iter().filter_map(|(_, d)| d.clone()));
    candidates.extend(link_types().into_iter().map(|t| format!("{t}:")));
    for (k, _) in &st.links {
        candidates.push(format!("{k}:"));
    }
    HISTORY.with(|h| candidates.extend(h.borrow().iter().cloned()));
    let default = stored.first().map(|(l, _)| l.clone());
    let prompt = match &default {
        Some(d) => format!("Insert link [{d}]: "),
        None => "Insert link: ".into(),
    };
    let keep_flip = arg == Prefix::U(3);
    super::complete(ed, &prompt, candidates, false, move |ed, mut link| {
        if link.trim().is_empty() {
            match &default {
                Some(d) => link = d.clone(),
                None => {
                    ed.set_err("No link selected");
                    return;
                }
            }
        }
        // A description chosen: its link.
        let stored = stored_links();
        if let Some((l, _)) = stored.iter().find(|(_, d)| d.as_deref() == Some(link.as_str())) {
            link = l.clone();
        }
        let link = link.strip_suffix(':').filter(|t| link_types().iter().any(|x| x == t)).map_or(link.clone(), |t| format!("{t}:"));
        let entry = stored.iter().find(|(l, _)| *l == link).cloned();
        if entry.is_none() {
            HISTORY.with(|h| h.borrow_mut().insert(0, link.clone()));
        }
        let desc = region_text.clone().or(entry.as_ref().and_then(|e| e.1.clone()));
        if let Some((lo, hi)) = remove {
            ed.undo.begin(ed.cur.pos());
            super::splice(ed, lo, hi - lo + 1, &[String::new()]);
            ed.set_cursor(lo, 0);
            ed.undo.end(ed.cur.pos());
        }
        let keep = super::options::bool("org-link-keep-stored-after-insertion", false) != keep_flip;
        if !keep && entry.is_some() {
            STORED.with(|s| s.borrow_mut().retain(|(l, _)| *l != link));
        }
        finish_insert(ed, link, desc, None, false, true);
    });
    Ok(())
}

/// Prompt for the description, then insert (replacing `replace`).
fn finish_insert(ed: &mut Editor, link: String, desc: Option<String>, replace: Option<(usize, std::ops::Range<usize>)>, abs: bool, ask: bool) {
    let go = move |ed: &mut Editor, desc: String| {
        let desc = if desc.trim().is_empty() { None } else { Some(desc) };
        match make_for_buffer(ed, &link, desc, abs) {
            Ok(s) => {
                ed.undo.begin(ed.cur.pos());
                let (l, b) = (ed.cur.line, ed.cur.byte);
                let line = ed.buf.line(l);
                let (new, at) = match &replace {
                    Some((rl, r)) if *rl == l => (format!("{}{s}{}", &line[..r.start], &line[r.end..]), r.start + s.len()),
                    _ => {
                        // Insert after the cursor character in Normal mode (like `a`).
                        let at = if ed.mode == crate::editor::Mode::Insert || line.is_empty() { b.min(line.len()) } else { crate::text::next_grapheme(&line, b) };
                        (format!("{}{s}{}", &line[..at], &line[at..]), at + s.len())
                    }
                };
                super::set_line(ed, l, &new);
                ed.cur.byte = at.saturating_sub(if ed.mode == crate::editor::Mode::Insert { 0 } else { 1 });
                ed.undo.end(ed.cur.pos());
            }
            Err(e) => ed.set_err(e),
        }
    };
    let default = desc.clone().unwrap_or_default();
    if ask {
        super::read(ed, "Description: ", &default, go);
    } else {
        go(ed, default);
    }
}

fn list_files(dir: &Path) -> Vec<String> {
    let mut out = vec![];
    for entry in ignore::WalkBuilder::new(dir).max_depth(Some(4)).build().flatten().take(5000) {
        if let Ok(rel) = entry.path().strip_prefix(dir)
            && !rel.as_os_str().is_empty()
        {
            out.push(rel.display().to_string());
        }
    }
    out
}

// ---- opening ----

/// org-mark-ring-push.
pub fn mark_ring_push(ed: &mut Editor) {
    let len = super::options::int("org-mark-ring-length", 4).max(1) as usize;
    let pos = (ed.path.clone(), ed.cur.line);
    RING.with(|r| {
        let mut r = r.borrow_mut();
        r.0.insert(0, pos);
        r.0.truncate(len);
        r.1 = None;
    });
    ed.set_msg("Position saved to mark ring, go back with `C-c &'.");
}

/// org-mark-ring-goto.
fn mark_ring_goto(ed: &mut Editor, n: usize) -> Result<(), String> {
    let repeat = ed.org.as_ref().and_then(|o| o.last_command.as_deref()) == Some("org-mark-ring-goto");
    let target = RING.with(|r| {
        let mut r = r.borrow_mut();
        if r.0.is_empty() {
            return None;
        }
        let i = if repeat { (r.1.unwrap_or(0) + n) % r.0.len() } else { 0 };
        r.1 = Some(i);
        Some(r.0[i].clone())
    });
    let Some((path, line)) = target else { return Err("No previous position in the mark ring".into()) };
    goto_position(ed, path, line);
    Ok(())
}

fn goto_position(ed: &mut Editor, path: Option<PathBuf>, line: usize) {
    let mine = path.as_deref().and_then(|p| std::path::absolute(p).ok()) == ed.path.as_deref().and_then(|p| std::path::absolute(p).ok());
    if mine || path.is_none() {
        ed.set_cursor(line.min(ed.line_count() - 1), 0);
        if fold::hidden(ed, ed.cur.line) {
            fold::show_context_for(ed, ed.cur.line, "mark-goto");
        }
    } else if let Some(p) = path {
        super::effect(ed, move |s| {
            if let Err(e) = s.org_visit(&p, line) {
                s.ed.set_err(e);
            }
        });
    }
}

/// org-link-search in the editor: moves the cursor. Returns the kind
/// (`dedicated` or `fuzzy`).
pub fn search(ed: &mut Editor, s: &str, avoid: Option<usize>) -> Result<&'static str, String> {
    if s.trim().is_empty() {
        return Err(format!("Invalid search string \"{s}\""));
    }
    let st = super::settings(ed);
    let normalized = s.replace('\n', " ");
    let starred = normalized.starts_with('*');
    let words: Vec<String> = (if starred { &s[1..] } else { s }).split_whitespace().map(str::to_owned).collect();
    let n = ed.line_count();
    let found = |ed: &mut Editor, l: usize, b: usize, kind: &'static str| {
        ed.set_cursor(l, b);
        if ed.org.is_some() {
            fold::show_context_for(ed, l, "link-search");
        }
        Ok(kind)
    };
    if let Some(id) = normalized.strip_prefix('#') {
        for l in 0..n {
            if let Some((k, v)) = props::parse_property(&ed.buf.line(l))
                && k.eq_ignore_ascii_case("CUSTOM_ID")
                && v == id
                && ctx::at_property(ed, l)
            {
                let h = fold::back_to_heading(ed, l).unwrap_or(0);
                return found(ed, h, 0, "dedicated");
            }
        }
        return Err(format!("No match for custom ID: {id}"));
    }
    if normalized.starts_with('(') && normalized.ends_with(')') {
        let label = &normalized[1..normalized.len() - 1];
        let pat = format!("(ref:{label})");
        for l in 0..n {
            let line = ed.buf.line(l);
            if let Some(i) = line.find(&pat)
                && ctx::in_block(ed, l)
            {
                return found(ed, l, i, "dedicated");
            }
        }
        return Err(format!("No match for coderef: {label}"));
    }
    if normalized.len() > 1 && normalized.starts_with('/') && normalized.ends_with('/') {
        super::todo::occur(ed, &normalized[1..normalized.len() - 1], false)?;
        return Ok("dedicated");
    }
    let words_re = words.iter().map(|w| regex::escape(w)).collect::<Vec<_>>().join(r"\s+");
    let ci = |p: &str| regex::RegexBuilder::new(p).case_insensitive(true).build().unwrap();
    if !starred {
        let t = ci(&format!("<<{words_re}>>"));
        for l in 0..n {
            if let Some(m) = t.find(&ed.buf.line(l))
                && !ed.buf.line(l)[m.start()..].starts_with("<<<")
            {
                return found(ed, l, m.start(), "dedicated");
            }
        }
        for l in 0..n {
            if let Some((k, v)) = syntax::keyword_line(&ed.buf.line(l))
                && k == "NAME"
                && v.split_whitespace().map(str::to_uppercase).eq(words.iter().map(|w| w.to_uppercase()))
            {
                return found(ed, l, 0, "dedicated");
            }
        }
    }
    if ed.org.is_some() {
        for pipes in [false, true] {
            for l in 0..n {
                let line = ed.buf.line(l);
                if syntax::headline(&line, &st).is_some() {
                    let t = normalize(&heading_text(&line, &st), false, pipes);
                    let parts: Vec<String> = t.split_whitespace().map(str::to_lowercase).collect();
                    if parts == words.iter().map(|w| w.to_lowercase()).collect::<Vec<_>>() {
                        return found(ed, l, 0, "dedicated");
                    }
                }
            }
        }
        let must = super::sexp::option("org-link-search-must-match-exact-headline").map_or("query-to-create".to_owned(), |v| v.sym().map_or("t".into(), str::to_owned));
        if must == "query-to-create" {
            let title = if starred { s[1..].to_owned() } else { s.to_owned() };
            super::yes_or_no(ed, "No match - create this as a new heading? ", move |ed| {
                ed.undo.begin(ed.cur.pos());
                let n = ed.line_count();
                super::insert_lines(ed, n, &[format!("* {title}")]);
                ed.set_cursor(ed.line_count() - 1, 0);
                ed.undo.end(ed.cur.pos());
            });
            return Ok("dedicated");
        }
        if starred || must != "nil" {
            return Err(format!("No match for fuzzy expression: {normalized}"));
        }
    }
    let re = ci(&words_re);
    for l in 0..n {
        let line = ed.buf.line(l);
        for m in re.find_iter(&line) {
            if avoid.is_some_and(|a| a == l) {
                continue;
            }
            return found(ed, l, m.start(), "fuzzy");
        }
    }
    Err(format!("No match for fuzzy expression: {normalized}"))
}

/// org-file-apps: open in Fred (Some(None)), or run a command.
fn file_app(path: &str, arg: Prefix, app: Option<&str>) -> Option<String> {
    if arg == Prefix::U(1) || app == Some("emacs") {
        return None;
    }
    let system = arg == Prefix::U(2) || app == Some("sys");
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let quoted = shell_quote(path);
    if system {
        return Some(format!("{opener} {quoted}"));
    }
    let apps: Vec<(Sexp, Sexp)> = super::sexp::option("org-file-apps")
        .and_then(|v| v.list().map(|l| l.iter().filter_map(|e| Some((e.car()?.clone(), e.cdr()))).collect()))
        .unwrap_or_else(|| {
            vec![
                (Sexp::Sym("auto-mode".into()), Sexp::Sym("emacs".into())),
                (Sexp::Sym("directory".into()), Sexp::Sym("emacs".into())),
                (Sexp::Str(r"\.mm\'".into()), Sexp::Sym("default".into())),
                (Sexp::Str(r"\.x?html?\'".into()), Sexp::Sym("default".into())),
                (Sexp::Str(r"\.pdf\'".into()), Sexp::Sym("default".into())),
            ]
        });
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let is_dir = Path::new(&*super::options::expand(path)).is_dir();
    for (k, v) in &apps {
        let hit = match k {
            Sexp::Sym(s) if s == "directory" => is_dir,
            Sexp::Sym(s) if s == "auto-mode" => !is_dir && text_like(&ext),
            Sexp::Sym(s) if s == "system" || s == "t" => true,
            Sexp::Str(s) if s.chars().all(char::is_alphanumeric) => *s == ext,
            Sexp::Str(re) => super::re::compile(re, false).is_ok_and(|r| r.is_match(path)),
            _ => false,
        };
        if !hit {
            continue;
        }
        return match v {
            Sexp::Sym(s) if s == "emacs" => None,
            Sexp::Sym(s) if s == "default" || s == "system" => Some(format!("{opener} {quoted}")),
            Sexp::Str(cmd) => Some(cmd.replace("%s", &quoted)),
            _ => None,
        };
    }
    if text_like(&ext) || is_dir { None } else { Some(format!("{opener} {quoted}")) }
}

/// Files Fred edits (Emacs would have a major mode for them).
fn text_like(ext: &str) -> bool {
    !matches!(
        ext,
        "pdf" | "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "tiff" | "mp3" | "mp4" | "mov" | "avi" | "mkv" | "wav"
            | "flac" | "ogg" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "zip" | "gz" | "tgz"
            | "bz2" | "xz" | "7z" | "rar" | "dmg" | "app" | "exe" | "html" | "htm" | "mm" | "epub" | "djvu"
    )
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run an external program without the terminal (detached).
fn spawn(cmd: &str) -> Result<(), String> {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{cmd}: {e}"))
}

/// org-open-file.
pub fn open_file(ed: &mut Editor, path: &str, search: Option<String>, arg: Prefix, app: Option<&str>) -> Result<(), String> {
    let base = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok()).and_then(|p| p.parent().map(Path::to_path_buf));
    let p = super::options::expand(path);
    let full = match base {
        Some(b) if p.is_relative() => b.join(&p),
        _ => p,
    };
    if let Some(cmd) = file_app(&full.display().to_string(), arg, app) {
        ed.set_msg(format!("Running {cmd}...done"));
        return spawn(&cmd);
    }
    let me = ed.path.as_deref().and_then(|p| std::path::absolute(p).ok());
    let line: Option<usize> = search.as_deref().and_then(|s| s.parse::<usize>().ok());
    if me.as_deref() == std::path::absolute(&full).ok().as_deref() {
        mark_ring_push(ed);
        return match (line, search) {
            (Some(n), _) => {
                ed.set_cursor(n.saturating_sub(1), 0);
                Ok(())
            }
            (None, Some(s)) => self::search(ed, &s, None).map(|_| ()),
            _ => Ok(()),
        };
    }
    mark_ring_push(ed);
    super::effect(ed, move |s| {
        if let Err(e) = s.org_visit(&full, line.map_or(0, |n| n.saturating_sub(1))) {
            s.ed.set_err(e);
            return;
        }
        if line.is_none()
            && let Some(q) = search
            && let Err(e) = super::links::search(&mut s.ed, &q, None)
        {
            s.ed.set_err(e);
        }
    });
    Ok(())
}

/// org-link-open.
pub fn open(ed: &mut Editor, lk: &Link, arg: Prefix) -> Result<(), String> {
    match lk.kind.as_str() {
        "file" => open_file(ed, &lk.path, lk.search.clone(), arg, lk.application.as_deref()),
        "custom-id" | "fuzzy" | "coderef" | "radio" => {
            mark_ring_push(ed);
            let s = match lk.kind.as_str() {
                "custom-id" => format!("#{}", lk.path),
                "coderef" => format!("({})", lk.path),
                _ => lk.path.clone(),
            };
            let here = ed.cur.line;
            search(ed, &s, (lk.kind == "fuzzy").then_some(here)).map(|_| ())
        }
        "http" | "https" | "ftp" | "mailto" | "news" | "doi" => {
            let url = if lk.kind == "doi" {
                format!("{}{}", super::options::string("org-link-doi-server-url", "https://doi.org/"), lk.path)
            } else {
                format!("{}:{}", lk.kind, lk.path)
            };
            let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
            spawn(&format!("{opener} {}", shell_quote(&url)))
        }
        "shell" => {
            let cmd = lk.path.clone();
            let skip = super::options::string("org-link-shell-skip-confirm-regexp", "");
            let run = move |ed: &mut Editor| {
                ed.pending_effect = Some(crate::ex::ExEffect::Shell(cmd));
            };
            if !skip.is_empty() && super::re::compile(&skip, false).is_ok_and(|r| r.is_match(&lk.path)) {
                run(ed);
            } else {
                super::yes_or_no(ed, &format!("Execute \"{}\" in shell? ", lk.path), run);
            }
            Ok(())
        }
        "elisp" => {
            let form = lk.path.clone();
            super::yes_or_no(ed, &format!("Execute \"{form}\" as elisp? "), move |ed| {
                let expr = if form.starts_with('(') { format!("(prin1 {form})") } else { format!("(call-interactively '{form})") };
                match std::process::Command::new("emacs").args(["--batch", "-Q", "--eval", &expr]).output() {
                    Ok(o) => ed.set_msg(format!("{form} => {}", String::from_utf8_lossy(&o.stdout).trim())),
                    Err(e) => ed.set_err(format!("emacs: {e}")),
                }
            });
            Ok(())
        }
        "id" => open_id(ed, &lk.path),
        "man" => {
            ed.pending_effect = Some(crate::ex::ExEffect::Shell(format!("man {}", shell_quote(&lk.path))));
            Ok(())
        }
        "info" => {
            let (file, node) = lk.path.split_once('#').unwrap_or((&lk.path, "Top"));
            ed.pending_effect = Some(crate::ex::ExEffect::Shell(format!("info {} -n {}", shell_quote(file), shell_quote(node))));
            Ok(())
        }
        "help" | "shortdoc" => {
            let expr = format!("(princ (documentation '{} t))", lk.path);
            let out = std::process::Command::new("emacs").args(["--batch", "-Q", "--eval", &expr]).output();
            match out {
                Ok(o) => {
                    ed.set_msg(String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_owned());
                    Ok(())
                }
                Err(e) => Err(format!("emacs: {e}")),
            }
        }
        "attachment" => super::call(ed, "org-attach-open-link", arg).or_else(|_| open_file(ed, &lk.path, lk.search.clone(), arg, None)),
        other => {
            // A configured :follow command (org-link-parameters), as a shell template.
            let follow = super::sexp::option("org-link-parameters").and_then(|v| {
                v.list()?
                    .iter()
                    .find(|e| e.car().and_then(Sexp::str) == Some(other))
                    .and_then(|e| e.cdr().plist_get(":follow").and_then(Sexp::str).map(str::to_owned))
            });
            match follow {
                Some(cmd) => spawn(&cmd.replace("%s", &shell_quote(&lk.path))),
                None => Err(format!("No follow function for link type \"{other}\"")),
            }
        }
    }
}

/// org-id-open: search this buffer, open buffers and the ID locations.
pub fn open_id(ed: &mut Editor, id: &str) -> Result<(), String> {
    // This buffer first.
    for l in 0..ed.line_count() {
        if let Some((k, v)) = props::parse_property(&ed.buf.line(l))
            && k.eq_ignore_ascii_case("ID")
            && v == id
            && ctx::at_property(ed, l)
        {
            mark_ring_push(ed);
            let h = fold::back_to_heading(ed, l).unwrap_or(0);
            ed.set_cursor(h, 0);
            fold::show_context_for(ed, h, "link-search");
            return Ok(());
        }
    }
    let id = id.to_owned();
    mark_ring_push(ed);
    super::effect(ed, move |s| match super::find_id_file(s, &id) {
        Some((path, line)) => {
            if let Err(e) = s.org_visit(&path, line) {
                s.ed.set_err(e);
            }
        }
        None => s.ed.set_err(format!("Cannot find entry with ID \"{id}\"")),
    });
    Ok(())
}

/// org-open-at-point.
pub fn open_at_point(ed: &mut Editor, arg: Prefix) -> Result<(), String> {
    let l = ed.cur.line;
    let line = ed.buf.line(l);
    let st = super::settings(ed);
    if let Some(lk) = link_at(ed) {
        return open(ed, &lk, arg);
    }
    if ctx::footnote_at(ed, l, ed.cur.byte).is_some() || line.trim_start().starts_with("[fn:") && ed.cur.byte < line.find(']').unwrap_or(0) + 1 {
        return super::call(ed, "org-footnote-action", arg);
    }
    if let Some(r) = ctx::timestamp_at(ed, l, ed.cur.byte) {
        let ts = line[r].to_owned();
        return super::call_with(ed, "org-follow-timestamp-link", arg, ts);
    }
    if ctx::src_block(ed, l).is_some() {
        return super::call(ed, "org-babel-open-src-block-result", arg);
    }
    if let Some(h) = syntax::headline(&line, &st) {
        if let Some(r) = &h.tags_range
            && ed.cur.byte >= r.start
            && ed.cur.byte < r.end
        {
            let tags = &line[r.clone()];
            let off = ed.cur.byte - r.start;
            let start = tags[..off].rfind(':').map_or(0, |i| i + 1);
            let end = tags[off..].find(':').map_or(tags.len(), |i| off + i);
            let tag = tags[start..end].trim().to_owned();
            return super::call_with(ed, "org-tags-view", arg, tag);
        }
        // org-offer-links-in-entry.
        let end = syntax::entry_end(&ed.buf, l);
        let mut found: Vec<(usize, Link)> = vec![];
        for i in l..end {
            for lk in links_in(&ed.buf.line(i), &st) {
                found.push((i, lk));
            }
        }
        return match found.len() {
            0 => match super::call(ed, "org-attach-reveal", arg) {
                Ok(()) => Ok(()),
                Err(_) => Err("No link found".into()),
            },
            1 => {
                let (_, lk) = found.remove(0);
                open(ed, &lk, arg)
            }
            _ => {
                let mut entries: Vec<(String, String)> = vec![];
                let keys = "0123456789abcdefghijklmnopqrstuvwxyz";
                for (i, (_, lk)) in found.iter().enumerate() {
                    if let Some(k) = keys.chars().nth(i) {
                        entries.push((k.to_string(), lk.desc.clone().unwrap_or_else(|| lk.raw.clone())));
                    }
                }
                entries.push(("A".into(), "open all".into()));
                let links: Vec<Link> = found.into_iter().map(|(_, lk)| lk).collect();
                super::menu(ed, "Select link to open:", entries, move |ed, k| {
                    let r = if k == "A" {
                        links.iter().try_for_each(|lk| open(ed, lk, arg))
                    } else {
                        match keys.find(&k).and_then(|i| links.get(i)) {
                            Some(lk) => open(ed, lk, arg),
                            None => Ok(()),
                        }
                    };
                    if let Err(e) = r {
                        ed.set_err(e);
                    }
                });
                Ok(())
            }
        };
    }
    // Comments, keywords, node properties: something that looks like a link.
    Err("No link found".into())
}

/// org-next-link.
fn next_link(ed: &mut Editor, backward: bool) {
    let st = super::settings(ed);
    let failed = ed.org.as_ref().is_some_and(|o| o.link_search_failed) && ed.org.as_ref().and_then(|o| o.last_command.as_deref()) == Some(if backward { "org-previous-link" } else { "org-next-link" });
    let all: Vec<(usize, usize)> = (0..ed.line_count()).flat_map(|l| links_in(&ed.buf.line(l), &st).into_iter().map(move |lk| (l, lk.range.start))).collect();
    let here = (ed.cur.line, ed.cur.byte);
    let target = if failed {
        if backward { all.last().copied() } else { all.first().copied() }
    } else if backward {
        all.iter().rev().find(|&&p| p < here).copied()
    } else {
        all.iter().find(|&&p| p > here).copied()
    };
    if failed {
        ed.set_msg(if backward { "Link search wrapped back to end of buffer" } else { "Link search wrapped back to beginning of buffer" });
    }
    match target {
        Some((l, b)) => {
            ed.set_cursor(l, b);
            if fold::hidden(ed, l) {
                fold::show_context_for(ed, l, "link-search");
            }
            if let Some(o) = &mut ed.org {
                o.link_search_failed = false;
            }
        }
        None => {
            if let Some(o) = &mut ed.org {
                o.link_search_failed = true;
            }
            ed.set_msg("No further link found");
        }
    }
}

pub fn command(ed: &mut Editor, name: &str, arg: Prefix) -> Option<Result<(), String>> {
    Some(match name {
        "org-store-link" => match store(ed, arg) {
            Ok((link, desc)) => {
                add_stored(ed, link.clone(), desc.clone());
                // An extra file link when the entry has a CUSTOM_ID.
                Ok(())
            }
            Err(e) => Err(e),
        },
        "org-insert-link" | "org-insert-link-global" => insert_link(ed, arg, None, None),
        "org-insert-last-stored-link" | "org-insert-all-links" => {
            let all = name == "org-insert-all-links";
            let links = stored_links();
            if links.is_empty() {
                ed.set_msg("No link to insert");
                return Some(Ok(()));
            }
            let count = if all && matches!(arg, Prefix::None | Prefix::U(_)) { links.len() } else { arg.value().max(1) as usize };
            let keep = all && arg == Prefix::U(1);
            let (pre, post) = if all { ("- ", "") } else { ("", "") };
            let mut lines = vec![];
            for (link, desc) in links.iter().take(count) {
                match make_for_buffer(ed, link, Some(desc.clone().unwrap_or_else(|| "<no description>".into())), false) {
                    Ok(s) => lines.push(format!("{pre}{s}{post}")),
                    Err(e) => return Some(Err(e)),
                }
            }
            if !keep && !super::options::bool("org-link-keep-stored-after-insertion", false) {
                STORED.with(|s| {
                    let mut s = s.borrow_mut();
                    let n = count.min(s.len());
                    s.drain(..n);
                });
            }
            let l = ed.cur.line;
            let line = ed.buf.line(l);
            if line.trim().is_empty() {
                super::splice(ed, l, 1, &lines);
            } else {
                super::insert_lines(ed, l + 1, &lines);
            }
            Ok(())
        }
        "org-open-at-point" | "org-open-at-point-global" => open_at_point(ed, arg),
        "org-open-link-from-string" | "org-link-open-from-string" => {
            super::read(ed, "Link: ", "", move |ed, s| {
                let st = super::settings(ed);
                match links_in(&s, &st).into_iter().next() {
                    Some(lk) if lk.range.end == s.trim_end().len() => {
                        if let Err(e) = open(ed, &lk, arg) {
                            ed.set_err(e);
                        }
                    }
                    Some(lk) => ed.set_err(format!("Garbage after link in {s:?} ({:?})", &s[lk.range.end..])),
                    None => {
                        // A bare string: a fuzzy link.
                        let (kind, path, search, app) = classify(&s, &st);
                        let lk = Link { kind, path, search, application: app, desc: None, raw: s.clone(), range: 0..s.len() };
                        if let Err(e) = open(ed, &lk, arg) {
                            ed.set_err(e);
                        }
                    }
                }
            });
            Ok(())
        }
        "org-next-link" => {
            next_link(ed, false);
            Ok(())
        }
        "org-previous-link" => {
            next_link(ed, true);
            Ok(())
        }
        "org-mark-ring-push" => {
            mark_ring_push(ed);
            Ok(())
        }
        "org-mark-ring-goto" => mark_ring_goto(ed, arg.value().max(1) as usize),
        "org-toggle-link-display" => {
            let on = !super::settings(ed).opt_bool("org-link-descriptive", true);
            super::options::put("org-link-descriptive", toml::Value::Boolean(on));
            ed.set_msg(if on { "Descriptive links display" } else { "Literal links display" });
            Ok(())
        }
        "org-update-radio-target-regexp" => {
            ed.set_msg("Radio targets updated");
            Ok(())
        }
        "org-link-search" => {
            super::read(ed, "Search: ", "", |ed, s| {
                if let Err(e) = search(ed, &s, None) {
                    ed.set_err(e);
                }
            });
            Ok(())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::org;
    use super::*;

    #[test]
    fn escaping_and_making() {
        assert_eq!(escape("a[b]"), "a\\[b\\]");
        assert_eq!(unescape("a\\[b\\]"), "a[b]");
        assert_eq!(make_string("https://x.org", Some("X")).unwrap(), "[[https://x.org][X]]");
        assert_eq!(make_string("file:a::*H", None).unwrap(), "[[file:a::*H]]");
        assert_eq!(display_format("see [[x][the X]] and [[y]]"), "see the X and y");
    }

    #[test]
    fn parses_links() {
        let st = syntax::settings("#+LINK: gh https://github.com/%s".lines(), None);
        let l = links_in("a [[gh:bzg/org][repo]] b https://x.org/p. <mailto:me@x> [[*Head]] [[#cid]] [[./f.org::12]]", &st);
        let kinds: Vec<_> = l.iter().map(|x| (x.kind.as_str(), x.path.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                ("https", "//github.com/bzg/org"),
                ("https", "//x.org/p"),
                ("mailto", "me@x"),
                ("fuzzy", "*Head"),
                ("custom-id", "cid"),
                ("file", "./f.org"),
            ]
        );
        assert_eq!(l[5].search.as_deref(), Some("12"));
        assert_eq!(l[0].desc.as_deref(), Some("repo"));
    }

    #[test]
    fn internal_links_open_and_mark_ring_returns() {
        let mut e = org("* A\n[[Target B]] [[#c]]\n* Target B\n* C\n:PROPERTIES:\n:CUSTOM_ID: c\n:END:", "j<C-c><C-o>");
        assert_eq!(e.cur.line, 2);
        for k in crate::key::parse_keys("<C-c>&") {
            e.handle_key(k);
        }
        assert_eq!(e.cur.line, 1);
        let e = org("* A\n[[Target B]] [[#c]]\n* Target B\n* C\n:PROPERTIES:\n:CUSTOM_ID: c\n:END:", "j$<C-c><C-o>");
        assert_eq!(e.cur.line, 3);
        let e = org("<<t1>>\n[[t1]]", "j<C-c><C-o>");
        assert_eq!(e.cur.line, 0);
    }

    #[test]
    fn store_and_insert() {
        let mut e = org("* Heading [1/2]\nx", "");
        e.path = Some("/tmp/a.org".into());
        let (link, desc) = store(&mut e, Prefix::None).unwrap();
        assert_eq!(link, "file:/tmp/a.org::*Heading");
        assert_eq!(desc.as_deref(), Some("Heading"));
        add_stored(&mut e, link, desc);
        for k in crate::key::parse_keys("j<C-c><C-l><Enter><Enter>") {
            e.handle_key(k);
        }
        assert_eq!(e.buf.line(1), "x[[*Heading][Heading]]");
    }
}
